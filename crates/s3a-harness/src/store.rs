use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use s3a_core::ADDON_EDU;
use s3a_engine::concurrent_writer::ConcurrentTileWriter;
use s3a_engine::lrs_store::uuid_to_u64_pair;
use s3a_engine::{MmapReader, QuerySieve};
use s3a_pack_scholar::records::{seed_default_agents, AgentRegistryRecord};
use s3a_pack_scholar::xapi::{
    enrich_scholar_statement, validate_ai_agent_allowlist, validate_scholar_profile, SCHOLAR_VERBS,
};
use s3a_pack_scholar::ScholarQueryExt;
use serde_json::Value;

#[derive(Debug)]
pub enum StoreError {
    Validation(String),
    ReadOnly,
    NotFound(String),
    Network(String),
    ServerError(u16, String),
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl std::error::Error for StoreError {}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Validation(msg) => write!(f, "Validation error: {}", msg),
            StoreError::ReadOnly => {
                write!(f, "Write rejected: store is configured in read-only mode")
            }
            StoreError::NotFound(msg) => write!(f, "Not found: {}", msg),
            StoreError::Network(msg) => write!(f, "Network error: {}", msg),
            StoreError::ServerError(code, msg) => {
                write!(f, "Server error (HTTP {}): {}", code, msg)
            }
            StoreError::Io(e) => write!(f, "I/O error: {}", e),
            StoreError::Json(e) => write!(f, "JSON serialization error: {}", e),
        }
    }
}

impl From<std::io::Error> for StoreError {
    fn from(e: std::io::Error) -> Self {
        StoreError::Io(e)
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(e: serde_json::Error) -> Self {
        StoreError::Json(e)
    }
}

/// Abstract Store interface backing the research agent harness learnmcp tools.
pub trait Store: Send + Sync {
    /// Validates and records an xAPI statement into persistent store.
    fn record_xapi_statement(&self, statement: &Value) -> Result<String, StoreError>;

    /// Retrieves contextual research activity and progress for a session.
    fn get_session_context(&self, session_id: &str) -> Result<Value, StoreError>;

    /// Lists all registered academic/research verbs supported by the profile.
    fn list_academic_verbs(&self) -> Result<Vec<Value>, StoreError>;

    /// Retrieves an ordered timeline of researcher decisions and AI reasoning steps.
    fn get_researcher_timeline(
        &self,
        agent_id: Option<&str>,
        session_id: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<Value>, StoreError>;
}

/// Local S3A file storage backend operating directly through s3a-engine.
pub struct LocalStore {
    db_path: PathBuf,
    read_only: bool,
    agent_registry: Vec<AgentRegistryRecord>,
}

impl LocalStore {
    pub fn new<P: AsRef<Path>>(db_path: P, read_only: bool) -> Self {
        Self {
            db_path: db_path.as_ref().to_path_buf(),
            read_only,
            agent_registry: seed_default_agents(),
        }
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }
}

impl Store for LocalStore {
    fn record_xapi_statement(&self, statement: &Value) -> Result<String, StoreError> {
        if self.read_only {
            return Err(StoreError::ReadOnly);
        }

        // 1. Validate compliance with research traceability / scholar profile
        validate_scholar_profile(statement).map_err(StoreError::Validation)?;

        // 2. Validate agent allowlist
        validate_ai_agent_allowlist(&self.agent_registry, statement)
            .map_err(|e| StoreError::Validation(format!("AI Agent allowlist denial: {:?}", e)))?;

        // 3. Extract or generate statement ID
        let mut final_stmt = statement.clone();
        let stmt_id = if let Some(id_str) = final_stmt.get("id").and_then(|v| v.as_str()) {
            id_str.to_string()
        } else {
            let gen_id = s3a_engine::lrs_store::generate_uuid_v4();
            final_stmt["id"] = Value::String(gen_id.clone());
            gen_id
        };

        // 4. Write into S3A Hyper-Tiles (Tile 10 Human LRS or Tile 11 AI Trace)
        let writer = if self.db_path.exists() {
            ConcurrentTileWriter::open_append(&self.db_path)?
        } else {
            ConcurrentTileWriter::create(&self.db_path)?
        };
        writer.enable_addon(ADDON_EDU)?;

        enrich_scholar_statement(&writer, &final_stmt, [0u8; 16])?;
        Ok(stmt_id)
    }

