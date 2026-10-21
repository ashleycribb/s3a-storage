//! Gate Directory & Relay Mailbox Service (§11).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ed25519_dalek::{Signature, VerifyingKey};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tiny_http::{Header, Method, Response, StatusCode};

use s3a_core::erasure::{ErasureReason, ErasureRecord};
use s3a_core::{FileHeader, HyperTileHeader, TileType, FILE_HEADER_SIZE, HYPER_TILE_SIZE};
use s3a_engine::sealed::{seal_buffer_v2, SealOptionsV2};
use s3a_engine::{MmapReader, TileWriter};
use s3a_vault::address::GateAddress;

/// Maximum payload capacity inside a 128 KB HyperTile (leaving room for header).
const CHUNK_PAYLOAD_CAPACITY: usize = 120 * 1024;

/// Configurable limits for Gate Directory and Relay Mailbox operations (§11).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct GateDirectoryLimits {
    /// Maximum number of queued items allowed per address (default: 100).
    pub max_mailbox_items_per_address: usize,
    /// Maximum size of an individual mailbox item in bytes (default: 1 MB = 1,048,576 bytes).
    pub max_item_size: usize,
    /// Maximum total size across all stored mailbox items in bytes (default: 100 MB = 104,857,600 bytes).
    pub max_total_mailbox_bytes: usize,
    /// Lifetime of queued mailbox items in seconds (default: 30 days = 2,592,000 seconds).
    pub mailbox_item_expiry_secs: u64,
    /// Maximum registration requests per client per 60-second window (default: 10).
    pub registrations_per_client_per_minute: u32,
}

impl Default for GateDirectoryLimits {
    fn default() -> Self {
        Self {
            max_mailbox_items_per_address: 100,
            max_item_size: 1024 * 1024,                 // 1 MB
            max_total_mailbox_bytes: 100 * 1024 * 1024, // 100 MB
            mailbox_item_expiry_secs: 30 * 86400,       // 30 days
            registrations_per_client_per_minute: 10,
        }
    }
}

/// Errors returned by Gate Directory and Relay Mailbox operations with distinct HTTP semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateDirectoryError {
    /// 429 Too Many Requests: Client exceeded registration rate limit
    RateLimitExceeded(String),
    /// 429 Too Many Requests: Mailbox queue full for address
    MailboxItemLimitExceeded(String),
    /// 413 Payload Too Large: Individual item exceeded max item size
    ItemTooLarge(String),
    /// 413 Payload Too Large: Aggregate mailbox size exceeded capacity
    TotalMailboxBytesExceeded(String),
    /// 401 Unauthorized: Challenge-response signature verification failed or challenge missing/expired
    Unauthorized(String),
    /// 404 Not Found: Name or address not registered
    NotFound(String),
    /// 409 Conflict: Name already registered by a different authority key
    NameConflict(String),
    /// 400 Bad Request: Malformed request payload, key or address mismatch
    BadRequest(String),
    /// 500 Internal Server Error
    Internal(String),
}

impl GateDirectoryError {
    pub fn status_code(&self) -> u16 {
        match self {
            Self::RateLimitExceeded(_) | Self::MailboxItemLimitExceeded(_) => 429,
            Self::ItemTooLarge(_) | Self::TotalMailboxBytesExceeded(_) => 413,
            Self::Unauthorized(_) => 401,
            Self::NotFound(_) => 404,
            Self::NameConflict(_) => 409,
            Self::BadRequest(_) => 400,
            Self::Internal(_) => 500,
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::RateLimitExceeded(m)
            | Self::MailboxItemLimitExceeded(m)
            | Self::ItemTooLarge(m)
            | Self::TotalMailboxBytesExceeded(m)
            | Self::Unauthorized(m)
            | Self::NotFound(m)
            | Self::NameConflict(m)
            | Self::BadRequest(m)
            | Self::Internal(m) => m,
        }
    }
}

impl std::fmt::Display for GateDirectoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.status_code(), self.message())
    }
}

