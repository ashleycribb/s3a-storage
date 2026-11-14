//! Credential proxy and host allowlist for S3A isolated sandboxes.
//!
//! Sandboxes reference a connection name only. Credentials never enter the sandbox
//! environment, logs, or records. Every injected-credential request is logged as a
//! signed audit record (agent, connection, host, time), never the secret.

use std::collections::HashMap;
use std::fmt;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Proxy errors for egress control and credential injection.
#[derive(Debug)]
pub enum ProxyError {
    HostBlocked(String),
    UnknownConnection(String),
    SecretNotFound(String),
    Io(std::io::Error),
    HttpError(String),
    SerializationError(String),
    InvalidRequest(String),
}

impl fmt::Display for ProxyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HostBlocked(h) => write!(f, "Egress proxy rejected unallowed host: '{}'", h),
            Self::UnknownConnection(c) => write!(f, "Unknown named connection: '{}'", c),
            Self::SecretNotFound(s) => write!(f, "Secret not found for ref: '{}'", s),
            Self::Io(e) => write!(f, "Proxy I/O error: {}", e),
            Self::HttpError(e) => write!(f, "HTTP upstream error: {}", e),
            Self::SerializationError(e) => write!(f, "Proxy serialization error: {}", e),
            Self::InvalidRequest(e) => write!(f, "Invalid proxy request: {}", e),
        }
    }
}

impl std::error::Error for ProxyError {}

impl From<std::io::Error> for ProxyError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Host allowlist enforcing network boundary limits per environment.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct HostAllowlist {
    pub allowed_patterns: Vec<String>,
}

impl HostAllowlist {
    pub fn new(patterns: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            allowed_patterns: patterns
                .into_iter()
                .map(|p| p.into().trim().to_lowercase())
                .filter(|p| !p.is_empty())
                .collect(),
        }
    }

    /// Verifies whether a given host or URL is permitted by the allowlist rules.
    /// Supports exact matches, wildcard subdomain matches (e.g. `*.plm.corp`), and port stripping.
    pub fn is_allowed(&self, host_or_url: &str) -> bool {
        let host = extract_host_from_str(host_or_url);
        let host_lower = host.to_lowercase();

        for pattern in &self.allowed_patterns {
            let p = pattern.to_lowercase();
            if p == "*" {
                return true;
            }
            if p == host_lower {
                return true;
            }
            if let Some(suffix) = p.strip_prefix('*') {
                if host_lower.ends_with(suffix) {
                    return true;
                }
                if let Some(bare) = suffix.strip_prefix('.') {
                    if host_lower == bare {
                        return true;
                    }
                }
            }
        }
        false
    }
}

/// Extracts the canonical hostname from a host string, URL, or host:port pair.
pub fn extract_host_from_str(s: &str) -> String {
    let trimmed = s.trim();
    let without_scheme = if let Some(idx) = trimmed.find("://") {
        &trimmed[idx + 3..]
    } else {
        trimmed
    };
    let host_port = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(without_scheme);
    let host = if host_port.starts_with('[') {
        if let Some(end) = host_port.find(']') {
            &host_port[1..end]
        } else {
            host_port
        }
    } else if let Some(idx) = host_port.rfind(':') {
        &host_port[..idx]
    } else {
        host_port
    };
    host.trim().to_string()
}

/// Abstract secret store provider (Vault, OS Keystore, Memory).
pub trait SecretStore: Send + Sync {
    fn get_secret(&self, secret_ref: &str) -> Result<String, ProxyError>;
}

/// In-memory secret store for testing and ephemeral secrets.
#[derive(Default)]
pub struct MemorySecretStore {
    secrets: RwLock<HashMap<String, String>>,
}

impl MemorySecretStore {
    pub fn new() -> Self {
        Self {
            secrets: RwLock::new(HashMap::new()),
        }
    }

    pub fn insert(&self, key: impl Into<String>, secret: impl Into<String>) {
        self.secrets
            .write()
            .unwrap()
            .insert(key.into(), secret.into());
    }
}

impl SecretStore for MemorySecretStore {
    fn get_secret(&self, secret_ref: &str) -> Result<String, ProxyError> {
        self.secrets
            .read()
            .unwrap()
            .get(secret_ref)
            .cloned()
            .ok_or_else(|| ProxyError::SecretNotFound(secret_ref.to_string()))
    }
}