    fn get_session_context(&self, session_id: &str) -> Result<Value, StoreError> {
        let session_pair = uuid_to_u64_pair(session_id)
            .map_err(|e| StoreError::Validation(format!("Invalid UUID format: {}", e)))?;

        if !self.db_path.exists() {
            return Ok(serde_json::json!({
                "session_id": session_id,
                "human_decisions_count": 0,
                "ai_traces_count": 0,
                "total_events": 0,
                "events": []
            }));
        }

        let reader = MmapReader::open(&self.db_path)?;
        let sieve = QuerySieve::new(&reader);

        let humans = sieve.query_human_lrs(Some(session_pair), None, 0, u64::MAX);
        let ais = sieve.query_ai_traces(Some(session_pair), None, 0, u64::MAX);

        let mut events = Vec::new();
        for (h, coord) in &humans {
            events.push(serde_json::json!({
                "type": "human_decision",
                "timestamp": h.timestamp_sec,
                "actor_hash": format!("0x{:016X}", h.actor_hash),
                "verb_id": h.verb_id,
                "decision_score": h.decision_score,
                "coordinate": coord.to_string(),
                "linked_ai_coord": h.ai_trace_coord.to_string()
            }));
        }
        for (a, coord) in &ais {
            events.push(serde_json::json!({
                "type": "ai_trace",
                "timestamp": a.timestamp_sec,
                "agent_id": format!("0x{:016X}", a.agent_id),
                "step_type": a.step_type,
                "confidence": a.confidence,
                "coordinate": coord.to_string(),
                "linked_human_coord": a.human_coord.to_string()
            }));
        }

        events.sort_by_key(|e| e.get("timestamp").and_then(|v| v.as_u64()).unwrap_or(0));

        Ok(serde_json::json!({
            "session_id": session_id,
            "human_decisions_count": humans.len(),
            "ai_traces_count": ais.len(),
            "total_events": humans.len() + ais.len(),
            "events": events
        }))
    }

    fn list_academic_verbs(&self) -> Result<Vec<Value>, StoreError> {
        let verbs = SCHOLAR_VERBS
            .iter()
            .map(|v| {
                serde_json::json!({
                    "verb_id": v.verb_id,
                    "iri": v.iri,
                    "label": v.label,
                    "verb_bit": v.verb_bit
                })
            })
            .collect();
        Ok(verbs)
    }

    fn get_researcher_timeline(
        &self,
        agent_id: Option<&str>,
        session_id: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<Value>, StoreError> {
        if !self.db_path.exists() {
            return Ok(Vec::new());
        }

        let session_pair = match session_id {
            Some(s) if !s.trim().is_empty() => {
                Some(uuid_to_u64_pair(s).map_err(|e| StoreError::Validation(e.to_string()))?)
            }
            _ => None,
        };

        let agent_num = match agent_id {
            Some(a) if !a.trim().is_empty() => {
                let clean = a.trim_start_matches("0x").trim();
                u64::from_str_radix(clean, 16)
                    .ok()
                    .or_else(|| clean.parse().ok())
            }
            _ => None,
        };

        let reader = MmapReader::open(&self.db_path)?;
        let sieve = QuerySieve::new(&reader);

        let humans = sieve.query_human_lrs(session_pair, agent_num, 0, u64::MAX);
        let ais = sieve.query_ai_traces(session_pair, agent_num, 0, u64::MAX);

        let mut timeline = Vec::new();
        for (h, coord) in humans {
            timeline.push(serde_json::json!({
                "timestamp": h.timestamp_sec,
                "actor_type": "human",
                "id": format!("0x{:016X}", h.actor_hash),
                "verb_id": h.verb_id,
                "score": h.decision_score,
                "coord": coord.to_string(),
                "linked": h.ai_trace_coord.to_string(),
                "session_uuid": format!("{:016X}{:016X}", h.session_uuid[0], h.session_uuid[1])
            }));
        }
        for (a, coord) in ais {
            timeline.push(serde_json::json!({
                "timestamp": a.timestamp_sec,
                "actor_type": "ai_agent",
                "id": format!("0x{:016X}", a.agent_id),
                "step_type": a.step_type,
                "confidence": a.confidence,
                "coord": coord.to_string(),
                "linked": a.human_coord.to_string(),
                "session_uuid": format!("{:016X}{:016X}", a.session_uuid[0], a.session_uuid[1])
            }));
        }

        timeline.sort_by_key(|e| e.get("timestamp").and_then(|v| v.as_u64()).unwrap_or(0));
        let max = limit.unwrap_or(100);
        if timeline.len() > max {
            timeline.truncate(max);
        }

        Ok(timeline)
    }
}

/// Remote LRS HTTP backend operating over xAPI 1.0.3 protocol with Basic auth.
pub struct LrsStore {
    endpoint: String,
    credentials: Option<(String, String)>,
    read_only: bool,
    allow_insecure_http: bool,
    agent_registry: Vec<AgentRegistryRecord>,
    agent: ureq::Agent,
}

impl LrsStore {
    pub fn new(
        endpoint: &str,
        user: Option<String>,
        pass: Option<String>,
        read_only: bool,
    ) -> Self {
        Self::with_options(endpoint, user, pass, read_only, false)
    }