impl std::error::Error for GateDirectoryError {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryEntry {
    pub name: String,
    pub address: String,
    pub public_key: String, // hex
    pub endpoints: Vec<String>,
    pub sequence: u32,
    pub valid_from: u64,
    pub valid_until: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterGateRequest {
    pub name: String,
    pub address: String,
    pub public_key: String,
    pub endpoints: Vec<String>,
    pub challenge: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrieveMailboxRequest {
    pub challenge: String,
    pub signature: String,
    #[serde(default)]
    pub public_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailboxChunkHeader {
    pub item_hash: String, // hex
    pub target_address: String,
    pub deposited_at: u64,
    pub chunk_index: u32,
    pub total_chunks: u32,
    pub total_bytes: usize,
}

#[derive(Debug, Clone)]
pub struct StoredMailboxItem {
    pub item_hash: [u8; 32],
    pub target_address: String,
    pub deposited_at: u64,
    pub payload: Vec<u8>,
}

/// Gate Directory & Store-and-Forward Relay Mailbox service with S3A archive persistence.
pub struct GateDirectoryService {
    /// Persistent S3A archive path (`gate_directory.s3a`)
    archive_path: Option<PathBuf>,
    /// Configurable limits
    limits: GateDirectoryLimits,
    /// Active challenge nonces mapping hex_nonce -> expires_at
    challenges: HashMap<String, u64>,
    /// Maps gate_name -> DirectoryEntry (latest wins)
    names: HashMap<String, DirectoryEntry>,
    /// Maps gate_address -> DirectoryEntry (latest wins)
    addresses: HashMap<String, DirectoryEntry>,
    /// Relay mailbox queues mapping gate_address -> queued items
    mailboxes: HashMap<String, Vec<StoredMailboxItem>>,
    /// Rate limiter: client -> (minute_window, count)
    rate_limits: HashMap<String, (u64, u32)>,
}

pub type SharedGateDirectory = Arc<Mutex<GateDirectoryService>>;

fn get_active_data_len(buf: &[u8]) -> Result<usize, GateDirectoryError> {
    if buf.len() < FILE_HEADER_SIZE {
        return Err(GateDirectoryError::Internal(
            "Archive buffer too small for S3A header".into(),
        ));
    }
    let slot_a: &FileHeader = &bytemuck::pod_read_unaligned::<FileHeader>(&buf[0..64]);
    let slot_b: &FileHeader = &bytemuck::pod_read_unaligned::<FileHeader>(&buf[64..128]);
    let (active, _) = s3a_engine::resolve_active_header(slot_a, slot_b)
        .map_err(|e| GateDirectoryError::Internal(e.to_string()))?;
    let len = FILE_HEADER_SIZE + (active.tile_count as usize * HYPER_TILE_SIZE);
    Ok(len.min(buf.len()))
}

impl GateDirectoryService {
    /// Creates an in-memory instance with default limits and no persistence.
    pub fn new() -> Self {
        Self::with_limits(GateDirectoryLimits::default())
    }

    /// Creates an in-memory instance with specific limits.
    pub fn with_limits(limits: GateDirectoryLimits) -> Self {
        Self {
            archive_path: None,
            limits,
            challenges: HashMap::new(),
            names: HashMap::new(),
            addresses: HashMap::new(),
            mailboxes: HashMap::new(),
            rate_limits: HashMap::new(),
        }
    }

    /// Opens or creates a persistent `GateDirectoryService` backed by `gate_directory.s3a`.
    /// On startup, scans all tiles to reconstruct registered names/addresses (latest wins)
    /// and active un-erased, non-expired mailbox items.
    pub fn open_or_create(
        archive_path: Option<PathBuf>,
        limits: GateDirectoryLimits,
    ) -> Result<Self, GateDirectoryError> {
        let mut dir = Self {
            archive_path: archive_path.clone(),
            limits,
            challenges: HashMap::new(),
            names: HashMap::new(),
            addresses: HashMap::new(),
            mailboxes: HashMap::new(),
            rate_limits: HashMap::new(),
        };

        if let Some(ref path) = archive_path {
            if path.exists() {
                let file_len = fs::metadata(path)
                    .map_err(|e| GateDirectoryError::Internal(e.to_string()))?
                    .len();

                if file_len >= FILE_HEADER_SIZE as u64 {
                    let reader = MmapReader::open(path)
                        .map_err(|e| GateDirectoryError::Internal(e.to_string()))?;

                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();

                    let mut temp_chunks: HashMap<
                        String,
                        (MailboxChunkHeader, Vec<(u32, Vec<u8>)>),
                    > = HashMap::new();
                    let mut erased_hashes: HashSet<[u8; 32]> = HashSet::new();

                    for i in 0..reader.tile_count() as usize {
                        if let Some((hdr, payload)) = reader.get_tile(i) {
                            if hdr.tile_type == TileType::GATE_DIRECTORY {
                                if let Ok(entry) = serde_json::from_slice::<DirectoryEntry>(payload)
                                {
                                    let clean_name = entry.name.trim().to_ascii_lowercase();
                                    dir.names.insert(clean_name, entry.clone());
                                    dir.addresses.insert(entry.address.clone(), entry);
                                }
                            } else if hdr.tile_type == TileType::GATE_MAILBOX {
                                if let Some(sep) = payload.iter().position(|&b| b == 0) {
                                    if let Ok(chunk_hdr) =
                                        serde_json::from_slice::<MailboxChunkHeader>(
                                            &payload[..sep],
                                        )
                                    {
                                        let chunk_bytes = payload[sep + 1..].to_vec();
                                        let entry = temp_chunks
                                            .entry(chunk_hdr.item_hash.clone())
                                            .or_insert_with(|| (chunk_hdr.clone(), Vec::new()));
                                        entry.1.push((chunk_hdr.chunk_index, chunk_bytes));
                                    }
                                }
                            } else if hdr.tile_type == TileType::ERASURE {
                                let rec_size = std::mem::size_of::<ErasureRecord>();
                                let count = payload.len() / rec_size;
                                for e in 0..count {
                                    if (e + 1) * rec_size <= payload.len() {
                                        let rec: &ErasureRecord =
                                            &bytemuck::pod_read_unaligned::<ErasureRecord>(
                                                &payload[e * rec_size..(e + 1) * rec_size],
                                            );
                                        erased_hashes.insert(rec.erased_record_hash);
                                    }
                                }
                            }
                        }
                    }

                    // Assemble un-erased, non-expired mailbox items
                    for (hash_hex, (chunk_hdr, mut chunks)) in temp_chunks {
                        let mut item_hash = [0u8; 32];
                        if hex::decode_to_slice(&hash_hex, &mut item_hash).is_err() {
                            continue;
                        }
                        if erased_hashes.contains(&item_hash) {
                            continue; // Erased via acknowledged pickup or purge
                        }
                        if now.saturating_sub(chunk_hdr.deposited_at)
                            > limits.mailbox_item_expiry_secs
                        {
                            continue; // Expired
                        }
                        chunks.sort_by_key(|(idx, _)| *idx);
                        if chunks.len() as u32 == chunk_hdr.total_chunks {
                            let mut full_payload = Vec::with_capacity(chunk_hdr.total_bytes);
                            for (_, c_bytes) in chunks {
                                full_payload.extend_from_slice(&c_bytes);
                            }
                            let computed_hash: [u8; 32] = Sha256::digest(&full_payload).into();
                            if computed_hash == item_hash {
                                dir.mailboxes
                                    .entry(chunk_hdr.target_address.clone())
                                    .or_default()
                                    .push(StoredMailboxItem {
                                        item_hash,
                                        target_address: chunk_hdr.target_address,
                                        deposited_at: chunk_hdr.deposited_at,
                                        payload: full_payload,
                                    });
                            }
                        }
                    }
                }
            }
        }

        Ok(dir)
    }

    /// Appends one or more tiles to the archive and seals with Sealed Footer v2.
    fn append_tiles_and_seal(&self, tiles: &[(u32, Vec<u8>)]) -> Result<(), GateDirectoryError> {
        let Some(ref path) = self.archive_path else {
            return Ok(());
        };

        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        if !path.exists() {
            let writer = TileWriter::create(path).map_err(|e| {
                GateDirectoryError::Internal(format!("Failed to create archive: {}", e))
            })?;
            drop(writer);
        }

        let mut writer = TileWriter::open_append(path).map_err(|e| {
            GateDirectoryError::Internal(format!("Failed to open archive for append: {}", e))
        })?;

        for (tile_type, payload) in tiles {
            let mut hdr = HyperTileHeader::new(0, *tile_type);
            hdr.record_count = if *tile_type == TileType::ERASURE {
                (payload.len() / std::mem::size_of::<ErasureRecord>()) as u32
            } else {
                1
            };
            hdr.payload_bytes = payload.len() as u32;
            hdr.data_crc32 = crc32c::crc32c(payload);
            writer.write_raw_hyper_tile(&hdr, payload).map_err(|e| {
                GateDirectoryError::Internal(format!("Failed to write hyper tile: {}", e))
            })?;
        }
        drop(writer);

        let in_bytes = fs::read(path)
            .map_err(|e| GateDirectoryError::Internal(format!("Failed to read archive: {}", e)))?;
        let active_len = get_active_data_len(&in_bytes)?;

        let mut opts = SealOptionsV2::default();
        opts.write_manifest = true;
        opts.title = Some("S3A Gate Directory State Archive".to_string());
        let sealed_bytes = seal_buffer_v2(&in_bytes[..active_len], &opts)
            .map_err(|e| GateDirectoryError::Internal(format!("Failed to seal archive: {}", e)))?;

        fs::write(path, sealed_bytes).map_err(|e| {
            GateDirectoryError::Internal(format!("Failed to write sealed archive: {}", e))
        })?;

        Ok(())
    }

    /// Issue a fresh 32-byte cryptographic challenge nonce valid for 60 seconds.
    pub fn create_challenge(&mut self) -> String {
        let mut nonce = [0u8; 32];
        OsRng.fill_bytes(&mut nonce);
        let hex_nonce = hex::encode(nonce);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        self.challenges.insert(hex_nonce.clone(), now + 60);
        hex_nonce
    }

    /// Verify challenge nonce and prove ownership of private key via Ed25519 signature.
    pub fn verify_challenge(
        &mut self,
        challenge_hex: &str,
        public_key_bytes: &[u8; 32],
        signature_hex: &str,
    ) -> Result<(), &'static str> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // 1. Validate challenge existence and expiry
        let expires_at = self
            .challenges
            .remove(challenge_hex)
            .ok_or("Challenge expired or not found")?;

        if now > expires_at {
            return Err("Challenge expired");
        }

        // 2. Decode challenge nonce
        let challenge_bytes =
            hex::decode(challenge_hex).map_err(|_| "Invalid challenge hex format")?;

        // 3. Decode signature
        let mut sig_bytes = [0u8; 64];
        hex::decode_to_slice(signature_hex, &mut sig_bytes)
            .map_err(|_| "Invalid signature hex format")?;
        let sig = Signature::from_bytes(&sig_bytes);

        // 4. Verify Ed25519 signature
        let vk =
            VerifyingKey::from_bytes(public_key_bytes).map_err(|_| "Invalid public key format")?;

        vk.verify_strict(&challenge_bytes, &sig)
            .map_err(|_| "Forged challenge rejected: signature verification failed")?;

        Ok(())
    }

    /// Register or re-point a gate name (§11.2).
    /// Invariant: Names are first-come, but only the original address authority key can re-point.
    /// Appends signed directory entry tile to `gate_directory.s3a` (latest wins).
    pub fn register_gate(
        &mut self,
        req: RegisterGateRequest,
        client_id: &str,
    ) -> Result<DirectoryEntry, GateDirectoryError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // 1. Rate limiting check: max registrations per minute per client
        let minute_bucket = now / 60;
        let entry = self
            .rate_limits
            .entry(client_id.to_string())
            .or_insert((minute_bucket, 0));
        if entry.0 == minute_bucket {
            entry.1 += 1;
            if entry.1 > self.limits.registrations_per_client_per_minute {
                return Err(GateDirectoryError::RateLimitExceeded(format!(
                    "Rate limit exceeded: maximum {} registrations per client per minute",
                    self.limits.registrations_per_client_per_minute
                )));
            }
        } else {
            *entry = (minute_bucket, 1);
        }

        let mut pk_bytes = [0u8; 32];
        hex::decode_to_slice(&req.public_key, &mut pk_bytes)
            .map_err(|_| GateDirectoryError::BadRequest("Invalid public key hex format".into()))?;

        // 2. Validate address matches derived address
        let expected_addr = GateAddress::from_public_key(&pk_bytes).to_address_string();
        if req.address != expected_addr {
            return Err(GateDirectoryError::BadRequest(
                "Gate address does not match public key".into(),
            ));
        }

        // 3. Verify challenge-response signature
        self.verify_challenge(&req.challenge, &pk_bytes, &req.signature)
            .map_err(|e| GateDirectoryError::Unauthorized(e.to_string()))?;

        // 4. Name conflict check: re-pointing requires same authority key
        let clean_name = req.name.trim().to_ascii_lowercase();
        if let Some(existing) = self.names.get(&clean_name) {
            if existing.public_key != req.public_key {
                return Err(GateDirectoryError::NameConflict(
                    "Gate name already registered by another authority".into(),
                ));
            }
        }

        let seq = self
            .names
            .get(&clean_name)
            .map(|e| e.sequence + 1)
            .unwrap_or(1);

        let dir_entry = DirectoryEntry {
            name: clean_name.clone(),
            address: req.address.clone(),
            public_key: req.public_key.clone(),
            endpoints: req.endpoints,
            sequence: seq,
            valid_from: now,
            valid_until: now + 86400 * 30, // 30 days
        };

        self.names.insert(clean_name, dir_entry.clone());
        self.addresses.insert(req.address, dir_entry.clone());

        // 5. Append signed directory record to archive
        let payload = serde_json::to_vec(&dir_entry).unwrap();
        self.append_tiles_and_seal(&[(TileType::GATE_DIRECTORY, payload)])?;

        Ok(dir_entry)
    }

    /// Resolve a gate by name or address (§11.2).
    pub fn resolve(&self, target: &str) -> Option<DirectoryEntry> {
        let clean = target.trim().to_ascii_lowercase();
        if clean.starts_with("s3a-") {
            self.addresses.get(&clean).cloned()
        } else {
            self.names.get(&clean).cloned()
        }
    }

    /// Deposit sealed tile buffer into an offline vault's relay mailbox (§11.5).
    /// Enforces size limits, item count limits, total bytes limits, and item expiry.
    pub fn deposit_mailbox(
        &mut self,
        target_address: &str,
        tile_bytes: Vec<u8>,
    ) -> Result<[u8; 32], GateDirectoryError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // 1. Max item size check
        if tile_bytes.len() > self.limits.max_item_size {
            return Err(GateDirectoryError::ItemTooLarge(format!(
                "Item size ({} bytes) exceeds maximum allowed size ({} bytes)",
                tile_bytes.len(),
                self.limits.max_item_size
            )));
        }

        // 2. Automatically purge any expired items
        self.purge_expired_items(now);

        // 3. Max total mailbox storage check
        let current_total: usize = self
            .mailboxes
            .values()
            .map(|q| q.iter().map(|it| it.payload.len()).sum::<usize>())
            .sum();

        if current_total + tile_bytes.len() > self.limits.max_total_mailbox_bytes {
            return Err(GateDirectoryError::TotalMailboxBytesExceeded(format!(
                "Total mailbox storage would exceed limit of {} bytes (current: {}, incoming: {})",
                self.limits.max_total_mailbox_bytes,
                current_total,
                tile_bytes.len()
            )));
        }

        // 4. Mailbox items per address check
        let clean_addr = target_address.trim().to_ascii_lowercase();
        let current_count = self
            .mailboxes
            .get(&clean_addr)
            .map(|q| q.len())
            .unwrap_or(0);
        if current_count >= self.limits.max_mailbox_items_per_address {
            return Err(GateDirectoryError::MailboxItemLimitExceeded(format!(
                "Mailbox items limit exceeded for address {} (maximum {})",
                clean_addr, self.limits.max_mailbox_items_per_address
            )));
        }

        let item_hash: [u8; 32] = Sha256::digest(&tile_bytes).into();
        let item_hash_hex = hex::encode(item_hash);

        // 5. Write tiles to archive if persistence is configured
        let chunks: Vec<&[u8]> = tile_bytes.chunks(CHUNK_PAYLOAD_CAPACITY).collect();
        let total_chunks = chunks.len() as u32;
        let mut tiles_to_write = Vec::new();

        for (idx, chunk) in chunks.iter().enumerate() {
            let chunk_hdr = MailboxChunkHeader {
                item_hash: item_hash_hex.clone(),
                target_address: clean_addr.clone(),
                deposited_at: now,
                chunk_index: idx as u32,
                total_chunks,
                total_bytes: tile_bytes.len(),
            };
            let mut buf = serde_json::to_vec(&chunk_hdr).unwrap();
            buf.push(0); // delimiter
            buf.extend_from_slice(chunk);
            tiles_to_write.push((TileType::GATE_MAILBOX, buf));
        }

        self.append_tiles_and_seal(&tiles_to_write)?;

        self.mailboxes
            .entry(clean_addr.clone())
            .or_default()
            .push(StoredMailboxItem {
                item_hash,
                target_address: clean_addr,
                deposited_at: now,
                payload: tile_bytes,
            });

        Ok(item_hash)
    }

