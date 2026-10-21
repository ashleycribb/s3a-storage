//! S3A Batched Synchronization Service for Scholar Explorer (§12).
//!
//! Handles POST /xapi/s3a/batch requests containing signed binary delta packets.
//! Enforces:
//! - Checksum verification (hardware CRC32C)
//! - Ed25519 cryptographic signature verification
//! - Rate limiting (per-session window)
//! - Body payload size limits (413)
//! - Strict gap detection and out-of-order rejection (409 Conflict)
//! - Idempotent duplicate re-send handling (200 OK no-op)
//! - Appending validated delta entries to `scholar.s3a`

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use tiny_http::{Header, Method, Response, StatusCode};

use s3a_core::HyperTileHeader;
use s3a_pack_scholar::{ScholarTileType, SrsSyncDeltaPacket};

use crate::AppState;

/// Configurable limits for S3A batch synchronization.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BatchSyncLimits {
    /// Maximum body size in bytes (default: 5 MB).
    pub max_body_bytes: usize,
    /// Maximum batch sync requests per session per minute (default: 60).
    pub max_requests_per_minute: u32,
}

impl Default for BatchSyncLimits {
    fn default() -> Self {
        Self {
            max_body_bytes: 5 * 1024 * 1024, // 5 MB
            max_requests_per_minute: 60,
        }
    }
}

/// Recorded synchronization state for an active session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSyncState {
    pub last_cursor: u64,
    pub last_from: u64,
    pub last_to: u64,
    pub last_sync_sec: u64,
    pub total_records_synced: u64,
}

/// Service managing session synchronization cursors and rate limiting.
pub struct BatchSyncService {
    pub limits: BatchSyncLimits,
    pub sessions: HashMap<String, SessionSyncState>,
    pub rate_limits: HashMap<String, Vec<u64>>,
}

impl BatchSyncService {
    pub fn new(limits: BatchSyncLimits) -> Self {
        Self {
            limits,
            sessions: HashMap::new(),
            rate_limits: HashMap::new(),
        }
    }

    /// Checks if a session has exceeded its rate limit. Returns false if exceeded.
    pub fn check_rate_limit(&mut self, session_id: &str, now_sec: u64) -> bool {
        let timestamps = self.rate_limits.entry(session_id.to_string()).or_default();
        timestamps.retain(|&ts| now_sec.saturating_sub(ts) < 60);
        if timestamps.len() >= self.limits.max_requests_per_minute as usize {
            return false;
        }
        timestamps.push(now_sec);
        true
    }
}

/// JSON Request payload for POST /xapi/s3a/batch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchSyncRequest {
    pub session_id: String,
    #[serde(default)]
    pub from_cursor: Option<u64>,
    #[serde(default)]
    pub to_cursor: Option<u64>,
    #[serde(default)]
    pub cursor_range: Option<[u64; 2]>,
    pub bytes: String,
    pub checksum: u32,
    pub signature: String,
    pub public_key: String,
}

impl BatchSyncRequest {
    pub fn get_cursor_range(&self) -> Result<(u64, u64), String> {
        if let Some(range) = self.cursor_range {
            return Ok((range[0], range[1]));
        }
        match (self.from_cursor, self.to_cursor) {
            (Some(f), Some(t)) => Ok((f, t)),
            _ => Err("Missing cursor range (either cursor_range: [from, to] or from_cursor and to_cursor must be provided)".to_string()),
        }
    }
}

/// JSON Response payload for POST /xapi/s3a/batch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchSyncResponse {
    pub status: String,
    pub session_id: String,
    pub accepted_cursor: u64,
    pub duplicate: bool,
    pub records_appended: usize,
}

fn current_timestamp_sec() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn decode_bytes_hex_or_base64(s: &str, expected_len: usize) -> Result<Vec<u8>, String> {
    let clean: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if clean.len() == expected_len * 2 {
        if let Ok(bytes) = hex::decode(&clean) {
            return Ok(bytes);
        }
    }
    if let Ok(bytes) = crate::base64_decode(&clean) {
        if bytes.len() == expected_len {
            return Ok(bytes);
        }
    }
    Err(format!(
        "Expected {} bytes encoded as hex or base64 (got length {})",
        expected_len,
        clean.len()
    ))
}