/// Vault-backed secret store wrapping `s3a_vault::keystore::KeyStore`.
pub struct VaultSecretStore {
    keystore: Arc<dyn s3a_vault::keystore::KeyStore>,
    named_seeds: RwLock<HashMap<String, [u8; 16]>>,
}

impl VaultSecretStore {
    pub fn new(keystore: Arc<dyn s3a_vault::keystore::KeyStore>) -> Self {
        Self {
            keystore,
            named_seeds: RwLock::new(HashMap::new()),
        }
    }

    pub fn register_identity(&self, name: impl Into<String>, identity_id: [u8; 16]) {
        self.named_seeds
            .write()
            .unwrap()
            .insert(name.into(), identity_id);
    }
}

impl SecretStore for VaultSecretStore {
    fn get_secret(&self, secret_ref: &str) -> Result<String, ProxyError> {
        let guard = self.named_seeds.read().unwrap();
        if let Some(id) = guard.get(secret_ref) {
            let pk = self
                .keystore
                .get_public_key(id)
                .map_err(|e| ProxyError::SecretNotFound(format!("Vault key error: {}", e)))?;
            Ok(hex::encode(pk))
        } else {
            Err(ProxyError::SecretNotFound(secret_ref.to_string()))
        }
    }
}

/// OS-backed secret store reading from system environment or OS secure store.
pub struct OsSecretStore {
    prefix: String,
}

impl OsSecretStore {
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
        }
    }
}

impl SecretStore for OsSecretStore {
    fn get_secret(&self, secret_ref: &str) -> Result<String, ProxyError> {
        let env_key = format!(
            "{}{}",
            self.prefix,
            secret_ref.replace(['-', '.'], "_").to_uppercase()
        );
        std::env::var(&env_key)
            .or_else(|_| std::env::var(secret_ref))
            .map_err(|_| ProxyError::SecretNotFound(secret_ref.to_string()))
    }
}

/// Chained secret store falling back across multiple stores in order.
pub struct ChainedSecretStore {
    stores: Vec<Arc<dyn SecretStore>>,
}

impl ChainedSecretStore {
    pub fn new(stores: Vec<Arc<dyn SecretStore>>) -> Self {
        Self { stores }
    }
}

impl SecretStore for ChainedSecretStore {
    fn get_secret(&self, secret_ref: &str) -> Result<String, ProxyError> {
        for s in &self.stores {
            if let Ok(sec) = s.get_secret(secret_ref) {
                return Ok(sec);
            }
        }
        Err(ProxyError::SecretNotFound(secret_ref.to_string()))
    }
}

/// Credential injection method for outbound requests.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AuthType {
    Bearer,
    Header {
        header_name: String,
        prefix: Option<String>,
    },
    Basic {
        username: String,
    },
    ApiKeyQuery {
        param_name: String,
    },
}

/// Configuration for a named connection available to sandboxes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamedConnection {
    pub name: String,
    pub target_host: String,
    pub auth_type: AuthType,
    pub secret_ref: String,
    #[serde(default)]
    pub upstream_url: Option<String>,
}

impl NamedConnection {
    pub fn new(
        name: impl Into<String>,
        target_host: impl Into<String>,
        auth_type: AuthType,
        secret_ref: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            target_host: target_host.into(),
            auth_type,
            secret_ref: secret_ref.into(),
            upstream_url: None,
        }
    }

    pub fn with_upstream_url(mut self, url: impl Into<String>) -> Self {
        self.upstream_url = Some(url.into());
        self
    }
}

/// Audit decision outcome.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ProxyDecision {
    Allowed,
    HostBlocked { host: String },
    UnknownConnection { connection: String },
    SecretNotFound { secret_ref: String },
    Failed { reason: String },
}

/// Signed audit record proving egress access without exposing the injected secret.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EgressAuditRecord {
    pub record_id: String,
    pub agent_id: String,
    pub connection: String,
    pub host: String,
    pub method: String,
    pub decision: ProxyDecision,
    pub timestamp: u64,
    #[serde(with = "hex_bytes_64")]
    pub signature: [u8; 64],
    #[serde(with = "hex_bytes_32")]
    pub verifying_key: [u8; 32],
}