    /// Retrieve and drain queued sealed tiles for an address (§11.5).
    /// Requires challenge-response proof of the address key.
    /// Writes cryptographic erasure receipts to the archive for all drained items.
    pub fn retrieve_mailbox(
        &mut self,
        target_address: &str,
        req: RetrieveMailboxRequest,
    ) -> Result<Vec<Vec<u8>>, GateDirectoryError> {
        let clean_addr = target_address.trim().to_ascii_lowercase();

        // 1. Resolve verifying key: from request or stored directory entry
        let pk_bytes = if let Some(ref pk_hex) = req.public_key {
            let mut pk = [0u8; 32];
            hex::decode_to_slice(pk_hex, &mut pk).map_err(|_| {
                GateDirectoryError::BadRequest("Invalid public key hex format".into())
            })?;
            let derived = GateAddress::from_public_key(&pk).to_address_string();
            if derived != clean_addr {
                return Err(GateDirectoryError::BadRequest(
                    "Provided public key does not match gate address".into(),
                ));
            }
            pk
        } else if let Some(entry) = self.addresses.get(&clean_addr) {
            let mut pk = [0u8; 32];
            hex::decode_to_slice(&entry.public_key, &mut pk)
                .map_err(|_| GateDirectoryError::Internal("Corrupted stored public key".into()))?;
            pk
        } else {
            return Err(GateDirectoryError::NotFound(
                "Target address not registered and public_key not provided in request".into(),
            ));
        };

        // 2. Authenticate request via challenge-response signature
        self.verify_challenge(&req.challenge, &pk_bytes, &req.signature)
            .map_err(|e| GateDirectoryError::Unauthorized(e.to_string()))?;

        // 3. Drain mailbox queue
        let items = self.mailboxes.remove(&clean_addr).unwrap_or_default();

        // 4. Record erasure receipts for acknowledged pickup
        if !items.is_empty() {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let authority = GateAddress::from_public_key(&pk_bytes).checksum as u64;

            let mut erasures = Vec::new();
            for item in &items {
                let rec = ErasureRecord::new(
                    item.item_hash,
                    TileType::GATE_MAILBOX,
                    0,
                    0,
                    ErasureReason::Withdrawal,
                    authority,
                    now,
                    now,
                    [0u8; 32],
                );
                erasures.push((TileType::ERASURE, bytemuck::bytes_of(&rec).to_vec()));
            }
            let _ = self.append_tiles_and_seal(&erasures);
        }

        Ok(items.into_iter().map(|it| it.payload).collect())
    }

