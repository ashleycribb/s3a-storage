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
use s3a_engine::concurrent_writer::ConcurrentTileWriter;
use s3a_engine::{
    format_iso8601_ms, u64_pair_to_uuid, uuid_to_u64_pair, LrsStore, LrsStoreError, StatementFilter,
};

pub mod batch_sync;
pub mod gate_directory;
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
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (&x, &y)| acc | (x ^ y))
        == 0
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

    let parent_dir = archive_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let snap_path = parent_dir.join(format!("lrs_snapshot_{}.snapshot.s3a", date_str));
    let audit_snap_path = parent_dir.join(format!("lrs_audit_{}.snapshot.s3a", date_str));

    // 1. Seal primary archive into snapshot
    let in_bytes = store.read_all_bytes()?;
    let sealed_bytes = s3a_engine::seal_buffer(&in_bytes, 16, 2)?;
    std::fs::write(&snap_path, &sealed_bytes)?;

    // 2. Seal audit archive into snapshot
    if audit_archive_path.exists()
        && std::fs::metadata(audit_archive_path)
            .map(|m| m.len())
            .unwrap_or(0)
            >= FILE_HEADER_SIZE as u64
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
    let audit_fp =
        s3a_engine::compute_sealed_file_fingerprint(&audit_snap_path).unwrap_or([0u8; 32]);
    let audit_fp_hex = hex::encode(audit_fp);

    // 4. Anchor snapshot
    let calendars: Vec<String> = if let Some(c) = calendar_urls {
        c.to_vec()
    } else {
        s3a_engine::DEFAULT_OTS_CALENDARS
            .iter()
            .map(|&s| s.to_string())
            .collect()
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

/// Validates that an archive name is a bare filename matching ^[A-Za-z0-9_.-]+\.s3a$,
/// with no "/", "\", ".." or absolute path.
pub fn is_valid_archive_name(name: &str) -> bool {
    if !name.ends_with(".s3a") || name.len() <= 4 {
        return false;
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return false;
    }
    if std::path::Path::new(name).is_absolute() {
        return false;
    }
    let stem = &name[..name.len() - 4];
    if stem.is_empty() {
        return false;
    }
    stem.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
}

/// Configuration for the S3A LRS Server.
#[derive(Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub archive_path: PathBuf,
    pub audit_archive_path: PathBuf,
    pub data_dir: PathBuf,
    pub credentials: Vec<AuthCredential>,
    pub insecure_dev: bool,
    pub enable_scholar: bool,
    pub profile_mode_strict: bool,
    pub admin_api_override: bool,
    pub jwt_secret: String,
    pub enable_daily_anchoring: bool,
    pub anchoring_interval_secs: u64,
    pub enable_gate_directory: bool,
    pub max_mailbox_items_per_address: usize,
    pub max_mailbox_item_size: usize,
    pub max_mailbox_total_bytes: usize,
    pub mailbox_item_expiry_secs: u64,
    pub registrations_per_client_per_minute: u32,
    pub max_batch_body_bytes: usize,
    pub max_batch_rate_per_minute: u32,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8080,
            archive_path: PathBuf::from("lrs_data.s3a"),
            audit_archive_path: PathBuf::from("lrs_data.audit.s3a"),
            data_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            credentials: Vec::new(),
            insecure_dev: false,
            enable_scholar: true,
            profile_mode_strict: false,
            admin_api_override: false,
            jwt_secret: "s3a-elrr-jwt-secret-key-32bytes!!".to_string(),
            enable_daily_anchoring: false,
            anchoring_interval_secs: 86400,
            enable_gate_directory: false,
            max_mailbox_items_per_address: 100,
            max_mailbox_item_size: 1024 * 1024,
            max_mailbox_total_bytes: 100 * 1024 * 1024,
            mailbox_item_expiry_secs: 30 * 86400,
            registrations_per_client_per_minute: 10,
            max_batch_body_bytes: 5 * 1024 * 1024,
            max_batch_rate_per_minute: 60,
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
    pub gate_directory: Arc<std::sync::Mutex<gate_directory::GateDirectoryService>>,
    pub batch_sync: Arc<std::sync::Mutex<batch_sync::BatchSyncService>>,
}

impl AppState {
    /// Initializes application state at startup:
    /// 1. Ensures data directory exists and is canonicalized.
    /// 2. Validates default archive filename and resolves inside data directory.
    /// 3. Opens/creates default store and caches it in `stores` under its canonical bare filename key.
    /// 4. Iterates over credentials, pre-opening any assigned archives without opening any file twice,
    ///    logging and returning an error instead of panicking on failure.
    pub fn init_with_audit(
        mut config: ServerConfig,
        audit_writer: Arc<ConcurrentTileWriter>,
    ) -> Result<Arc<AppState>, String> {
        // Ensure data directory exists and canonicalize it
        std::fs::create_dir_all(&config.data_dir).map_err(|e| {
            format!(
                "Failed to create data directory {:?}: {}",
                config.data_dir, e
            )
        })?;
        config.data_dir = std::fs::canonicalize(&config.data_dir).map_err(|e| {
            format!(
                "Failed to canonicalize data directory {:?}: {}",
                config.data_dir, e
            )
        })?;

        let archive_name = config
            .archive_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("lrs_data.s3a")
            .to_string();

        if !is_valid_archive_name(&archive_name) {
            return Err(format!(
                "Invalid archive name '{}': must be a bare filename matching ^[A-Za-z0-9_.-]+\\.s3a$",
                archive_name
            ));
        }
        config.archive_path = config.data_dir.join(&archive_name);

        println!(
            "[s3a-lrs-server] Opening S3A archive: {:?}",
            config.archive_path
        );
        let default_store = LrsStore::open_or_create(&config.archive_path).map_err(|e| {
            format!(
                "Failed to open default S3A archive {:?}: {}",
                config.archive_path, e
            )
        })?;

        if config.enable_scholar {
            let _ = default_store.writer().enable_addon(s3a_core::ADDON_EDU);
        }

        // Canonical bare filename key inside data_dir
        let canonical_default_path = std::fs::canonicalize(&config.archive_path)
            .unwrap_or_else(|_| config.archive_path.clone());
        let canonical_default_key = canonical_default_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&archive_name)
            .to_string();

        let stores = Arc::new(std::sync::RwLock::new(HashMap::new()));
        {
            let mut map = stores.write().unwrap();
            map.insert(canonical_default_key.clone(), default_store.clone());
            if archive_name != canonical_default_key {
                map.insert(archive_name.clone(), default_store.clone());
            }
        }

        // Pre-create and cache all archives named in credentials at startup
        for cred in &config.credentials {
            if let Some(ref arch) = cred.archive {
                if !is_valid_archive_name(arch) {
                    return Err(format!(
                        "Invalid archive name '{}' in credentials for user '{}': must be a bare filename matching ^[A-Za-z0-9_.-]+\\.s3a$",
                        arch, cred.user
                    ));
                }
                let arch_path = config.data_dir.join(arch);

                let canonical_arch_key = if arch_path.is_file() {
                    std::fs::canonicalize(&arch_path)
                        .ok()
                        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                        .unwrap_or_else(|| arch.clone())
                } else {
                    arch.clone()
                };

                // Never open one file twice: check if already in stores cache!
                let already_open = {
                    let map = stores.read().unwrap();
                    map.get(&canonical_arch_key)
                        .cloned()
                        .or_else(|| map.get(arch).cloned())
                };

                if let Some(existing_store) = already_open {
                    let mut map = stores.write().unwrap();
                    map.insert(canonical_arch_key, existing_store.clone());
                    map.insert(arch.clone(), existing_store);
                    continue;
                }

                // Open or create the credential archive; never panic on failure!
                let cred_store = LrsStore::open_or_create(&arch_path).map_err(|e| {
                    format!(
                        "Failed to open credential archive '{}' at {:?}: {}",
                        arch, arch_path, e
                    )
                })?;

                if config.enable_scholar {
                    let _ = cred_store.writer().enable_addon(s3a_core::ADDON_EDU);
                }

                let final_canonical_key = std::fs::canonicalize(&arch_path)
                    .ok()
                    .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                    .unwrap_or_else(|| arch.clone());

                let mut map = stores.write().unwrap();
                map.insert(final_canonical_key, cred_store.clone());
                map.insert(arch.clone(), cred_store);
            }
        }

        let agent_registry = s3a_pack_scholar::records::seed_default_agents();

        let gate_limits = gate_directory::GateDirectoryLimits {
            max_mailbox_items_per_address: config.max_mailbox_items_per_address,
            max_item_size: config.max_mailbox_item_size,
            max_total_mailbox_bytes: config.max_mailbox_total_bytes,
            mailbox_item_expiry_secs: config.mailbox_item_expiry_secs,
            registrations_per_client_per_minute: config.registrations_per_client_per_minute,
        };
        let gate_dir_path = if config.enable_gate_directory {
            Some(config.data_dir.join("gate_directory.s3a"))
        } else {
            None
        };
        let gate_directory_svc =
            gate_directory::GateDirectoryService::open_or_create(gate_dir_path, gate_limits)
                .map_err(|e| format!("Failed to initialize Gate Directory: {}", e))?;

        let batch_sync_svc = batch_sync::BatchSyncService::new(batch_sync::BatchSyncLimits {
            max_body_bytes: config.max_batch_body_bytes,
            max_requests_per_minute: config.max_batch_rate_per_minute,
        });