mod hex_bytes_64 {
    use serde::{self, Deserialize, Deserializer, Serializer};
    pub fn serialize<S>(bytes: &[u8; 64], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex::encode(bytes))
    }
    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 64], D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let vec = hex::decode(&s).map_err(serde::de::Error::custom)?;
        if vec.len() != 64 {
            return Err(serde::de::Error::custom("expected 64 bytes"));
        }
        let mut arr = [0u8; 64];
        arr.copy_from_slice(&vec);
        Ok(arr)
    }
}

mod hex_bytes_32 {
    use serde::{self, Deserialize, Deserializer, Serializer};
    pub fn serialize<S>(bytes: &[u8; 32], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex::encode(bytes))
    }
    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 32], D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let vec = hex::decode(&s).map_err(serde::de::Error::custom)?;
        if vec.len() != 32 {
            return Err(serde::de::Error::custom("expected 32 bytes"));
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&vec);
        Ok(arr)
    }
}

impl EgressAuditRecord {
    pub fn canonical_message(
        agent_id: &str,
        connection: &str,
        host: &str,
        method: &str,
        decision: &ProxyDecision,
        timestamp: u64,
    ) -> Vec<u8> {
        let dec_str = match decision {
            ProxyDecision::Allowed => "ALLOWED",
            ProxyDecision::HostBlocked { .. } => "BLOCKED",
            ProxyDecision::UnknownConnection { .. } => "UNKNOWN_CONN",
            ProxyDecision::SecretNotFound { .. } => "NO_SECRET",
            ProxyDecision::Failed { .. } => "FAILED",
        };
        format!(
            "{}:{}:{}:{}:{}:{}",
            agent_id, connection, host, method, dec_str, timestamp
        )
        .into_bytes()
    }

    /// Cryptographic SHA-256 digest of this canonical signed record.
    pub fn sha256_digest(&self) -> String {
        use sha2::{Digest, Sha256};
        let msg = Self::canonical_message(
            &self.agent_id,
            &self.connection,
            &self.host,
            &self.method,
            &self.decision,
            self.timestamp,
        );
        let mut hasher = Sha256::new();
        hasher.update(&msg);
        hasher.update(self.signature);
        hex::encode(hasher.finalize())
    }

    /// Verifies cryptographic signature of the audit record using its verifying key.
    pub fn verify(&self) -> bool {
        let msg = Self::canonical_message(
            &self.agent_id,
            &self.connection,
            &self.host,
            &self.method,
            &self.decision,
            self.timestamp,
        );
        if let Ok(vk) = VerifyingKey::from_bytes(&self.verifying_key) {
            let sig = Signature::from_bytes(&self.signature);
            vk.verify(&msg, &sig).is_ok()
        } else {
            false
        }
    }
}

/// Inbound request from sandbox through egress proxy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EgressRequest {
    pub connection: String,
    #[serde(default)]
    pub path: String,
    #[serde(default = "default_method")]
    pub method: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub body: Option<String>,
}

fn default_method() -> String {
    "GET".to_string()
}

/// Outbound response delivered back to sandbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EgressResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: String,
}

/// Credential proxy managing connection dispatch, secret injection, and signed egress audits.
pub struct CredentialProxy {
    agent_id: String,
    allowlist: HostAllowlist,
    connections: RwLock<HashMap<String, NamedConnection>>,
    secret_store: Arc<dyn SecretStore>,
    audit_log: RwLock<Vec<EgressAuditRecord>>,
    signing_key: SigningKey,
    #[cfg(feature = "otel")]
    exporter: Option<Arc<dyn crate::otel::SpanExporter>>,
}

impl CredentialProxy {
    pub fn new(
        agent_id: impl Into<String>,
        allowlist: HostAllowlist,
        secret_store: Arc<dyn SecretStore>,
    ) -> Self {
        let mut csprng = OsRng;
        let signing_key = SigningKey::generate(&mut csprng);
        Self {
            agent_id: agent_id.into(),
            allowlist,
            connections: RwLock::new(HashMap::new()),
            secret_store,
            audit_log: RwLock::new(Vec::new()),
            signing_key,
            #[cfg(feature = "otel")]
            exporter: None,
        }
    }

    #[cfg(feature = "otel")]
    pub fn with_exporter(mut self, exporter: Arc<dyn crate::otel::SpanExporter>) -> Self {
        self.exporter = Some(exporter);
        self
    }