    /// Purges all expired mailbox items according to `mailbox_item_expiry_secs`.
    /// Writes cryptographic erasure receipts to the archive for purged items.
    pub fn purge_expired_items(&mut self, now: u64) -> usize {
        let expiry = self.limits.mailbox_item_expiry_secs;
        let mut purged_count = 0;
        let mut erasures = Vec::new();

        for (_addr, queue) in self.mailboxes.iter_mut() {
            let mut active = Vec::new();
            for item in queue.drain(..) {
                if now.saturating_sub(item.deposited_at) > expiry {
                    purged_count += 1;
                    let rec = ErasureRecord::new(
                        item.item_hash,
                        TileType::GATE_MAILBOX,
                        0,
                        0,
                        ErasureReason::RetentionExpiry,
                        0,
                        now,
                        now,
                        [0u8; 32],
                    );
                    erasures.push((TileType::ERASURE, bytemuck::bytes_of(&rec).to_vec()));
                } else {
                    active.push(item);
                }
            }
            *queue = active;
        }

        if !erasures.is_empty() {
            let _ = self.append_tiles_and_seal(&erasures);
        }

        purged_count
    }

    /// Returns the number of queued items for an address.
    pub fn mailbox_item_count(&self, address: &str) -> usize {
        let clean = address.trim().to_ascii_lowercase();
        self.mailboxes.get(&clean).map(|q| q.len()).unwrap_or(0)
    }