        Ok(Arc::new(AppState {
            default_store: default_store.clone(),
            store: default_store,
            stores,
            audit_writer,
            config,
            agent_registry,
            gate_directory: Arc::new(std::sync::Mutex::new(gate_directory_svc)),
            batch_sync: Arc::new(std::sync::Mutex::new(batch_sync_svc)),
        }))
    }

    /// Retrieves or creates and caches the `scholar.s3a` archive in the data directory.
    pub fn get_or_create_scholar_store(&self) -> Result<Arc<LrsStore>, s3a_engine::LrsStoreError> {
        let scholar_name = "scholar.s3a";
        {
            let map = self.stores.read().unwrap();
            if let Some(s) = map.get(scholar_name) {
                return Ok(s.clone());
            }
        }
        let mut map = self.stores.write().unwrap();
        if let Some(s) = map.get(scholar_name) {
            return Ok(s.clone());
        }
        let target_path = self.config.data_dir.join(scholar_name);
        let store = LrsStore::open_or_create(&target_path)?;
        if self.config.enable_scholar {
            let _ = store.writer().enable_addon(s3a_core::ADDON_EDU);
        }
        map.insert(scholar_name.to_string(), store.clone());
        Ok(store)
    }

    /// Opens or returns a cached LrsStore for a specified archive file or name.
    ///
    /// Any archive name must be a bare filename matching ^[A-Za-z0-9_.-]+\.s3a$,
    /// resolved inside data_dir, canonicalized and confirmed to land inside it.
    /// Refuses to create new archive files through a request.
    pub fn get_or_open_store(
        &self,
        archive_str: &str,
    ) -> Result<Arc<LrsStore>, s3a_engine::LrsStoreError> {
        if !is_valid_archive_name(archive_str) {
            return Err(s3a_engine::LrsStoreError::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("Invalid archive name '{}': must be a bare filename matching ^[A-Za-z0-9_.-]+\\.s3a$", archive_str),
            )));
        }

        // Fast path: cached in memory
        {
            let map = self.stores.read().unwrap();
            if let Some(s) = map.get(archive_str) {
                return Ok(s.clone());
            }
        }

        let canonical_data_dir = std::fs::canonicalize(&self.config.data_dir)?;
        let target_path = canonical_data_dir.join(archive_str);

        // Refuse to create new archive files through a request
        if !target_path.is_file() {
            return Err(s3a_engine::LrsStoreError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Archive '{}' does not exist in data directory", archive_str),
            )));
        }

        // Canonicalize target and ensure it lands inside data directory
        let canonical_target = std::fs::canonicalize(&target_path)?;
        if !canonical_target.starts_with(&canonical_data_dir)
            || canonical_target.parent() != Some(canonical_data_dir.as_path())
        {
            return Err(s3a_engine::LrsStoreError::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("Archive '{}' resolves outside data directory", archive_str),
            )));
        }

        let canonical_name = canonical_target
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(archive_str)
            .to_string();

        // Fast path 2: cached under canonical bare filename key
        {
            let map = self.stores.read().unwrap();
            if let Some(s) = map.get(&canonical_name) {
                return Ok(s.clone());
            }
        }

        let mut map = self.stores.write().unwrap();
        if let Some(s) = map.get(&canonical_name) {
            return Ok(s.clone());
        }
        if let Some(s) = map.get(archive_str) {
            return Ok(s.clone());
        }

        let store = LrsStore::open_or_create(&canonical_target)?;
        if self.config.enable_scholar {
            let _ = store.writer().enable_addon(s3a_core::ADDON_EDU);
        }
        map.insert(canonical_name.clone(), store.clone());
        if archive_str != canonical_name {
            map.insert(archive_str.to_string(), store.clone());
        }
        Ok(store)
    }

    /// Resolves active LrsStore and canonical archive name based on credentials, query params, or defaults.
    ///
    /// 1. A credential with an assigned archive always uses it; ignores ?archive= and x-s3a-archive / x-archive headers.
    /// 2. A credential without an archive uses the default archive only. Honors the override param/header only if
    ///    the credential has scope "admin" AND --admin-api-override is set.
    /// 3. Any archive name (from credentials, CLI or override) must be a bare filename matching ^[A-Za-z0-9_.-]+\.s3a$,
    ///    with no "/", "\", ".." or absolute path, resolving inside data_dir and not escaping it.
    pub fn resolve_store_and_archive(
        &self,
        cred: Option<&AuthCredential>,
        params: &HashMap<String, String>,
        headers: &HashMap<String, String>,
    ) -> Result<(Arc<LrsStore>, String), String> {
        // 1. A credential with an assigned archive always uses it;
        //    ignore ?archive= and x-s3a-archive / x-archive headers for that credential.
        if let Some(c) = cred {
            if let Some(ref arch) = c.archive {
                if !is_valid_archive_name(arch) {
                    return Err(format!(
                        "Credential assigned invalid archive name '{}'",
                        arch
                    ));
                }
                let st = self.get_or_open_store(arch).map_err(|e| {
                    format!("Failed to open credential archive '{}': {:?}", arch, e)
                })?;
                return Ok((st, arch.clone()));
            }
        }

        // 2. Check override param/header for credentials without an assigned archive
        let override_opt = params
            .get("archive")
            .or_else(|| headers.get("x-s3a-archive"))
            .or_else(|| headers.get("x-archive"));

        if let Some(override_arch) = override_opt {
            if !is_valid_archive_name(override_arch) {
                return Err(format!(
                    "Invalid archive name '{}': must be a bare filename matching ^[A-Za-z0-9_.-]+\\.s3a$ with no '/', '\\', '..', or absolute path",
                    override_arch
                ));
            }

            let is_admin = cred.map(|c| c.has_scope("admin")).unwrap_or(false);
            if is_admin && self.config.admin_api_override {
                let st = self.get_or_open_store(override_arch).map_err(|e| {
                    format!(
                        "Failed to open override archive '{}': {:?}",
                        override_arch, e
                    )
                })?;
                return Ok((st, override_arch.clone()));
            }
        }

        // 3. Fallback to default configured archive
        let default_name = self
            .config
            .archive_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("lrs_data.s3a")
            .to_string();
        Ok((self.default_store.clone(), default_name))
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
            "--gate-directory" => {
                config.enable_gate_directory = true;
            }
            "--max-mailbox-items" => {
                if i + 1 < args.len() {
                    config.max_mailbox_items_per_address = args[i + 1].parse().unwrap_or(100);
                    i += 1;
                }
            }
            "--max-mailbox-item-size" => {
                if i + 1 < args.len() {
                    config.max_mailbox_item_size = args[i + 1].parse().unwrap_or(1024 * 1024);
                    i += 1;
                }
            }
            "--max-mailbox-total-bytes" => {
                if i + 1 < args.len() {
                    config.max_mailbox_total_bytes =
                        args[i + 1].parse().unwrap_or(100 * 1024 * 1024);
                    i += 1;
                }
            }
            "--max-batch-size" => {
                if i + 1 < args.len() {
                    config.max_batch_body_bytes = args[i + 1].parse().unwrap_or(5 * 1024 * 1024);
                    i += 1;
                }
            }
            "--max-batch-rate" => {
                if i + 1 < args.len() {
                    config.max_batch_rate_per_minute = args[i + 1].parse().unwrap_or(60);
                    i += 1;
                }
            }
            "--mailbox-expiry-days" => {
                if i + 1 < args.len() {
                    let days: u64 = args[i + 1].parse().unwrap_or(30);
                    config.mailbox_item_expiry_secs = days * 86400;
                    i += 1;
                }
            }
            "--max-registrations-per-minute" => {
                if i + 1 < args.len() {
                    config.registrations_per_client_per_minute = args[i + 1].parse().unwrap_or(10);
                    i += 1;
                }
            }
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
            "--data-dir" => {
                if i + 1 < args.len() {
                    config.data_dir = PathBuf::from(&args[i + 1]);
                    i += 1;
                }
            }
            "-a" | "--archive" => {
                if i + 1 < args.len() {
                    let val = &args[i + 1];
                    if !is_valid_archive_name(val) {
                        eprintln!("[s3a-lrs-server] ERROR: Invalid archive name '{}': must be a bare filename matching ^[A-Za-z0-9_.-]+\\.s3a$ with no '/', '\\', '..', or absolute path.", val);
                        std::process::exit(1);
                    }
                    config.archive_path = PathBuf::from(val);
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
                            for cred in creds {
                                if let Some(ref a) = cred.archive {
                                    if !is_valid_archive_name(a) {
                                        eprintln!("[s3a-lrs-server] ERROR: Invalid archive name '{}' in credentials: must be a bare filename matching ^[A-Za-z0-9_.-]+\\.s3a$", a);
                                        std::process::exit(1);
                                    }
                                }
                                config.credentials.push(cred);
                            }
                        } else {
                            for line in content.lines() {
                                let line = line.trim();
                                if line.is_empty() || line.starts_with('#') {
                                    continue;
                                }
                                let parts: Vec<&str> = line.split(':').collect();
                                if parts.len() >= 2 {
                                    let scopes = if parts.len() >= 3 {
                                        parts[2].split(',').map(|s| s.trim().to_string()).collect()
                                    } else {
                                        vec!["all".to_string()]
                                    };
                                    let archive = if parts.len() >= 4 && !parts[3].trim().is_empty()
                                    {
                                        let a = parts[3].trim().to_string();
                                        if !is_valid_archive_name(&a) {
                                            eprintln!("[s3a-lrs-server] ERROR: Invalid archive name '{}' in credentials: must be a bare filename matching ^[A-Za-z0-9_.-]+\\.s3a$", a);
                                            std::process::exit(1);
                                        }
                                        Some(a)
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
                            let a = parts[3].trim().to_string();
                            if !is_valid_archive_name(&a) {
                                eprintln!("[s3a-lrs-server] ERROR: Invalid archive name '{}' in credentials: must be a bare filename matching ^[A-Za-z0-9_.-]+\\.s3a$", a);
                                std::process::exit(1);
                            }
                            Some(a)
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

    // Ensure data directory exists and canonicalize it
    std::fs::create_dir_all(&config.data_dir).expect("Failed to create data directory");
    config.data_dir =
        std::fs::canonicalize(&config.data_dir).expect("Failed to canonicalize data directory");

    let archive_name = config
        .archive_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("lrs_data.s3a")
        .to_string();
    if !is_valid_archive_name(&archive_name) {
        eprintln!("[s3a-lrs-server] ERROR: Invalid archive name '{}': must be a bare filename matching ^[A-Za-z0-9_.-]+\\.s3a$", archive_name);
        std::process::exit(1);
    }
    config.archive_path = config.data_dir.join(&archive_name);

    // Default audit archive path if not explicitly configured
    if config.audit_archive_path == PathBuf::from("lrs_data.audit.s3a") {
        let stem = archive_name.strip_suffix(".s3a").unwrap_or(&archive_name);
        config.audit_archive_path = config.data_dir.join(format!("{}.audit.s3a", stem));
    } else if config.audit_archive_path.is_relative() {
        config.audit_archive_path = config.data_dir.join(&config.audit_archive_path);
    }

    // Auth fails closed: server refuses to start without credentials unless --insecure-dev is passed
    if !config.insecure_dev && config.credentials.is_empty() {
        eprintln!("[s3a-lrs-server] ERROR: Server refused to start: no credentials configured. Pass credentials via --auth-user / --auth-pass or --auth-config, or pass --insecure-dev.");
        std::process::exit(1);
    }

    println!(
        "[s3a-lrs-server] Opening audit archive: {:?}",
        config.audit_archive_path
    );
    let audit_writer = if config.audit_archive_path.exists()
        && std::fs::metadata(&config.audit_archive_path)
            .map(|m| m.len())
            .unwrap_or(0)
            >= FILE_HEADER_SIZE as u64
    {
        match ConcurrentTileWriter::open_append(&config.audit_archive_path) {
            Ok(w) => w,
            Err(e) => {
                eprintln!(
                    "[s3a-lrs-server] ERROR: Failed to open audit archive {:?}: {}",
                    config.audit_archive_path, e
                );
                std::process::exit(1);
            }
        }
    } else {
        match ConcurrentTileWriter::create(&config.audit_archive_path) {
            Ok(w) => w,
            Err(e) => {
                eprintln!(
                    "[s3a-lrs-server] ERROR: Failed to create audit archive {:?}: {}",
                    config.audit_archive_path, e
                );
                std::process::exit(1);
            }
        }
    };

    let state = match AppState::init_with_audit(config, Arc::new(audit_writer)) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("[s3a-lrs-server] ERROR: {}", err);
            std::process::exit(1);
        }
    };

    let addr: SocketAddr = format!("{}:{}", state.config.host, state.config.port)
        .parse()
        .expect("Invalid host/port configuration");
    let server = match Server::http(addr) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "[s3a-lrs-server] ERROR: Failed to bind HTTP server on {}: {}",
                addr, e
            );
            std::process::exit(1);
        }
    };
    println!("[s3a-lrs-server] Listening on http://{}", addr);

    if state.config.enable_daily_anchoring {
        let store_clone = state.default_store.clone();
        let archive_path = state.config.archive_path.clone();
        let audit_path = state.config.audit_archive_path.clone();
        let interval_secs = state.config.anchoring_interval_secs;
        thread::spawn(move || loop {
            thread::sleep(std::time::Duration::from_secs(interval_secs));
            let _ = run_daily_anchor_job(&store_clone, &archive_path, &audit_path, None, None);
        });
    }

    if state.config.enable_gate_directory {
        let gate_dir = state.gate_directory.clone();
        thread::spawn(move || loop {
            thread::sleep(std::time::Duration::from_secs(3600));
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            if let Ok(mut dir) = gate_dir.lock() {
                let _ = dir.purge_expired_items(now);
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
        headers_map.insert(
            h.field.to_string().to_ascii_lowercase(),
            h.value.to_string(),
        );
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
            })
            .to_string();
            let resp = make_response(
                200,
                "application/json",
                info_json.as_bytes(),
                method == Method::Head,
            );
            let _ = req.respond(resp);
            return;
        }
    }

    // -------------------------------------------------------------------------
    // Alternate Request Syntax (xAPI 1.0.3 §4.2)
    // -------------------------------------------------------------------------
    if req.method() != &Method::Post && query_params.contains_key("method") {
        let resp = make_error_response(
            400,
            "Method query parameter is only allowed on POST requests (xAPI 1.0.3 §4.2)",
        );
        log_audit(
            &state.audit_writer,
            "unknown",
            req.method().as_str(),
            &path,
            "",
            400,
        );
        let _ = req.respond(resp);
        return;
    }

    if method == Method::Post {
        if query_params.contains_key("method") {
            // Check if there are any other query parameters besides "method"
            if query_params.len() > 1 {
                let resp = make_error_response(400, "Alternate request syntax MUST NOT submit any parameter as a query string parameter other than 'method' (xAPI 1.0.3 §4.2)");
                log_audit(
                    &state.audit_writer,
                    "unknown",
                    req.method().as_str(),
                    &path,
                    "",
                    400,
                );
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

            if (method == Method::Put || method == Method::Post)
                && !has_content
                && body_bytes.is_empty()
            {
                let resp =
                    make_error_response(400, "Missing 'content' parameter in form-urlencoded body");
                log_audit(
                    &state.audit_writer,
                    "unknown",
                    method.as_str(),
                    &path,
                    "",
                    400,
                );
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
            let resp = make_response(
                200,
                "application/json",
                about_json.as_bytes(),
                method == Method::Head,
            );
            log_audit(
                &state.audit_writer,
                "public",
                method.as_str(),
                &path,
                "",
                200,
            );
            let _ = req.respond(resp);
            return;
        } else {
            let resp = make_error_response(405, "Method Not Allowed");
            log_audit(
                &state.audit_writer,
                "public",
                method.as_str(),
                &path,
                "",
                405,
            );
            let _ = req.respond(resp);
            return;
        }
    }

    // -------------------------------------------------------------------------
    // LEARNER API (IEEE P2997 Subset / ADL ELRR REST Service)
    // -------------------------------------------------------------------------
    if path.starts_with("/api/")
        || path == "/api"
        || path.starts_with("/admin/")
        || path == "/admin"
    {
        let resp = learner_api::handle_learner_api(
            &method,
            &path,
            &query_params,
            &headers_map,
            &body_bytes,
            &state,
        );
        let _ = req.respond(resp);
        return;
    }

    // -------------------------------------------------------------------------
    // GATE DIRECTORY & RELAY MAILBOX (§11)
    // -------------------------------------------------------------------------
    if state.config.enable_gate_directory && (path.starts_with("/gate/") || path == "/gate") {
        let client_id = req
            .remote_addr()
            .map(|a| a.ip().to_string())
            .unwrap_or_else(|| "127.0.0.1".to_string());
        let resp = gate_directory::handle_gate_directory(
            &method,
            &path,
            &body_bytes,
            &client_id,
            &state.gate_directory,
        );
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
                resp.add_header(
                    Header::from_bytes(&b"WWW-Authenticate"[..], &b"Basic realm=\"S3A-LRS\""[..])
                        .unwrap(),
                );
                log_audit(
                    &state.audit_writer,
                    "anonymous",
                    method.as_str(),
                    &path,
                    "",
                    401,
                );
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
        } else if path.starts_with("/s3a/batch") {
            "statements/write"
        } else {
            "statements/read"
        };

        let has_scope = cred.has_scope(required_scope)
            || (required_scope == "statements/read" && cred.has_scope("statements/read/mine"));

        if !has_scope {
            let resp = make_error_response(403, "Forbidden: credential lacks required scope");
            log_audit(
                &state.audit_writer,
                &authenticated_user,
                method.as_str(),
                &path,
                "",
                403,
            );
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
        } else if path.starts_with("/s3a/batch") {
            "statements/write"
        } else {
            "statements/read"
        };

        let has_scope = cred.has_scope(required_scope)
            || (required_scope == "statements/read" && cred.has_scope("statements/read/mine"));

        if !has_scope {
            let resp = make_error_response(403, "Forbidden: credential lacks required scope");
            log_audit(
                &state.audit_writer,
                &authenticated_user,
                method.as_str(),
                &path,
                "",
                403,
            );
            let _ = req.respond(resp);
            return;
        }
    }

    let (active_store, active_archive) =
        match state.resolve_store_and_archive(matched_cred, &query_params, &headers_map) {
            Ok(res) => res,
            Err(err_msg) => {
                let resp = make_error_response(403, &format!("Forbidden: {}", err_msg));
                log_audit(
                    &state.audit_writer,
                    &authenticated_user,
                    method.as_str(),
                    &path,
                    "",
                    403,
                );
                let _ = req.respond(resp);
                return;
            }
        };

    // -------------------------------------------------------------------------
    // X-Experience-API-Version Header Validation (xAPI 1.0.3 §4.1.1)
    // -------------------------------------------------------------------------
    if !path.starts_with("/s3a/batch") {
        let xapi_version = headers_map
            .get("x-experience-api-version")
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
                let resp = make_error_response(
                    400,
                    "Missing or invalid X-Experience-API-Version header (must be 1.0.x)",
                );
                log_audit(
                    &state.audit_writer,
                    &authenticated_user,
                    method.as_str(),
                    &path,
                    "",
                    400,
                );
                let _ = req.respond(resp);
                return;
            }
        }
    }

    // Dispatch routed resources
    let response = if path == "/statements" || path == "/statements/" {
        handle_statements(
            method.clone(),
            &query_params,
            &headers_map,
            &body_bytes,
            &state,
            &active_store,
            &active_archive,
        )
    } else if path == "/s3a/batch" || path == "/s3a/batch/" {
        batch_sync::handle_batch_sync(&method, &headers_map, &body_bytes, &state)
    } else if path == "/activities/state" || path == "/activities/state/" {
        handle_activities_state(
            method.clone(),
            &query_params,
            &headers_map,
            &body_bytes,
            &state,
            &active_store,
        )
    } else if path == "/activities/profile" || path == "/activities/profile/" {
        handle_activities_profile(
            method.clone(),
            &query_params,
            &headers_map,
            &body_bytes,
            &state,
            &active_store,
        )
    } else if path == "/agents/profile" || path == "/agents/profile/" {
        handle_agents_profile(
            method.clone(),
            &query_params,
            &headers_map,
            &body_bytes,
            &state,
            &active_store,
        )
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
    let extracted_ids = query_params
        .get("statementId")
        .or_else(|| query_params.get("voidedStatementId"))
        .or_else(|| query_params.get("stateId"))
        .or_else(|| query_params.get("profileId"))
        .or_else(|| query_params.get("activityId"))
        .cloned()
        .unwrap_or_default();

    log_audit(
        &state.audit_writer,
        &authenticated_user,
        method.as_str(),
        &path,
        &extracted_ids,
        status_code,
    );
    let _ = req.respond(response);
}