    pub fn with_signing_key(
        agent_id: impl Into<String>,
        allowlist: HostAllowlist,
        secret_store: Arc<dyn SecretStore>,
        signing_key: SigningKey,
    ) -> Self {
        Self {
            agent_id: agent_id.into(),
            allowlist,
            connections: RwLock::new(HashMap::new()),
            secret_store,
            audit_log: RwLock::new(Vec::new()),
            signing_key,
            #[cfg(feature = "otel")]
            exporter: None,
        }
    }

    pub fn register_connection(&self, conn: NamedConnection) {
        self.connections
            .write()
            .unwrap()
            .insert(conn.name.clone(), conn);
    }

    pub fn audit_records(&self) -> Vec<EgressAuditRecord> {
        self.audit_log.read().unwrap().clone()
    }

    fn record_audit(
        &self,
        connection: &str,
        host: &str,
        method: &str,
        decision: ProxyDecision,
    ) -> EgressAuditRecord {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let msg = EgressAuditRecord::canonical_message(
            &self.agent_id,
            connection,
            host,
            method,
            &decision,
            timestamp,
        );

        let signature: Signature = self.signing_key.sign(&msg);
        let verifying_key = self.signing_key.verifying_key().to_bytes();

        let record = EgressAuditRecord {
            record_id: crate::generate_uuid_v7().to_string(),
            agent_id: self.agent_id.clone(),
            connection: connection.to_string(),
            host: host.to_string(),
            method: method.to_string(),
            decision: decision.clone(),
            timestamp,
            signature: signature.to_bytes(),
            verifying_key,
        };

        self.audit_log.write().unwrap().push(record.clone());

        #[cfg(feature = "otel")]
        {
            let record_hash = record.sha256_digest();
            let span = crate::otel::Span::new(
                format!("proxy_request:{}", connection),
                &record.agent_id,
                &record.record_id,
                &record_hash,
            )
            .with_attribute("proxy.connection", connection)
            .with_attribute("proxy.host", host)
            .with_attribute("proxy.method", method)
            .with_attribute("proxy.decision", format!("{:?}", decision));
            if let Some(ref exp) = self.exporter {
                let _ = exp.export(&[span]);
            } else {
                crate::otel::emit_span(span);
            }
        }

        record
    }

