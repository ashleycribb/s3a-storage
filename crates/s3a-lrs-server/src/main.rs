//! S3A xAPI 1.0.3 LRS Server.
//!
//! Fully conformant Learning Record Store (LRS) implementing the xAPI 1.0.3 REST API
//! directly on top of native S3A binary archives (zero SQL / DuckDB / external DB dependencies).

use std::collections::HashMap;
use std::io::Cursor;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use sha2::{Digest, Sha256};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

use s3a_core::*;
use s3a_engine::{
    format_iso8601_ms, u64_pair_to_uuid, uuid_to_u64_pair, LrsStore, LrsStoreError,
    StatementFilter,
};
use s3a_engine::concurrent_writer::ConcurrentTileWriter;

pub mod learner_api;

/// Represents an authorized credential with granular xAPI scopes and optional archive mapping.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct AuthCredential {
    pub user: String,
    pub pass: String,
    pub scopes: Vec<String>,
    #[serde(default)]
    pub archive: Option<String>,
}

impl AuthCredential {
    pub fn has_scope(&self, req_scope: &str) -> bool {
        self.scopes.iter().any(|s| s == "all" || s == req_scope)
    }
}

/// Constant-time slice comparison to prevent timing attacks.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b.iter()).fold(0u8, |acc, (&x, &y)| acc | (x ^ y)) == 0
}

/// Writes an audit entry to the dedicated audit archive.
pub fn log_audit(
    writer: &ConcurrentTileWriter,
    who: &str,
    method: &str,
    endpoint: &str,
    ids: &str,
    status_code: u16,
) {
    let now = s3a_engine::lrs_store::now_ms();
    let audit_rec = serde_json::json!({
        "id": s3a_engine::lrs_store::generate_uuid_v4(),
        "timestamp": format_iso8601_ms(now),
        "who": who,
        "method": method,
        "endpoint": endpoint,
        "ids": ids,
        "status": status_code,
    });
    let bytes = serde_json::to_vec(&audit_rec).unwrap_or_default();
    let mut hdr = HyperTileHeader::new(0, TILE_STATEMENT_BODY);
    hdr.record_count = 1;
    hdr.payload_bytes = bytes.len() as u32;
    hdr.data_crc32 = crc32c::crc32c(&bytes);
    let _ = writer.write_raw_hyper_tile(&hdr, &bytes);
}

/// Executes the daily snapshot sealing, anchoring, and xAPI audit anchoring statement creation.
pub fn run_daily_anchor_job(
    store: &LrsStore,
    archive_path: &std::path::Path,
    audit_archive_path: &std::path::Path,
    calendar_urls: Option<&[String]>,
    tsa_url: Option<&str>,
) -> std::io::Result<(PathBuf, PathBuf, String)> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let date_str = s3a_engine::format_iso8601_ms(now * 1000)[..10].to_string();

    let parent_dir = archive_path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let snap_path = parent_dir.join(format!("lrs_snapshot_{}.snapshot.s3a", date_str));
    let audit_snap_path = parent_dir.join(format!("lrs_audit_{}.snapshot.s3a", date_str));

    // 1. Seal primary archive into snapshot
    let in_bytes = store.read_all_bytes()?;
    let sealed_bytes = s3a_engine::seal_buffer(&in_bytes, 16, 2)?;
    std::fs::write(&snap_path, &sealed_bytes)?;

    // 2. Seal audit archive into snapshot
    if audit_archive_path.exists()
        && std::fs::metadata(audit_archive_path).map(|m| m.len()).unwrap_or(0) >= FILE_HEADER_SIZE as u64
    {
        if let Ok(audit_bytes) = std::fs::read(audit_archive_path) {
            if let Ok(sealed_audit) = s3a_engine::seal_buffer(&audit_bytes, 16, 2) {
                let _ = std::fs::write(&audit_snap_path, sealed_audit);
            }
        }
    }
    if !audit_snap_path.exists() {
        std::fs::write(&audit_snap_path, &sealed_bytes)?;
    }

    // 3. Compute fingerprints
    let fp = s3a_engine::compute_sealed_file_fingerprint(&snap_path)?;
    let fp_hex = hex::encode(fp);
    let audit_fp = s3a_engine::compute_sealed_file_fingerprint(&audit_snap_path).unwrap_or([0u8; 32]);
    let audit_fp_hex = hex::encode(audit_fp);

    // 4. Anchor snapshot
    let calendars: Vec<String> = if let Some(c) = calendar_urls {
        c.to_vec()
    } else {
        s3a_engine::DEFAULT_OTS_CALENDARS.iter().map(|&s| s.to_string()).collect()
    };

    if let Some(tsa) = tsa_url {
        if let Ok(proof) = s3a_engine::submit_rfc3161(fp, tsa) {
            let _ = s3a_engine::attach_anchor_to_file(&snap_path, proof);
        }
    } else if let Ok(proofs) = s3a_engine::submit_ots(fp, &calendars) {
        for proof in proofs {
            let _ = s3a_engine::attach_anchor_to_file(&snap_path, proof);
        }
    }

    // 5. Append xAPI "anchored" statement
    let statement_id = s3a_engine::lrs_store::generate_uuid_v4();
    let stmt = serde_json::json!({
        "id": statement_id,
        "timestamp": s3a_engine::format_iso8601_ms(now * 1000),
        "actor": {
            "objectType": "Agent",
            "name": "S3A Daily Anchoring Service",
            "mbox": "mailto:trust@s3a.local"
        },
        "verb": {
            "id": "https://ashecribb.dev/xapi/verbs/anchored",
            "display": { "en-US": "anchored" }
        },
        "object": {
            "id": format!("urn:s3a:snapshot:{}", date_str),
            "definition": {
                "name": { "en-US": format!("Daily Snapshot {}", date_str) },
                "type": "https://ashecribb.dev/xapi/activities/snapshot",
                "extensions": {
                    "https://ashecribb.dev/xapi/ext/fingerprint": fp_hex,
                    "https://ashecribb.dev/xapi/ext/audit-fingerprint": audit_fp_hex,
                    "https://ashecribb.dev/xapi/ext/snapshot-path": snap_path.to_string_lossy().to_string(),
                    "https://ashecribb.dev/xapi/ext/method": if tsa_url.is_some() { "rfc3161" } else { "opentimestamps" }
                }
            }
        }
    });

    let stmt_str = serde_json::to_string(&stmt).unwrap_or_default();
    let _ = store.store_statement(&stmt_str, &[]);

    Ok((snap_path, audit_snap_path, fp_hex))
}

/// Configuration for the S3A LRS Server.
#[derive(Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub archive_path: PathBuf,
    pub audit_archive_path: PathBuf,
    pub credentials: Vec<AuthCredential>,
    pub insecure_dev: bool,
    pub enable_scholar: bool,
    pub profile_mode_strict: bool,
    pub admin_api_override: bool,
    pub jwt_secret: String,
    pub enable_daily_anchoring: bool,
    pub anchoring_interval_secs: u64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8080,
            archive_path: PathBuf::from("lrs_data.s3a"),
            audit_archive_path: PathBuf::from("lrs_data.audit.s3a"),
            credentials: Vec::new(),
            insecure_dev: false,
            enable_scholar: true,
            profile_mode_strict: false,
            admin_api_override: false,
            jwt_secret: "s3a-elrr-jwt-secret-key-32bytes!!".to_string(),
            enable_daily_anchoring: false,
            anchoring_interval_secs: 86400,
        }
    }
}

/// Shared application state with multi-archive routing.
#[derive(Clone)]
pub struct AppState {
    pub default_store: Arc<LrsStore>,
    pub store: Arc<LrsStore>,
    pub stores: Arc<std::sync::RwLock<HashMap<String, Arc<LrsStore>>>>,
    pub audit_writer: Arc<ConcurrentTileWriter>,
    pub config: ServerConfig,
    pub agent_registry: Vec<s3a_pack_scholar::records::AgentRegistryRecord>,
}

impl AppState {
    /// Opens or returns a cached LrsStore for a specified archive file or name.
    pub fn get_or_open_store(&self, archive_str: &str) -> Result<Arc<LrsStore>, s3a_engine::LrsStoreError> {
        let p = PathBuf::from(archive_str);
        let key = p.file_name().and_then(|s| s.to_str()).unwrap_or(archive_str).to_string();

        {
            let map = self.stores.read().unwrap();
            if let Some(s) = map.get(&key) {
                return Ok(s.clone());
            }
        }

        let mut map = self.stores.write().unwrap();
        if let Some(s) = map.get(&key) {
            return Ok(s.clone());
        }

        let store = LrsStore::open_or_create(&p)?;
        if self.config.enable_scholar {
            let _ = store.writer().enable_addon(s3a_core::ADDON_EDU);
        }
        map.insert(key, store.clone());
        Ok(store)
    }

    /// Resolves active LrsStore and canonical archive name based on credentials, query params, or defaults.
    pub fn resolve_store_and_archive(
        &self,
        cred: Option<&AuthCredential>,
        params: &HashMap<String, String>,
        headers: &HashMap<String, String>,
    ) -> (Arc<LrsStore>, String) {
        // 1. Check credential mapping
        if let Some(c) = cred {
            if let Some(ref arch) = c.archive {
                if let Ok(st) = self.get_or_open_store(arch) {
                    return (st, arch.clone());
                }
            }
        }

        // 2. Check query param or header override
        if let Some(arch) = params.get("archive").or_else(|| headers.get("x-s3a-archive")).or_else(|| headers.get("x-archive")) {
            if let Ok(st) = self.get_or_open_store(arch) {
                return (st, arch.clone());
            }
        }

        // 3. Fallback to default configured archive
        let default_name = self.config.archive_path.file_name().and_then(|s| s.to_str()).unwrap_or("lrs_data.s3a").to_string();
        (self.default_store.clone(), default_name)
    }
}