    pub fn with_options(
        endpoint: &str,
        user: Option<String>,
        pass: Option<String>,
        read_only: bool,
        allow_insecure_http: bool,
    ) -> Self {
        let creds = match (user, pass) {
            (Some(u), Some(p)) => Some((u, p)),
            _ => None,
        };
        let agent = ureq::builder()
            .timeout(Duration::from_secs(10))
            .build();
        Self {
            endpoint: endpoint.trim_end_matches('/').to_string(),
            credentials: creds,
            read_only,
            allow_insecure_http,
            agent_registry: seed_default_agents(),
            agent,
        }
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn allow_insecure_http(&self) -> bool {
        self.allow_insecure_http
    }

    pub fn make_request(
        &self,
        method: &str,
        path: &str,
        body: Option<&str>,
    ) -> Result<(u16, String), StoreError> {
        let full_url = if path.starts_with('/') {
            format!("{}{}", self.endpoint, path)
        } else {
            format!("{}/{}", self.endpoint, path)
        };

        // Protocol enforcement and downgrade prevention
        if full_url.starts_with("http://") {
            let host = extract_host_from_url(&full_url);
            let is_local = is_loopback_host(&host);
            if !is_local && !self.allow_insecure_http {
                return Err(StoreError::Network(format!(
                    "Insecure HTTP connection to non-localhost host '{}' refused. Use HTTPS or pass --allow-insecure-http",
                    host
                )));
            }
        } else if !full_url.starts_with("https://") {
            return Err(StoreError::Network(format!(
                "Unsupported scheme in endpoint '{}'. Must start with https:// or http://",
                full_url
            )));
        }

        let mut req = match method {
            "GET" => self.agent.get(&full_url),
            "POST" => self.agent.post(&full_url),
            "PUT" => self.agent.put(&full_url),
            _ => return Err(StoreError::Network(format!("Unsupported HTTP method: {}", method))),
        };

        req = req.set("X-Experience-API-Version", "1.0.3");

        if let Some((ref u, ref p)) = self.credentials {
            let cred_str = format!("{}:{}", u, p);
            let b64 = base64_encode(cred_str.as_bytes());
            req = req.set("Authorization", &format!("Basic {}", b64));
        }

        let res = if let Some(b) = body {
            req = req.set("Content-Type", "application/json");
            req.send_string(b)
        } else {
            req.call()
        };

        match res {
            Ok(response) => {
                let status = response.status();
                let text = response.into_string().map_err(StoreError::Io)?;
                Ok((status, text))
            }
            Err(ureq::Error::Status(code, response)) => {
                let text = response.into_string().unwrap_or_default();
                Ok((code, text))
            }
            Err(ureq::Error::Transport(transport)) => {
                Err(StoreError::Network(transport.to_string()))
            }
        }
    }
}

impl Store for LrsStore {
    fn record_xapi_statement(&self, statement: &Value) -> Result<String, StoreError> {
        if self.read_only {
            return Err(StoreError::ReadOnly);
        }

        // 1. Local profile validation
        validate_scholar_profile(statement).map_err(StoreError::Validation)?;

        // 2. Agent allowlist validation
        validate_ai_agent_allowlist(&self.agent_registry, statement)
            .map_err(|e| StoreError::Validation(format!("AI Agent allowlist denial: {:?}", e)))?;

        let mut final_stmt = statement.clone();
        let stmt_id = if let Some(id_str) = final_stmt.get("id").and_then(|v| v.as_str()) {
            id_str.to_string()
        } else {
            let gen_id = s3a_engine::lrs_store::generate_uuid_v4();
            final_stmt["id"] = Value::String(gen_id.clone());
            gen_id
        };

        // HTTP POST /xapi/statements with statement array
        let payload = serde_json::json!([final_stmt]).to_string();
        let (code, resp_body) = self.make_request("POST", "/xapi/statements", Some(&payload))?;

        if (200..300).contains(&code) {
            Ok(stmt_id)
        } else {
            Err(StoreError::ServerError(code, resp_body))
        }
    }

    fn get_session_context(&self, session_id: &str) -> Result<Value, StoreError> {
        let path = format!("/xapi/statements?registration={}", session_id);
        let (code, resp_body) = self.make_request("GET", &path, None)?;

        if code != 200 {
            return Err(StoreError::ServerError(code, resp_body));
        }

        let parsed: Value = serde_json::from_str(&resp_body)?;
        let statements = parsed
            .get("statements")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        Ok(serde_json::json!({
            "session_id": session_id,
            "statements_count": statements.len(),
            "total_events": statements.len(),
            "statements": statements
        }))
    }