    /// Primary execution endpoint: checks allowlist, fetches secret, injects credentials,
    /// emits signed audit log, and forwards request.
    pub fn handle_request(&self, req: &EgressRequest) -> Result<EgressResponse, ProxyError> {
        let conn = {
            let guard = self.connections.read().unwrap();
            guard.get(&req.connection).cloned()
        };

        let conn = match conn {
            Some(c) => c,
            None => {
                self.record_audit(
                    &req.connection,
                    "",
                    &req.method,
                    ProxyDecision::UnknownConnection {
                        connection: req.connection.clone(),
                    },
                );
                return Err(ProxyError::UnknownConnection(req.connection.clone()));
            }
        };

        let host = extract_host_from_str(&conn.target_host);

        // Enforce per-environment host allowlist
        if !self.allowlist.is_allowed(&host) {
            self.record_audit(
                &req.connection,
                &host,
                &req.method,
                ProxyDecision::HostBlocked { host: host.clone() },
            );
            return Err(ProxyError::HostBlocked(host));
        }

        // Fetch secret strictly from secret store
        let secret = match self.secret_store.get_secret(&conn.secret_ref) {
            Ok(s) => s,
            Err(e) => {
                self.record_audit(
                    &req.connection,
                    &host,
                    &req.method,
                    ProxyDecision::SecretNotFound {
                        secret_ref: conn.secret_ref.clone(),
                    },
                );
                return Err(e);
            }
        };

        // Record signed audit record. NEVER logs secret.
        self.record_audit(&req.connection, &host, &req.method, ProxyDecision::Allowed);

        // Prepare outbound request with injected credentials
        let mut outbound_headers = req.headers.clone();
        match &conn.auth_type {
            AuthType::Bearer => {
                outbound_headers.insert("Authorization".to_string(), format!("Bearer {}", secret));
            }
            AuthType::Header {
                header_name,
                prefix,
            } => {
                let val = if let Some(p) = prefix {
                    format!("{}{}", p, secret)
                } else {
                    secret.clone()
                };
                outbound_headers.insert(header_name.clone(), val);
            }
            AuthType::Basic { username } => {
                let user_pass = format!("{}:{}", username, secret);
                let encoded = hex::encode(user_pass.as_bytes());
                outbound_headers.insert("Authorization".to_string(), format!("Basic {}", encoded));
            }
            AuthType::ApiKeyQuery { param_name } => {
                outbound_headers.insert(
                    format!("X-Injected-Query-Param-{}", param_name),
                    secret.clone(),
                );
            }
        }

        // Forward to upstream service if configured, or return simulated response
        if let Some(upstream_base) = &conn.upstream_url {
            let target_url = format!(
                "{}/{}",
                upstream_base.trim_end_matches('/'),
                req.path.trim_start_matches('/')
            );
            let mut ureq_req = match req.method.to_uppercase().as_str() {
                "POST" => ureq::post(&target_url),
                "PUT" => ureq::put(&target_url),
                "DELETE" => ureq::delete(&target_url),
                _ => ureq::get(&target_url),
            };

            for (k, v) in &outbound_headers {
                ureq_req = ureq_req.set(k, v);
            }

            let resp = if let Some(b) = &req.body {
                ureq_req.send_string(b)
            } else {
                ureq_req.call()
            };

            match resp {
                Ok(r) => {
                    let status = r.status();
                    let mut resp_headers = HashMap::new();
                    for name in r.headers_names() {
                        if let Some(val) = r.header(&name) {
                            resp_headers.insert(name, val.to_string());
                        }
                    }
                    let body = r
                        .into_string()
                        .map_err(|e| ProxyError::HttpError(e.to_string()))?;
                    Ok(EgressResponse {
                        status,
                        headers: resp_headers,
                        body,
                    })
                }
                Err(ureq::Error::Status(code, r)) => {
                    let body = r.into_string().unwrap_or_default();
                    Ok(EgressResponse {
                        status: code,
                        headers: HashMap::new(),
                        body,
                    })
                }
                Err(e) => Err(ProxyError::HttpError(e.to_string())),
            }
        } else {
            // Simulated/mock connection response for test harnesses without external network
            Ok(EgressResponse {
                status: 200,
                headers: [(
                    "X-S3A-Proxied-By".to_string(),
                    "s3a-harness-egress".to_string(),
                )]
                .into_iter()
                .collect(),
                body: json!({
                    "success": true,
                    "connection": conn.name,
                    "host": host,
                    "path": req.path,
                    "message": "Proxy authenticated and authorized egress request",
                })
                .to_string(),
            })
        }
    }

    /// Spawns a lightweight loopback HTTP server on `127.0.0.1:0` allowing isolated
    /// sandbox subprocesses to route egress requests.
    pub fn start_loopback_server(self: Arc<Self>) -> Result<ProxyServerHandle, std::io::Error> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let shutdown = Arc::new(AtomicBool::new(false));

        let shutdown_clone = Arc::clone(&shutdown);
        let proxy_clone = Arc::clone(&self);

        let handle = thread::spawn(move || {
            for stream in listener.incoming() {
                if shutdown_clone.load(Ordering::SeqCst) {
                    break;
                }
                match stream {
                    Ok(mut socket) => {
                        let _ = socket.set_read_timeout(Some(Duration::from_secs(5)));
                        let _ = socket.set_write_timeout(Some(Duration::from_secs(5)));
                        handle_http_connection(&mut socket, &proxy_clone);
                    }
                    Err(_) => {
                        if shutdown_clone.load(Ordering::SeqCst) {
                            break;
                        }
                    }
                }
            }
        });

        Ok(ProxyServerHandle {
            port,
            shutdown,
            server_thread: Some(handle),
        })
    }
}