    /// Returns the active configured limits.
    pub fn limits(&self) -> GateDirectoryLimits {
        self.limits
    }
}

pub fn handle_gate_directory(
    method: &Method,
    path: &str,
    body: &[u8],
    client_id: &str,
    service: &Arc<Mutex<GateDirectoryService>>,
) -> Response<Cursor<Vec<u8>>> {
    if path == "/gate/challenge" {
        if method == &Method::Get || method == &Method::Post {
            let challenge = service.lock().unwrap().create_challenge();
            let json = serde_json::json!({ "challenge": challenge });
            let bytes = serde_json::to_vec(&json).unwrap();
            return make_json_response(200, &bytes);
        } else {
            return make_err_response(405, "Method Not Allowed");
        }
    }

    if path == "/gate/register" {
        if method == &Method::Post {
            let req: Result<RegisterGateRequest, _> = serde_json::from_slice(body);
            match req {
                Ok(r) => match service.lock().unwrap().register_gate(r, client_id) {
                    Ok(entry) => {
                        let bytes = serde_json::to_vec(&entry).unwrap_or_default();
                        make_json_response(200, &bytes)
                    }
                    Err(err) => make_gate_error_response(&err),
                },
                Err(_) => make_err_response(400, "Invalid JSON payload for RegisterGateRequest"),
            }
        } else {
            make_err_response(405, "Method Not Allowed")
        }
    } else if path.starts_with("/gate/resolve/") {
        if method == &Method::Get || method == &Method::Head {
            let target = &path["/gate/resolve/".len()..];
            if let Some(entry) = service.lock().unwrap().resolve(target) {
                let bytes = serde_json::to_vec(&entry).unwrap_or_default();
                make_json_response(200, &bytes)
            } else {
                make_err_response(404, "Gate not found")
            }
        } else {
            make_err_response(405, "Method Not Allowed")
        }
    } else if path.starts_with("/gate/mailbox/") {
        let rest = &path["/gate/mailbox/".len()..];
        if rest.ends_with("/retrieve") {
            let target_addr = &rest[..rest.len() - "/retrieve".len()];
            if method == &Method::Post {
                let req: Result<RetrieveMailboxRequest, _> = serde_json::from_slice(body);
                match req {
                    Ok(r) => match service.lock().unwrap().retrieve_mailbox(target_addr, r) {
                        Ok(tiles) => {
                            let hex_tiles: Vec<String> =
                                tiles.into_iter().map(hex::encode).collect();
                            let bytes = serde_json::to_vec(&hex_tiles).unwrap_or_default();
                            make_json_response(200, &bytes)
                        }
                        Err(err) => make_gate_error_response(&err),
                    },
                    Err(_) => make_err_response(400, "Invalid JSON for RetrieveMailboxRequest"),
                }
            } else {
                make_err_response(405, "Method Not Allowed")
            }
        } else {
            // Deposit mailbox
            let target_addr = rest;
            if method == &Method::Post {
                match service
                    .lock()
                    .unwrap()
                    .deposit_mailbox(target_addr, body.to_vec())
                {
                    Ok(_) => Response::new(
                        StatusCode(202),
                        vec![
                            Header::from_bytes(&b"Content-Type"[..], &b"text/plain"[..]).unwrap(),
                            Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..])
                                .unwrap(),
                        ],
                        Cursor::new(b"Accepted".to_vec()),
                        Some(8),
                        None,
                    ),
                    Err(err) => make_gate_error_response(&err),
                }
            } else {
                make_err_response(405, "Method Not Allowed")
            }
        }
    } else {
        make_err_response(404, "Endpoint not found")
    }
}