    fn list_academic_verbs(&self) -> Result<Vec<Value>, StoreError> {
        let verbs = SCHOLAR_VERBS
            .iter()
            .map(|v| {
                serde_json::json!({
                    "verb_id": v.verb_id,
                    "iri": v.iri,
                    "label": v.label,
                    "verb_bit": v.verb_bit
                })
            })
            .collect();
        Ok(verbs)
    }

    fn get_researcher_timeline(
        &self,
        agent_id: Option<&str>,
        session_id: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<Value>, StoreError> {
        let mut query = Vec::new();
        if let Some(s) = session_id {
            if !s.is_empty() {
                query.push(format!("registration={}", s));
            }
        }
        if let Some(l) = limit {
            query.push(format!("limit={}", l));
        }
        let q_str = if query.is_empty() {
            String::new()
        } else {
            format!("?{}", query.join("&"))
        };
        let path = format!("/xapi/statements{}", q_str);

        let (code, resp_body) = self.make_request("GET", &path, None)?;
        if code != 200 {
            return Err(StoreError::ServerError(code, resp_body));
        }

        let parsed: Value = serde_json::from_str(&resp_body)?;
        let stmts = parsed
            .get("statements")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut events = Vec::new();
        for s in stmts {
            let id = s.get("id").and_then(|v| v.as_str()).unwrap_or_default();
            let timestamp = s
                .get("timestamp")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let verb_id = s
                .get("verb")
                .and_then(|v| v.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let actor_name = s
                .get("actor")
                .and_then(|a| a.get("name").or_else(|| a.get("mbox")))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            if let Some(filter_agent) = agent_id {
                if !actor_name.contains(filter_agent) {
                    continue;
                }
            }

            events.push(serde_json::json!({
                "statement_id": id,
                "timestamp": timestamp,
                "actor": actor_name,
                "verb": verb_id,
                "object": s.get("object")
            }));
        }

        Ok(events)
    }
}

// -----------------------------------------------------------------------------
// HTTP and Base64 Utilities (Zero-dependency)
// -----------------------------------------------------------------------------

pub fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0];
        let b1 = if chunk.len() > 1 { chunk[1] } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] } else { 0 };
        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(((b1 & 0x0F) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(b2 & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

pub fn extract_host_from_url(url_str: &str) -> String {
    let without_scheme = url_str
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let host_port = match without_scheme.find('/') {
        Some(idx) => &without_scheme[..idx],
        None => without_scheme,
    };
    match host_port.find(':') {
        Some(idx) => host_port[..idx].to_string(),
        None => host_port.to_string(),
    }
}

pub fn is_loopback_host(host: &str) -> bool {
    let h = host.trim().to_ascii_lowercase();
    h == "localhost"
        || h == "127.0.0.1"
        || h == "::1"
        || h == "[::1]"
        || h == "0.0.0.0"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_https_urls_not_downgraded() {
        let https_url = "https://scholar.example.org:8443/xapi";
        let store = LrsStore::new(https_url, None, None, false);
        assert_eq!(store.endpoint(), "https://scholar.example.org:8443/xapi");
        assert!(store.endpoint().starts_with("https://"));
        assert!(!store.endpoint().starts_with("http://"));
    }

    #[test]
    fn test_lrs_store_refuses_remote_http_without_flag() {
        let store = LrsStore::new("http://remote-lrs.institution.edu/xapi", None, None, false);
        let res = store.make_request("GET", "/about", None);
        match res {
            Err(StoreError::Network(msg)) => {
                assert!(msg.contains("Insecure HTTP connection to non-localhost host"));
                assert!(msg.contains("remote-lrs.institution.edu"));
            }
            other => panic!("Expected Network error for remote HTTP, got {:?}", other),
        }
    }

    #[test]
    fn test_lrs_store_allows_remote_http_with_flag() {
        let store = LrsStore::with_options("http://remote-lrs.institution.edu:9999/xapi", None, None, false, true);
        assert!(store.allow_insecure_http());
        // Will fail on transport/connection since port is not listening, but MUST NOT be rejected by security policy
        let res = store.make_request("GET", "/about", None);
        match res {
            Err(StoreError::Network(msg)) => {
                assert!(!msg.contains("Insecure HTTP connection to non-localhost host"));
            }
            _ => {}
        }
    }

    #[test]
    fn test_lrs_store_allows_localhost_http() {
        let store_local = LrsStore::new("http://localhost:8080/xapi", None, None, false);
        let store_loopback = LrsStore::new("http://127.0.0.1:8080/xapi", None, None, false);
        let res1 = store_local.make_request("GET", "/about", None);
        if let Err(StoreError::Network(msg)) = res1 {
            assert!(!msg.contains("Insecure HTTP connection to non-localhost host"));
        }
        let res2 = store_loopback.make_request("GET", "/about", None);
        if let Err(StoreError::Network(msg)) = res2 {
            assert!(!msg.contains("Insecure HTTP connection to non-localhost host"));
        }
    }
}