fn handle_http_connection(stream: &mut TcpStream, proxy: &CredentialProxy) {
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .unwrap_or_else(|_| stream.try_clone().unwrap()),
    );
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() || request_line.is_empty() {
        return;
    }

    let mut headers = HashMap::new();
    let mut content_length = 0usize;

    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some((k, v)) = trimmed.split_once(':') {
            let key = k.trim().to_lowercase();
            let val = v.trim().to_string();
            if key == "content-length" {
                content_length = val.parse().unwrap_or(0);
            }
            headers.insert(key, val);
        }
    }

    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        let _ = reader.read_exact(&mut body);
    }

    let req_parts: Vec<&str> = request_line.split_whitespace().collect();
    if req_parts.len() < 2 {
        return;
    }

    let _method = req_parts[0];
    let path = req_parts[1];

    if path == "/egress" || path == "/request" {
        if let Ok(req) = serde_json::from_slice::<EgressRequest>(&body) {
            match proxy.handle_request(&req) {
                Ok(resp) => {
                    let resp_body = json!({
                        "status": resp.status,
                        "headers": resp.headers,
                        "body": resp.body,
                    })
                    .to_string();

                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        resp_body.len(),
                        resp_body
                    );
                    let _ = stream.write_all(response.as_bytes());
                }
                Err(ProxyError::HostBlocked(host)) => {
                    let err_body = json!({
                        "error": "HostBlocked",
                        "host": host,
                        "message": format!("Host '{}' refused by proxy allowlist", host)
                    })
                    .to_string();

                    let response = format!(
                        "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        err_body.len(),
                        err_body
                    );
                    let _ = stream.write_all(response.as_bytes());
                }
                Err(ProxyError::UnknownConnection(conn)) => {
                    let err_body = json!({
                        "error": "UnknownConnection",
                        "connection": conn,
                    })
                    .to_string();

                    let response = format!(
                        "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        err_body.len(),
                        err_body
                    );
                    let _ = stream.write_all(response.as_bytes());
                }
                Err(e) => {
                    let err_body = json!({
                        "error": "ProxyError",
                        "details": e.to_string()
                    })
                    .to_string();

                    let response = format!(
                        "HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        err_body.len(),
                        err_body
                    );
                    let _ = stream.write_all(response.as_bytes());
                }
            }
        }
    } else {
        // Fallback or health endpoint
        let response = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK";
        let _ = stream.write_all(response.as_bytes());
    }
}

/// Control handle for background loopback proxy server.
pub struct ProxyServerHandle {
    port: u16,
    shutdown: Arc<AtomicBool>,
    server_thread: Option<thread::JoinHandle<()>>,
}

impl ProxyServerHandle {
    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn shutdown(mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(format!("127.0.0.1:{}", self.port));
        if let Some(handle) = self.server_thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for ProxyServerHandle {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(format!("127.0.0.1:{}", self.port));
        if let Some(handle) = self.server_thread.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_host_allowlist_matching() {
        let allowlist = HostAllowlist::new(vec![
            "api.internal.corp".to_string(),
            "*.plm.corp".to_string(),
            "localhost".to_string(),
        ]);

        assert!(allowlist.is_allowed("api.internal.corp"));
        assert!(allowlist.is_allowed("https://api.internal.corp:8443/v1/cad"));
        assert!(allowlist.is_allowed("viewer.plm.corp"));
        assert!(allowlist.is_allowed("sub.viewer.plm.corp"));
        assert!(allowlist.is_allowed("localhost:8080"));

        assert!(!allowlist.is_allowed("malicious.com"));
        assert!(!allowlist.is_allowed("https://evil.corp/steal"));
        assert!(!allowlist.is_allowed("otherplm.corp"));
    }

    #[test]
    fn test_signed_egress_audit_record_verification() {
        let mut csprng = OsRng;
        let signing_key = SigningKey::generate(&mut csprng);
        let timestamp = 1700000000u64;

        let msg = EgressAuditRecord::canonical_message(
            "agent_1",
            "plm_adapter",
            "plm.internal",
            "GET",
            &ProxyDecision::Allowed,
            timestamp,
        );

        let sig: Signature = signing_key.sign(&msg);
        let vk = signing_key.verifying_key().to_bytes();

        let record = EgressAuditRecord {
            record_id: "test-rec-1".to_string(),
            agent_id: "agent_1".to_string(),
            connection: "plm_adapter".to_string(),
            host: "plm.internal".to_string(),
            method: "GET".to_string(),
            decision: ProxyDecision::Allowed,
            timestamp,
            signature: sig.to_bytes(),
            verifying_key: vk,
        };

        assert!(record.verify());

        // Assert record serializes and contains NO secret
        let json_str = serde_json::to_string(&record).unwrap();
        assert!(!json_str.contains("secret"));
        assert!(!json_str.contains("bearer"));
    }
}