fn make_json_response(status: u16, body: &[u8]) -> Response<Cursor<Vec<u8>>> {
    Response::new(
        StatusCode(status),
        vec![
            Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap(),
            Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap(),
        ],
        Cursor::new(body.to_vec()),
        Some(body.len()),
        None,
    )
}

fn make_err_response(status: u16, msg: &str) -> Response<Cursor<Vec<u8>>> {
    let json = serde_json::json!({ "error": msg }).to_string();
    make_json_response(status, json.as_bytes())
}

fn make_gate_error_response(err: &GateDirectoryError) -> Response<Cursor<Vec<u8>>> {
    make_err_response(err.status_code(), err.message())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn temp_archive_path(label: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "s3a_gate_dir_test_{}_{}.s3a",
            label,
            std::process::id()
        ));
        p
    }

    #[test]
    fn test_gate_directory_challenge_registration_and_relay() {
        let mut dir = GateDirectoryService::new();
        let signing_key = SigningKey::generate(&mut OsRng);
        let pubkey_bytes = signing_key.verifying_key().to_bytes();
        let pubkey_hex = hex::encode(pubkey_bytes);
        let addr = GateAddress::from_public_key(&pubkey_bytes).to_address_string();

        // 1. Request challenge
        let challenge = dir.create_challenge();
        let challenge_bytes = hex::decode(&challenge).unwrap();

        // 2. Sign challenge
        let sig = signing_key.sign(&challenge_bytes);
        let sig_hex = hex::encode(sig.to_bytes());

        // 3. Register gate
        let reg_req = RegisterGateRequest {
            name: "station-alpha".into(),
            address: addr.clone(),
            public_key: pubkey_hex.clone(),
            endpoints: vec!["https://station-alpha.org".into()],
            challenge: challenge.clone(),
            signature: sig_hex.clone(),
        };

        let registered = dir
            .register_gate(reg_req, "127.0.0.1")
            .expect("Registration must succeed");
        assert_eq!(registered.name, "station-alpha");
        assert_eq!(registered.address, addr);

        // Resolve by name and address
        let resolved_name = dir.resolve("station-alpha").expect("Must resolve by name");
        assert_eq!(resolved_name.address, addr);
        let resolved_addr = dir.resolve(&addr).expect("Must resolve by address");
        assert_eq!(resolved_addr.name, "station-alpha");

        // 4. Forged challenge rejection
        let challenge2 = dir.create_challenge();
        let bad_sig_hex = hex::encode([0xFFu8; 64]);
        let forged_req = RegisterGateRequest {
            name: "station-beta".into(),
            address: addr.clone(),
            public_key: pubkey_hex.clone(),
            endpoints: vec![],
            challenge: challenge2,
            signature: bad_sig_hex,
        };
        assert_eq!(
            dir.register_gate(forged_req, "127.0.0.1")
                .unwrap_err()
                .status_code(),
            401
        );

        // 5. Offline relay mailbox deposit and retrieval
        let sample_tile = b"SEALED_TILE_PAYLOAD_12345".to_vec();
        dir.deposit_mailbox(&addr, sample_tile.clone()).unwrap();

        // Retrieve mailbox with challenge-response
        let retrieve_chal = dir.create_challenge();
        let chal_bytes = hex::decode(&retrieve_chal).unwrap();
        let retrieve_sig = signing_key.sign(&chal_bytes);

        let retrieved = dir
            .retrieve_mailbox(
                &addr,
                RetrieveMailboxRequest {
                    challenge: retrieve_chal,
                    signature: hex::encode(retrieve_sig.to_bytes()),
                    public_key: None,
                },
            )
            .expect("Retrieve must succeed with valid auth");

        assert_eq!(retrieved.len(), 1);
        assert_eq!(retrieved[0], sample_tile);

        // Queue is drained
        let retrieve_chal2 = dir.create_challenge();
        let chal2_bytes = hex::decode(&retrieve_chal2).unwrap();
        let retrieve_sig2 = signing_key.sign(&chal2_bytes);
        let empty = dir
            .retrieve_mailbox(
                &addr,
                RetrieveMailboxRequest {
                    challenge: retrieve_chal2,
                    signature: hex::encode(retrieve_sig2.to_bytes()),
                    public_key: None,
                },
            )
            .unwrap();
        assert_eq!(empty.len(), 0);
    }

    #[test]
    fn test_gate_directory_restart_preserves_names_and_mailbox() {
        let archive_path = temp_archive_path("restart_persist");
        let _ = fs::remove_file(&archive_path);

        let signing_key = SigningKey::generate(&mut OsRng);
        let pubkey_bytes = signing_key.verifying_key().to_bytes();
        let pubkey_hex = hex::encode(pubkey_bytes);
        let addr = GateAddress::from_public_key(&pubkey_bytes).to_address_string();

        let limits = GateDirectoryLimits::default();

        // 1. First session: register and re-point name, deposit mailbox item
        {
            let mut dir =
                GateDirectoryService::open_or_create(Some(archive_path.clone()), limits).unwrap();

            // Initial registration
            let c1 = dir.create_challenge();
            let s1 = signing_key.sign(&hex::decode(&c1).unwrap());
            dir.register_gate(
                RegisterGateRequest {
                    name: "vault-one".into(),
                    address: addr.clone(),
                    public_key: pubkey_hex.clone(),
                    endpoints: vec!["gate://127.0.0.1:8001".into()],
                    challenge: c1,
                    signature: hex::encode(s1.to_bytes()),
                },
                "client1",
            )
            .unwrap();

            // Re-point name with updated endpoints (latest wins)
            let c2 = dir.create_challenge();
            let s2 = signing_key.sign(&hex::decode(&c2).unwrap());
            let repointed = dir
                .register_gate(
                    RegisterGateRequest {
                        name: "vault-one".into(),
                        address: addr.clone(),
                        public_key: pubkey_hex.clone(),
                        endpoints: vec!["gate://127.0.0.1:9999".into()],
                        challenge: c2,
                        signature: hex::encode(s2.to_bytes()),
                    },
                    "client1",
                )
                .unwrap();
            assert_eq!(repointed.sequence, 2);
            assert_eq!(repointed.endpoints, vec!["gate://127.0.0.1:9999"]);

            // Deposit mailbox item
            dir.deposit_mailbox(&addr, b"PERSISTED_MAILBOX_TILE_DATA".to_vec())
                .unwrap();
            assert_eq!(dir.mailbox_item_count(&addr), 1);
        }

        // 2. Restart session: re-open from disk
        {
            let dir =
                GateDirectoryService::open_or_create(Some(archive_path.clone()), limits).unwrap();

            // Name preserved and latest wins (endpoints updated, sequence 2)
            let resolved = dir
                .resolve("vault-one")
                .expect("Name must be preserved across restart");
            assert_eq!(resolved.address, addr);
            assert_eq!(resolved.sequence, 2);
            assert_eq!(resolved.endpoints, vec!["gate://127.0.0.1:9999"]);

            // Mailbox items preserved
            assert_eq!(dir.mailbox_item_count(&addr), 1);
        }

        let _ = fs::remove_file(&archive_path);
    }

    #[test]
    fn test_gate_directory_limits_enforced() {
        let limits = GateDirectoryLimits {
            max_mailbox_items_per_address: 2,
            max_item_size: 500,
            max_total_mailbox_bytes: 1000,
            mailbox_item_expiry_secs: 86400,
            registrations_per_client_per_minute: 3,
        };

        let mut dir = GateDirectoryService::with_limits(limits);
        let signing_key = SigningKey::generate(&mut OsRng);
        let pubkey_bytes = signing_key.verifying_key().to_bytes();
        let pubkey_hex = hex::encode(pubkey_bytes);
        let addr = GateAddress::from_public_key(&pubkey_bytes).to_address_string();

        // 1. Max item size rejection (413)
        let large_item = vec![0xAB; 600];
        let err = dir.deposit_mailbox(&addr, large_item).unwrap_err();
        assert_eq!(err.status_code(), 413);
        assert!(err.message().contains("exceeds maximum allowed size"));

        // 2. Queue capacity rejection (429)
        dir.deposit_mailbox(&addr, vec![0x1; 300]).unwrap();
        dir.deposit_mailbox(&addr, vec![0x2; 300]).unwrap();
        let err = dir.deposit_mailbox(&addr, vec![0x3; 300]).unwrap_err();
        assert_eq!(err.status_code(), 429);
        assert!(err.message().contains("limit exceeded for address"));

        // 3. Total mailbox bytes rejection (413) on a second address
        let other_addr = "s3a-02ffffffffffffffffffffffffffffffff";
        let err = dir.deposit_mailbox(other_addr, vec![0x4; 500]).unwrap_err();
        assert_eq!(err.status_code(), 413);
        assert!(err.message().contains("Total mailbox storage"));

        // 4. Registration rate limit rejection (429)
        for i in 0..3 {
            let c = dir.create_challenge();
            let s = signing_key.sign(&hex::decode(&c).unwrap());
            dir.register_gate(
                RegisterGateRequest {
                    name: format!("station-{}", i),
                    address: addr.clone(),
                    public_key: pubkey_hex.clone(),
                    endpoints: vec![],
                    challenge: c,
                    signature: hex::encode(s.to_bytes()),
                },
                "ratelimited_client",
            )
            .unwrap();
        }

        let c4 = dir.create_challenge();
        let s4 = signing_key.sign(&hex::decode(&c4).unwrap());
        let err = dir
            .register_gate(
                RegisterGateRequest {
                    name: "station-overflow".into(),
                    address: addr.clone(),
                    public_key: pubkey_hex.clone(),
                    endpoints: vec![],
                    challenge: c4,
                    signature: hex::encode(s4.to_bytes()),
                },
                "ratelimited_client",
            )
            .unwrap_err();
        assert_eq!(err.status_code(), 429);
        assert!(err.message().contains("Rate limit exceeded"));
    }

    #[test]
    fn test_gate_directory_pickup_without_proof_rejected_and_erasure_recorded() {
        let archive_path = temp_archive_path("pickup_proof");
        let _ = fs::remove_file(&archive_path);

        let signing_key = SigningKey::generate(&mut OsRng);
        let pubkey_bytes = signing_key.verifying_key().to_bytes();
        let pubkey_hex = hex::encode(pubkey_bytes);
        let addr = GateAddress::from_public_key(&pubkey_bytes).to_address_string();
        let limits = GateDirectoryLimits::default();

        let mut dir =
            GateDirectoryService::open_or_create(Some(archive_path.clone()), limits).unwrap();

        // Register gate
        let c = dir.create_challenge();
        let s = signing_key.sign(&hex::decode(&c).unwrap());
        dir.register_gate(
            RegisterGateRequest {
                name: "secure-node".into(),
                address: addr.clone(),
                public_key: pubkey_hex.clone(),
                endpoints: vec![],
                challenge: c,
                signature: hex::encode(s.to_bytes()),
            },
            "c1",
        )
        .unwrap();

        // Deposit item
        dir.deposit_mailbox(&addr, b"SENSITIVE_SEALED_PAYLOAD".to_vec())
            .unwrap();
        assert_eq!(dir.mailbox_item_count(&addr), 1);

        // 1. Pickup without proof (forged signature) is rejected with 401
        let chal = dir.create_challenge();
        let err = dir
            .retrieve_mailbox(
                &addr,
                RetrieveMailboxRequest {
                    challenge: chal,
                    signature: hex::encode([0x00u8; 64]),
                    public_key: None,
                },
            )
            .unwrap_err();
        assert_eq!(err.status_code(), 401);

        // Item remains queued
        assert_eq!(dir.mailbox_item_count(&addr), 1);

        // 2. Pickup with valid proof succeeds
        let valid_chal = dir.create_challenge();
        let valid_sig = signing_key.sign(&hex::decode(&valid_chal).unwrap());
        let items = dir
            .retrieve_mailbox(
                &addr,
                RetrieveMailboxRequest {
                    challenge: valid_chal,
                    signature: hex::encode(valid_sig.to_bytes()),
                    public_key: None,
                },
            )
            .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0], b"SENSITIVE_SEALED_PAYLOAD");

        // Queue is drained
        assert_eq!(dir.mailbox_item_count(&addr), 0);
        drop(dir);

        // 3. Restart: erasure record guarantees item is deleted and not restored
        let restarted =
            GateDirectoryService::open_or_create(Some(archive_path.clone()), limits).unwrap();
        assert_eq!(restarted.mailbox_item_count(&addr), 0);

        let _ = fs::remove_file(&archive_path);
    }

    #[test]
    fn test_gate_directory_expired_items_purged() {
        let archive_path = temp_archive_path("expiry_purge");
        let _ = fs::remove_file(&archive_path);

        let limits = GateDirectoryLimits {
            max_mailbox_items_per_address: 10,
            max_item_size: 1024,
            max_total_mailbox_bytes: 10240,
            mailbox_item_expiry_secs: 5, // 5 second expiry
            registrations_per_client_per_minute: 10,
        };

        let signing_key = SigningKey::generate(&mut OsRng);
        let pubkey_bytes = signing_key.verifying_key().to_bytes();
        let addr = GateAddress::from_public_key(&pubkey_bytes).to_address_string();

        let mut dir =
            GateDirectoryService::open_or_create(Some(archive_path.clone()), limits).unwrap();
        dir.deposit_mailbox(&addr, b"SHORT_LIVED_TILE".to_vec())
            .unwrap();
        assert_eq!(dir.mailbox_item_count(&addr), 1);

        // Advance simulated time past expiry (now + 10s)
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            + 10;

        let purged = dir.purge_expired_items(now);
        assert_eq!(purged, 1);
        assert_eq!(dir.mailbox_item_count(&addr), 0);
        drop(dir);

        // On restart, item was erased in archive and remains absent
        let restarted =
            GateDirectoryService::open_or_create(Some(archive_path.clone()), limits).unwrap();
        assert_eq!(restarted.mailbox_item_count(&addr), 0);

        let _ = fs::remove_file(&archive_path);
    }
}