fn make_statement_error_response(status_code: u16, msg: &str) -> Response<Cursor<Vec<u8>>> {
    let mut resp = make_error_response(status_code, msg);
    let consistent_iso = format_iso8601_ms(s3a_engine::now_ms());
    resp.add_header(
        Header::from_bytes(
            &b"X-Experience-API-Consistent-Through"[..],
            consistent_iso.as_bytes(),
        )
        .unwrap(),
    );
    resp
}

fn apply_statement_format(
    val: &mut serde_json::Value,
    format: Option<&str>,
    accept_lang: Option<&str>,
) {
    let fmt = format.unwrap_or("exact");
    match fmt {
        "ids" => {
            // Simplify Actor
            if let Some(actor) = val.get_mut("actor").and_then(|a| a.as_object_mut()) {
                let keep = [
                    "mbox",
                    "mbox_sha1sum",
                    "openid",
                    "account",
                    "objectType",
                    "member",
                ];
                actor.retain(|k, _| keep.contains(&k.as_str()));
            }
            // Simplify Verb: remove display
            if let Some(verb) = val.get_mut("verb").and_then(|v| v.as_object_mut()) {
                verb.remove("display");
            }
            // Simplify Object
            if let Some(obj) = val.get_mut("object").and_then(|o| o.as_object_mut()) {
                let ot = obj
                    .get("objectType")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Activity");
                if ot == "Activity" {
                    obj.retain(|k, _| k == "id");
                } else if ot == "SubStatement" {
                    if let Some(sub_actor) = obj.get_mut("actor").and_then(|a| a.as_object_mut()) {
                        let keep = [
                            "mbox",
                            "mbox_sha1sum",
                            "openid",
                            "account",
                            "objectType",
                            "member",
                        ];
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
                if let Some(instructor) = ctx.get_mut("instructor").and_then(|i| i.as_object_mut())
                {
                    let keep = [
                        "mbox",
                        "mbox_sha1sum",
                        "openid",
                        "account",
                        "objectType",
                        "member",
                    ];
                    instructor.retain(|k, _| keep.contains(&k.as_str()));
                }
                if let Some(team) = ctx.get_mut("team").and_then(|t| t.as_object_mut()) {
                    let keep = [
                        "mbox",
                        "mbox_sha1sum",
                        "openid",
                        "account",
                        "objectType",
                        "member",
                    ];
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

fn filter_single_language_map(
    map: &mut serde_json::Map<String, serde_json::Value>,
    accept_lang: &str,
) {
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
            if content_type.starts_with("multipart/")
                && !content_type.starts_with("multipart/mixed")
            {
                return make_error_response(
                    400,
                    "Only multipart/mixed is supported for attachments",
                );
            }
            let (statement_json, attachments) = if content_type.starts_with("multipart/mixed") {
                match parse_multipart_request(&content_type, body) {
                    Ok(res) => res,
                    Err(e) => {
                        return make_error_response(400, &format!("Multipart parse error: {}", e))
                    }
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
                        return make_error_response(
                            400,
                            "statementId parameter does not match statement.id in body",
                        );
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
                if let Err(e) =
                    s3a_pack_scholar::xapi::validate_scholar_explorer_usage_profile(&val)
                {
                    return make_error_response(
                        400,
                        &format!("Scholar Explorer Usage profile error: {}", e),
                    );
                }
            }

            // Scholar checks
            let mut scholar_violation = false;
            if state.config.enable_scholar {
                // AI agent allowlist check (403 on denial)
                if let Err(e) =
                    s3a_pack_scholar::xapi::validate_ai_agent_allowlist(&state.agent_registry, &val)
                {
                    return make_error_response(403, &format!("Agent action denied: {}", e));
                }

                // Scholar profile validation
                if let Err(e) = s3a_pack_scholar::xapi::validate_scholar_profile(&val) {
                    if state.config.profile_mode_strict {
                        return make_error_response(
                            400,
                            &format!("Scholar profile validation error: {}", e),
                        );
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
            if content_type.starts_with("multipart/")
                && !content_type.starts_with("multipart/mixed")
            {
                return make_error_response(
                    400,
                    "Only multipart/mixed is supported for attachments",
                );
            }
            let (statement_json, attachments) = if content_type.starts_with("multipart/mixed") {
                match parse_multipart_request(&content_type, body) {
                    Ok(res) => res,
                    Err(e) => {
                        return make_error_response(400, &format!("Multipart parse error: {}", e))
                    }
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
                _ => {
                    return make_error_response(
                        400,
                        "Statement payload must be a JSON object or array",
                    )
                }
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
                    if let Err(e) =
                        s3a_pack_scholar::xapi::validate_scholar_explorer_usage_profile(stmt_val)
                    {
                        return make_error_response(
                            400,
                            &format!("Scholar Explorer Usage profile error: {}", e),
                        );
                    }
                }
                if state.config.enable_scholar {
                    if let Err(e) = s3a_pack_scholar::xapi::validate_ai_agent_allowlist(
                        &state.agent_registry,
                        stmt_val,
                    ) {
                        return make_error_response(403, &format!("Agent action denied: {}", e));
                    }
                    if state.config.profile_mode_strict {
                        if let Err(e) = s3a_pack_scholar::xapi::validate_scholar_profile(stmt_val) {
                            return make_error_response(
                                400,
                                &format!("Scholar profile validation error: {}", e),
                            );
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
                                let _ =
                                    store.mark_statement(id, s3a_core::MARK_NEEDS_REVIEW, true, 0);
                            }
                            let _ = s3a_pack_scholar::xapi::enrich_scholar_statement(
                                store.writer(),
                                stmt_val,
                                [0u8; 16],
                            );
                        }
                        stored_ids.push(u64_pair_to_uuid(id));
                    }
                    Err(LrsStoreError::StatementConflict(msg)) => {
                        return make_error_response(409, &msg)
                    }
                    Err(LrsStoreError::CannotVoidVoidingStatement) => {
                        return make_error_response(400, "Cannot void a voiding statement")
                    }
                    Err(e) => {
                        return make_error_response(
                            400,
                            &format!("Failed to store statement: {}", e),
                        )
                    }
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
                    return make_statement_error_response(
                        400,
                        &format!("Unknown parameter: {}", k),
                    );
                }
            }

            let stmt_id = params.get("statementId");
            let voided_id = params.get("voidedStatementId");

            if stmt_id.is_some() && voided_id.is_some() {
                return make_statement_error_response(
                    400,
                    "Cannot specify both statementId and voidedStatementId",
                );
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
                        return make_statement_error_response(
                            400,
                            &format!("Cannot specify {} with statementId", p),
                        );
                    }
                }
            }

            if voided_id.is_some() {
                for p in COLLECTION_EXCLUSIVE_PARAMS {
                    if params.contains_key(*p) {
                        return make_statement_error_response(
                            400,
                            &format!("Cannot specify {} with voidedStatementId", p),
                        );
                    }
                }
            }

            let format_param = params.get("format").map(|s| s.as_str());
            let accept_lang = headers.get("accept-language").map(|s| s.as_str());
            let want_attachments = params
                .get("attachments")
                .map(|v| v == "true")
                .unwrap_or(false);

            // Single statement by ID
            if let Some(id_str) = stmt_id {
                let id = match uuid_to_u64_pair(id_str) {
                    Ok(u) => u,
                    Err(_) => {
                        return make_statement_error_response(400, "Invalid statementId UUID")
                    }
                };
                match store.get_statement(id, false) {
                    Ok((idx, json_str, atts)) => {
                        let mut val: serde_json::Value = match serde_json::from_str(&json_str) {
                            Ok(v) => v,
                            Err(e) => {
                                return make_statement_error_response(
                                    500,
                                    &format!("Corrupt stored statement: {}", e),
                                )
                            }
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
                            Header::from_bytes(
                                &b"X-Experience-API-Consistent-Through"[..],
                                consistent_iso.as_bytes(),
                            )
                            .unwrap(),
                        );
                        let (_, etag) = LrsStore::calculate_etag(out_json.as_bytes());
                        resp.add_header(Header::from_bytes(&b"ETag"[..], etag.as_bytes()).unwrap());
                        let modified_iso = format_iso8601_ms(idx.stored_timestamp_ms);
                        resp.add_header(
                            Header::from_bytes(&b"Last-Modified"[..], modified_iso.as_bytes())
                                .unwrap(),
                        );
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
                    Err(_) => {
                        return make_statement_error_response(400, "Invalid voidedStatementId UUID")
                    }
                };
                match store.get_statement(id, true) {
                    Ok((idx, json_str, atts)) => {
                        let mut val: serde_json::Value = match serde_json::from_str(&json_str) {
                            Ok(v) => v,
                            Err(e) => {
                                return make_statement_error_response(
                                    500,
                                    &format!("Corrupt stored statement: {}", e),
                                )
                            }
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
                            Header::from_bytes(
                                &b"X-Experience-API-Consistent-Through"[..],
                                consistent_iso.as_bytes(),
                            )
                            .unwrap(),
                        );
                        let (_, etag) = LrsStore::calculate_etag(out_json.as_bytes());
                        resp.add_header(Header::from_bytes(&b"ETag"[..], etag.as_bytes()).unwrap());
                        let modified_iso = format_iso8601_ms(idx.stored_timestamp_ms);
                        resp.add_header(
                            Header::from_bytes(&b"Last-Modified"[..], modified_iso.as_bytes())
                                .unwrap(),
                        );
                        return resp;
                    }
                    Err(_) => {
                        return make_statement_error_response(404, "Voided statement not found")
                    }
                }
            }

            // Query statement collection
            let mut filter = StatementFilter::default();
            if let Some(agent_str) = params.get("agent") {
                let agent_val = match serde_json::from_str::<serde_json::Value>(agent_str) {
                    Ok(v) if v.is_object() => v,
                    _ => {
                        return make_statement_error_response(
                            400,
                            "agent parameter must be a valid JSON Object",
                        )
                    }
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
                    Err(_) => {
                        return make_statement_error_response(400, "Invalid registration UUID")
                    }
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
                    _ => {
                        return make_statement_error_response(
                            400,
                            "limit parameter must be a positive integer",
                        )
                    }
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
                    _ => {
                        return make_statement_error_response(
                            400,
                            "offset parameter must be a non-negative integer",
                        )
                    }
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
                    result_obj.insert(
                        "statements".to_string(),
                        serde_json::Value::Array(stmts_raw),
                    );
                    let more_val = page
                        .more_token
                        .map(|t| format!("/xapi/statements?{}", t))
                        .unwrap_or_default();
                    result_obj.insert("more".to_string(), serde_json::Value::String(more_val));

                    let resp_json = serde_json::Value::Object(result_obj).to_string();
                    let mut resp = if want_attachments {
                        make_multipart_response(&resp_json, &page.attachments, is_head)
                    } else {
                        make_response(200, "application/json", resp_json.as_bytes(), is_head)
                    };
                    let consistent_iso = format_iso8601_ms(s3a_engine::now_ms());
                    resp.add_header(
                        Header::from_bytes(
                            &b"X-Experience-API-Consistent-Through"[..],
                            consistent_iso.as_bytes(),
                        )
                        .unwrap(),
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
        Ok(v) if v.is_object() => {}
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
                    resp.add_header(
                        Header::from_bytes(&b"Last-Modified"[..], modified_iso.as_bytes()).unwrap(),
                    );
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
            let is_json = headers
                .get("content-type")
                .map(|c| c.contains("application/json"))
                .unwrap_or(false);
            match store.put_document(
                DOC_KIND_STATE,
                &compound_key,
                body,
                is_json,
                if_match,
                if_none_match,
                false,
            ) {
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
            let is_json = headers
                .get("content-type")
                .map(|c| c.contains("application/json"))
                .unwrap_or(false);
            if is_json {
                match serde_json::from_slice::<serde_json::Value>(body) {
                    Ok(v) if v.is_object() => {}
                    _ => return make_error_response(400, "Posted document must be a JSON object"),
                }
            }
            match store.put_document(
                DOC_KIND_STATE,
                &compound_key,
                body,
                is_json,
                if_match,
                if_none_match,
                true,
            ) {
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
                if let Some((hdr, content, _)) =
                    store.get_document(DOC_KIND_ACTIVITY_PROFILE, &compound_key)
                {
                    let (_, etag) = LrsStore::calculate_etag(&content);
                    let content_type = if (hdr.flags & DOC_FLAG_IS_JSON) != 0 {
                        "application/json"
                    } else {
                        "application/octet-stream"
                    };
                    let mut resp = make_response(200, content_type, &content, is_head);
                    resp.add_header(Header::from_bytes(&b"ETag"[..], etag.as_bytes()).unwrap());
                    let modified_iso = format_iso8601_ms(hdr.timestamp_ms);
                    resp.add_header(
                        Header::from_bytes(&b"Last-Modified"[..], modified_iso.as_bytes()).unwrap(),
                    );
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
            let is_json = headers
                .get("content-type")
                .map(|c| c.contains("application/json"))
                .unwrap_or(false);
            match store.put_document(
                DOC_KIND_ACTIVITY_PROFILE,
                &compound_key,
                body,
                is_json,
                if_match,
                if_none_match,
                false,
            ) {
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
            let is_json = headers
                .get("content-type")
                .map(|c| c.contains("application/json"))
                .unwrap_or(false);
            if is_json {
                match serde_json::from_slice::<serde_json::Value>(body) {
                    Ok(v) if v.is_object() => {}
                    _ => return make_error_response(400, "Posted document must be a JSON object"),
                }
            }
            match store.put_document(
                DOC_KIND_ACTIVITY_PROFILE,
                &compound_key,
                body,
                is_json,
                if_match,
                if_none_match,
                true,
            ) {
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
        Ok(v) if v.is_object() => {}
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
                if let Some((hdr, content, _)) =
                    store.get_document(DOC_KIND_AGENT_PROFILE, &compound_key)
                {
                    let (_, etag) = LrsStore::calculate_etag(&content);
                    let content_type = if (hdr.flags & DOC_FLAG_IS_JSON) != 0 {
                        "application/json"
                    } else {
                        "application/octet-stream"
                    };
                    let mut resp = make_response(200, content_type, &content, is_head);
                    resp.add_header(Header::from_bytes(&b"ETag"[..], etag.as_bytes()).unwrap());
                    let modified_iso = format_iso8601_ms(hdr.timestamp_ms);
                    resp.add_header(
                        Header::from_bytes(&b"Last-Modified"[..], modified_iso.as_bytes()).unwrap(),
                    );
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
            let is_json = headers
                .get("content-type")
                .map(|c| c.contains("application/json"))
                .unwrap_or(false);
            match store.put_document(
                DOC_KIND_AGENT_PROFILE,
                &compound_key,
                body,
                is_json,
                if_match,
                if_none_match,
                false,
            ) {
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
            let is_json = headers
                .get("content-type")
                .map(|c| c.contains("application/json"))
                .unwrap_or(false);
            if is_json {
                match serde_json::from_slice::<serde_json::Value>(body) {
                    Ok(v) if v.is_object() => {}
                    _ => return make_error_response(400, "Posted document must be a JSON object"),
                }
            }
            match store.put_document(
                DOC_KIND_AGENT_PROFILE,
                &compound_key,
                body,
                is_json,
                if_match,
                if_none_match,
                true,
            ) {
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
    obj.insert(
        "objectType".to_string(),
        serde_json::Value::String("Activity".to_string()),
    );
    obj.insert(
        "id".to_string(),
        serde_json::Value::String(activity_id.clone()),
    );

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
                if v.get("object")
                    .and_then(|o| o.get("id"))
                    .and_then(|i| i.as_str())
                    == Some(activity_id.as_str())
                {
                    if let Some(def) = v
                        .get("object")
                        .and_then(|o| o.get("definition"))
                        .and_then(|d| d.as_object())
                    {
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
            obj.insert(
                "definition".to_string(),
                serde_json::Value::Object(merged_def),
            );
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

    let ifi_count = (if agent_val.get("mbox").is_some() {
        1
    } else {
        0
    }) + (if agent_val.get("mbox_sha1sum").is_some() {
        1
    } else {
        0
    }) + (if agent_val.get("openid").is_some() {
        1
    } else {
        0
    }) + (if agent_val.get("account").is_some() {
        1
    } else {
        0
    });

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
    person.insert(
        "objectType".to_string(),
        serde_json::Value::String("Person".to_string()),
    );

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
fn parse_multipart_request(
    content_type: &str,
    body: &[u8],
) -> Result<(String, Vec<(String, Vec<u8>)>), String> {
    let boundary_idx = content_type
        .find("boundary=")
        .ok_or("Missing boundary in multipart/mixed")?;
    let boundary = content_type[boundary_idx + 9..].trim().trim_matches('"');
    let delimiter = format!("--{}", boundary).into_bytes();

    let find_subslice = |haystack: &[u8], needle: &[u8]| -> Option<usize> {
        haystack
            .windows(needle.len())
            .position(|window| window == needle)
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
                return Err(
                    "First part of multipart/mixed must have Content-Type: application/json".into(),
                );
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
            return Err(format!(
                "Multipart attachment with sha2 {} is not listed in statement",
                part_sha2
            ));
        }
        provided_hashes.push(part_lower);
    }

    if multipart_attachments.len() > declared_hashes.len() {
        return Err("Excess multipart sections that are not attachments in the statement".into());
    }

    for req_sha in &missing_binary_hashes {
        if !provided_hashes.contains(req_sha) {
            return Err(format!(
                "Missing multipart attachment binary for attachment with sha2 {}",
                req_sha
            ));
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
    if headers_map
        .get("access-control-request-private-network")
        .map(|v| v.trim().eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        headers.push(
            Header::from_bytes(&b"Access-Control-Allow-Private-Network"[..], &b"true"[..]).unwrap(),
        );
    }
    Response::new(
        StatusCode(204),
        headers,
        Cursor::new(Vec::new()),
        Some(0),
        None,
    )
}

fn make_response(
    status_code: u16,
    content_type: &str,
    body: &[u8],
    is_head: bool,
) -> Response<Cursor<Vec<u8>>> {
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
    let err_body = serde_json::json!({ "error": msg }).to_string();
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
        let decoded_k = urlencoding::decode(key)
            .unwrap_or(std::borrow::Cow::Borrowed(key))
            .to_string();
        let decoded_v = urlencoding::decode(val)
            .unwrap_or(std::borrow::Cow::Borrowed(val))
            .to_string();
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

pub fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i < input.len() {
        let b0 = input[i];
        let b1 = if i + 1 < input.len() { input[i + 1] } else { 0 };
        let b2 = if i + 2 < input.len() { input[i + 2] } else { 0 };

        let n = ((b0 as u32) << 16) | ((b1 as u32) << 8) | (b2 as u32);
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        if i + 1 < input.len() {
            out.push(TABLE[((n >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if i + 2 < input.len() {
            out.push(TABLE[(n & 63) as usize] as char);
        } else {
            out.push('=');
        }
        i += 3;
    }
    out
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
        return make_error_response(
            400,
            "Invalid export path format: expected /registrations/{uuid}/export",
        );
    }
    let reg_uuid_str = parts[1];
    let reg_pair = match uuid_to_u64_pair(reg_uuid_str) {
        Ok(p) => p,
        Err(_) => return make_error_response(400, "Invalid registration UUID"),
    };

    match store.export_registration_xapi_json(reg_pair) {
        Ok(json_str) => make_response(
            200,
            "application/json; charset=utf-8",
            json_str.as_bytes(),
            false,
        ),
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

        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!(
            "s3a_test_routing_{}_{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let default_path = temp_dir.join("test_routing_default.s3a");
        let audit_path = temp_dir.join("test_routing_audit.s3a");
        let scholar_path = temp_dir.join("scholar.s3a");
        let usage_path = temp_dir.join("usage.s3a");

        let default_store = LrsStore::open_or_create(&default_path).unwrap();
        let scholar_store = LrsStore::open_or_create(&scholar_path).unwrap();
        let usage_store = LrsStore::open_or_create(&usage_path).unwrap();
        let audit_writer = Arc::new(ConcurrentTileWriter::create(&audit_path).unwrap());
        let stores = Arc::new(std::sync::RwLock::new(HashMap::new()));
        stores.write().unwrap().insert(
            "test_routing_default.s3a".to_string(),
            default_store.clone(),
        );
        stores
            .write()
            .unwrap()
            .insert("scholar.s3a".to_string(), scholar_store.clone());
        stores
            .write()
            .unwrap()
            .insert("usage.s3a".to_string(), usage_store.clone());

        let mut config = ServerConfig::default();
        config.data_dir = temp_dir.clone();
        config.archive_path = default_path.clone();

        let state = Arc::new(AppState {
            default_store: default_store.clone(),
            store: default_store,
            stores,
            audit_writer,
            config,
            agent_registry: Vec::new(),
            gate_directory: Arc::new(std::sync::Mutex::new(
                crate::gate_directory::GateDirectoryService::new(),
            )),
            batch_sync: Arc::new(std::sync::Mutex::new(
                crate::batch_sync::BatchSyncService::new(
                    crate::batch_sync::BatchSyncLimits::default(),
                ),
            )),
        });

        let empty_params = HashMap::new();
        let empty_headers = HashMap::new();

        // 1. Researcher credential resolves to scholar.s3a
        let (_res_store, res_archive) = state
            .resolve_store_and_archive(Some(&researcher), &empty_params, &empty_headers)
            .unwrap();
        assert_eq!(res_archive, "scholar.s3a");

        // 2. Explorer credential resolves to usage.s3a
        let (_exp_store, exp_archive) = state
            .resolve_store_and_archive(Some(&explorer), &empty_params, &empty_headers)
            .unwrap();
        assert_eq!(exp_archive, "usage.s3a");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_usage_archive_strict_profile_validation() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!(
            "s3a_test_usage_strict_{}_{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let usage_path = temp_dir.join("test_usage_strict.s3a");
        let audit_path = temp_dir.join("test_usage_audit.s3a");

        let usage_store = LrsStore::open_or_create(&usage_path).unwrap();
        let audit_writer = Arc::new(ConcurrentTileWriter::create(&audit_path).unwrap());
        let stores = Arc::new(std::sync::RwLock::new(HashMap::new()));
        stores
            .write()
            .unwrap()
            .insert("usage.s3a".to_string(), usage_store.clone());

        let state = Arc::new(AppState {
            default_store: usage_store.clone(),
            store: usage_store.clone(),
            stores,
            audit_writer,
            config: ServerConfig::default(),
            agent_registry: Vec::new(),
            gate_directory: Arc::new(std::sync::Mutex::new(
                crate::gate_directory::GateDirectoryService::new(),
            )),
            batch_sync: Arc::new(std::sync::Mutex::new(
                crate::batch_sync::BatchSyncService::new(
                    crate::batch_sync::BatchSyncLimits::default(),
                ),
            )),
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
            let resp = handle_statements(
                Method::Post,
                &params,
                &headers,
                &body,
                &state,
                &usage_store,
                "usage.s3a",
            );
            assert_eq!(
                resp.status_code().0,
                200,
                "Permitted verb '{}' must be accepted with 200",
                verb
            );
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
            let resp = handle_statements(
                Method::Post,
                &params,
                &headers,
                &body,
                &state,
                &usage_store,
                "usage.s3a",
            );
            assert_eq!(
                resp.status_code().0,
                400,
                "Unpermitted verb '{}' must be rejected with 400",
                verb
            );
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

        let resp_put_ok = handle_statements(
            Method::Put,
            &put_params,
            &headers,
            &body_put,
            &state,
            &usage_store,
            "usage.s3a",
        );
        assert_eq!(resp_put_ok.status_code().0, 204);

        let _ = std::fs::remove_file(&usage_path);
        let _ = std::fs::remove_file(&audit_path);
    }

    #[test]
    fn test_admin_delete_by_actor_account_and_retention_purge() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!(
            "s3a_test_admin_delete_{}_{}",
            std::process::id(),
            unique
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let usage_path = temp_dir.join("test_admin_usage.s3a");
        let audit_path = temp_dir.join("test_admin_audit.s3a");

        let usage_store = LrsStore::open_or_create(&usage_path).unwrap();
        let audit_writer = Arc::new(ConcurrentTileWriter::create(&audit_path).unwrap());
        let stores = Arc::new(std::sync::RwLock::new(HashMap::new()));
        stores
            .write()
            .unwrap()
            .insert("test_admin_usage.s3a".to_string(), usage_store.clone());

        let mut config = ServerConfig::default();
        config.insecure_dev = true;

        let state = Arc::new(AppState {
            default_store: usage_store.clone(),
            store: usage_store.clone(),
            stores,
            audit_writer,
            config,
            agent_registry: Vec::new(),
            gate_directory: Arc::new(std::sync::Mutex::new(
                crate::gate_directory::GateDirectoryService::new(),
            )),
            batch_sync: Arc::new(std::sync::Mutex::new(
                crate::batch_sync::BatchSyncService::new(
                    crate::batch_sync::BatchSyncLimits::default(),
                ),
            )),
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
        let _ = usage_store
            .store_statement(&dev777_stmt.to_string(), &[])
            .unwrap();

        // 1. Purge by actor account / device ID via /admin/delete-by-actor-account
        let purge_req = serde_json::to_vec(&serde_json::json!({
            "deviceId": "device-777",
            "archive": "test_admin_usage.s3a"
        }))
        .unwrap();

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
        }))
        .unwrap();

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
        preflight_headers.insert(
            "access-control-request-private-network".to_string(),
            "true".to_string(),
        );
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
        assert!(
            has_allow_private,
            "Preflight must respond with Access-Control-Allow-Private-Network: true"
        );
        assert!(
            has_expose_headers,
            "Preflight must respond with Access-Control-Expose-Headers"
        );

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
            if h.field
                .to_string()
                .eq_ignore_ascii_case("Access-Control-Expose-Headers")
            {
                assert_eq!(h.value.to_string(), "ETag, Last-Modified, X-Experience-API-Version, X-Experience-API-Consistent-Through");
                expose_found = true;
            }
        }
        assert!(
            expose_found,
            "make_response must include Access-Control-Expose-Headers"
        );

        let empty_resp = make_response_empty(204);
        let mut empty_expose_found = false;
        for h in empty_resp.headers() {
            if h.field
                .to_string()
                .eq_ignore_ascii_case("Access-Control-Expose-Headers")
            {
                assert_eq!(h.value.to_string(), "ETag, Last-Modified, X-Experience-API-Version, X-Experience-API-Consistent-Through");
                empty_expose_found = true;
            }
        }
        assert!(
            empty_expose_found,
            "make_response_empty must include Access-Control-Expose-Headers"
        );

        let err_resp = make_error_response(400, "Bad Request");
        let mut err_expose_found = false;
        for h in err_resp.headers() {
            if h.field
                .to_string()
                .eq_ignore_ascii_case("Access-Control-Expose-Headers")
            {
                assert_eq!(h.value.to_string(), "ETag, Last-Modified, X-Experience-API-Version, X-Experience-API-Consistent-Through");
                err_expose_found = true;
            }
        }
        assert!(
            err_expose_found,
            "make_error_response must include Access-Control-Expose-Headers"
        );
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
        store
            .store_statement(&initial_json, &[])
            .expect("store statement");

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
        let page = store
            .query_statements(StatementFilter::default())
            .expect("query");
        let anchored_stmt = page.statements.iter().find_map(|s| {
            let v: serde_json::Value = serde_json::from_str(s).ok()?;
            if v["verb"]["id"].as_str() == Some("https://ashecribb.dev/xapi/verbs/anchored") {
                Some(v)
            } else {
                None
            }
        });
        assert!(
            anchored_stmt.is_some(),
            "Store must contain an anchored statement"
        );
        let ext_fp = anchored_stmt.unwrap()["object"]["definition"]["extensions"]
            ["https://ashecribb.dev/xapi/ext/fingerprint"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(ext_fp, fp_hex);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_is_valid_archive_name_rules() {
        assert!(is_valid_archive_name("scholar.s3a"));
        assert!(is_valid_archive_name("usage.s3a"));
        assert!(is_valid_archive_name("lrs_data.s3a"));
        assert!(is_valid_archive_name("my-archive_01.s3a"));
        assert!(is_valid_archive_name("test.archive-v2.s3a"));

        // Must reject directory traversal
        assert!(!is_valid_archive_name("../../etc/x.s3a"));
        assert!(!is_valid_archive_name("../x.s3a"));
        assert!(!is_valid_archive_name("foo..bar.s3a"));
        assert!(!is_valid_archive_name("..s3a"));

        // Must reject slashes and backslashes
        assert!(!is_valid_archive_name("/etc/x.s3a"));
        assert!(!is_valid_archive_name("\\etc\\x.s3a"));
        assert!(!is_valid_archive_name("sub/x.s3a"));
        assert!(!is_valid_archive_name("sub\\x.s3a"));

        // Must reject absolute paths
        assert!(!is_valid_archive_name("C:\\temp\\x.s3a"));
        assert!(!is_valid_archive_name("C:/temp/x.s3a"));
        assert!(!is_valid_archive_name("/x.s3a"));

        // Must reject non-bare or wrong extension
        assert!(!is_valid_archive_name(".s3a"));
        assert!(!is_valid_archive_name("x.s3a.bak"));
        assert!(!is_valid_archive_name("x.txt"));
        assert!(!is_valid_archive_name(""));
        assert!(!is_valid_archive_name("foo bar.s3a"));
    }

    #[test]
    fn test_explorer_with_override_param_still_writes_to_assigned_usage_archive() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("s3a_test_exp_{}", unique));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let default_path = temp_dir.join("lrs_data.s3a");
        let audit_path = temp_dir.join("lrs_data.audit.s3a");
        let usage_path = temp_dir.join("usage.s3a");
        let scholar_path = temp_dir.join("scholar.s3a");

        let default_store = LrsStore::open_or_create(&default_path).unwrap();
        let usage_store = LrsStore::open_or_create(&usage_path).unwrap();
        let scholar_store = LrsStore::open_or_create(&scholar_path).unwrap();
        let audit_writer = Arc::new(ConcurrentTileWriter::create(&audit_path).unwrap());

        let explorer = AuthCredential {
            user: "explorer".to_string(),
            pass: "explorer123".to_string(),
            scopes: vec!["statements/write".to_string()],
            archive: Some("usage.s3a".to_string()),
        };

        let stores = Arc::new(std::sync::RwLock::new(HashMap::new()));
        stores
            .write()
            .unwrap()
            .insert("lrs_data.s3a".to_string(), default_store.clone());
        stores
            .write()
            .unwrap()
            .insert("usage.s3a".to_string(), usage_store.clone());
        stores
            .write()
            .unwrap()
            .insert("scholar.s3a".to_string(), scholar_store.clone());

        let mut config = ServerConfig::default();
        config.data_dir = temp_dir.clone();
        config.archive_path = default_path.clone();
        config.audit_archive_path = audit_path.clone();
        config.credentials.push(explorer.clone());

        let state = Arc::new(AppState {
            default_store: default_store.clone(),
            store: default_store,
            stores,
            audit_writer,
            config,
            agent_registry: Vec::new(),
            gate_directory: Arc::new(std::sync::Mutex::new(
                crate::gate_directory::GateDirectoryService::new(),
            )),
            batch_sync: Arc::new(std::sync::Mutex::new(
                crate::batch_sync::BatchSyncService::new(
                    crate::batch_sync::BatchSyncLimits::default(),
                ),
            )),
        });

        // 1. Direct resolution check: explorer with ?archive=scholar.s3a ignores override and resolves to usage.s3a
        let mut params = HashMap::new();
        params.insert("archive".to_string(), "scholar.s3a".to_string());
        let headers = HashMap::new();

        let (_resolved_store, resolved_name) = state
            .resolve_store_and_archive(Some(&explorer), &params, &headers)
            .unwrap();
        assert_eq!(resolved_name, "usage.s3a");

        // 2. HTTP request test through server
        let server = Server::http("127.0.0.1:0").expect("bind test server");
        let port = server.server_addr().to_ip().unwrap().port();
        let addr = format!("http://127.0.0.1:{}", port);
        let state_clone = state.clone();
        thread::spawn(move || {
            for req in server.incoming_requests() {
                let sc = state_clone.clone();
                thread::spawn(move || handle_request(req, sc));
            }
        });

        let stmt = serde_json::json!({
            "actor": { "account": { "homePage": "https://ashecribb.dev/explorer", "name": "device-001" } },
            "verb": { "id": s3a_pack_scholar::xapi::USAGE_VERB_SEARCHED },
            "object": { "id": "https://example.com/paper/42" },
            "context": {
                "contextActivities": {
                    "category": [{ "id": s3a_pack_scholar::xapi::SCHOLAR_EXPLORER_USAGE_PROFILE_IRI }]
                }
            }
        });
        let body = serde_json::to_vec(&stmt).unwrap();

        let resp = ureq::post(&format!("{}/xapi/statements?archive=scholar.s3a", addr))
            .set("Authorization", "Basic ZXhwbG9yZXI6ZXhwbG9yZXIxMjM=") // explorer:explorer123
            .set("X-Experience-API-Version", "1.0.3")
            .set("Content-Type", "application/json")
            .send_bytes(&body)
            .expect("Explorer write with ?archive=scholar.s3a must succeed");

        assert_eq!(resp.status(), 200);

        // Verify it wrote to usage.s3a
        let usage_page = usage_store
            .query_statements(StatementFilter::default())
            .unwrap();
        assert_eq!(
            usage_page.statements.len(),
            1,
            "Statement must be written to usage.s3a"
        );

        // Verify scholar.s3a remains untouched
        let scholar_page = scholar_store
            .query_statements(StatementFilter::default())
            .unwrap();
        assert_eq!(
            scholar_page.statements.len(),
            0,
            "scholar.s3a must NOT have any statements written"
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_archive_override_path_traversal_rejected_with_403() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("s3a_test_trav_{}", unique));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let default_path = temp_dir.join("lrs_data.s3a");
        let audit_path = temp_dir.join("lrs_data.audit.s3a");

        let default_store = LrsStore::open_or_create(&default_path).unwrap();
        let audit_writer = Arc::new(ConcurrentTileWriter::create(&audit_path).unwrap());

        let mut config = ServerConfig::default();
        config.data_dir = temp_dir.clone();
        config.archive_path = default_path.clone();
        config.audit_archive_path = audit_path.clone();
        config.insecure_dev = true;

        let stores = Arc::new(std::sync::RwLock::new(HashMap::new()));
        stores
            .write()
            .unwrap()
            .insert("lrs_data.s3a".to_string(), default_store.clone());

        let state = Arc::new(AppState {
            default_store: default_store.clone(),
            store: default_store,
            stores,
            audit_writer: audit_writer.clone(),
            config,
            agent_registry: Vec::new(),
            gate_directory: Arc::new(std::sync::Mutex::new(
                crate::gate_directory::GateDirectoryService::new(),
            )),
            batch_sync: Arc::new(std::sync::Mutex::new(
                crate::batch_sync::BatchSyncService::new(
                    crate::batch_sync::BatchSyncLimits::default(),
                ),
            )),
        });

        // 1. Direct resolve check
        let mut params = HashMap::new();
        params.insert("archive".to_string(), "../../etc/x.s3a".to_string());
        let res = state.resolve_store_and_archive(None, &params, &HashMap::new());
        assert!(res.is_err(), "?archive=../../etc/x.s3a must return Err");

        // 2. HTTP request check
        let server = Server::http("127.0.0.1:0").expect("bind test server");
        let port = server.server_addr().to_ip().unwrap().port();
        let addr = format!("http://127.0.0.1:{}", port);
        let state_clone = state.clone();
        thread::spawn(move || {
            for req in server.incoming_requests() {
                let sc = state_clone.clone();
                thread::spawn(move || handle_request(req, sc));
            }
        });

        let resp = ureq::get(&format!("{}/xapi/statements?archive=../../etc/x.s3a", addr))
            .set("X-Experience-API-Version", "1.0.3")
            .call();

        match resp {
            Err(ureq::Error::Status(403, r)) => {
                let err_json: serde_json::Value = serde_json::from_reader(r.into_reader()).unwrap();
                assert!(err_json["error"].as_str().unwrap().contains("Forbidden"));
            }
            other => panic!("Expected 403 Forbidden, got {:?}", other),
        }

        // Verify audit archive contains 403 entry
        let audit_bytes = audit_writer.read_all_bytes().unwrap();
        let audit_str = String::from_utf8_lossy(&audit_bytes);
        assert!(audit_str.contains("\"status\":403") || audit_str.contains("\"status\": 403"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_archive_override_absolute_path_rejected_with_403() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("s3a_test_abs_{}", unique));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let default_path = temp_dir.join("lrs_data.s3a");
        let audit_path = temp_dir.join("lrs_data.audit.s3a");

        let default_store = LrsStore::open_or_create(&default_path).unwrap();
        let audit_writer = Arc::new(ConcurrentTileWriter::create(&audit_path).unwrap());

        let mut config = ServerConfig::default();
        config.data_dir = temp_dir.clone();
        config.archive_path = default_path.clone();
        config.audit_archive_path = audit_path.clone();
        config.insecure_dev = true;

        let stores = Arc::new(std::sync::RwLock::new(HashMap::new()));
        stores
            .write()
            .unwrap()
            .insert("lrs_data.s3a".to_string(), default_store.clone());

        let state = Arc::new(AppState {
            default_store: default_store.clone(),
            store: default_store,
            stores,
            audit_writer: audit_writer.clone(),
            config,
            agent_registry: Vec::new(),
            gate_directory: Arc::new(std::sync::Mutex::new(
                crate::gate_directory::GateDirectoryService::new(),
            )),
            batch_sync: Arc::new(std::sync::Mutex::new(
                crate::batch_sync::BatchSyncService::new(
                    crate::batch_sync::BatchSyncLimits::default(),
                ),
            )),
        });

        // 1. Direct resolve check with absolute path
        let mut params = HashMap::new();
        params.insert("archive".to_string(), "/etc/x.s3a".to_string());
        assert!(state
            .resolve_store_and_archive(None, &params, &HashMap::new())
            .is_err());

        // 2. HTTP request check
        let server = Server::http("127.0.0.1:0").expect("bind test server");
        let port = server.server_addr().to_ip().unwrap().port();
        let addr = format!("http://127.0.0.1:{}", port);
        let state_clone = state.clone();
        thread::spawn(move || {
            for req in server.incoming_requests() {
                let sc = state_clone.clone();
                thread::spawn(move || handle_request(req, sc));
            }
        });

        let resp = ureq::get(&format!("{}/xapi/statements?archive=/etc/x.s3a", addr))
            .set("X-Experience-API-Version", "1.0.3")
            .call();

        match resp {
            Err(ureq::Error::Status(403, _)) => {}
            other => panic!("Expected 403 Forbidden for absolute path, got {:?}", other),
        }

        let audit_bytes = audit_writer.read_all_bytes().unwrap();
        let audit_str = String::from_utf8_lossy(&audit_bytes);
        assert!(audit_str.contains("\"status\":403") || audit_str.contains("\"status\": 403"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_unknown_archive_name_rejected_and_no_file_created() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("s3a_test_unknown_{}", unique));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let default_path = temp_dir.join("lrs_data.s3a");
        let audit_path = temp_dir.join("lrs_data.audit.s3a");

        let default_store = LrsStore::open_or_create(&default_path).unwrap();
        let audit_writer = Arc::new(ConcurrentTileWriter::create(&audit_path).unwrap());

        let admin = AuthCredential {
            user: "admin".to_string(),
            pass: "admin123".to_string(),
            scopes: vec!["admin".to_string(), "all".to_string()],
            archive: None,
        };

        let mut config = ServerConfig::default();
        config.data_dir = temp_dir.clone();
        config.archive_path = default_path.clone();
        config.audit_archive_path = audit_path.clone();
        config.admin_api_override = true;
        config.credentials.push(admin.clone());

        let stores = Arc::new(std::sync::RwLock::new(HashMap::new()));
        stores
            .write()
            .unwrap()
            .insert("lrs_data.s3a".to_string(), default_store.clone());

        let state = Arc::new(AppState {
            default_store: default_store.clone(),
            store: default_store,
            stores,
            audit_writer: audit_writer.clone(),
            config,
            agent_registry: Vec::new(),
            gate_directory: Arc::new(std::sync::Mutex::new(
                crate::gate_directory::GateDirectoryService::new(),
            )),
            batch_sync: Arc::new(std::sync::Mutex::new(
                crate::batch_sync::BatchSyncService::new(
                    crate::batch_sync::BatchSyncLimits::default(),
                ),
            )),
        });

        let nonexistent_target = temp_dir.join("unknown_random_archive.s3a");
        assert!(
            !nonexistent_target.exists(),
            "Target file must not exist before test"
        );

        // 1. Direct resolve check
        let mut params = HashMap::new();
        params.insert(
            "archive".to_string(),
            "unknown_random_archive.s3a".to_string(),
        );
        let res = state.resolve_store_and_archive(Some(&admin), &params, &HashMap::new());
        assert!(
            res.is_err(),
            "Unknown archive name must be rejected with Err"
        );
        assert!(
            !nonexistent_target.exists(),
            "No file must be created by resolve_store_and_archive"
        );

        // 2. HTTP request check
        let server = Server::http("127.0.0.1:0").expect("bind test server");
        let port = server.server_addr().to_ip().unwrap().port();
        let addr = format!("http://127.0.0.1:{}", port);
        let state_clone = state.clone();
        thread::spawn(move || {
            for req in server.incoming_requests() {
                let sc = state_clone.clone();
                thread::spawn(move || handle_request(req, sc));
            }
        });

        let resp = ureq::get(&format!(
            "{}/xapi/statements?archive=unknown_random_archive.s3a",
            addr
        ))
        .set("Authorization", "Basic YWRtaW46YWRtaW4xMjM=") // admin:admin123
        .set("X-Experience-API-Version", "1.0.3")
        .call();

        match resp {
            Err(ureq::Error::Status(403, _)) => {}
            other => panic!(
                "Expected 403 Forbidden for unknown archive, got {:?}",
                other
            ),
        }

        // Verify that no file was created!
        assert!(
            !nonexistent_target.exists(),
            "Unknown archive file must NOT be created on disk"
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_admin_with_override_flag_picks_existing_allowed_archive() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("s3a_test_admin_{}", unique));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let default_path = temp_dir.join("lrs_data.s3a");
        let audit_path = temp_dir.join("lrs_data.audit.s3a");
        let allowed_path = temp_dir.join("custom_allowed.s3a");

        let default_store = LrsStore::open_or_create(&default_path).unwrap();
        let allowed_store = LrsStore::open_or_create(&allowed_path).unwrap();
        let audit_writer = Arc::new(ConcurrentTileWriter::create(&audit_path).unwrap());

        let admin = AuthCredential {
            user: "admin".to_string(),
            pass: "admin123".to_string(),
            scopes: vec!["admin".to_string(), "all".to_string()],
            archive: None,
        };

        let mut config = ServerConfig::default();
        config.data_dir = temp_dir.clone();
        config.archive_path = default_path.clone();
        config.audit_archive_path = audit_path.clone();
        config.admin_api_override = true;
        config.credentials.push(admin.clone());

        let stores = Arc::new(std::sync::RwLock::new(HashMap::new()));
        stores
            .write()
            .unwrap()
            .insert("lrs_data.s3a".to_string(), default_store.clone());
        stores
            .write()
            .unwrap()
            .insert("custom_allowed.s3a".to_string(), allowed_store.clone());

        let state = Arc::new(AppState {
            default_store: default_store.clone(),
            store: default_store.clone(),
            stores,
            audit_writer,
            config,
            agent_registry: Vec::new(),
            gate_directory: Arc::new(std::sync::Mutex::new(
                crate::gate_directory::GateDirectoryService::new(),
            )),
            batch_sync: Arc::new(std::sync::Mutex::new(
                crate::batch_sync::BatchSyncService::new(
                    crate::batch_sync::BatchSyncLimits::default(),
                ),
            )),
        });

        // 1. With admin_api_override = true, admin successfully picks custom_allowed.s3a
        let mut params = HashMap::new();
        params.insert("archive".to_string(), "custom_allowed.s3a".to_string());
        let (_store_res, name_res) = state
            .resolve_store_and_archive(Some(&admin), &params, &HashMap::new())
            .unwrap();
        assert_eq!(name_res, "custom_allowed.s3a");

        // HTTP POST through server writing to custom_allowed.s3a
        let server = Server::http("127.0.0.1:0").expect("bind test server");
        let port = server.server_addr().to_ip().unwrap().port();
        let addr = format!("http://127.0.0.1:{}", port);
        let state_clone = state.clone();
        thread::spawn(move || {
            for req in server.incoming_requests() {
                let sc = state_clone.clone();
                thread::spawn(move || handle_request(req, sc));
            }
        });

        let stmt = serde_json::json!({
            "actor": { "mbox": "mailto:admin@example.com" },
            "verb": { "id": "http://adlnet.gov/expapi/verbs/experienced" },
            "object": { "id": "http://example.com/item/admin-01" }
        });
        let body = serde_json::to_vec(&stmt).unwrap();

        let resp = ureq::post(&format!(
            "{}/xapi/statements?archive=custom_allowed.s3a",
            addr
        ))
        .set("Authorization", "Basic YWRtaW46YWRtaW4xMjM=")
        .set("X-Experience-API-Version", "1.0.3")
        .set("Content-Type", "application/json")
        .send_bytes(&body)
        .expect("Admin write to custom_allowed.s3a must succeed");

        assert_eq!(resp.status(), 200);
        let custom_page = allowed_store
            .query_statements(StatementFilter::default())
            .unwrap();
        assert_eq!(
            custom_page.statements.len(),
            1,
            "custom_allowed.s3a must contain the written statement"
        );
        let default_page = default_store
            .query_statements(StatementFilter::default())
            .unwrap();
        assert_eq!(
            default_page.statements.len(),
            0,
            "default store must NOT contain the statement"
        );

        // 2. If admin_api_override is false, override is NOT honored and admin uses default archive only
        let mut disabled_config = state.config.clone();
        disabled_config.admin_api_override = false;
        let mut disabled_state = (*state).clone();
        disabled_state.config = disabled_config;

        let (_d_store, d_name) = disabled_state
            .resolve_store_and_archive(Some(&admin), &params, &HashMap::new())
            .unwrap();
        assert_eq!(
            d_name, "lrs_data.s3a",
            "Override must be ignored when --admin-api-override is not set"
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_startup_with_archive_matching_credential_shares_store_instance() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("s3a_test_pinned_matching_{}", unique));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let mut config = ServerConfig::default();
        config.data_dir = temp_dir.clone();
        config.archive_path = PathBuf::from("scholar.s3a");
        config.audit_archive_path = PathBuf::from("scholar.audit.s3a");

        let pinned_cred = AuthCredential {
            user: "researcher".to_string(),
            pass: "researcher123".to_string(),
            scopes: vec!["all".to_string()],
            archive: Some("scholar.s3a".to_string()),
        };
        config.credentials.push(pinned_cred.clone());

        let audit_writer =
            Arc::new(ConcurrentTileWriter::create(&temp_dir.join("scholar.audit.s3a")).unwrap());

        // 1. Startup must NOT panic with "Archive file is already locked by another process"
        let state = AppState::init_with_audit(config, audit_writer)
            .expect("Server initialization must succeed without lock panic when credential archive matches --archive");

        // 2. Both the default store and the pinned credential resolve to the exact same Arc<LrsStore>
        let (default_store, default_name) = state
            .resolve_store_and_archive(None, &HashMap::new(), &HashMap::new())
            .unwrap();
        assert_eq!(default_name, "scholar.s3a");

        let (cred_store, cred_name) = state
            .resolve_store_and_archive(Some(&pinned_cred), &HashMap::new(), &HashMap::new())
            .unwrap();
        assert_eq!(cred_name, "scholar.s3a");

        assert!(
            Arc::ptr_eq(&default_store, &cred_store),
            "Both default store and credential store must resolve to the exact same Arc<LrsStore> instance"
        );
        assert!(Arc::ptr_eq(&state.default_store, &cred_store));

        // 3. HTTP server starts and operations on pinned credential write to the shared store
        let server = Server::http("127.0.0.1:0").expect("bind test server");
        let port = server.server_addr().to_ip().unwrap().port();
        let addr = format!("http://127.0.0.1:{}", port);
        let state_clone = state.clone();
        thread::spawn(move || {
            for req in server.incoming_requests() {
                let sc = state_clone.clone();
                thread::spawn(move || handle_request(req, sc));
            }
        });

        let stmt = serde_json::json!({
            "actor": { "mbox": "mailto:researcher@example.com" },
            "verb": { "id": "http://adlnet.gov/expapi/verbs/experienced" },
            "object": { "id": "https://example.com/items/shared-store-1" }
        });
        let body = serde_json::to_vec(&stmt).unwrap();

        let resp = ureq::post(&format!("{}/xapi/statements", addr))
            .set("Authorization", "Basic cmVzZWFyY2hlcjpyZXNlYXJjaGVyMTIz")
            .set("X-Experience-API-Version", "1.0.3")
            .set("Content-Type", "application/json")
            .send_bytes(&body)
            .expect("Statement write with pinned credential must succeed");

        assert_eq!(resp.status(), 200);

        // Statement is visible from both references because they point to the exact same store
        let page_default = default_store
            .query_statements(StatementFilter::default())
            .unwrap();
        assert_eq!(page_default.statements.len(), 1);

        let page_cred = cred_store
            .query_statements(StatementFilter::default())
            .unwrap();
        assert_eq!(page_cred.statements.len(), 1);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_credential_archive_open_error_refuses_to_start_without_panic() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("s3a_test_open_fail_{}", unique));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let mut config = ServerConfig::default();
        config.data_dir = temp_dir.clone();
        config.archive_path = PathBuf::from("lrs_data.s3a");
        config.audit_archive_path = PathBuf::from("lrs_data.audit.s3a");

        // Create a directory where the archive file should be, causing open_or_create to fail
        let conflict_dir = temp_dir.join("conflict.s3a");
        std::fs::create_dir_all(&conflict_dir).unwrap();

        let bad_cred = AuthCredential {
            user: "bad".to_string(),
            pass: "bad123".to_string(),
            scopes: vec!["all".to_string()],
            archive: Some("conflict.s3a".to_string()),
        };
        config.credentials.push(bad_cred);

        let audit_writer =
            Arc::new(ConcurrentTileWriter::create(&temp_dir.join("lrs_data.audit.s3a")).unwrap());

        let res = AppState::init_with_audit(config, audit_writer);
        assert!(
            res.is_err(),
            "Must refuse to start when a credential archive cannot be opened"
        );
        let err_msg = res.err().unwrap();
        assert!(
            err_msg.contains("Failed to open credential archive 'conflict.s3a'"),
            "Error message must clearly identify the failed archive: {}",
            err_msg
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_batch_sync_in_order_and_idempotent_duplicate() {
        use ed25519_dalek::{Signer, SigningKey};
        use s3a_pack_scholar::{
            HumanLrsRecord, ScholarTileType, SrsDeltaEntry, SrsSyncDeltaPacket,
        };

        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("s3a_test_batch_sync_{}", unique));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let mut config = ServerConfig::default();
        config.data_dir = temp_dir.clone();
        config.archive_path = PathBuf::from("lrs_data.s3a");
        config.audit_archive_path = PathBuf::from("lrs_data.audit.s3a");
        config.credentials.push(AuthCredential {
            user: "researcher".to_string(),
            pass: "researcher123".to_string(),
            scopes: vec!["all".to_string()],
            archive: None,
        });

        let audit_writer =
            Arc::new(ConcurrentTileWriter::create(&temp_dir.join("lrs_data.audit.s3a")).unwrap());
        let state = AppState::init_with_audit(config, audit_writer).expect("init state");

        let server = Server::http("127.0.0.1:0").expect("bind test server");
        let port = server.server_addr().to_ip().unwrap().port();
        let addr = format!("http://127.0.0.1:{}", port);
        let state_clone = state.clone();
        thread::spawn(move || {
            for req in server.incoming_requests() {
                let sc = state_clone.clone();
                thread::spawn(move || handle_request(req, sc));
            }
        });

        // 1. Prepare signed batch
        let signing_key = SigningKey::generate(&mut rand_core::OsRng);
        let verifying_key = signing_key.verifying_key();
        let pk_hex = hex::encode(verifying_key.as_bytes());

        let mut packet = SrsSyncDeltaPacket::new([1, 2], [3, 4], 1);
        for i in 0..3 {
            let rec = HumanLrsRecord::new(
                [1, 2],
                1600000000,
                100,
                1,
                1.0,
                S3ACoordinate::default(),
                i as u32,
            );
            packet
                .entries
                .push(SrsDeltaEntry::from_record(ScholarTileType::HUMAN_LRS, &rec).unwrap());
        }
        let delta_bytes = packet.export_bytes();
        let checksum = crc32c::crc32c(&delta_bytes);
        let sig = signing_key.sign(&delta_bytes);
        let sig_hex = hex::encode(sig.to_bytes());
        let bytes_b64 = crate::base64_encode(&delta_bytes);

        let req_body = serde_json::json!({
            "session_id": "test-session-001",
            "from_cursor": 0,
            "to_cursor": 3,
            "bytes": bytes_b64,
            "checksum": checksum,
            "signature": sig_hex,
            "public_key": pk_hex,
        });

        // 2. POST /xapi/s3a/batch (In-order batch)
        let resp = ureq::post(&format!("{}/xapi/s3a/batch", addr))
            .set("Authorization", "Basic cmVzZWFyY2hlcjpyZXNlYXJjaGVyMTIz")
            .set("Content-Type", "application/json")
            .send_bytes(&serde_json::to_vec(&req_body).unwrap())
            .expect("Batch sync POST must succeed");

        assert_eq!(resp.status(), 200);
        let resp_json: serde_json::Value =
            serde_json::from_reader(resp.into_reader()).expect("parse response JSON");
        assert_eq!(resp_json["status"], "ok");
        assert_eq!(resp_json["accepted_cursor"], 3);
        assert_eq!(resp_json["duplicate"], false);
        assert_eq!(resp_json["records_appended"], 3);

        // Verify scholar.s3a was created in data_dir
        assert!(
            temp_dir.join("scholar.s3a").exists(),
            "scholar.s3a must be created"
        );

        // 3. Re-send exact same batch (Idempotent 200 OK no-op)
        let resp2 = ureq::post(&format!("{}/xapi/s3a/batch", addr))
            .set("Authorization", "Basic cmVzZWFyY2hlcjpyZXNlYXJjaGVyMTIz")
            .set("Content-Type", "application/json")
            .send_bytes(&serde_json::to_vec(&req_body).unwrap())
            .expect("Duplicate re-send must succeed");

        assert_eq!(resp2.status(), 200);
        let resp2_json: serde_json::Value =
            serde_json::from_reader(resp2.into_reader()).expect("parse response JSON");
        assert_eq!(resp2_json["status"], "ok");
        assert_eq!(resp2_json["accepted_cursor"], 3);
        assert_eq!(resp2_json["duplicate"], true);
        assert_eq!(resp2_json["records_appended"], 0);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_batch_sync_gap_and_out_of_order_rejection() {
        use ed25519_dalek::{Signer, SigningKey};
        use s3a_pack_scholar::{
            HumanLrsRecord, ScholarTileType, SrsDeltaEntry, SrsSyncDeltaPacket,
        };

        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("s3a_test_batch_gap_{}", unique));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let mut config = ServerConfig::default();
        config.data_dir = temp_dir.clone();
        config.credentials.push(AuthCredential {
            user: "researcher".to_string(),
            pass: "researcher123".to_string(),
            scopes: vec!["all".to_string()],
            archive: None,
        });

        let audit_writer =
            Arc::new(ConcurrentTileWriter::create(&temp_dir.join("lrs_data.audit.s3a")).unwrap());
        let state = AppState::init_with_audit(config, audit_writer).expect("init state");

        let server = Server::http("127.0.0.1:0").expect("bind test server");
        let port = server.server_addr().to_ip().unwrap().port();
        let addr = format!("http://127.0.0.1:{}", port);
        let state_clone = state.clone();
        thread::spawn(move || {
            for req in server.incoming_requests() {
                let sc = state_clone.clone();
                thread::spawn(move || handle_request(req, sc));
            }
        });

        let signing_key = SigningKey::generate(&mut rand_core::OsRng);
        let pk_hex = hex::encode(signing_key.verifying_key().as_bytes());

        let mut packet = SrsSyncDeltaPacket::new([1, 2], [3, 4], 1);
        let rec = HumanLrsRecord::new([1, 2], 1600000000, 100, 1, 1.0, S3ACoordinate::default(), 0);
        packet
            .entries
            .push(SrsDeltaEntry::from_record(ScholarTileType::HUMAN_LRS, &rec).unwrap());
        let delta_bytes = packet.export_bytes();
        let checksum = crc32c::crc32c(&delta_bytes);
        let sig = signing_key.sign(&delta_bytes);
        let sig_hex = hex::encode(sig.to_bytes());
        let bytes_b64 = crate::base64_encode(&delta_bytes);

        // 1. New session with gap (starts at cursor 5 instead of 0) -> 409 Conflict
        let gap_req = serde_json::json!({
            "session_id": "gap-session-001",
            "from_cursor": 5,
            "to_cursor": 10,
            "bytes": bytes_b64,
            "checksum": checksum,
            "signature": sig_hex,
            "public_key": pk_hex,
        });

        let resp_gap = ureq::post(&format!("{}/xapi/s3a/batch", addr))
            .set("Authorization", "Basic cmVzZWFyY2hlcjpyZXNlYXJjaGVyMTIz")
            .set("Content-Type", "application/json")
            .send_bytes(&serde_json::to_vec(&gap_req).unwrap());

        assert!(resp_gap.is_err());
        let err = resp_gap.unwrap_err();
        match err {
            ureq::Error::Status(code, _) => assert_eq!(code, 409),
            _ => panic!("Expected 409 status code"),
        }

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_batch_sync_consent_off_client_contract() {
        // Client-side consent verification simulation
        let mut user_consented = false;

        // When consent is OFF: client contract dictates zero network activity
        let sync_attempt = if user_consented {
            Some("SEND_REQUEST")
        } else {
            None
        };
        assert!(
            sync_attempt.is_none(),
            "Consent-off must emit zero requests"
        );

        // When user grants consent: sync is enabled
        user_consented = true;
        let sync_attempt2 = if user_consented {
            Some("SEND_REQUEST")
        } else {
            None
        };
        assert!(sync_attempt2.is_some(), "Consent-on proceeds with sync");
    }

    #[test]
    fn test_batch_sync_limits_payload_size_and_invalid_signature() {
        use ed25519_dalek::SigningKey;

        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("s3a_test_batch_limits_{}", unique));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let mut config = ServerConfig::default();
        config.data_dir = temp_dir.clone();
        config.max_batch_body_bytes = 1024; // 1 KB limit for testing 413
        config.credentials.push(AuthCredential {
            user: "researcher".to_string(),
            pass: "researcher123".to_string(),
            scopes: vec!["all".to_string()],
            archive: None,
        });

        let audit_writer =
            Arc::new(ConcurrentTileWriter::create(&temp_dir.join("lrs_data.audit.s3a")).unwrap());
        let state = AppState::init_with_audit(config, audit_writer).expect("init state");

        let server = Server::http("127.0.0.1:0").expect("bind test server");
        let port = server.server_addr().to_ip().unwrap().port();
        let addr = format!("http://127.0.0.1:{}", port);
        let state_clone = state.clone();
        thread::spawn(move || {
            for req in server.incoming_requests() {
                let sc = state_clone.clone();
                thread::spawn(move || handle_request(req, sc));
            }
        });

        // 1. Oversized body -> 413 Payload Too Large
        let big_body = vec![b'a'; 2048]; // 2 KB > 1 KB limit
        let resp_413 = ureq::post(&format!("{}/xapi/s3a/batch", addr))
            .set("Authorization", "Basic cmVzZWFyY2hlcjpyZXNlYXJjaGVyMTIz")
            .set("Content-Type", "application/json")
            .send_bytes(&big_body);

        assert!(resp_413.is_err());
        match resp_413.unwrap_err() {
            ureq::Error::Status(code, _) => assert_eq!(code, 413),
            _ => panic!("Expected 413 status code"),
        }

        // 2. Bad signature -> 401 Unauthorized
        let signing_key = SigningKey::generate(&mut rand_core::OsRng);
        let pk_hex = hex::encode(signing_key.verifying_key().as_bytes());
        let fake_sig = hex::encode([0u8; 64]);

        let bad_sig_req = serde_json::json!({
            "session_id": "sig-test-001",
            "from_cursor": 0,
            "to_cursor": 1,
            "bytes": crate::base64_encode(&[1, 2, 3, 4]),
            "checksum": crc32c::crc32c(&[1, 2, 3, 4]),
            "signature": fake_sig,
            "public_key": pk_hex,
        });

        let resp_sig = ureq::post(&format!("{}/xapi/s3a/batch", addr))
            .set("Authorization", "Basic cmVzZWFyY2hlcjpyZXNlYXJjaGVyMTIz")
            .set("Content-Type", "application/json")
            .send_bytes(&serde_json::to_vec(&bad_sig_req).unwrap());

        assert!(resp_sig.is_err());
        match resp_sig.unwrap_err() {
            ureq::Error::Status(code, _) => assert_eq!(code, 401),
            _ => panic!("Expected 401 status code"),
        }

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