fn main() {
    let mut config = ServerConfig::default();
    let args: Vec<String> = std::env::args().collect();
    let mut cli_user = None;
    let mut cli_pass = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                if i + 1 < args.len() {
                    config.port = args[i + 1].parse().unwrap_or(8080);
                    i += 1;
                }
            }
            "-h" | "--host" => {
                if i + 1 < args.len() {
                    config.host = args[i + 1].clone();
                    i += 1;
                }
            }
            "-a" | "--archive" => {
                if i + 1 < args.len() {
                    config.archive_path = PathBuf::from(&args[i + 1]);
                    i += 1;
                }
            }
            "--audit-archive" => {
                if i + 1 < args.len() {
                    config.audit_archive_path = PathBuf::from(&args[i + 1]);
                    i += 1;
                }
            }
            "-u" | "--auth-user" | "--user" => {
                if i + 1 < args.len() {
                    cli_user = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "-p" | "--auth-pass" | "--password" => {
                if i + 1 < args.len() {
                    cli_pass = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "--auth-config" => {
                if i + 1 < args.len() {
                    let path = PathBuf::from(&args[i + 1]);
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        if let Ok(creds) = serde_json::from_str::<Vec<AuthCredential>>(&content) {
                            config.credentials.extend(creds);
                        } else {
                            for line in content.lines() {
                                let line = line.trim();
                                if line.is_empty() || line.starts_with('#') { continue; }
                                let parts: Vec<&str> = line.split(':').collect();
                                if parts.len() >= 2 {
                                    let scopes = if parts.len() >= 3 {
                                        parts[2].split(',').map(|s| s.trim().to_string()).collect()
                                    } else {
                                        vec!["all".to_string()]
                                    };
                                    let archive = if parts.len() >= 4 && !parts[3].trim().is_empty() {
                                        Some(parts[3].trim().to_string())
                                    } else {
                                        None
                                    };
                                    config.credentials.push(AuthCredential {
                                        user: parts[0].to_string(),
                                        pass: parts[1].to_string(),
                                        scopes,
                                        archive,
                                    });
                                }
                            }
                        }
                    }
                    i += 1;
                }
            }
            "--insecure-dev" => {
                config.insecure_dev = true;
            }
            "--strict-profiles" => {
                config.profile_mode_strict = true;
            }
            "--no-scholar" => {
                config.enable_scholar = false;
            }
            "--admin-api-override" => {
                config.admin_api_override = true;
            }
            "--auth-cred" => {
                if i + 1 < args.len() {
                    let parts: Vec<&str> = args[i + 1].split(':').collect();
                    if parts.len() >= 2 {
                        let scopes = if parts.len() >= 3 {
                            parts[2].split(',').map(|s| s.trim().to_string()).collect()
                        } else {
                            vec!["all".to_string()]
                        };
                        let archive = if parts.len() >= 4 && !parts[3].trim().is_empty() {
                            Some(parts[3].trim().to_string())
                        } else {
                            None
                        };
                        config.credentials.push(AuthCredential {
                            user: parts[0].to_string(),
                            pass: parts[1].to_string(),
                            scopes,
                            archive,
                        });
                    }
                    i += 1;
                }
            }
            "--jwt-secret" => {
                if i + 1 < args.len() {
                    config.jwt_secret = args[i + 1].clone();
                    i += 1;
                }
            }
            "--daily-anchor" => {
                config.enable_daily_anchoring = true;
            }
            "--anchoring-interval" => {
                if i + 1 < args.len() {
                    config.anchoring_interval_secs = args[i + 1].parse().unwrap_or(86400);
                    config.enable_daily_anchoring = true;
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    if let (Some(u), Some(p)) = (cli_user, cli_pass) {
        config.credentials.push(AuthCredential {
            user: u,
            pass: p,
            scopes: vec!["all".to_string()],
            archive: None,
        });
    }

    if config.insecure_dev {
        if !config.credentials.iter().any(|c| c.user == "researcher") {
            config.credentials.push(AuthCredential {
                user: "researcher".to_string(),
                pass: "researcher123".to_string(),
                scopes: vec!["all".to_string()],
                archive: Some("scholar.s3a".to_string()),
            });
        }
        if !config.credentials.iter().any(|c| c.user == "explorer") {
            config.credentials.push(AuthCredential {
                user: "explorer".to_string(),
                pass: "explorer123".to_string(),
                scopes: vec!["statements/write".to_string()],
                archive: Some("usage.s3a".to_string()),
            });
        }
    }

    // Default audit archive path if not explicitly configured
    if config.audit_archive_path == PathBuf::from("lrs_data.audit.s3a") && config.archive_path != PathBuf::from("lrs_data.s3a") {
        let mut p = config.archive_path.clone();
        let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("lrs");
        p.set_file_name(format!("{}.audit.s3a", stem));
        config.audit_archive_path = p;
    }

    // Auth fails closed: server refuses to start without credentials unless --insecure-dev is passed
    if !config.insecure_dev && config.credentials.is_empty() {
        eprintln!("[s3a-lrs-server] ERROR: Server refused to start: no credentials configured. Pass credentials via --auth-user / --auth-pass or --auth-config, or pass --insecure-dev.");
        std::process::exit(1);
    }

    println!("[s3a-lrs-server] Opening S3A archive: {:?}", config.archive_path);
    let store = LrsStore::open_or_create(&config.archive_path).expect("Failed to open S3A LRS archive");

    println!("[s3a-lrs-server] Opening audit archive: {:?}", config.audit_archive_path);
    let audit_writer = if config.audit_archive_path.exists()
        && std::fs::metadata(&config.audit_archive_path).map(|m| m.len()).unwrap_or(0) >= FILE_HEADER_SIZE as u64
    {
        ConcurrentTileWriter::open_append(&config.audit_archive_path)
            .expect("Failed to open audit archive")
    } else {
        ConcurrentTileWriter::create(&config.audit_archive_path)
            .expect("Failed to create audit archive")
    };

    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .expect("Invalid host/port configuration");
    let server = Server::http(addr).expect("Failed to bind HTTP server");
    println!("[s3a-lrs-server] Listening on http://{}", addr);

    if config.enable_scholar {
        let _ = store.writer().enable_addon(s3a_core::ADDON_EDU);
    }

    let agent_registry = s3a_pack_scholar::records::seed_default_agents();

    let stores = Arc::new(std::sync::RwLock::new(HashMap::new()));
    let default_key = config.archive_path.file_name().and_then(|s| s.to_str()).unwrap_or("lrs_data.s3a").to_string();
    stores.write().unwrap().insert(default_key, store.clone());

    let state = Arc::new(AppState {
        default_store: store.clone(),
        store,
        stores,
        audit_writer: Arc::new(audit_writer),
        config,
        agent_registry,
    });

    if state.config.enable_daily_anchoring {
        let store_clone = state.default_store.clone();
        let archive_path = state.config.archive_path.clone();
        let audit_path = state.config.audit_archive_path.clone();
        let interval_secs = state.config.anchoring_interval_secs;
        thread::spawn(move || {
            loop {
                thread::sleep(std::time::Duration::from_secs(interval_secs));
                let _ = run_daily_anchor_job(&store_clone, &archive_path, &audit_path, None, None);
            }
        });
    }

    for request in server.incoming_requests() {
        let state_clone = state.clone();
        thread::spawn(move || {
            handle_request(request, state_clone);
        });
    }
}

/// Main HTTP request handler conforming to xAPI 1.0.3.
fn handle_request(mut req: Request, state: Arc<AppState>) {
    let raw_url = req.url().to_string();
    let (mut path, query_str) = parse_url(&raw_url);

    // Normalize path: collapse any double slashes and strip leading /xapi
    while path.contains("//") {
        path = path.replace("//", "/");
    }
    if path.starts_with("/xapi/") {
        path = path[5..].to_string();
    } else if path == "/xapi" {
        path = "/".to_string();
    } else if path.starts_with("/XAPI/") {
        path = path[5..].to_string();
    } else if path == "/XAPI" {
        path = "/".to_string();
    }
    while path.contains("//") {
        path = path.replace("//", "/");
    }

    // Read full request body
    let mut body_bytes = Vec::new();
    let _ = req.as_reader().read_to_end(&mut body_bytes);

    let mut method = req.method().clone();
    let mut headers_map: HashMap<String, String> = HashMap::new();
    for h in req.headers() {
        headers_map.insert(h.field.to_string().to_ascii_lowercase(), h.value.to_string());
    }

    let mut query_params = parse_query(&query_str);

    // -------------------------------------------------------------------------
    // CORS Preflight (OPTIONS)
    // -------------------------------------------------------------------------
    if method == Method::Options {
        let resp = build_options_response(&headers_map);
        let _ = req.respond(resp);
        return;
    }

    // -------------------------------------------------------------------------
    // Root status discovery (Unauthenticated GET /)
    // -------------------------------------------------------------------------
    if path == "/" || path.is_empty() {
        if method == Method::Get || method == Method::Head {
            let info_json = serde_json::json!({
                "name": "S3A Native LRS Server",
                "version": ["1.0.3"],
                "endpoints": {
                    "about": "/xapi/about",
                    "statements": "/xapi/statements",
                    "activities": "/xapi/activities",
                    "state": "/xapi/activities/state",
                    "learner_person": "/api/person",
                    "learner_events": "/api/learning-event",
                    "learner_goals": "/api/goal"
                },
                "status": "ready"
            }).to_string();
            let resp = make_response(200, "application/json", info_json.as_bytes(), method == Method::Head);
            let _ = req.respond(resp);
            return;
        }
    }

    // -------------------------------------------------------------------------
    // Alternate Request Syntax (xAPI 1.0.3 §4.2)
    // -------------------------------------------------------------------------
    if req.method() != &Method::Post && query_params.contains_key("method") {
        let resp = make_error_response(400, "Method query parameter is only allowed on POST requests (xAPI 1.0.3 §4.2)");
        log_audit(&state.audit_writer, "unknown", req.method().as_str(), &path, "", 400);
        let _ = req.respond(resp);
        return;
    }

    if method == Method::Post {
        if query_params.contains_key("method") {
            // Check if there are any other query parameters besides "method"
            if query_params.len() > 1 {
                let resp = make_error_response(400, "Alternate request syntax MUST NOT submit any parameter as a query string parameter other than 'method' (xAPI 1.0.3 §4.2)");
                log_audit(&state.audit_writer, "unknown", req.method().as_str(), &path, "", 400);
                let _ = req.respond(resp);
                return;
            }
            let target_method = query_params.remove("method").unwrap();
            method = match target_method.to_uppercase().as_str() {
                "GET" => Method::Get,
                "PUT" => Method::Put,
                "DELETE" => Method::Delete,
                "HEAD" => Method::Head,
                _ => Method::Post,
            };
        }

        // If body is form-urlencoded, extract parameters and content
        let content_type = headers_map.get("content-type").cloned().unwrap_or_default();
        if content_type.starts_with("application/x-www-form-urlencoded") {
            let body_str = String::from_utf8_lossy(&body_bytes);
            let form_params = parse_query(&body_str);
            let mut has_content = false;
            for (k, v) in form_params {
                let k_lower = k.to_ascii_lowercase();
                if k_lower == "method" {
                    method = match v.to_uppercase().as_str() {
                        "GET" => Method::Get,
                        "PUT" => Method::Put,
                        "DELETE" => Method::Delete,
                        "HEAD" => Method::Head,
                        _ => method,
                    };
                } else if k_lower == "content" {
                    body_bytes = v.into_bytes();
                    has_content = true;
                } else if k_lower == "authorization"
                    || k_lower == "x-experience-api-version"
                    || k_lower == "content-type"
                    || k_lower == "content-length"
                    || k_lower == "if-match"
                    || k_lower == "if-none-match"
                {
                    headers_map.insert(k_lower, v);
                } else {
                    query_params.insert(k, v);
                }
            }

            if (method == Method::Put || method == Method::Post) && !has_content && body_bytes.is_empty() {
                let resp = make_error_response(400, "Missing 'content' parameter in form-urlencoded body");
                log_audit(&state.audit_writer, "unknown", method.as_str(), &path, "", 400);
                let _ = req.respond(resp);
                return;
            }
        }
    }

    // -------------------------------------------------------------------------
    // /about endpoint (Unauthenticated, no X-Experience-API-Version check needed)
    // -------------------------------------------------------------------------
    if path == "/about" || path == "/about/" {
        if method == Method::Get || method == Method::Head {
            let about_json = r#"{"version":["1.0.3"]}"#;
            let resp = make_response(200, "application/json", about_json.as_bytes(), method == Method::Head);
            log_audit(&state.audit_writer, "public", method.as_str(), &path, "", 200);
            let _ = req.respond(resp);
            return;
        } else {
            let resp = make_error_response(405, "Method Not Allowed");
            log_audit(&state.audit_writer, "public", method.as_str(), &path, "", 405);
            let _ = req.respond(resp);
            return;
        }
    }

    // -------------------------------------------------------------------------
    // LEARNER API (IEEE P2997 Subset / ADL ELRR REST Service)
    // -------------------------------------------------------------------------
    if path.starts_with("/api/") || path == "/api" || path.starts_with("/admin/") || path == "/admin" {
        let resp = learner_api::handle_learner_api(&method, &path, &query_params, &headers_map, &body_bytes, &state);
        let _ = req.respond(resp);
        return;
    }

    // -------------------------------------------------------------------------
    // Authentication & Scope Check (fails closed unless --insecure-dev)
    // -------------------------------------------------------------------------
    let mut authenticated_user = "insecure-dev".to_string();
    let mut matched_cred: Option<&AuthCredential> = None;

    let auth_hdr = headers_map.get("authorization");
    if let Some(val) = auth_hdr {
        if val.starts_with("Basic ") {
            let encoded = val[6..].trim();
            if let Ok(decoded) = base64_decode(encoded) {
                if let Ok(user_pass) = String::from_utf8(decoded) {
                    if let Some((user, pass)) = user_pass.split_once(':') {
                        for cred in &state.config.credentials {
                            if constant_time_eq(user.as_bytes(), cred.user.as_bytes())
                                && constant_time_eq(pass.as_bytes(), cred.pass.as_bytes())
                            {
                                matched_cred = Some(cred);
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    if !state.config.insecure_dev {
        let cred = match matched_cred {
            Some(c) => c,
            None => {
                let mut resp = make_error_response(401, "Unauthorized");
                resp.add_header(Header::from_bytes(&b"WWW-Authenticate"[..], &b"Basic realm=\"S3A-LRS\""[..]).unwrap());
                log_audit(&state.audit_writer, "anonymous", method.as_str(), &path, "", 401);
                let _ = req.respond(resp);
                return;
            }
        };

        authenticated_user = cred.user.clone();

        // Enforce required scope per endpoint
        let required_scope = if path.starts_with("/statements") {
            if method == Method::Get || method == Method::Head {
                "statements/read"
            } else {
                "statements/write"
            }
        } else if path.starts_with("/activities/state") {
            "state"
        } else if path.starts_with("/activities/profile") || path.starts_with("/agents/profile") {
            "profile"
        } else {
            "statements/read"
        };

        let has_scope = cred.has_scope(required_scope)
            || (required_scope == "statements/read" && cred.has_scope("statements/read/mine"));

        if !has_scope {
            let resp = make_error_response(403, "Forbidden: credential lacks required scope");
            log_audit(&state.audit_writer, &authenticated_user, method.as_str(), &path, "", 403);
            let _ = req.respond(resp);
            return;
        }
    } else if let Some(cred) = matched_cred {
        authenticated_user = cred.user.clone();
        // In insecure-dev mode with explicit credentials provided, enforce the credential's scopes
        let required_scope = if path.starts_with("/statements") {
            if method == Method::Get || method == Method::Head {
                "statements/read"
            } else {
                "statements/write"
            }
        } else if path.starts_with("/activities/state") {
            "state"
        } else if path.starts_with("/activities/profile") || path.starts_with("/agents/profile") {
            "profile"
        } else {
            "statements/read"
        };

        let has_scope = cred.has_scope(required_scope)
            || (required_scope == "statements/read" && cred.has_scope("statements/read/mine"));

        if !has_scope {
            let resp = make_error_response(403, "Forbidden: credential lacks required scope");
            log_audit(&state.audit_writer, &authenticated_user, method.as_str(), &path, "", 403);
            let _ = req.respond(resp);
            return;
        }
    }

    // -------------------------------------------------------------------------
    // Resolve Active Store and Archive (Multi-Archive Support)
    // -------------------------------------------------------------------------
    let (active_store, active_archive) = state.resolve_store_and_archive(matched_cred, &query_params, &headers_map);

    // -------------------------------------------------------------------------
    // X-Experience-API-Version Header Validation (xAPI 1.0.3 §4.1.1)
    // -------------------------------------------------------------------------
    let xapi_version = headers_map.get("x-experience-api-version")
        .or_else(|| query_params.get("x-experience-api-version"))
        .or_else(|| query_params.get("version"));
    match xapi_version {
        Some(ver) if ver.starts_with("1.0") => {
            // Valid xAPI version
        }
        None if state.config.insecure_dev => {
            // Permitted in insecure-dev mode (e.g. Scholar Explorer UI, direct browser inspection)
        }
        _ => {
            let resp = make_error_response(400, "Missing or invalid X-Experience-API-Version header (must be 1.0.x)");
            log_audit(&state.audit_writer, &authenticated_user, method.as_str(), &path, "", 400);
            let _ = req.respond(resp);
            return;
        }
    }

    // Dispatch routed resources
    let response = if path == "/statements" || path == "/statements/" {
        handle_statements(method.clone(), &query_params, &headers_map, &body_bytes, &state, &active_store, &active_archive)
    } else if path == "/activities/state" || path == "/activities/state/" {
        handle_activities_state(method.clone(), &query_params, &headers_map, &body_bytes, &state, &active_store)
    } else if path == "/activities/profile" || path == "/activities/profile/" {
        handle_activities_profile(method.clone(), &query_params, &headers_map, &body_bytes, &state, &active_store)
    } else if path == "/agents/profile" || path == "/agents/profile/" {
        handle_agents_profile(method.clone(), &query_params, &headers_map, &body_bytes, &state, &active_store)
    } else if path == "/activities" || path == "/activities/" {
        handle_activities(method.clone(), &query_params, &headers_map, &active_store)
    } else if path == "/agents" || path == "/agents/" {
        handle_agents(method.clone(), &query_params, &headers_map, &state)
    } else if path.starts_with("/registrations/") && path.ends_with("/export") {
        handle_registration_export(method.clone(), &path, &active_store)
    } else {
        make_error_response(404, "Not Found")
    };

    let status_code = response.status_code().0;
    let extracted_ids = query_params.get("statementId")
        .or_else(|| query_params.get("voidedStatementId"))
        .or_else(|| query_params.get("stateId"))
        .or_else(|| query_params.get("profileId"))
        .or_else(|| query_params.get("activityId"))
        .cloned()
        .unwrap_or_default();

    log_audit(&state.audit_writer, &authenticated_user, method.as_str(), &path, &extracted_ids, status_code);
    let _ = req.respond(response);
}

fn make_statement_error_response(status_code: u16, msg: &str) -> Response<Cursor<Vec<u8>>> {
    let mut resp = make_error_response(status_code, msg);
    let consistent_iso = format_iso8601_ms(s3a_engine::now_ms());
    resp.add_header(Header::from_bytes(&b"X-Experience-API-Consistent-Through"[..], consistent_iso.as_bytes()).unwrap());
    resp
}

fn apply_statement_format(val: &mut serde_json::Value, format: Option<&str>, accept_lang: Option<&str>) {
    let fmt = format.unwrap_or("exact");
    match fmt {
        "ids" => {
            // Simplify Actor
            if let Some(actor) = val.get_mut("actor").and_then(|a| a.as_object_mut()) {
                let keep = ["mbox", "mbox_sha1sum", "openid", "account", "objectType", "member"];
                actor.retain(|k, _| keep.contains(&k.as_str()));
            }
            // Simplify Verb: remove display
            if let Some(verb) = val.get_mut("verb").and_then(|v| v.as_object_mut()) {
                verb.remove("display");
            }
            // Simplify Object
            if let Some(obj) = val.get_mut("object").and_then(|o| o.as_object_mut()) {
                let ot = obj.get("objectType").and_then(|v| v.as_str()).unwrap_or("Activity");
                if ot == "Activity" {
                    obj.retain(|k, _| k == "id");
                } else if ot == "SubStatement" {
                    if let Some(sub_actor) = obj.get_mut("actor").and_then(|a| a.as_object_mut()) {
                        let keep = ["mbox", "mbox_sha1sum", "openid", "account", "objectType", "member"];
                        sub_actor.retain(|k, _| keep.contains(&k.as_str()));
                    }
                    if let Some(sub_verb) = obj.get_mut("verb").and_then(|v| v.as_object_mut()) {
                        sub_verb.remove("display");
                    }
                    if let Some(sub_obj) = obj.get_mut("object").and_then(|o| o.as_object_mut()) {
                        sub_obj.retain(|k, _| k == "id");
                    }
                }
            }
            // Simplify Context
            if let Some(ctx) = val.get_mut("context").and_then(|c| c.as_object_mut()) {
                if let Some(instructor) = ctx.get_mut("instructor").and_then(|i| i.as_object_mut()) {
                    let keep = ["mbox", "mbox_sha1sum", "openid", "account", "objectType", "member"];
                    instructor.retain(|k, _| keep.contains(&k.as_str()));
                }
                if let Some(team) = ctx.get_mut("team").and_then(|t| t.as_object_mut()) {
                    let keep = ["mbox", "mbox_sha1sum", "openid", "account", "objectType", "member"];
                    team.retain(|k, _| keep.contains(&k.as_str()));
                }
            }
        }
        "canonical" => {
            if let Some(al) = accept_lang {
                filter_language_maps_recursive(val, al);
            }
        }
        _ => {}
    }
}

fn filter_language_maps_recursive(val: &mut serde_json::Value, accept_lang: &str) {
    match val {
        serde_json::Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                if k == "display" || k == "name" || k == "description" || k == "scale" {
                    if let Some(lang_map) = v.as_object_mut() {
                        filter_single_language_map(lang_map, accept_lang);
                    }
                } else {
                    filter_language_maps_recursive(v, accept_lang);
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                filter_language_maps_recursive(item, accept_lang);
            }
        }
        _ => {}
    }
}

fn filter_single_language_map(map: &mut serde_json::Map<String, serde_json::Value>, accept_lang: &str) {
    if map.is_empty() {
        return;
    }
    let mut preferred = Vec::new();
    for part in accept_lang.split(',') {
        let tag = part.split(';').next().unwrap_or("").trim();
        if !tag.is_empty() {
            preferred.push(tag);
        }
    }

    let mut chosen_key = None;
    for pref in &preferred {
        for k in map.keys() {
            if k.eq_ignore_ascii_case(pref) {
                chosen_key = Some(k.clone());
                break;
            }
        }
        if chosen_key.is_some() {
            break;
        }
    }

    if chosen_key.is_none() {
        for pref in &preferred {
            let pref_prefix = pref.split('-').next().unwrap_or("");
            for k in map.keys() {
                let k_prefix = k.split('-').next().unwrap_or("");
                if !pref_prefix.is_empty() && k_prefix.eq_ignore_ascii_case(pref_prefix) {
                    chosen_key = Some(k.clone());
                    break;
                }
            }
            if chosen_key.is_some() {
                break;
            }
        }
    }

    let keep_key = chosen_key.or_else(|| map.keys().next().cloned());
    if let Some(k) = keep_key {
        map.retain(|key, _| key == &k);
    }
}

// -----------------------------------------------------------------------------
// STATEMENTS RESOURCE (/xapi/statements)
// -----------------------------------------------------------------------------
fn handle_statements(
    method: Method,
    params: &HashMap<String, String>,
    headers: &HashMap<String, String>,
    body: &[u8],
    state: &AppState,
    store: &LrsStore,
    archive_name: &str,
) -> Response<Cursor<Vec<u8>>> {
    match method {
        Method::Put => {
            let stmt_id_str = match params.get("statementId") {
                Some(id) => id,
                None => return make_error_response(400, "Missing statementId parameter on PUT"),
            };
            let stmt_uuid = match uuid_to_u64_pair(stmt_id_str) {
                Ok(u) => u,
                Err(_) => return make_error_response(400, "Invalid statementId UUID format"),
            };

            // Check if multipart/mixed
            let content_type = headers.get("content-type").cloned().unwrap_or_default();
            if content_type.starts_with("multipart/") && !content_type.starts_with("multipart/mixed") {
                return make_error_response(400, "Only multipart/mixed is supported for attachments");
            }
            let (statement_json, attachments) = if content_type.starts_with("multipart/mixed") {
                match parse_multipart_request(&content_type, body) {
                    Ok(res) => res,
                    Err(e) => return make_error_response(400, &format!("Multipart parse error: {}", e)),
                }
            } else {
                let json_str = String::from_utf8_lossy(body).to_string();
                (json_str, Vec::new())
            };

            let mut val: serde_json::Value = match serde_json::from_str(&statement_json) {
                Ok(v) => v,
                Err(e) => return make_error_response(400, &format!("JSON syntax error: {}", e)),
            };

            if let Err(e) = validate_batch_attachments(&[val.clone()], &attachments) {
                return make_error_response(400, &format!("Attachment validation error: {}", e));
            }

            // If statement has an 'id', it must match query statementId
            if let Some(body_id) = val.get("id").and_then(|v| v.as_str()) {
                if let Ok(body_uuid) = uuid_to_u64_pair(body_id) {
                    if body_uuid != stmt_uuid {
                        return make_error_response(400, "statementId parameter does not match statement.id in body");
                    }
                } else {
                    return make_error_response(400, "Invalid statement.id UUID format");
                }
            } else {
                val["id"] = serde_json::Value::String(u64_pair_to_uuid(stmt_uuid));
            }

            // Validate xAPI statement structure
            if let Err(e) = s3a_engine::lrs_store::validate_xapi_statement(&val) {
                return make_error_response(400, &format!("Invalid statement: {}", e));
            }

            // Strict validation for scholar-explorer-usage/v1 profile on usage archives
            let is_usage_archive = archive_name.to_ascii_lowercase().contains("usage");
            if is_usage_archive {
                if let Err(e) = s3a_pack_scholar::xapi::validate_scholar_explorer_usage_profile(&val) {
                    return make_error_response(400, &format!("Scholar Explorer Usage profile error: {}", e));
                }
            }

            // Scholar checks
            let mut scholar_violation = false;
            if state.config.enable_scholar {
                // AI agent allowlist check (403 on denial)
                if let Err(e) = s3a_pack_scholar::xapi::validate_ai_agent_allowlist(&state.agent_registry, &val) {
                    return make_error_response(403, &format!("Agent action denied: {}", e));
                }

                // Scholar profile validation
                if let Err(e) = s3a_pack_scholar::xapi::validate_scholar_profile(&val) {
                    if state.config.profile_mode_strict {
                        return make_error_response(400, &format!("Scholar profile validation error: {}", e));
                    } else {
                        scholar_violation = true;
                    }
                }
            }

            let canonical_str = val.to_string();
            match store.store_statement(&canonical_str, &attachments) {
                Ok(id) => {
                    if scholar_violation {
                        let _ = store.mark_statement(id, s3a_core::MARK_NEEDS_REVIEW, true, 0);
                    }
                    if state.config.enable_scholar {
                        let _ = s3a_pack_scholar::xapi::enrich_scholar_statement(
                            store.writer(),
                            &val,
                            [0u8; 16],
                        );
                    }
                    make_response_empty(204)
                }
                Err(LrsStoreError::StatementConflict(msg)) => make_error_response(409, &msg),
                Err(LrsStoreError::CannotVoidVoidingStatement) => {
                    make_error_response(400, "Cannot void a voiding statement")
                }
                Err(e) => make_error_response(400, &format!("Failed to store statement: {}", e)),
            }
        }

        Method::Post => {
            let content_type = headers.get("content-type").cloned().unwrap_or_default();
            if content_type.starts_with("multipart/") && !content_type.starts_with("multipart/mixed") {
                return make_error_response(400, "Only multipart/mixed is supported for attachments");
            }
            let (statement_json, attachments) = if content_type.starts_with("multipart/mixed") {
                match parse_multipart_request(&content_type, body) {
                    Ok(res) => res,
                    Err(e) => return make_error_response(400, &format!("Multipart parse error: {}", e)),
                }
            } else {
                let json_str = String::from_utf8_lossy(body).to_string();
                (json_str, Vec::new())
            };

            let val: serde_json::Value = match serde_json::from_str(&statement_json) {
                Ok(v) => v,
                Err(e) => return make_error_response(400, &format!("JSON syntax error: {}", e)),
            };

            let statements_to_process: Vec<serde_json::Value> = match val {
                serde_json::Value::Array(arr) => {
                    if arr.is_empty() {
                        return make_error_response(400, "Statement array cannot be empty");
                    }
                    arr
                }
                serde_json::Value::Object(_) => vec![val],
                _ => return make_error_response(400, "Statement payload must be a JSON object or array"),
            };

            if let Err(e) = validate_batch_attachments(&statements_to_process, &attachments) {
                return make_error_response(400, &format!("Attachment validation error: {}", e));
            }

            let is_usage_archive = archive_name.to_ascii_lowercase().contains("usage");

            // Pre-validation pass: ALL statements must be valid before any is stored!
            for stmt_val in &statements_to_process {
                if let Err(e) = s3a_engine::lrs_store::validate_xapi_statement(stmt_val) {
                    return make_error_response(400, &format!("Invalid statement: {}", e));
                }
                if is_usage_archive {
                    if let Err(e) = s3a_pack_scholar::xapi::validate_scholar_explorer_usage_profile(stmt_val) {
                        return make_error_response(400, &format!("Scholar Explorer Usage profile error: {}", e));
                    }
                }
                if state.config.enable_scholar {
                    if let Err(e) = s3a_pack_scholar::xapi::validate_ai_agent_allowlist(&state.agent_registry, stmt_val) {
                        return make_error_response(403, &format!("Agent action denied: {}", e));
                    }
                    if state.config.profile_mode_strict {
                        if let Err(e) = s3a_pack_scholar::xapi::validate_scholar_profile(stmt_val) {
                            return make_error_response(400, &format!("Scholar profile validation error: {}", e));
                        }
                    }
                }
            }

            // Storage pass
            let mut stored_ids = Vec::new();
            for stmt_val in &statements_to_process {
                let s_str = stmt_val.to_string();
                match store.store_statement(&s_str, &attachments) {
                    Ok(id) => {
                        if state.config.enable_scholar {
                            if s3a_pack_scholar::xapi::validate_scholar_profile(stmt_val).is_err() {
                                let _ = store.mark_statement(id, s3a_core::MARK_NEEDS_REVIEW, true, 0);
                            }
                            let _ = s3a_pack_scholar::xapi::enrich_scholar_statement(
                                store.writer(),
                                stmt_val,
                                [0u8; 16],
                            );
                        }
                        stored_ids.push(u64_pair_to_uuid(id));
                    }
                    Err(LrsStoreError::StatementConflict(msg)) => return make_error_response(409, &msg),
                    Err(LrsStoreError::CannotVoidVoidingStatement) => {
                        return make_error_response(400, "Cannot void a voiding statement")
                    }
                    Err(e) => return make_error_response(400, &format!("Failed to store statement: {}", e)),
                }
            }

            let resp_json = serde_json::to_string(&stored_ids).unwrap();
            make_response(200, "application/json", resp_json.as_bytes(), false)
        }

        Method::Get | Method::Head => {
            let is_head = method == Method::Head;

            // Validate allowed query parameters per xAPI 1.0.3 §4.1.2
            const ALLOWED_GET_PARAMS: &[&str] = &[
                "statementId",
                "voidedStatementId",
                "agent",
                "verb",
                "activity",
                "registration",
                "related_activities",
                "related_agents",
                "since",
                "until",
                "limit",
                "format",
                "attachments",
                "ascending",
                "offset",
                "include_hidden",
            ];
            for k in params.keys() {
                if !ALLOWED_GET_PARAMS.contains(&k.as_str()) {
                    return make_statement_error_response(400, &format!("Unknown parameter: {}", k));
                }
            }

            let stmt_id = params.get("statementId");
            let voided_id = params.get("voidedStatementId");

            if stmt_id.is_some() && voided_id.is_some() {
                return make_statement_error_response(400, "Cannot specify both statementId and voidedStatementId");
            }

            const COLLECTION_EXCLUSIVE_PARAMS: &[&str] = &[
                "agent",
                "verb",
                "activity",
                "registration",
                "related_activities",
                "related_agents",
                "since",
                "until",
                "limit",
                "ascending",
            ];

            if stmt_id.is_some() {
                for p in COLLECTION_EXCLUSIVE_PARAMS {
                    if params.contains_key(*p) {
                        return make_statement_error_response(400, &format!("Cannot specify {} with statementId", p));
                    }
                }
            }

            if voided_id.is_some() {
                for p in COLLECTION_EXCLUSIVE_PARAMS {
                    if params.contains_key(*p) {
                        return make_statement_error_response(400, &format!("Cannot specify {} with voidedStatementId", p));
                    }
                }
            }

            let format_param = params.get("format").map(|s| s.as_str());
            let accept_lang = headers.get("accept-language").map(|s| s.as_str());
            let want_attachments = params.get("attachments").map(|v| v == "true").unwrap_or(false);

            // Single statement by ID
            if let Some(id_str) = stmt_id {
                let id = match uuid_to_u64_pair(id_str) {
                    Ok(u) => u,
                    Err(_) => return make_statement_error_response(400, "Invalid statementId UUID"),
                };
                match store.get_statement(id, false) {
                    Ok((idx, json_str, atts)) => {
                        let mut val: serde_json::Value = match serde_json::from_str(&json_str) {
                            Ok(v) => v,
                            Err(e) => return make_statement_error_response(500, &format!("Corrupt stored statement: {}", e)),
                        };
                        apply_statement_format(&mut val, format_param, accept_lang);
                        let out_json = val.to_string();

                        let mut resp = if want_attachments {
                            make_multipart_response(&out_json, &atts, is_head)
                        } else {
                            make_response(200, "application/json", out_json.as_bytes(), is_head)
                        };
                        let consistent_iso = format_iso8601_ms(s3a_engine::now_ms());
                        resp.add_header(
                            Header::from_bytes(&b"X-Experience-API-Consistent-Through"[..], consistent_iso.as_bytes()).unwrap(),
                        );
                        let (_, etag) = LrsStore::calculate_etag(out_json.as_bytes());
                        resp.add_header(Header::from_bytes(&b"ETag"[..], etag.as_bytes()).unwrap());
                        let modified_iso = format_iso8601_ms(idx.stored_timestamp_ms);
                        resp.add_header(Header::from_bytes(&b"Last-Modified"[..], modified_iso.as_bytes()).unwrap());
                        return resp;
                    }
                    Err(LrsStoreError::StatementNotFound) | Err(LrsStoreError::StatementVoided) => {
                        return make_statement_error_response(404, "Statement not found or voided");
                    }
                    Err(e) => return make_statement_error_response(400, &format!("{}", e)),
                }
            }

            // Single voided statement by ID
            if let Some(id_str) = voided_id {
                let id = match uuid_to_u64_pair(id_str) {
                    Ok(u) => u,
                    Err(_) => return make_statement_error_response(400, "Invalid voidedStatementId UUID"),
                };
                match store.get_statement(id, true) {
                    Ok((idx, json_str, atts)) => {
                        let mut val: serde_json::Value = match serde_json::from_str(&json_str) {
                            Ok(v) => v,
                            Err(e) => return make_statement_error_response(500, &format!("Corrupt stored statement: {}", e)),
                        };
                        apply_statement_format(&mut val, format_param, accept_lang);
                        let out_json = val.to_string();

                        let mut resp = if want_attachments {
                            make_multipart_response(&out_json, &atts, is_head)
                        } else {
                            make_response(200, "application/json", out_json.as_bytes(), is_head)
                        };
                        let consistent_iso = format_iso8601_ms(s3a_engine::now_ms());
                        resp.add_header(
                            Header::from_bytes(&b"X-Experience-API-Consistent-Through"[..], consistent_iso.as_bytes()).unwrap(),
                        );
                        let (_, etag) = LrsStore::calculate_etag(out_json.as_bytes());
                        resp.add_header(Header::from_bytes(&b"ETag"[..], etag.as_bytes()).unwrap());
                        let modified_iso = format_iso8601_ms(idx.stored_timestamp_ms);
                        resp.add_header(Header::from_bytes(&b"Last-Modified"[..], modified_iso.as_bytes()).unwrap());
                        return resp;
                    }
                    Err(_) => return make_statement_error_response(404, "Voided statement not found"),
                }
            }

            // Query statement collection
            let mut filter = StatementFilter::default();
            if let Some(agent_str) = params.get("agent") {
                let agent_val = match serde_json::from_str::<serde_json::Value>(agent_str) {
                    Ok(v) if v.is_object() => v,
                    _ => return make_statement_error_response(400, "agent parameter must be a valid JSON Object"),
                };
                filter.agent_hash = Some(s3a_engine::hash_actor_value(&agent_val));
            }
            if let Some(v) = params.get("verb") {
                filter.verb_iri = Some(v.clone());
            }
            if let Some(act) = params.get("activity") {
                filter.activity_id = Some(act.clone());
            }
            if let Some(reg) = params.get("registration") {
                match uuid_to_u64_pair(reg) {
                    Ok(u) => filter.registration = Some(u),
                    Err(_) => return make_statement_error_response(400, "Invalid registration UUID"),
                }
            }
            if let Some(rel_act) = params.get("related_activities") {
                filter.related_activities = rel_act == "true";
            }
            if let Some(rel_ag) = params.get("related_agents") {
                filter.related_agents = rel_ag == "true";
            }
            if let Some(since) = params.get("since") {
                match s3a_engine::chrono_parse_ms(since) {
                    Some(ms) => filter.since_ms = Some(ms),
                    None => return make_statement_error_response(400, "Invalid since timestamp"),
                }
            }
            if let Some(until) = params.get("until") {
                match s3a_engine::chrono_parse_ms(until) {
                    Some(ms) => filter.until_ms = Some(ms),
                    None => return make_statement_error_response(400, "Invalid until timestamp"),
                }
            }
            if let Some(lim) = params.get("limit") {
                match lim.parse::<usize>() {
                    Ok(n) if n > 0 => filter.limit = n,
                    _ => return make_statement_error_response(400, "limit parameter must be a positive integer"),
                }
            }
            if let Some(asc) = params.get("ascending") {
                filter.ascending = asc == "true";
            }
            if let Some(att) = params.get("attachments") {
                filter.attachments = att == "true";
            }
            if let Some(off) = params.get("offset") {
                match off.parse::<usize>() {
                    Ok(n) => filter.offset = n,
                    _ => return make_statement_error_response(400, "offset parameter must be a non-negative integer"),
                }
            }
            if let Some(inc) = params.get("include_hidden") {
                filter.include_hidden = inc == "true";
            }

            match store.query_statements(filter) {
                Ok(page) => {
                    let mut stmts_raw = Vec::new();
                    for s in &page.statements {
                        if let Ok(mut v) = serde_json::from_str::<serde_json::Value>(s) {
                            apply_statement_format(&mut v, format_param, accept_lang);
                            stmts_raw.push(v);
                        }
                    }
                    let mut result_obj = serde_json::Map::new();
                    result_obj.insert("statements".to_string(), serde_json::Value::Array(stmts_raw));
                    let more_val = page.more_token.map(|t| format!("/xapi/statements?{}", t)).unwrap_or_default();
                    result_obj.insert("more".to_string(), serde_json::Value::String(more_val));

                    let resp_json = serde_json::Value::Object(result_obj).to_string();
                    let mut resp = if want_attachments {
                        make_multipart_response(&resp_json, &page.attachments, is_head)
                    } else {
                        make_response(200, "application/json", resp_json.as_bytes(), is_head)
                    };
                    let consistent_iso = format_iso8601_ms(s3a_engine::now_ms());
                    resp.add_header(
                        Header::from_bytes(&b"X-Experience-API-Consistent-Through"[..], consistent_iso.as_bytes()).unwrap(),
                    );
                    resp
                }
                Err(e) => make_statement_error_response(400, &format!("Query error: {}", e)),
            }
        }

        _ => make_error_response(405, "Method Not Allowed"),
    }
}

// -----------------------------------------------------------------------------
// STATE API (/xapi/activities/state)
// -----------------------------------------------------------------------------
fn handle_activities_state(
    method: Method,
    params: &HashMap<String, String>,
    headers: &HashMap<String, String>,
    body: &[u8],
    _state: &AppState,
    store: &LrsStore,
) -> Response<Cursor<Vec<u8>>> {
    let activity_id = match params.get("activityId") {
        Some(a) => a,
        None => return make_error_response(400, "Missing activityId"),
    };
    let agent_str = match params.get("agent") {
        Some(a) => a,
        None => return make_error_response(400, "Missing agent"),
    };

    // Agent must be a valid JSON Object
    match serde_json::from_str::<serde_json::Value>(agent_str) {
        Ok(v) if v.is_object() => {},
        _ => return make_error_response(400, "agent parameter must be a valid JSON Object"),
    }

    if let Some(reg) = params.get("registration") {
        if uuid_to_u64_pair(reg).is_err() {
            return make_error_response(400, "Invalid registration UUID");
        }
    }
    let registration = params.get("registration").cloned().unwrap_or_default();

    let prefix = format!("{}\0{}\0{}\0", activity_id, agent_str, registration);
    let state_id = params.get("stateId");
    let compound_key = if let Some(sid) = state_id {
        format!("{}{}", prefix, sid)
    } else {
        prefix.clone()
    };

    let if_match = headers.get("if-match").map(|s| s.as_str());
    let if_none_match = headers.get("if-none-match").map(|s| s.as_str());

    match method {
        Method::Get | Method::Head => {
            let is_head = method == Method::Head;
            if state_id.is_some() {
                // Get state document
                if let Some((hdr, content, _)) = store.get_document(DOC_KIND_STATE, &compound_key) {
                    let (_, etag) = LrsStore::calculate_etag(&content);
                    let content_type = if (hdr.flags & DOC_FLAG_IS_JSON) != 0 {
                        "application/json"
                    } else {
                        "application/octet-stream"
                    };
                    let mut resp = make_response(200, content_type, &content, is_head);
                    resp.add_header(Header::from_bytes(&b"ETag"[..], etag.as_bytes()).unwrap());
                    let modified_iso = format_iso8601_ms(hdr.timestamp_ms);
                    resp.add_header(Header::from_bytes(&b"Last-Modified"[..], modified_iso.as_bytes()).unwrap());
                    resp
                } else {
                    make_error_response(404, "State document not found")
                }
            } else {
                // List state IDs
                let since_ms = if let Some(s) = params.get("since") {
                    match s3a_engine::chrono_parse_ms(s) {
                        Some(ms) => Some(ms),
                        None => return make_error_response(400, "Invalid since timestamp"),
                    }
                } else {
                    None
                };
                let ids = store.list_document_ids(DOC_KIND_STATE, &prefix, since_ms);
                let json_resp = serde_json::to_string(&ids).unwrap();
                make_response(200, "application/json", json_resp.as_bytes(), is_head)
            }
        }

        Method::Put => {
            if state_id.is_none() {
                return make_error_response(400, "Missing stateId on PUT");
            }
            let is_json = headers.get("content-type").map(|c| c.contains("application/json")).unwrap_or(false);
            match store.put_document(DOC_KIND_STATE, &compound_key, body, is_json, if_match, if_none_match, false) {
                Ok(etag) => {
                    let mut resp = make_response_empty(204);
                    resp.add_header(Header::from_bytes(&b"ETag"[..], etag.as_bytes()).unwrap());
                    resp
                }
                Err(LrsStoreError::PreconditionFailed(msg)) => make_error_response(412, &msg),
                Err(LrsStoreError::StatementConflict(msg)) => make_error_response(409, &msg),
                Err(e) => make_error_response(400, &format!("{}", e)),
            }
        }

        Method::Post => {
            if state_id.is_none() {
                return make_error_response(400, "Missing stateId on POST");
            }
            let is_json = headers.get("content-type").map(|c| c.contains("application/json")).unwrap_or(false);
            if is_json {
                match serde_json::from_slice::<serde_json::Value>(body) {
                    Ok(v) if v.is_object() => {},
                    _ => return make_error_response(400, "Posted document must be a JSON object"),
                }
            }
            match store.put_document(DOC_KIND_STATE, &compound_key, body, is_json, if_match, if_none_match, true) {
                Ok(etag) => {
                    let mut resp = make_response_empty(204);
                    resp.add_header(Header::from_bytes(&b"ETag"[..], etag.as_bytes()).unwrap());
                    resp
                }
                Err(LrsStoreError::PreconditionFailed(msg)) => make_error_response(412, &msg),
                Err(e) => make_error_response(400, &format!("{}", e)),
            }
        }

        Method::Delete => {
            if state_id.is_some() {
                match store.delete_document(DOC_KIND_STATE, &compound_key, if_match) {
                    Ok(_) => make_response_empty(204),
                    Err(LrsStoreError::DocumentNotFound) => make_response_empty(204), // Idempotent delete
                    Err(LrsStoreError::PreconditionFailed(msg)) => make_error_response(412, &msg),
                    Err(e) => make_error_response(400, &format!("{}", e)),
                }
            } else {
                // Delete all state documents matching prefix
                let ids = store.list_document_ids(DOC_KIND_STATE, &prefix, None);
                for id in ids {
                    let k = format!("{}{}", prefix, id);
                    let _ = store.delete_document(DOC_KIND_STATE, &k, None);
                }
                make_response_empty(204)
            }
        }

        _ => make_error_response(405, "Method Not Allowed"),
    }
}

// -----------------------------------------------------------------------------
// ACTIVITY PROFILE API (/xapi/activities/profile)
// -----------------------------------------------------------------------------
fn handle_activities_profile(
    method: Method,
    params: &HashMap<String, String>,
    headers: &HashMap<String, String>,
    body: &[u8],
    _state: &AppState,
    store: &LrsStore,
) -> Response<Cursor<Vec<u8>>> {
    let activity_id = match params.get("activityId") {
        Some(a) => a,
        None => return make_error_response(400, "Missing activityId"),
    };
    let profile_id = params.get("profileId");

    let prefix = format!("{}\0", activity_id);
    let compound_key = if let Some(pid) = profile_id {
        format!("{}{}", prefix, pid)
    } else {
        prefix.clone()
    };

    let if_match = headers.get("if-match").map(|s| s.as_str());
    let if_none_match = headers.get("if-none-match").map(|s| s.as_str());

    match method {
        Method::Get | Method::Head => {
            let is_head = method == Method::Head;
            if profile_id.is_some() {
                if let Some((hdr, content, _)) = store.get_document(DOC_KIND_ACTIVITY_PROFILE, &compound_key) {
                    let (_, etag) = LrsStore::calculate_etag(&content);
                    let content_type = if (hdr.flags & DOC_FLAG_IS_JSON) != 0 {
                        "application/json"
                    } else {
                        "application/octet-stream"
                    };
                    let mut resp = make_response(200, content_type, &content, is_head);
                    resp.add_header(Header::from_bytes(&b"ETag"[..], etag.as_bytes()).unwrap());
                    let modified_iso = format_iso8601_ms(hdr.timestamp_ms);
                    resp.add_header(Header::from_bytes(&b"Last-Modified"[..], modified_iso.as_bytes()).unwrap());
                    resp
                } else {
                    make_error_response(404, "Activity profile not found")
                }
            } else {
                let since_ms = if let Some(s) = params.get("since") {
                    match s3a_engine::chrono_parse_ms(s) {
                        Some(ms) => Some(ms),
                        None => return make_error_response(400, "Invalid since timestamp"),
                    }
                } else {
                    None
                };
                let ids = store.list_document_ids(DOC_KIND_ACTIVITY_PROFILE, &prefix, since_ms);
                let json_resp = serde_json::to_string(&ids).unwrap();
                make_response(200, "application/json", json_resp.as_bytes(), is_head)
            }
        }

        Method::Put => {
            if profile_id.is_none() {
                return make_error_response(400, "Missing profileId on PUT");
            }
            let is_json = headers.get("content-type").map(|c| c.contains("application/json")).unwrap_or(false);
            match store.put_document(DOC_KIND_ACTIVITY_PROFILE, &compound_key, body, is_json, if_match, if_none_match, false) {
                Ok(etag) => {
                    let mut resp = make_response_empty(204);
                    resp.add_header(Header::from_bytes(&b"ETag"[..], etag.as_bytes()).unwrap());
                    resp
                }
                Err(LrsStoreError::PreconditionFailed(msg)) => make_error_response(412, &msg),
                Err(LrsStoreError::StatementConflict(msg)) => make_error_response(409, &msg),
                Err(e) => make_error_response(400, &format!("{}", e)),
            }
        }

        Method::Post => {
            if profile_id.is_none() {
                return make_error_response(400, "Missing profileId on POST");
            }
            let is_json = headers.get("content-type").map(|c| c.contains("application/json")).unwrap_or(false);
            if is_json {
                match serde_json::from_slice::<serde_json::Value>(body) {
                    Ok(v) if v.is_object() => {},
                    _ => return make_error_response(400, "Posted document must be a JSON object"),
                }
            }
            match store.put_document(DOC_KIND_ACTIVITY_PROFILE, &compound_key, body, is_json, if_match, if_none_match, true) {
                Ok(etag) => {
                    let mut resp = make_response_empty(204);
                    resp.add_header(Header::from_bytes(&b"ETag"[..], etag.as_bytes()).unwrap());
                    resp
                }
                Err(LrsStoreError::PreconditionFailed(msg)) => make_error_response(412, &msg),
                Err(e) => make_error_response(400, &format!("{}", e)),
            }
        }

        Method::Delete => {
            if profile_id.is_none() {
                return make_error_response(400, "Missing profileId on DELETE");
            }
            match store.delete_document(DOC_KIND_ACTIVITY_PROFILE, &compound_key, if_match) {
                Ok(_) => make_response_empty(204),
                Err(LrsStoreError::DocumentNotFound) => make_response_empty(204),
                Err(LrsStoreError::PreconditionFailed(msg)) => make_error_response(412, &msg),
                Err(e) => make_error_response(400, &format!("{}", e)),
            }
        }

        _ => make_error_response(405, "Method Not Allowed"),
    }
}

// -----------------------------------------------------------------------------
// AGENT PROFILE API (/xapi/agents/profile)
// -----------------------------------------------------------------------------
fn handle_agents_profile(
    method: Method,
    params: &HashMap<String, String>,
    headers: &HashMap<String, String>,
    body: &[u8],
    _state: &AppState,
    store: &LrsStore,
) -> Response<Cursor<Vec<u8>>> {
    let agent_str = match params.get("agent") {
        Some(a) => a,
        None => return make_error_response(400, "Missing agent"),
    };

    // Agent must be a valid JSON Object
    match serde_json::from_str::<serde_json::Value>(agent_str) {
        Ok(v) if v.is_object() => {},
        _ => return make_error_response(400, "agent parameter must be a valid JSON Object"),
    }

    let profile_id = params.get("profileId");

    let prefix = format!("{}\0", agent_str);
    let compound_key = if let Some(pid) = profile_id {
        format!("{}{}", prefix, pid)
    } else {
        prefix.clone()
    };

    let if_match = headers.get("if-match").map(|s| s.as_str());
    let if_none_match = headers.get("if-none-match").map(|s| s.as_str());

    match method {
        Method::Get | Method::Head => {
            let is_head = method == Method::Head;
            if profile_id.is_some() {
                if let Some((hdr, content, _)) = store.get_document(DOC_KIND_AGENT_PROFILE, &compound_key) {
                    let (_, etag) = LrsStore::calculate_etag(&content);
                    let content_type = if (hdr.flags & DOC_FLAG_IS_JSON) != 0 {
                        "application/json"
                    } else {
                        "application/octet-stream"
                    };
                    let mut resp = make_response(200, content_type, &content, is_head);
                    resp.add_header(Header::from_bytes(&b"ETag"[..], etag.as_bytes()).unwrap());
                    let modified_iso = format_iso8601_ms(hdr.timestamp_ms);
                    resp.add_header(Header::from_bytes(&b"Last-Modified"[..], modified_iso.as_bytes()).unwrap());
                    resp
                } else {
                    make_error_response(404, "Agent profile not found")
                }
            } else {
                let since_ms = if let Some(s) = params.get("since") {
                    match s3a_engine::chrono_parse_ms(s) {
                        Some(ms) => Some(ms),
                        None => return make_error_response(400, "Invalid since timestamp"),
                    }
                } else {
                    None
                };
                let ids = store.list_document_ids(DOC_KIND_AGENT_PROFILE, &prefix, since_ms);
                let json_resp = serde_json::to_string(&ids).unwrap();
                make_response(200, "application/json", json_resp.as_bytes(), is_head)
            }
        }

        Method::Put => {
            if profile_id.is_none() {
                return make_error_response(400, "Missing profileId on PUT");
            }
            let is_json = headers.get("content-type").map(|c| c.contains("application/json")).unwrap_or(false);
            match store.put_document(DOC_KIND_AGENT_PROFILE, &compound_key, body, is_json, if_match, if_none_match, false) {
                Ok(etag) => {
                    let mut resp = make_response_empty(204);
                    resp.add_header(Header::from_bytes(&b"ETag"[..], etag.as_bytes()).unwrap());
                    resp
                }
                Err(LrsStoreError::PreconditionFailed(msg)) => make_error_response(412, &msg),
                Err(LrsStoreError::StatementConflict(msg)) => make_error_response(409, &msg),
                Err(e) => make_error_response(400, &format!("{}", e)),
            }
        }

        Method::Post => {
            if profile_id.is_none() {
                return make_error_response(400, "Missing profileId on POST");
            }
            let is_json = headers.get("content-type").map(|c| c.contains("application/json")).unwrap_or(false);
            if is_json {
                match serde_json::from_slice::<serde_json::Value>(body) {
                    Ok(v) if v.is_object() => {},
                    _ => return make_error_response(400, "Posted document must be a JSON object"),
                }
            }
            match store.put_document(DOC_KIND_AGENT_PROFILE, &compound_key, body, is_json, if_match, if_none_match, true) {
                Ok(etag) => {
                    let mut resp = make_response_empty(204);
                    resp.add_header(Header::from_bytes(&b"ETag"[..], etag.as_bytes()).unwrap());
                    resp
                }
                Err(LrsStoreError::PreconditionFailed(msg)) => make_error_response(412, &msg),
                Err(e) => make_error_response(400, &format!("{}", e)),
            }
        }

        Method::Delete => {
            if profile_id.is_none() {
                return make_error_response(400, "Missing profileId on DELETE");
            }
            match store.delete_document(DOC_KIND_AGENT_PROFILE, &compound_key, if_match) {
                Ok(_) => make_response_empty(204),
                Err(LrsStoreError::DocumentNotFound) => make_response_empty(204),
                Err(LrsStoreError::PreconditionFailed(msg)) => make_error_response(412, &msg),
                Err(e) => make_error_response(400, &format!("{}", e)),
            }
        }

        _ => make_error_response(405, "Method Not Allowed"),
    }
}

fn deep_merge_json(target: &mut serde_json::Value, source: &serde_json::Value) {
    match (target, source) {
        (serde_json::Value::Object(t_map), serde_json::Value::Object(s_map)) => {
            for (k, v) in s_map {
                if let Some(existing) = t_map.get_mut(k) {
                    deep_merge_json(existing, v);
                } else {
                    t_map.insert(k.clone(), v.clone());
                }
            }
        }
        (t, s) => {
            *t = s.clone();
        }
    }
}

// -----------------------------------------------------------------------------
// ACTIVITIES API (/xapi/activities)
// -----------------------------------------------------------------------------
fn handle_activities(
    method: Method,
    params: &HashMap<String, String>,
    _headers: &HashMap<String, String>,
    store: &LrsStore,
) -> Response<Cursor<Vec<u8>>> {
    if method != Method::Get && method != Method::Head {
        return make_error_response(405, "Method Not Allowed");
    }
    let is_head = method == Method::Head;
    let activity_id = match params.get("activityId") {
        Some(a) => a,
        None => return make_error_response(400, "Missing activityId parameter"),
    };

    // Construct Activity object
    let mut obj = serde_json::Map::new();
    obj.insert("objectType".to_string(), serde_json::Value::String("Activity".to_string()));
    obj.insert("id".to_string(), serde_json::Value::String(activity_id.clone()));

    // Try to find definitions from stored statements and merge them chronologically
    let query_filter = StatementFilter {
        activity_id: Some(activity_id.clone()),
        ascending: true,
        limit: 10_000,
        ..Default::default()
    };
    if let Ok(page) = store.query_statements(query_filter) {
        let mut merged_def = serde_json::Map::new();
        for s in &page.statements {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(s) {
                if v.get("object").and_then(|o| o.get("id")).and_then(|i| i.as_str()) == Some(activity_id.as_str()) {
                    if let Some(def) = v.get("object").and_then(|o| o.get("definition")).and_then(|d| d.as_object()) {
                        for (k, val) in def {
                            match merged_def.get_mut(k) {
                                Some(existing) => {
                                    deep_merge_json(existing, val);
                                }
                                None => {
                                    merged_def.insert(k.clone(), val.clone());
                                }
                            }
                        }
                    }
                }
            }
        }
        if !merged_def.is_empty() {
            obj.insert("definition".to_string(), serde_json::Value::Object(merged_def));
        }
    }

    let resp_json = serde_json::Value::Object(obj).to_string();
    make_response(200, "application/json", resp_json.as_bytes(), is_head)
}

// -----------------------------------------------------------------------------
// AGENTS API (/xapi/agents)
// -----------------------------------------------------------------------------
fn handle_agents(
    method: Method,
    params: &HashMap<String, String>,
    _headers: &HashMap<String, String>,
    _state: &AppState,
) -> Response<Cursor<Vec<u8>>> {
    if method != Method::Get && method != Method::Head {
        return make_error_response(405, "Method Not Allowed");
    }
    let is_head = method == Method::Head;
    let agent_str = match params.get("agent") {
        Some(a) => a,
        None => return make_error_response(400, "Missing agent parameter"),
    };

    let agent_val: serde_json::Value = match serde_json::from_str(agent_str) {
        Ok(v) => v,
        Err(e) => return make_error_response(400, &format!("Invalid agent JSON: {}", e)),
    };

    if !agent_val.is_object() {
        return make_error_response(400, "agent parameter must be a valid JSON Object");
    }

    if let Some(ot) = agent_val.get("objectType").and_then(|v| v.as_str()) {
        if ot != "Agent" && ot != "Group" {
            return make_error_response(400, "Invalid objectType for agent parameter");
        }
    }

    let ifi_count = (if agent_val.get("mbox").is_some() { 1 } else { 0 })
        + (if agent_val.get("mbox_sha1sum").is_some() { 1 } else { 0 })
        + (if agent_val.get("openid").is_some() { 1 } else { 0 })
        + (if agent_val.get("account").is_some() { 1 } else { 0 });

    if ifi_count != 1 {
        return make_error_response(400, "Agent must contain exactly one Inverse Functional Identifier (mbox, mbox_sha1sum, openid, or account)");
    }

    if let Some(acc) = agent_val.get("account") {
        if !acc.is_object() || acc.get("homePage").is_none() || acc.get("name").is_none() {
            return make_error_response(400, "Agent account must contain homePage and name");
        }
    }

    // Construct Person object per xAPI 1.0.3 §4.1.4
    let mut person = serde_json::Map::new();
    person.insert("objectType".to_string(), serde_json::Value::String("Person".to_string()));

    if let Some(name) = agent_val.get("name").and_then(|v| v.as_str()) {
        person.insert("name".to_string(), serde_json::json!([name]));
    }
    if let Some(mbox) = agent_val.get("mbox").and_then(|v| v.as_str()) {
        person.insert("mbox".to_string(), serde_json::json!([mbox]));
    }
    if let Some(sha1) = agent_val.get("mbox_sha1sum").and_then(|v| v.as_str()) {
        person.insert("mbox_sha1sum".to_string(), serde_json::json!([sha1]));
    }
    if let Some(openid) = agent_val.get("openid").and_then(|v| v.as_str()) {
        person.insert("openid".to_string(), serde_json::json!([openid]));
    }
    if let Some(acc) = agent_val.get("account") {
        person.insert("account".to_string(), serde_json::json!([acc]));
    }

    let resp_json = serde_json::Value::Object(person).to_string();
    make_response(200, "application/json", resp_json.as_bytes(), is_head)
}

// -----------------------------------------------------------------------------
// MULTIPART/MIXED PARSING AND RECONSTRUCTION
// -----------------------------------------------------------------------------
fn parse_multipart_request(content_type: &str, body: &[u8]) -> Result<(String, Vec<(String, Vec<u8>)>), String> {
    let boundary_idx = content_type.find("boundary=").ok_or("Missing boundary in multipart/mixed")?;
    let boundary = content_type[boundary_idx + 9..].trim().trim_matches('"');
    let delimiter = format!("--{}", boundary).into_bytes();

    let find_subslice = |haystack: &[u8], needle: &[u8]| -> Option<usize> {
        haystack.windows(needle.len()).position(|window| window == needle)
    };

    let mut parts: Vec<&[u8]> = Vec::new();
    let mut current = body;

    while let Some(pos) = find_subslice(current, &delimiter) {
        let after_delim = &current[pos + delimiter.len()..];
        if after_delim.starts_with(b"--") {
            break;
        }
        let part_start = if after_delim.starts_with(b"\r\n") {
            &after_delim[2..]
        } else if after_delim.starts_with(b"\n") {
            &after_delim[1..]
        } else {
            after_delim
        };

        if let Some(next_pos) = find_subslice(part_start, &delimiter) {
            let part_data = &part_start[..next_pos];
            parts.push(part_data);
            current = &part_start[next_pos..];
        } else {
            break;
        }
    }

    if parts.is_empty() {
        return Err("Malformed multipart body: no parts found".into());
    }

    let mut statement_json = String::new();
    let mut attachments = Vec::new();

    for (idx, part) in parts.iter().enumerate() {
        let header_end_opt = find_subslice(part, b"\r\n\r\n")
            .map(|p| (p, 4))
            .or_else(|| find_subslice(part, b"\n\n").map(|p| (p, 2)));

        let (header_end, header_skip) = match header_end_opt {
            Some(res) => res,
            None => return Err("Missing part headers delimiter".into()),
        };

        let headers_bytes = &part[..header_end];
        let headers_str = String::from_utf8_lossy(headers_bytes);
        let mut content = &part[header_end + header_skip..];

        // Trim trailing CRLF before next boundary
        if content.ends_with(b"\r\n") {
            content = &content[..content.len() - 2];
        } else if content.ends_with(b"\n") {
            content = &content[..content.len() - 1];
        }

        if idx == 0 {
            let mut is_json = false;
            for line in headers_str.lines() {
                let lower = line.to_ascii_lowercase();
                if lower.starts_with("content-type:") {
                    let ct = line["content-type:".len()..].trim();
                    if ct.starts_with("application/json") {
                        is_json = true;
                    }
                }
            }
            if !is_json {
                return Err("First part of multipart/mixed must have Content-Type: application/json".into());
            }

            statement_json = String::from_utf8(content.to_vec())
                .map_err(|e| format!("Statement part is not valid UTF-8: {}", e))?;
        } else {
            let mut sha2 = None;
            let mut has_binary_encoding = false;

            for line in headers_str.lines() {
                let lower = line.to_ascii_lowercase();
                if lower.starts_with("x-experience-api-hash:") {
                    sha2 = Some(line["x-experience-api-hash:".len()..].trim().to_string());
                } else if lower.starts_with("content-transfer-encoding:") {
                    let enc = line["content-transfer-encoding:".len()..].trim();
                    if enc.eq_ignore_ascii_case("binary") {
                        has_binary_encoding = true;
                    }
                }
            }

            if !has_binary_encoding {
                return Err("Attachment part missing Content-Transfer-Encoding: binary".into());
            }

            let sha2_hex = match sha2 {
                Some(h) if h.len() == 64 => h,
                _ => return Err("Attachment missing or invalid X-Experience-API-Hash (must be 64 hex characters)".into()),
            };

            let decoded_bytes = hex::decode(&sha2_hex)
                .map_err(|e| format!("Invalid attachment sha2 hex: {}", e))?;
            if decoded_bytes.len() != 32 {
                return Err("Attachment sha2 must decode to 32 bytes".into());
            }

            let computed_sha2 = Sha256::digest(content);
            if computed_sha2.as_slice() != decoded_bytes.as_slice() {
                return Err(format!(
                    "Attachment sha2 hash mismatch: declared {}, computed {:x}",
                    sha2_hex, computed_sha2
                ));
            }

            attachments.push((sha2_hex, content.to_vec()));
        }
    }

    Ok((statement_json, attachments))
}

fn validate_batch_attachments(
    stmts: &[serde_json::Value],
    multipart_attachments: &[(String, Vec<u8>)],
) -> Result<(), String> {
    let mut declared_hashes = Vec::new();
    let mut missing_binary_hashes = Vec::new();

    for stmt in stmts {
        if let Some(atts) = stmt.get("attachments").and_then(|a| a.as_array()) {
            for att in atts {
                let has_file_url = att.get("fileUrl").is_some();
                if let Some(sha2) = att.get("sha2").and_then(|s| s.as_str()) {
                    let sha_lower = sha2.to_ascii_lowercase();
                    declared_hashes.push(sha_lower.clone());
                    if !has_file_url {
                        missing_binary_hashes.push(sha_lower);
                    }
                }
            }
        }
    }

    if multipart_attachments.is_empty() && declared_hashes.is_empty() {
        return Ok(());
    }

    let mut provided_hashes = Vec::new();
    for (part_sha2, _) in multipart_attachments {
        let part_lower = part_sha2.to_ascii_lowercase();
        if !declared_hashes.contains(&part_lower) {
            return Err(format!("Multipart attachment with sha2 {} is not listed in statement", part_sha2));
        }
        provided_hashes.push(part_lower);
    }

    if multipart_attachments.len() > declared_hashes.len() {
        return Err("Excess multipart sections that are not attachments in the statement".into());
    }

    for req_sha in &missing_binary_hashes {
        if !provided_hashes.contains(req_sha) {
            return Err(format!("Missing multipart attachment binary for attachment with sha2 {}", req_sha));
        }
    }

    Ok(())
}

fn make_multipart_response(
    statement_json: &str,
    attachments: &[(AttachmentBlobHeader, Vec<u8>)],
    is_head: bool,
) -> Response<Cursor<Vec<u8>>> {
    let boundary = "----s3a-multipart-boundary-987654321";
    let mut body = Vec::new();

    // Part 1: statement
    body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
    body.extend_from_slice(b"Content-Type: application/json\r\n\r\n");
    body.extend_from_slice(statement_json.as_bytes());
    body.extend_from_slice(b"\r\n");

    // Parts 2..N: attachments
    for (hdr, data) in attachments {
        let sha2_hex = hex::encode(hdr.sha2);
        body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
        body.extend_from_slice(b"Content-Type: application/octet-stream\r\n");
        body.extend_from_slice(b"Content-Transfer-Encoding: binary\r\n");
        body.extend_from_slice(format!("X-Experience-API-Hash: {}\r\n\r\n", sha2_hex).as_bytes());
        body.extend_from_slice(data);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{}--\r\n", boundary).as_bytes());

    let content_type = format!("multipart/mixed; boundary=\"{}\"", boundary);
    make_response(200, &content_type, &body, is_head)
}

// -----------------------------------------------------------------------------
// RESPONSE BUILDERS
// -----------------------------------------------------------------------------
pub fn build_options_response(headers_map: &HashMap<String, String>) -> Response<Cursor<Vec<u8>>> {
    let mut headers = vec![
        Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap(),
        Header::from_bytes(&b"Access-Control-Allow-Headers"[..], &b"Content-Type, Authorization, X-Experience-API-Version, If-Match, If-None-Match, Access-Control-Request-Private-Network"[..]).unwrap(),
        Header::from_bytes(&b"Access-Control-Allow-Methods"[..], &b"GET, POST, PUT, DELETE, HEAD, OPTIONS"[..]).unwrap(),
        Header::from_bytes(&b"Access-Control-Max-Age"[..], &b"86400"[..]).unwrap(),
        Header::from_bytes(&b"Access-Control-Expose-Headers"[..], &b"ETag, Last-Modified, X-Experience-API-Version, X-Experience-API-Consistent-Through"[..]).unwrap(),
    ];
    if headers_map.get("access-control-request-private-network").map(|v| v.trim().eq_ignore_ascii_case("true")).unwrap_or(false) {
        headers.push(Header::from_bytes(&b"Access-Control-Allow-Private-Network"[..], &b"true"[..]).unwrap());
    }
    Response::new(
        StatusCode(204),
        headers,
        Cursor::new(Vec::new()),
        Some(0),
        None,
    )
}

fn make_response(status_code: u16, content_type: &str, body: &[u8], is_head: bool) -> Response<Cursor<Vec<u8>>> {
    let final_body = if is_head { Vec::new() } else { body.to_vec() };
    let resp = Response::new(
        StatusCode(status_code),
        vec![
            Header::from_bytes(&b"Content-Type"[..], content_type.as_bytes()).unwrap(),
            Header::from_bytes(&b"X-Experience-API-Version"[..], &b"1.0.3"[..]).unwrap(),
            Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap(),
            Header::from_bytes(&b"Access-Control-Allow-Headers"[..], &b"Content-Type, Authorization, X-Experience-API-Version, If-Match, If-None-Match, Access-Control-Request-Private-Network"[..]).unwrap(),
            Header::from_bytes(&b"Access-Control-Allow-Methods"[..], &b"GET, POST, PUT, DELETE, HEAD, OPTIONS"[..]).unwrap(),
            Header::from_bytes(&b"Access-Control-Expose-Headers"[..], &b"ETag, Last-Modified, X-Experience-API-Version, X-Experience-API-Consistent-Through"[..]).unwrap(),
            Header::from_bytes(&b"Date"[..], format_http_date().as_bytes()).unwrap(),
        ],
        Cursor::new(final_body),
        Some(body.len()),
        None,
    );
    resp
}

fn make_response_empty(status_code: u16) -> Response<Cursor<Vec<u8>>> {
    Response::new(
        StatusCode(status_code),
        vec![
            Header::from_bytes(&b"X-Experience-API-Version"[..], &b"1.0.3"[..]).unwrap(),
            Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap(),
            Header::from_bytes(&b"Access-Control-Allow-Headers"[..], &b"Content-Type, Authorization, X-Experience-API-Version, If-Match, If-None-Match, Access-Control-Request-Private-Network"[..]).unwrap(),
            Header::from_bytes(&b"Access-Control-Allow-Methods"[..], &b"GET, POST, PUT, DELETE, HEAD, OPTIONS"[..]).unwrap(),
            Header::from_bytes(&b"Access-Control-Expose-Headers"[..], &b"ETag, Last-Modified, X-Experience-API-Version, X-Experience-API-Consistent-Through"[..]).unwrap(),
            Header::from_bytes(&b"Date"[..], format_http_date().as_bytes()).unwrap(),
        ],
        Cursor::new(Vec::new()),
        Some(0),
        None,
    )
}

fn make_error_response(status_code: u16, msg: &str) -> Response<Cursor<Vec<u8>>> {
    let err_body = format!("{{\"error\":\"{}\"}}", msg);
    Response::new(
        StatusCode(status_code),
        vec![
            Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap(),
            Header::from_bytes(&b"X-Experience-API-Version"[..], &b"1.0.3"[..]).unwrap(),
            Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap(),
            Header::from_bytes(&b"Access-Control-Allow-Headers"[..], &b"Content-Type, Authorization, X-Experience-API-Version, If-Match, If-None-Match, Access-Control-Request-Private-Network"[..]).unwrap(),
            Header::from_bytes(&b"Access-Control-Allow-Methods"[..], &b"GET, POST, PUT, DELETE, HEAD, OPTIONS"[..]).unwrap(),
            Header::from_bytes(&b"Access-Control-Expose-Headers"[..], &b"ETag, Last-Modified, X-Experience-API-Version, X-Experience-API-Consistent-Through"[..]).unwrap(),
            Header::from_bytes(&b"Date"[..], format_http_date().as_bytes()).unwrap(),
        ],
        Cursor::new(err_body.into_bytes()),
        None,
        None,
    )
}

fn format_http_date() -> String {
    let now = std::time::SystemTime::now();
    humantime::format_rfc3339_seconds(now).to_string()
}

// -----------------------------------------------------------------------------
// URL & QUERY PARSING HELPERS
// -----------------------------------------------------------------------------
fn parse_url(url: &str) -> (String, String) {
    if let Some(pos) = url.find('?') {
        (url[..pos].to_string(), url[pos + 1..].to_string())
    } else {
        (url.to_string(), String::new())
    }
}

fn parse_query(query: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let mut parts = pair.splitn(2, '=');
        let key = parts.next().unwrap_or_default();
        let val = parts.next().unwrap_or_default();
        let decoded_k = urlencoding::decode(key).unwrap_or(std::borrow::Cow::Borrowed(key)).to_string();
        let decoded_v = urlencoding::decode(val).unwrap_or(std::borrow::Cow::Borrowed(val)).to_string();
        map.insert(decoded_k, decoded_v);
    }
    map
}

fn base64_decode(input: &str) -> Result<Vec<u8>, ()> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut buf = Vec::new();
    let mut acc = 0u32;
    let mut bits = 0;
    for &b in input.as_bytes() {
        if b == b'=' || b.is_ascii_whitespace() {
            continue;
        }
        let val = TABLE.iter().position(|&x| x == b).ok_or(())? as u32;
        acc = (acc << 6) | val;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            buf.push(((acc >> bits) & 0xFF) as u8);
        }
    }
    Ok(buf)
}

fn handle_registration_export(
    method: Method,
    path: &str,
    store: &LrsStore,
) -> Response<Cursor<Vec<u8>>> {
    if method != Method::Get {
        return make_error_response(405, "Method Not Allowed");
    }
    // Path format: /registrations/<uuid>/export
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() != 3 || parts[0] != "registrations" || parts[2] != "export" {
        return make_error_response(400, "Invalid export path format: expected /registrations/{uuid}/export");
    }
    let reg_uuid_str = parts[1];
    let reg_pair = match uuid_to_u64_pair(reg_uuid_str) {
        Ok(p) => p,
        Err(_) => return make_error_response(400, "Invalid registration UUID"),
    };

    match store.export_registration_xapi_json(reg_pair) {
        Ok(json_str) => {
            make_response(200, "application/json; charset=utf-8", json_str.as_bytes(), false)
        }
        Err(e) => make_error_response(500, &format!("Export failed: {}", e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_usage_tier_multi_archive_routing_and_scopes() {
        let researcher = AuthCredential {
            user: "researcher".to_string(),
            pass: "researcher123".to_string(),
            scopes: vec!["all".to_string()],
            archive: Some("scholar.s3a".to_string()),
        };
        let explorer = AuthCredential {
            user: "explorer".to_string(),
            pass: "explorer123".to_string(),
            scopes: vec!["statements/write".to_string()],
            archive: Some("usage.s3a".to_string()),
        };

        // Explorer can write but CANNOT read others
        assert!(explorer.has_scope("statements/write"));
        assert!(!explorer.has_scope("statements/read"));
        assert!(!explorer.has_scope("statements/read/mine"));
        assert!(!explorer.has_scope("state"));
        assert!(!explorer.has_scope("profile"));

        // Researcher has all scopes
        assert!(researcher.has_scope("statements/write"));
        assert!(researcher.has_scope("statements/read"));
        assert!(researcher.has_scope("all"));

        let temp_dir = std::env::temp_dir();
        let default_path = temp_dir.join("test_routing_default.s3a");
        let audit_path = temp_dir.join("test_routing_audit.s3a");

        let default_store = LrsStore::open_or_create(&default_path).unwrap();
        let audit_writer = Arc::new(ConcurrentTileWriter::create(&audit_path).unwrap());
        let stores = Arc::new(std::sync::RwLock::new(HashMap::new()));
        stores.write().unwrap().insert("test_routing_default.s3a".to_string(), default_store.clone());

        let state = Arc::new(AppState {
            default_store: default_store.clone(),
            store: default_store,
            stores,
            audit_writer,
            config: ServerConfig::default(),
            agent_registry: Vec::new(),
        });

        let empty_params = HashMap::new();
        let empty_headers = HashMap::new();

        // 1. Researcher credential resolves to scholar.s3a
        let (_res_store, res_archive) = state.resolve_store_and_archive(Some(&researcher), &empty_params, &empty_headers);
        assert_eq!(res_archive, "scholar.s3a");

        // 2. Explorer credential resolves to usage.s3a
        let (_exp_store, exp_archive) = state.resolve_store_and_archive(Some(&explorer), &empty_params, &empty_headers);
        assert_eq!(exp_archive, "usage.s3a");

        let _ = std::fs::remove_file(&default_path);
        let _ = std::fs::remove_file(&audit_path);
    }

    #[test]
    fn test_usage_archive_strict_profile_validation() {
        let temp_dir = std::env::temp_dir();
        let usage_path = temp_dir.join("test_usage_strict.s3a");
        let audit_path = temp_dir.join("test_usage_audit.s3a");

        let usage_store = LrsStore::open_or_create(&usage_path).unwrap();
        let audit_writer = Arc::new(ConcurrentTileWriter::create(&audit_path).unwrap());
        let stores = Arc::new(std::sync::RwLock::new(HashMap::new()));
        stores.write().unwrap().insert("usage.s3a".to_string(), usage_store.clone());

        let state = Arc::new(AppState {
            default_store: usage_store.clone(),
            store: usage_store.clone(),
            stores,
            audit_writer,
            config: ServerConfig::default(),
            agent_registry: Vec::new(),
        });

        let params = HashMap::new();
        let headers = HashMap::new();

        // 1. All 5 permitted usage profile verbs must be accepted
        let usage_verbs = [
            s3a_pack_scholar::xapi::USAGE_VERB_SEARCHED,
            s3a_pack_scholar::xapi::USAGE_VERB_PROMPTED,
            s3a_pack_scholar::xapi::USAGE_VERB_AI_RESPONDED,
            s3a_pack_scholar::xapi::USAGE_VERB_VIEWED_SOURCE,
            s3a_pack_scholar::xapi::USAGE_VERB_EXPORTED,
        ];
        for &verb in &usage_verbs {
            let stmt = serde_json::json!({
                "actor": { "account": { "homePage": "https://ashecribb.dev/explorer", "name": "device-001" } },
                "verb": { "id": verb },
                "object": { "id": "https://example.com/paper/123" },
                "context": {
                    "contextActivities": {
                        "category": [{ "id": s3a_pack_scholar::xapi::SCHOLAR_EXPLORER_USAGE_PROFILE_IRI }]
                    }
                }
            });
            let body = serde_json::to_vec(&stmt).unwrap();
            let resp = handle_statements(Method::Post, &params, &headers, &body, &state, &usage_store, "usage.s3a");
            assert_eq!(resp.status_code().0, 200, "Permitted verb '{}' must be accepted with 200", verb);
        }

        // 2. Removed and other non-usage verbs must be rejected with 400
        let rejected_verbs = [
            "https://ashecribb.dev/xapi/verbs/reviewed",
            "https://ashecribb.dev/xapi/verbs/verified-claim",
            "https://ashecribb.dev/xapi/verbs/raise-hand-to-ta",
            "http://adlnet.gov/expapi/verbs/completed",
            "http://adlnet.gov/expapi/verbs/experienced",
        ];
        for &verb in &rejected_verbs {
            let stmt = serde_json::json!({
                "actor": { "account": { "homePage": "https://ashecribb.dev/explorer", "name": "device-001" } },
                "verb": { "id": verb },
                "object": { "id": "https://example.com/paper/123" },
                "context": {
                    "contextActivities": {
                        "category": [{ "id": s3a_pack_scholar::xapi::SCHOLAR_EXPLORER_USAGE_PROFILE_IRI }]
                    }
                }
            });
            let body = serde_json::to_vec(&stmt).unwrap();
            let resp = handle_statements(Method::Post, &params, &headers, &body, &state, &usage_store, "usage.s3a");
            assert_eq!(resp.status_code().0, 400, "Unpermitted verb '{}' must be rejected with 400", verb);
        }

        // 3. Statement with usage profile category via PUT
        let put_id = s3a_engine::lrs_store::generate_uuid_v4();
        let mut put_params = HashMap::new();
        put_params.insert("statementId".to_string(), put_id.clone());

        let valid_put = serde_json::json!({
            "id": put_id,
            "actor": { "account": { "homePage": "https://ashecribb.dev/explorer", "name": "device-001" } },
            "verb": { "id": s3a_pack_scholar::xapi::USAGE_VERB_SEARCHED },
            "object": { "id": "https://example.com/search/456" },
            "context": {
                "contextActivities": {
                    "category": [{ "id": s3a_pack_scholar::xapi::SCHOLAR_EXPLORER_USAGE_PROFILE_IRI }]
                }
            }
        });
        let body_put = serde_json::to_vec(&valid_put).unwrap();

        let resp_put_ok = handle_statements(Method::Put, &put_params, &headers, &body_put, &state, &usage_store, "usage.s3a");
        assert_eq!(resp_put_ok.status_code().0, 204);

        let _ = std::fs::remove_file(&usage_path);
        let _ = std::fs::remove_file(&audit_path);
    }

    #[test]
    fn test_admin_delete_by_actor_account_and_retention_purge() {
        let temp_dir = std::env::temp_dir();
        let usage_path = temp_dir.join("test_admin_usage.s3a");
        let audit_path = temp_dir.join("test_admin_audit.s3a");

        let usage_store = LrsStore::open_or_create(&usage_path).unwrap();
        let audit_writer = Arc::new(ConcurrentTileWriter::create(&audit_path).unwrap());
        let stores = Arc::new(std::sync::RwLock::new(HashMap::new()));
        stores.write().unwrap().insert("test_admin_usage.s3a".to_string(), usage_store.clone());

        let mut config = ServerConfig::default();
        config.insecure_dev = true;

        let state = Arc::new(AppState {
            default_store: usage_store.clone(),
            store: usage_store.clone(),
            stores,
            audit_writer,
            config,
            agent_registry: Vec::new(),
        });

        // Seed statement for device-777
        let dev777_stmt = serde_json::json!({
            "actor": { "account": { "homePage": "https://ashecribb.dev/explorer", "name": "device-777" } },
            "verb": { "id": s3a_pack_scholar::xapi::USAGE_VERB_SEARCHED },
            "object": { "id": "https://example.com/paper/777" },
            "context": {
                "contextActivities": {
                    "category": [{ "id": s3a_pack_scholar::xapi::SCHOLAR_EXPLORER_USAGE_PROFILE_IRI }]
                }
            }
        });
        let _ = usage_store.store_statement(&dev777_stmt.to_string(), &[]).unwrap();

        // 1. Purge by actor account / device ID via /admin/delete-by-actor-account
        let purge_req = serde_json::to_vec(&serde_json::json!({
            "deviceId": "device-777",
            "archive": "test_admin_usage.s3a"
        })).unwrap();

        let resp = learner_api::handle_learner_api(
            &Method::Post,
            "/admin/delete-by-actor-account",
            &HashMap::new(),
            &HashMap::new(),
            &purge_req,
            &state,
        );
        assert_eq!(resp.status_code().0, 200);

        // 2. Compaction & retention purge via /admin/compact
        let compact_req = serde_json::to_vec(&serde_json::json!({
            "retention_days": 90,
            "archive": "test_admin_usage.s3a"
        })).unwrap();

        let resp_compact = learner_api::handle_learner_api(
            &Method::Post,
            "/admin/compact",
            &HashMap::new(),
            &HashMap::new(),
            &compact_req,
            &state,
        );
        assert_eq!(resp_compact.status_code().0, 200);

        let _ = std::fs::remove_file(&usage_path);
        let _ = std::fs::remove_file(&audit_path);
    }

    #[test]
    fn test_cors_preflight_private_network_and_expose_headers() {
        // 1. Preflight with Access-Control-Request-Private-Network: true
        let mut preflight_headers = HashMap::new();
        preflight_headers.insert("access-control-request-private-network".to_string(), "true".to_string());
        preflight_headers.insert("origin".to_string(), "http://localhost:3000".to_string());

        let preflight_resp = build_options_response(&preflight_headers);
        assert_eq!(preflight_resp.status_code().0, 204);

        let mut has_allow_private = false;
        let mut has_expose_headers = false;
        for h in preflight_resp.headers() {
            let field_str = h.field.to_string();
            let value_str = h.value.to_string();
            if field_str.eq_ignore_ascii_case("Access-Control-Allow-Private-Network") {
                assert_eq!(value_str, "true");
                has_allow_private = true;
            }
            if field_str.eq_ignore_ascii_case("Access-Control-Expose-Headers") {
                assert!(value_str.contains("ETag"));
                assert!(value_str.contains("Last-Modified"));
                assert!(value_str.contains("X-Experience-API-Version"));
                assert!(value_str.contains("X-Experience-API-Consistent-Through"));
                has_expose_headers = true;
            }
        }
        assert!(has_allow_private, "Preflight must respond with Access-Control-Allow-Private-Network: true");
        assert!(has_expose_headers, "Preflight must respond with Access-Control-Expose-Headers");

        // 2. Preflight without Access-Control-Request-Private-Network
        let preflight_normal_headers = HashMap::new();
        let preflight_normal_resp = build_options_response(&preflight_normal_headers);
        assert_eq!(preflight_normal_resp.status_code().0, 204);
        for h in preflight_normal_resp.headers() {
            assert!(
                !h.field.to_string().eq_ignore_ascii_case("Access-Control-Allow-Private-Network"),
                "Preflight without private network request must not return Access-Control-Allow-Private-Network"
            );
        }

        // 3. Verify all standard responses include Access-Control-Expose-Headers
        let ok_resp = make_response(200, "application/json", b"{}", false);
        let mut expose_found = false;
        for h in ok_resp.headers() {
            if h.field.to_string().eq_ignore_ascii_case("Access-Control-Expose-Headers") {
                assert_eq!(h.value.to_string(), "ETag, Last-Modified, X-Experience-API-Version, X-Experience-API-Consistent-Through");
                expose_found = true;
            }
        }
        assert!(expose_found, "make_response must include Access-Control-Expose-Headers");

        let empty_resp = make_response_empty(204);
        let mut empty_expose_found = false;
        for h in empty_resp.headers() {
            if h.field.to_string().eq_ignore_ascii_case("Access-Control-Expose-Headers") {
                assert_eq!(h.value.to_string(), "ETag, Last-Modified, X-Experience-API-Version, X-Experience-API-Consistent-Through");
                empty_expose_found = true;
            }
        }
        assert!(empty_expose_found, "make_response_empty must include Access-Control-Expose-Headers");

        let err_resp = make_error_response(400, "Bad Request");
        let mut err_expose_found = false;
        for h in err_resp.headers() {
            if h.field.to_string().eq_ignore_ascii_case("Access-Control-Expose-Headers") {
                assert_eq!(h.value.to_string(), "ETag, Last-Modified, X-Experience-API-Version, X-Experience-API-Consistent-Through");
                err_expose_found = true;
            }
        }
        assert!(err_expose_found, "make_error_response must include Access-Control-Expose-Headers");
    }

    #[test]
    fn test_daily_anchor_job_creates_snapshot_and_anchored_statement() {
        use s3a_engine::anchor::mock_calendar::MockOtsCalendar;

        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("s3a_anchor_job_{}", unique));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let archive_path = temp_dir.join("lrs_data.s3a");
        let audit_path = temp_dir.join("lrs_data.audit.s3a");

        let store = LrsStore::open_or_create(&archive_path).expect("open lrs store");

        // Store a sample statement
        let initial_stmt = serde_json::json!({
            "id": s3a_engine::lrs_store::generate_uuid_v4(),
            "actor": { "mbox": "mailto:test@example.com" },
            "verb": { "id": "http://adlnet.gov/expapi/verbs/experienced" },
            "object": { "id": "http://example.com/activity/1" }
        });
        let initial_json = serde_json::to_string(&initial_stmt).unwrap();
        store.store_statement(&initial_json, &[]).expect("store statement");

        // Start mock OTS calendars
        let cal1 = MockOtsCalendar::start();
        let cal2 = MockOtsCalendar::start();
        let cals = vec![cal1.url(), cal2.url()];

        let (snap_path, audit_snap_path, fp_hex) =
            run_daily_anchor_job(&store, &archive_path, &audit_path, Some(&cals), None)
                .expect("run daily anchor job");

        assert!(snap_path.exists());
        assert!(audit_snap_path.exists());

        // Verify sealed snapshot
        let rep = s3a_engine::verify_sealed_archive(&snap_path, None).expect("verify snapshot");
        assert!(rep.is_sealed);
        assert_eq!(rep.anchors.len(), 2);
        for a in &rep.anchors {
            assert_eq!(a.fingerprint, fp_hex);
            assert_eq!(a.status, "pending");
        }

        // Verify the store has the xAPI "anchored" statement
        let page = store.query_statements(StatementFilter::default()).expect("query");
        let anchored_stmt = page.statements.iter().find_map(|s| {
            let v: serde_json::Value = serde_json::from_str(s).ok()?;
            if v["verb"]["id"].as_str() == Some("https://ashecribb.dev/xapi/verbs/anchored") {
                Some(v)
            } else {
                None
            }
        });
        assert!(anchored_stmt.is_some(), "Store must contain an anchored statement");
        let ext_fp = anchored_stmt.unwrap()["object"]["definition"]["extensions"]["https://ashecribb.dev/xapi/ext/fingerprint"].as_str().unwrap().to_string();
        assert_eq!(ext_fp, fp_hex);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