fn json_response<T: Serialize>(status_code: u16, data: &T) -> Response<Cursor<Vec<u8>>> {
    let body = serde_json::to_vec(data).unwrap_or_default();
    Response::new(
        StatusCode(status_code),
        vec![
            Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap(),
            Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap(),
            Header::from_bytes(
                &b"Access-Control-Allow-Headers"[..],
                &b"Content-Type, Authorization, X-Experience-API-Version"[..],
            )
            .unwrap(),
            Header::from_bytes(&b"Access-Control-Allow-Methods"[..], &b"POST, OPTIONS"[..])
                .unwrap(),
            Header::from_bytes(&b"Date"[..], crate::format_http_date().as_bytes()).unwrap(),
        ],
        Cursor::new(body.clone()),
        Some(body.len()),
        None,
    )
}

fn error_response(status_code: u16, message: &str) -> Response<Cursor<Vec<u8>>> {
    let err = serde_json::json!({ "error": message });
    json_response(status_code, &err)
}

/// Handles incoming POST /xapi/s3a/batch requests.
pub fn handle_batch_sync(
    method: &Method,
    _headers: &HashMap<String, String>,
    body_bytes: &[u8],
    state: &Arc<AppState>,
) -> Response<Cursor<Vec<u8>>> {
    if method != &Method::Post {
        return error_response(405, "Method Not Allowed: Batch sync only accepts POST");
    }

    // 1. Limit body size (413 Payload Too Large)
    let max_bytes = {
        let service = state.batch_sync.lock().unwrap();
        service.limits.max_body_bytes
    };
    if body_bytes.len() > max_bytes {
        return error_response(
            413,
            &format!(
                "Payload Too Large: batch size {} bytes exceeds maximum allowed {} bytes",
                body_bytes.len(),
                max_bytes
            ),
        );
    }

    // 2. Parse JSON Request
    let req: BatchSyncRequest = match serde_json::from_slice(body_bytes) {
        Ok(r) => r,
        Err(e) => return error_response(400, &format!("Invalid JSON payload: {}", e)),
    };

    let (from_cursor, to_cursor) = match req.get_cursor_range() {
        Ok(r) => r,
        Err(e) => return error_response(400, &e),
    };

    if from_cursor > to_cursor {
        return error_response(
            400,
            &format!(
                "Invalid cursor range: from_cursor {} cannot be greater than to_cursor {}",
                from_cursor, to_cursor
            ),
        );
    }

    let now_sec = current_timestamp_sec();

    // 3. Per-session rate limiting (429 Too Many Requests)
    {
        let mut service = state.batch_sync.lock().unwrap();
        if !service.check_rate_limit(&req.session_id, now_sec) {
            return error_response(
                429,
                &format!(
                    "Rate limit exceeded: maximum {} batch requests per minute for session",
                    service.limits.max_requests_per_minute
                ),
            );
        }
    }

    // 4. Decode delta bytes
    let raw_delta = match crate::base64_decode(&req.bytes) {
        Ok(b) => b,
        Err(_) => return error_response(400, "Invalid base64 encoding in 'bytes' field"),
    };

    // 5. Verify Checksum (CRC32C)
    let computed_crc = crc32c::crc32c(&raw_delta);
    if computed_crc != req.checksum {
        return error_response(
            400,
            &format!(
                "Checksum mismatch: computed CRC32C 0x{:08X} ({}) != expected 0x{:08X} ({})",
                computed_crc, computed_crc, req.checksum, req.checksum
            ),
        );
    }

    // 6. Verify Ed25519 cryptographic signature
    let pk_bytes = match decode_bytes_hex_or_base64(&req.public_key, 32) {
        Ok(b) => b,
        Err(e) => return error_response(400, &format!("Invalid public_key: {}", e)),
    };
    let verifying_key = match VerifyingKey::from_bytes(pk_bytes[..32].try_into().unwrap()) {
        Ok(vk) => vk,
        Err(e) => return error_response(400, &format!("Malformed Ed25519 public key: {}", e)),
    };

    let sig_bytes = match decode_bytes_hex_or_base64(&req.signature, 64) {
        Ok(b) => b,
        Err(e) => return error_response(400, &format!("Invalid signature format: {}", e)),
    };
    let signature = Signature::from_bytes(sig_bytes[..64].try_into().unwrap());

    // Allow signing either raw delta bytes directly or canonical envelope
    let envelope_str = format!(
        "{}:{}:{}:{}",
        req.session_id, from_cursor, to_cursor, req.checksum
    );
    let mut sig_valid = verifying_key.verify(&raw_delta, &signature).is_ok();
    if !sig_valid {
        sig_valid = verifying_key
            .verify(envelope_str.as_bytes(), &signature)
            .is_ok();
    }
    if !sig_valid {
        let envelope_short = format!("{}:{}:{}", req.session_id, from_cursor, to_cursor);
        sig_valid = verifying_key
            .verify(envelope_short.as_bytes(), &signature)
            .is_ok();
    }
    if !sig_valid {
        return error_response(
            401,
            "Signature verification failed: invalid Ed25519 signature",
        );
    }

    // 7. Parse and validate S3ASYNC1 delta packet
    let packet = match SrsSyncDeltaPacket::parse_bytes(&raw_delta) {
        Ok(p) => p,
        Err(e) => return error_response(400, &format!("Invalid sync delta packet: {:?}", e)),
    };

    // 8. Session Cursor Tracking & Deduplication / Gap Rejection
    let mut service = state.batch_sync.lock().unwrap();
    let maybe_session = service.sessions.get(&req.session_id).cloned();

    match maybe_session {
        Some(existing) => {
            // Check for exact duplicate re-send (Idempotent 200 no-op)
            if from_cursor == existing.last_from && to_cursor == existing.last_to {
                return json_response(
                    200,
                    &BatchSyncResponse {
                        status: "ok".to_string(),
                        session_id: req.session_id,
                        accepted_cursor: existing.last_cursor,
                        duplicate: true,
                        records_appended: 0,
                    },
                );
            }

            // Check for gaps
            if from_cursor > existing.last_cursor {
                return error_response(
                    409,
                    &format!(
                        "Gap detected: session expected from_cursor {}, but received {}",
                        existing.last_cursor, from_cursor
                    ),
                );
            }

            // Check for out-of-order delta
            if from_cursor < existing.last_cursor {
                return error_response(
                    409,
                    &format!(
                        "Out-of-order delta rejected: received from_cursor {} but session has already advanced to cursor {}",
                        from_cursor, existing.last_cursor
                    ),
                );
            }
        }
        None => {
            // New session must begin at cursor 0
            if from_cursor > 0 {
                return error_response(
                    409,
                    &format!(
                        "Gap detected: new session '{}' must begin from_cursor 0, but received {}",
                        req.session_id, from_cursor
                    ),
                );
            }
        }
    }

    // 9. Append records to scholar.s3a
    let records_to_append = packet.entries.len();
    if records_to_append > 0 {
        let store = match state.get_or_create_scholar_store() {
            Ok(s) => s,
            Err(e) => {
                return error_response(500, &format!("Failed to open scholar.s3a archive: {:?}", e))
            }
        };

        // Group entries by tile_type to write coherent S3A hyper-tiles
        let mut grouped_entries: HashMap<u32, Vec<u8>> = HashMap::new();
        let mut group_counts: HashMap<u32, u32> = HashMap::new();

        for entry in &packet.entries {
            let record_len = match entry.tile_type {
                ScholarTileType::NOTE_CLAIMS => 160,
                ScholarTileType::CITATION_FINGERPRINT => 96,
                _ => 64,
            };
            let slice = &entry.record_bytes()[..record_len];
            grouped_entries
                .entry(entry.tile_type)
                .or_default()
                .extend_from_slice(slice);
            *group_counts.entry(entry.tile_type).or_default() += 1;
        }

        for (tile_type, payload) in grouped_entries {
            let count = group_counts.get(&tile_type).copied().unwrap_or(1);
            let mut hdr = HyperTileHeader::new(0, tile_type);
            hdr.record_count = count;
            hdr.payload_bytes = payload.len() as u32;
            hdr.data_crc32 = crc32c::crc32c(&payload);

            if let Err(e) = store.writer().write_raw_hyper_tile(&hdr, &payload) {
                return error_response(
                    500,
                    &format!(
                        "Failed to append tile type 0x{:08X} to scholar.s3a: {}",
                        tile_type, e
                    ),
                );
            }
        }
    }

    // 10. Advance session cursor state
    let updated_state = SessionSyncState {
        last_cursor: to_cursor,
        last_from: from_cursor,
        last_to: to_cursor,
        last_sync_sec: now_sec,
        total_records_synced: service
            .sessions
            .get(&req.session_id)
            .map(|s| s.total_records_synced + records_to_append as u64)
            .unwrap_or(records_to_append as u64),
    };
    service
        .sessions
        .insert(req.session_id.clone(), updated_state);

    json_response(
        200,
        &BatchSyncResponse {
            status: "ok".to_string(),
            session_id: req.session_id,
            accepted_cursor: to_cursor,
            duplicate: false,
            records_appended: records_to_append,
        },
    )
}
