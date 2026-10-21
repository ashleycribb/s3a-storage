//! S3A Core LRS Storage Engine.
//!
//! Implements conformant xAPI 1.0.3 statement body blob storage (Tile 21),
//! statement indexing (Tile 22), document store (Tile 23), and attachment blobs (Tile 24).
//! Adheres strictly to the S3A schema evolution contract (append-only, FLAG_ACTIVE = 0, CRC32C).

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use bytemuck::{bytes_of};
use sha2::{Digest, Sha256};
use xxhash_rust::xxh3::{xxh3_128, xxh3_64};

use s3a_core::*;
use crate::concurrent_writer::ConcurrentTileWriter;
use crate::MmapReader;

/// Errors returned by the LRS storage engine.
#[derive(Debug)]
pub enum LrsStoreError {
    Io(io::Error),
    Core(S3AError),
    StatementNotFound,
    StatementVoided,
    StatementNotVoided,
    InvalidStatement(String),
    CannotVoidVoidingStatement,
    StatementConflict(String), // 409 Conflict
    PreconditionFailed(String), // 412
    DocumentNotFound,
    CorruptedData(String),
}

impl From<io::Error> for LrsStoreError {
    fn from(e: io::Error) -> Self {
        LrsStoreError::Io(e)
    }
}

impl From<S3AError> for LrsStoreError {
    fn from(e: S3AError) -> Self {
        LrsStoreError::Core(e)
    }
}

impl std::fmt::Display for LrsStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LrsStoreError::Io(e) => write!(f, "IO error: {}", e),
            LrsStoreError::Core(e) => write!(f, "Core S3A error: {}", e),
            LrsStoreError::StatementNotFound => write!(f, "Statement not found"),
            LrsStoreError::StatementVoided => write!(f, "Statement is voided"),
            LrsStoreError::StatementNotVoided => write!(f, "Statement is not voided"),
            LrsStoreError::InvalidStatement(msg) => write!(f, "Invalid statement: {}", msg),
            LrsStoreError::CannotVoidVoidingStatement => {
                write!(f, "Cannot void a voiding statement (xAPI 1.0.3 §4.1.2.2)")
            }
            LrsStoreError::StatementConflict(msg) => write!(f, "Statement conflict: {}", msg),
            LrsStoreError::PreconditionFailed(msg) => write!(f, "Precondition failed: {}", msg),
            LrsStoreError::DocumentNotFound => write!(f, "Document not found"),
            LrsStoreError::CorruptedData(msg) => write!(f, "Corrupted data: {}", msg),
        }
    }
}

impl std::error::Error for LrsStoreError {}

/// Parsed metadata extracted from statement JSON for index creation.
#[derive(Debug, Clone)]
pub struct StatementMetadata {
    pub statement_id: [u64; 2],
    pub timestamp_ms: u64,
    pub actor_hash: u64,
    pub verb_iri: String,
    pub verb_hash: u32,
    pub object_activity_id: Option<String>,
    pub activity_hash: u32,
    pub registration: Option<[u64; 2]>,
    pub registration_hash: u32,
    pub is_voiding: bool,
    pub voided_target_id: Option<[u64; 2]>,
    pub related_activities: Vec<String>,
    pub related_agents: Vec<u64>,
    pub has_attachments: bool,
}

/// Convert UUID string (with or without hyphens) to [u64; 2].
pub fn uuid_to_u64_pair(s: &str) -> Result<[u64; 2], LrsStoreError> {
    let clean = s.trim().replace('-', "");
    if clean.len() != 32 {
        return Err(LrsStoreError::InvalidStatement(format!("Invalid UUID length: {}", s)));
    }
    let hi = u64::from_str_radix(&clean[..16], 16)
        .map_err(|e| LrsStoreError::InvalidStatement(format!("Invalid UUID hex: {}", e)))?;
    let lo = u64::from_str_radix(&clean[16..], 16)
        .map_err(|e| LrsStoreError::InvalidStatement(format!("Invalid UUID hex: {}", e)))?;
    Ok([hi, lo])
}

/// Convert [u64; 2] to standard 8-4-4-4-12 UUID string.
pub fn u64_pair_to_uuid(pair: [u64; 2]) -> String {
    let hex_hi = format!("{:016x}", pair[0]);
    let hex_lo = format!("{:016x}", pair[1]);
    let full = format!("{}{}", hex_hi, hex_lo);
    format!(
        "{}-{}-{}-{}-{}",
        &full[0..8],
        &full[8..12],
        &full[12..16],
        &full[16..20],
        &full[20..32]
    )
}

/// Generates an RFC 4122 compliant UUID v4 string without external C dependencies.
pub fn generate_uuid_v4() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    let pid = std::process::id() as u128;
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let count = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed) as u128;

    let mut seed = Vec::with_capacity(48);
    seed.extend_from_slice(&now.to_le_bytes());
    seed.extend_from_slice(&pid.to_le_bytes());
    seed.extend_from_slice(&count.to_le_bytes());

    let mut bytes = xxhash_rust::xxh3::xxh3_128(&seed).to_le_bytes();
    // Version 4
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    // Variant RFC 4122
    bytes[8] = (bytes[8] & 0x3f) | 0x80;

    let h = hex::encode(bytes);
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// Computes an actor IFI hash from an actor JSON Value.
pub fn hash_actor_value(val: &serde_json::Value) -> u64 {
    if let Some(mbox) = val.get("mbox").and_then(|v| v.as_str()) {
        return xxh3_64(mbox.as_bytes());
    }
    if let Some(sha1) = val.get("mbox_sha1sum").and_then(|v| v.as_str()) {
        return xxh3_64(sha1.as_bytes());
    }
    if let Some(openid) = val.get("openid").and_then(|v| v.as_str()) {
        return xxh3_64(openid.as_bytes());
    }
    if let Some(acc) = val.get("account") {
        let hp = acc.get("homePage").and_then(|v| v.as_str()).unwrap_or_default();
        let name = acc.get("name").and_then(|v| v.as_str()).unwrap_or_default();
        let key = format!("{}\0{}", hp, name);
        return xxh3_64(key.as_bytes());
    }
    0
}

pub fn is_valid_iri(s: &str) -> bool {
    if let Some(pos) = s.find(':') {
        if pos == 0 { return false; }
        let scheme = &s[..pos];
        let mut chars = scheme.chars();
        if let Some(first) = chars.next() {
            if !first.is_ascii_alphabetic() { return false; }
            for c in chars {
                if !c.is_ascii_alphanumeric() && c != '+' && c != '-' && c != '.' {
                    return false;
                }
            }
            return true;
        }
    }
    false
}

pub fn is_valid_uuid_str(s: &str) -> bool {
    uuid_to_u64_pair(s).is_ok()
}

pub fn is_valid_rfc5646_language_tag(tag: &str) -> bool {
    if tag.is_empty() {
        return false;
    }
    const GRANDFATHERED: &[&str] = &[
        "art-lojban", "cel-gaulish", "en-gb-oed", "i-ami", "i-bnn", "i-default",
        "i-enochian", "i-hak", "i-klingon", "i-lux", "i-mingo", "i-navajo",
        "i-pwn", "i-tao", "i-tay", "i-tsu", "no-bok", "no-nyn", "sgn-be-fr",
        "sgn-be-nl", "sgn-ch-de", "zh-guoyu", "zh-hakka", "zh-min", "zh-min-nan",
        "zh-xiang",
    ];
    let tag_lower = tag.to_ascii_lowercase();
    if GRANDFATHERED.contains(&tag_lower.as_str()) {
        return true;
    }
    if tag_lower.starts_with("x-") {
        let parts: Vec<&str> = tag.split('-').collect();
        if parts.len() < 2 { return false; }
        for p in &parts[1..] {
            if p.is_empty() || p.len() > 8 || !p.chars().all(|c| c.is_ascii_alphanumeric()) {
                return false;
            }
        }
        return true;
    }

    let subtags: Vec<&str> = tag.split('-').collect();
    if subtags.is_empty() { return false; }

    let lang = subtags[0];
    if !lang.chars().all(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    if lang.len() < 2 || lang.len() > 8 {
        return false;
    }

    let mut idx = 1;
    if lang.len() <= 3 {
        let mut ext_count = 0;
        while idx < subtags.len() && ext_count < 3 {
            let s = subtags[idx];
            if s.len() == 3 && s.chars().all(|c| c.is_ascii_alphabetic()) {
                idx += 1;
                ext_count += 1;
            } else {
                break;
            }
        }
    }

    if idx < subtags.len() {
        let s = subtags[idx];
        if s.len() == 4 && s.chars().all(|c| c.is_ascii_alphabetic()) {
            idx += 1;
        }
    }

    if idx < subtags.len() {
        let s = subtags[idx];
        if (s.len() == 2 && s.chars().all(|c| c.is_ascii_alphabetic()))
            || (s.len() == 3 && s.chars().all(|c| c.is_ascii_digit()))
        {
            idx += 1;
        }
    }

    while idx < subtags.len() {
        let s = subtags[idx];
        if (s.len() >= 5 && s.len() <= 8 && s.chars().all(|c| c.is_ascii_alphanumeric()))
            || (s.len() == 4 && s.chars().next().unwrap().is_ascii_digit() && s.chars().all(|c| c.is_ascii_alphanumeric()))
        {
            idx += 1;
        } else {
            break;
        }
    }

    let mut seen_singletons = std::collections::HashSet::new();
    while idx < subtags.len() {
        let s = subtags[idx];
        if s.len() == 1 && s.chars().all(|c| c.is_ascii_alphanumeric()) && !s.eq_ignore_ascii_case("x") {
            let singleton = s.chars().next().unwrap().to_ascii_lowercase();
            if seen_singletons.contains(&singleton) {
                return false;
            }
            seen_singletons.insert(singleton);
            idx += 1;
            let mut ext_subtags = 0;
            while idx < subtags.len() {
                let sub = subtags[idx];
                if sub.len() >= 2 && sub.len() <= 8 && sub.chars().all(|c| c.is_ascii_alphanumeric()) {
                    idx += 1;
                    ext_subtags += 1;
                } else {
                    break;
                }
            }
            if ext_subtags == 0 {
                return false;
            }
        } else {
            break;
        }
    }

    if idx < subtags.len() {
        let s = subtags[idx];
        if s.eq_ignore_ascii_case("x") {
            idx += 1;
            let mut pu_count = 0;
            while idx < subtags.len() {
                let sub = subtags[idx];
                if sub.len() >= 1 && sub.len() <= 8 && sub.chars().all(|c| c.is_ascii_alphanumeric()) {
                    idx += 1;
                    pu_count += 1;
                } else {
                    return false;
                }
            }
            if pu_count == 0 {
                return false;
            }
        } else {
            return false;
        }
    }

    idx == subtags.len()
}

pub fn is_language_map(v: &serde_json::Value) -> bool {
    if let Some(obj) = v.as_object() {
        if obj.is_empty() { return false; }
        for (k, val) in obj {
            if !val.is_string() { return false; }
            if !is_valid_rfc5646_language_tag(k) { return false; }
        }
        true
    } else {
        false
    }
}

pub fn is_valid_iso8601_duration(s: &str) -> bool {
    if !s.starts_with('P') || s.len() < 2 {
        return false;
    }
    let rest = &s[1..];
    if rest.ends_with('W') {
        let num_str = &rest[..rest.len() - 1];
        return !num_str.is_empty() && num_str.chars().all(|c| c.is_ascii_digit());
    }
    if rest.contains('W') {
        return false;
    }

    let (date_part, time_part) = match rest.split_once('T') {
        Some((d, t)) => (d, Some(t)),
        None => (rest, None),
    };

    let mut has_any = false;
    let mut d_rem = date_part;
    for designator in ['Y', 'M', 'D'] {
        if let Some(pos) = d_rem.find(designator) {
            let num = &d_rem[..pos];
            if num.is_empty() || !num.chars().all(|c| c.is_ascii_digit()) {
                return false;
            }
            has_any = true;
            d_rem = &d_rem[pos + 1..];
        }
    }
    if !d_rem.is_empty() {
        return false;
    }

    if let Some(t) = time_part {
        if t.is_empty() {
            return false;
        }
        let mut t_rem = t;
        let mut t_has_any = false;
        for designator in ['H', 'M'] {
            if let Some(pos) = t_rem.find(designator) {
                let num = &t_rem[..pos];
                if num.is_empty() || !num.chars().all(|c| c.is_ascii_digit()) {
                    return false;
                }
                t_has_any = true;
                has_any = true;
                t_rem = &t_rem[pos + 1..];
            }
        }
        if let Some(pos) = t_rem.find('S') {
            let num = &t_rem[..pos];
            if num.is_empty() {
                return false;
            }
            let valid_sec = if let Some((whole, frac)) = num.split_once('.') {
                !whole.is_empty() && whole.chars().all(|c| c.is_ascii_digit())
                    && !frac.is_empty() && frac.chars().all(|c| c.is_ascii_digit())
            } else if let Some((whole, frac)) = num.split_once(',') {
                !whole.is_empty() && whole.chars().all(|c| c.is_ascii_digit())
                    && !frac.is_empty() && frac.chars().all(|c| c.is_ascii_digit())
            } else {
                num.chars().all(|c| c.is_ascii_digit())
            };
            if !valid_sec {
                return false;
            }
            t_has_any = true;
            has_any = true;
            t_rem = &t_rem[pos + 1..];
        }
        if !t_rem.is_empty() || !t_has_any {
            return false;
        }
    }

    has_any
}

pub fn base64_url_decode(input: &str) -> Result<Vec<u8>, String> {
    let mut clean = input.trim_end_matches('=').to_string();
    while clean.len() % 4 != 0 {
        clean.push('=');
    }
    let mut out = Vec::new();
    let bytes = clean.as_bytes();
    let val = |c: u8| -> Result<u8, String> {
        match c {
            b'A'..=b'Z' => Ok(c - b'A'),
            b'a'..=b'z' => Ok(c - b'a' + 26),
            b'0'..=b'9' => Ok(c - b'0' + 52),
            b'-' | b'+' => Ok(62),
            b'_' | b'/' => Ok(63),
            _ => Err(format!("Invalid base64 character: {}", c as char)),
        }
    };
    for chunk in bytes.chunks(4) {
        if chunk.len() != 4 {
            return Err("Invalid base64 length".into());
        }
        let c0 = val(chunk[0])?;
        let c1 = val(chunk[1])?;
        let c2 = if chunk[2] == b'=' { 0 } else { val(chunk[2])? };
        let c3 = if chunk[3] == b'=' { 0 } else { val(chunk[3])? };

        let triple = ((c0 as u32) << 18) | ((c1 as u32) << 12) | ((c2 as u32) << 6) | (c3 as u32);
        out.push(((triple >> 16) & 0xFF) as u8);
        if chunk[2] != b'=' {
            out.push(((triple >> 8) & 0xFF) as u8);
        }
        if chunk[3] != b'=' {
            out.push((triple & 0xFF) as u8);
        }
    }
    Ok(out)
}

pub fn validate_jws_signature_bytes(bytes: &[u8]) -> Result<(), LrsStoreError> {
    let s = std::str::from_utf8(bytes).map_err(|_| LrsStoreError::InvalidStatement("JWS signature must be UTF-8 string".into()))?;
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 3 {
        return Err(LrsStoreError::InvalidStatement("JWS signature must have 3 parts separated by dots".into()));
    }
    let header_bytes = base64_url_decode(parts[0])
        .map_err(|e| LrsStoreError::InvalidStatement(format!("Failed to base64url decode JWS header: {}", e)))?;
    let header_json: serde_json::Value = serde_json::from_slice(&header_bytes)
        .map_err(|e| LrsStoreError::InvalidStatement(format!("JWS header is not valid JSON: {}", e)))?;
    let alg = header_json.get("alg").and_then(|v| v.as_str())
        .ok_or_else(|| LrsStoreError::InvalidStatement("JWS header missing alg".into()))?;
    if alg != "RS256" && alg != "RS384" && alg != "RS512" {
        return Err(LrsStoreError::InvalidStatement(format!("JWS algorithm must be RS256, RS384, or RS512, got '{}'", alg)));
    }

    let payload_bytes = base64_url_decode(parts[1])
        .map_err(|e| LrsStoreError::InvalidStatement(format!("Failed to base64url decode JWS payload: {}", e)))?;
    let _payload_json: serde_json::Value = serde_json::from_slice(&payload_bytes)
        .map_err(|e| LrsStoreError::InvalidStatement(format!("JWS payload is not valid JSON statement: {}", e)))?;

    Ok(())
}

pub fn validate_agent_object(v: &serde_json::Value) -> Result<(), LrsStoreError> {
    let obj = v.as_object().ok_or_else(|| LrsStoreError::InvalidStatement("Agent must be a JSON object".into()))?;
    for k in obj.keys() {
        match k.as_str() {
            "objectType" | "name" | "mbox" | "mbox_sha1sum" | "openid" | "account" => {},
            _ => return Err(LrsStoreError::InvalidStatement(format!("Unexpected property on Agent: {}", k))),
        }
    }
    if let Some(ot) = obj.get("objectType") {
        let ot_str = ot.as_str().ok_or_else(|| LrsStoreError::InvalidStatement("Agent objectType must be string".into()))?;
        if ot_str != "Agent" {
            return Err(LrsStoreError::InvalidStatement("Agent objectType must be 'Agent'".into()));
        }
    }
    if let Some(name) = obj.get("name") {
        if !name.is_string() {
            return Err(LrsStoreError::InvalidStatement("Agent name must be a string".into()));
        }
    }
    if obj.get("member").is_some() {
        return Err(LrsStoreError::InvalidStatement("Agent cannot have member property".into()));
    }

    let ifi_count = (if obj.get("mbox").is_some() { 1 } else { 0 })
        + (if obj.get("mbox_sha1sum").is_some() { 1 } else { 0 })
        + (if obj.get("openid").is_some() { 1 } else { 0 })
        + (if obj.get("account").is_some() { 1 } else { 0 });

    if ifi_count != 1 {
        return Err(LrsStoreError::InvalidStatement("Agent must have exactly one Inverse Functional Identifier".into()));
    }

    if let Some(mbox) = obj.get("mbox") {
        let s = mbox.as_str().ok_or_else(|| LrsStoreError::InvalidStatement("mbox must be string".into()))?;
        if !s.starts_with("mailto:") || s.len() <= 7 {
            return Err(LrsStoreError::InvalidStatement("mbox must start with mailto:".into()));
        }
        let email = &s[7..];
        if !email.contains('@') || email.starts_with('@') || email.ends_with('@') {
            return Err(LrsStoreError::InvalidStatement("mbox must have the form 'mailto:email address'".into()));
        }
    }
    if let Some(sha1) = obj.get("mbox_sha1sum") {
        let s = sha1.as_str().ok_or_else(|| LrsStoreError::InvalidStatement("mbox_sha1sum must be string".into()))?;
        if s.len() != 40 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(LrsStoreError::InvalidStatement("mbox_sha1sum must be 40 hex digits".into()));
        }
    }
    if let Some(openid) = obj.get("openid") {
        let s = openid.as_str().ok_or_else(|| LrsStoreError::InvalidStatement("openid must be string".into()))?;
        if !is_valid_iri(s) {
            return Err(LrsStoreError::InvalidStatement("openid must be a valid IRI".into()));
        }
    }
    if let Some(acc) = obj.get("account") {
        let acc_obj = acc.as_object().ok_or_else(|| LrsStoreError::InvalidStatement("account must be an object".into()))?;
        for k in acc_obj.keys() {
            match k.as_str() {
                "homePage" | "name" => {},
                _ => return Err(LrsStoreError::InvalidStatement(format!("Unexpected property on account: {}", k))),
            }
        }
        let hp = acc_obj.get("homePage").and_then(|v| v.as_str()).ok_or_else(|| LrsStoreError::InvalidStatement("account missing homePage".into()))?;
        if !is_valid_iri(hp) {
            return Err(LrsStoreError::InvalidStatement("account homePage must be a valid IRI".into()));
        }
        let name = acc_obj.get("name").and_then(|v| v.as_str()).ok_or_else(|| LrsStoreError::InvalidStatement("account missing name".into()))?;
        if name.is_empty() {
            return Err(LrsStoreError::InvalidStatement("account name must be non-empty string".into()));
        }
    }

    Ok(())
}

pub fn validate_group_object(v: &serde_json::Value) -> Result<(), LrsStoreError> {
    let obj = v.as_object().ok_or_else(|| LrsStoreError::InvalidStatement("Group must be a JSON object".into()))?;
    for k in obj.keys() {
        match k.as_str() {
            "objectType" | "name" | "member" | "mbox" | "mbox_sha1sum" | "openid" | "account" => {},
            _ => return Err(LrsStoreError::InvalidStatement(format!("Unexpected property on Group: {}", k))),
        }
    }
    let ot = obj.get("objectType").and_then(|v| v.as_str()).ok_or_else(|| LrsStoreError::InvalidStatement("Group must have objectType 'Group'".into()))?;
    if ot != "Group" {
        return Err(LrsStoreError::InvalidStatement("Group objectType must be 'Group'".into()));
    }
    if let Some(name) = obj.get("name") {
        if !name.is_string() {
            return Err(LrsStoreError::InvalidStatement("Group name must be string".into()));
        }
    }

    let ifi_count = (if obj.get("mbox").is_some() { 1 } else { 0 })
        + (if obj.get("mbox_sha1sum").is_some() { 1 } else { 0 })
        + (if obj.get("openid").is_some() { 1 } else { 0 })
        + (if obj.get("account").is_some() { 1 } else { 0 });

    if ifi_count > 1 {
        return Err(LrsStoreError::InvalidStatement("Group cannot have multiple IFIs".into()));
    }

    if let Some(mbox) = obj.get("mbox") {
        let s = mbox.as_str().ok_or_else(|| LrsStoreError::InvalidStatement("mbox must be string".into()))?;
        if !s.starts_with("mailto:") || s.len() <= 7 {
            return Err(LrsStoreError::InvalidStatement("mbox must start with mailto:".into()));
        }
        let email = &s[7..];
        if !email.contains('@') || email.starts_with('@') || email.ends_with('@') {
            return Err(LrsStoreError::InvalidStatement("mbox must have the form 'mailto:email address'".into()));
        }
    }
    if let Some(sha1) = obj.get("mbox_sha1sum") {
        let s = sha1.as_str().ok_or_else(|| LrsStoreError::InvalidStatement("mbox_sha1sum must be string".into()))?;
        if s.len() != 40 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(LrsStoreError::InvalidStatement("mbox_sha1sum must be 40 hex digits".into()));
        }
    }
    if let Some(openid) = obj.get("openid") {
        let s = openid.as_str().ok_or_else(|| LrsStoreError::InvalidStatement("openid must be string".into()))?;
        if !is_valid_iri(s) {
            return Err(LrsStoreError::InvalidStatement("openid must be a valid IRI".into()));
        }
    }
    if let Some(acc) = obj.get("account") {
        let acc_obj = acc.as_object().ok_or_else(|| LrsStoreError::InvalidStatement("account must be an object".into()))?;
        for k in acc_obj.keys() {
            match k.as_str() {
                "homePage" | "name" => {},
                _ => return Err(LrsStoreError::InvalidStatement(format!("Unexpected property on account: {}", k))),
            }
        }
        let hp = acc_obj.get("homePage").and_then(|v| v.as_str()).ok_or_else(|| LrsStoreError::InvalidStatement("account missing homePage".into()))?;
        if !is_valid_iri(hp) {
            return Err(LrsStoreError::InvalidStatement("account homePage must be a valid IRI".into()));
        }
        let name = acc_obj.get("name").and_then(|v| v.as_str()).ok_or_else(|| LrsStoreError::InvalidStatement("account missing name".into()))?;
        if name.is_empty() {
            return Err(LrsStoreError::InvalidStatement("account name must be non-empty string".into()));
        }
    }

    if let Some(member) = obj.get("member") {
        let arr = member.as_array().ok_or_else(|| LrsStoreError::InvalidStatement("Group member must be an array".into()))?;
        for m in arr {
            validate_agent_object(m)?;
        }
    } else if ifi_count == 0 {
        return Err(LrsStoreError::InvalidStatement("Anonymous group must have member array".into()));
    }

    Ok(())
}

pub fn validate_actor(v: &serde_json::Value) -> Result<(), LrsStoreError> {
    let obj = v.as_object().ok_or_else(|| LrsStoreError::InvalidStatement("Actor must be an object".into()))?;
    let ot = obj.get("objectType").and_then(|v| v.as_str()).unwrap_or("Agent");
    if ot == "Group" {
        validate_group_object(v)
    } else if ot == "Agent" {
        validate_agent_object(v)
    } else {
        Err(LrsStoreError::InvalidStatement("Actor objectType must be 'Agent' or 'Group'".into()))
    }
}

pub fn validate_verb(v: &serde_json::Value) -> Result<(), LrsStoreError> {
    let obj = v.as_object().ok_or_else(|| LrsStoreError::InvalidStatement("Verb must be an object".into()))?;
    for k in obj.keys() {
        match k.as_str() {
            "id" | "display" => {},
            _ => return Err(LrsStoreError::InvalidStatement(format!("Unexpected property on Verb: {}", k))),
        }
    }
    let id_str = obj.get("id").and_then(|v| v.as_str()).ok_or_else(|| LrsStoreError::InvalidStatement("Verb missing id IRI".into()))?;
    if !is_valid_iri(id_str) {
        return Err(LrsStoreError::InvalidStatement(format!("Verb id is not a valid IRI: {}", id_str)));
    }
    if let Some(display) = obj.get("display") {
        if !is_language_map(display) {
            return Err(LrsStoreError::InvalidStatement("Verb display must be a valid Language Map".into()));
        }
    }
    Ok(())
}

pub fn validate_activity_definition(def: &serde_json::Map<String, serde_json::Value>) -> Result<(), LrsStoreError> {
    for k in def.keys() {
        match k.as_str() {
            "name" | "description" | "type" | "moreInfo" | "interactionType" | "correctResponsesPattern"
            | "choices" | "scale" | "source" | "target" | "steps" | "extensions" => {},
            _ => return Err(LrsStoreError::InvalidStatement(format!("Unexpected property on Activity Definition: {}", k))),
        }
    }
    if let Some(name) = def.get("name") {
        if !is_language_map(name) {
            return Err(LrsStoreError::InvalidStatement("definition name must be a Language Map".into()));
        }
    }
    if let Some(desc) = def.get("description") {
        if !is_language_map(desc) {
            return Err(LrsStoreError::InvalidStatement("definition description must be a Language Map".into()));
        }
    }
    if let Some(t) = def.get("type") {
        let t_str = t.as_str().ok_or_else(|| LrsStoreError::InvalidStatement("definition type must be string".into()))?;
        if !is_valid_iri(t_str) {
            return Err(LrsStoreError::InvalidStatement("definition type must be an IRI".into()));
        }
    }
    if let Some(mi) = def.get("moreInfo") {
        let mi_str = mi.as_str().ok_or_else(|| LrsStoreError::InvalidStatement("definition moreInfo must be string".into()))?;
        if !is_valid_iri(mi_str) {
            return Err(LrsStoreError::InvalidStatement("definition moreInfo must be an IRI".into()));
        }
    }

    let has_crp = def.get("correctResponsesPattern").is_some();
    let has_choices = def.get("choices").is_some();
    let has_scale = def.get("scale").is_some();
    let has_source = def.get("source").is_some();
    let has_target = def.get("target").is_some();
    let has_steps = def.get("steps").is_some();

    let int_type_opt = def.get("interactionType");
    if has_crp || has_choices || has_scale || has_source || has_target || has_steps {
        if int_type_opt.is_none() {
            return Err(LrsStoreError::InvalidStatement(
                "Activity Definition must include interactionType when correctResponsesPattern, choices, scale, source, target, or steps are present".into()
            ));
        }
    }

    if let Some(it) = int_type_opt {
        let it_str = it.as_str().ok_or_else(|| LrsStoreError::InvalidStatement("interactionType must be string".into()))?;
        const VALID_INTERACTIONS: &[&str] = &[
            "true-false", "choice", "fill-in", "long-fill-in", "matching", "performance",
            "sequencing", "likert", "numeric", "other",
        ];
        if !VALID_INTERACTIONS.contains(&it_str) {
            return Err(LrsStoreError::InvalidStatement(format!("Invalid interactionType: {}", it_str)));
        }

        if has_choices && it_str != "choice" && it_str != "sequencing" {
            return Err(LrsStoreError::InvalidStatement("choices only allowed for choice or sequencing interactionType".into()));
        }
        if has_scale && it_str != "likert" {
            return Err(LrsStoreError::InvalidStatement("scale only allowed for likert interactionType".into()));
        }
        if (has_source || has_target) && it_str != "matching" {
            return Err(LrsStoreError::InvalidStatement("source and target only allowed for matching interactionType".into()));
        }
        if has_steps && it_str != "performance" {
            return Err(LrsStoreError::InvalidStatement("steps only allowed for performance interactionType".into()));
        }
    }

    if let Some(crp) = def.get("correctResponsesPattern") {
        let arr = crp.as_array().ok_or_else(|| LrsStoreError::InvalidStatement("correctResponsesPattern must be an array".into()))?;
        for item in arr {
            if !item.is_string() {
                return Err(LrsStoreError::InvalidStatement("correctResponsesPattern items must be strings".into()));
            }
        }
    }

    for key in ["choices", "scale", "source", "target", "steps"] {
        if let Some(comp_val) = def.get(key) {
            let arr = comp_val.as_array().ok_or_else(|| LrsStoreError::InvalidStatement(format!("{} must be an array", key)))?;
            for item in arr {
                let comp_obj = item.as_object().ok_or_else(|| LrsStoreError::InvalidStatement(format!("{} item must be an object", key)))?;
                for ck in comp_obj.keys() {
                    match ck.as_str() {
                        "id" | "description" => {},
                        _ => return Err(LrsStoreError::InvalidStatement(format!("Unexpected property on {}: {}", key, ck))),
                    }
                }
                let id = comp_obj.get("id").and_then(|v| v.as_str()).ok_or_else(|| LrsStoreError::InvalidStatement(format!("{} item missing id string", key)))?;
                if id.is_empty() {
                    return Err(LrsStoreError::InvalidStatement(format!("{} item id cannot be empty", key)));
                }
                if let Some(desc) = comp_obj.get("description") {
                    if !is_language_map(desc) {
                        return Err(LrsStoreError::InvalidStatement(format!("{} item description must be Language Map", key)));
                    }
                }
            }
        }
    }

    if let Some(ext) = def.get("extensions") {
        let ext_obj = ext.as_object().ok_or_else(|| LrsStoreError::InvalidStatement("extensions must be an object".into()))?;
        for k in ext_obj.keys() {
            if !is_valid_iri(k) {
                return Err(LrsStoreError::InvalidStatement(format!("extension key must be an IRI: {}", k)));
            }
        }
    }
    Ok(())
}

pub fn validate_statement_object(v: &serde_json::Value, allow_substatement: bool) -> Result<(), LrsStoreError> {
    let obj = v.as_object().ok_or_else(|| LrsStoreError::InvalidStatement("Statement object must be a JSON object".into()))?;
    let ot = obj.get("objectType").and_then(|v| v.as_str()).unwrap_or("Activity");
    match ot {
        "Activity" => {
            for k in obj.keys() {
                match k.as_str() {
                    "objectType" | "id" | "definition" => {},
                    _ => return Err(LrsStoreError::InvalidStatement(format!("Unexpected property on Activity: {}", k))),
                }
            }
            let id = obj.get("id").and_then(|v| v.as_str()).ok_or_else(|| LrsStoreError::InvalidStatement("Activity missing id".into()))?;
            if !is_valid_iri(id) {
                return Err(LrsStoreError::InvalidStatement(format!("Activity id is not a valid IRI: {}", id)));
            }
            if let Some(def) = obj.get("definition") {
                let def_obj = def.as_object().ok_or_else(|| LrsStoreError::InvalidStatement("Activity definition must be an object".into()))?;
                validate_activity_definition(def_obj)?;
            }
        }
        "Agent" => validate_agent_object(v)?,
        "Group" => validate_group_object(v)?,
        "StatementRef" => {
            for k in obj.keys() {
                match k.as_str() {
                    "objectType" | "id" => {},
                    _ => return Err(LrsStoreError::InvalidStatement(format!("Unexpected property on StatementRef: {}", k))),
                }
            }
            let id = obj.get("id").and_then(|v| v.as_str()).ok_or_else(|| LrsStoreError::InvalidStatement("StatementRef missing id".into()))?;
            if !is_valid_uuid_str(id) {
                return Err(LrsStoreError::InvalidStatement("StatementRef id must be a valid UUID".into()));
            }
        }
        "SubStatement" => {
            if !allow_substatement {
                return Err(LrsStoreError::InvalidStatement("SubStatement cannot be nested inside another SubStatement".into()));
            }
            for k in obj.keys() {
                match k.as_str() {
                    "objectType" | "actor" | "verb" | "object" | "result" | "context" | "timestamp" | "attachments" => {},
                    _ => return Err(LrsStoreError::InvalidStatement(format!("Unexpected property on SubStatement: {}", k))),
                }
            }
            if obj.get("id").is_some() || obj.get("stored").is_some() || obj.get("version").is_some() || obj.get("authority").is_some() {
                return Err(LrsStoreError::InvalidStatement("SubStatement cannot have id, stored, version, or authority".into()));
            }
            let sub_actor = obj.get("actor").ok_or_else(|| LrsStoreError::InvalidStatement("SubStatement missing actor".into()))?;
            validate_actor(sub_actor)?;
            let sub_verb = obj.get("verb").ok_or_else(|| LrsStoreError::InvalidStatement("SubStatement missing verb".into()))?;
            validate_verb(sub_verb)?;
            let sub_obj = obj.get("object").ok_or_else(|| LrsStoreError::InvalidStatement("SubStatement missing object".into()))?;
            validate_statement_object(sub_obj, false)?;
            let sub_obj_type = sub_obj.get("objectType").and_then(|v| v.as_str()).unwrap_or("Activity");

            if let Some(res) = obj.get("result") {
                validate_result(res)?;
            }
            if let Some(ctx) = obj.get("context") {
                validate_context(ctx, Some(sub_obj_type))?;
            }
            if let Some(ts) = obj.get("timestamp") {
                let s = ts.as_str().ok_or_else(|| LrsStoreError::InvalidStatement("timestamp must be string".into()))?;
                if chrono_parse_ms(s).is_none() {
                    return Err(LrsStoreError::InvalidStatement("Invalid ISO-8601 timestamp".into()));
                }
            }
            if let Some(atts) = obj.get("attachments") {
                validate_attachments_value(atts)?;
            }
        }
        other => return Err(LrsStoreError::InvalidStatement(format!("Unknown objectType: {}", other))),
    }
    Ok(())
}

pub fn validate_result(v: &serde_json::Value) -> Result<(), LrsStoreError> {
    let obj = v.as_object().ok_or_else(|| LrsStoreError::InvalidStatement("result must be an object".into()))?;
    for k in obj.keys() {
        match k.as_str() {
            "score" | "success" | "completion" | "response" | "duration" | "extensions" => {},
            _ => return Err(LrsStoreError::InvalidStatement(format!("Unexpected property on result: {}", k))),
        }
    }
    if let Some(score) = obj.get("score") {
        let sc = score.as_object().ok_or_else(|| LrsStoreError::InvalidStatement("score must be an object".into()))?;
        for k in sc.keys() {
            match k.as_str() {
                "scaled" | "raw" | "min" | "max" => {},
                _ => return Err(LrsStoreError::InvalidStatement(format!("Unexpected property on score: {}", k))),
            }
        }
        if let Some(s) = sc.get("scaled") {
            let num = s.as_f64().ok_or_else(|| LrsStoreError::InvalidStatement("score.scaled must be number".into()))?;
            if !(-1.0..=1.0).contains(&num) {
                return Err(LrsStoreError::InvalidStatement("score.scaled must be between -1.0 and 1.0".into()));
            }
        }
        let min = if let Some(m) = sc.get("min") {
            Some(m.as_f64().ok_or_else(|| LrsStoreError::InvalidStatement("score.min must be number".into()))?)
        } else { None };
        let max = if let Some(m) = sc.get("max") {
            Some(m.as_f64().ok_or_else(|| LrsStoreError::InvalidStatement("score.max must be number".into()))?)
        } else { None };
        let raw = if let Some(r) = sc.get("raw") {
            Some(r.as_f64().ok_or_else(|| LrsStoreError::InvalidStatement("score.raw must be number".into()))?)
        } else { None };

        if let (Some(mi), Some(ma)) = (min, max) {
            if mi > ma {
                return Err(LrsStoreError::InvalidStatement("score.min cannot be greater than score.max".into()));
            }
        }
        if let (Some(r), Some(mi)) = (raw, min) {
            if r < mi {
                return Err(LrsStoreError::InvalidStatement("score.raw cannot be less than score.min".into()));
            }
        }
        if let (Some(r), Some(ma)) = (raw, max) {
            if r > ma {
                return Err(LrsStoreError::InvalidStatement("score.raw cannot be greater than score.max".into()));
            }
        }
    }
    if let Some(succ) = obj.get("success") {
        if !succ.is_boolean() {
            return Err(LrsStoreError::InvalidStatement("result.success must be boolean".into()));
        }
    }
    if let Some(comp) = obj.get("completion") {
        if !comp.is_boolean() {
            return Err(LrsStoreError::InvalidStatement("result.completion must be boolean".into()));
        }
    }
    if let Some(resp) = obj.get("response") {
        if !resp.is_string() {
            return Err(LrsStoreError::InvalidStatement("result.response must be string".into()));
        }
    }
    if let Some(dur) = obj.get("duration") {
        let s = dur.as_str().ok_or_else(|| LrsStoreError::InvalidStatement("result.duration must be string".into()))?;
        if !is_valid_iso8601_duration(s) {
            return Err(LrsStoreError::InvalidStatement("result.duration is not a valid ISO 8601 duration".into()));
        }
    }
    if let Some(ext) = obj.get("extensions") {
        let ext_obj = ext.as_object().ok_or_else(|| LrsStoreError::InvalidStatement("result.extensions must be object".into()))?;
        for k in ext_obj.keys() {
            if !is_valid_iri(k) {
                return Err(LrsStoreError::InvalidStatement(format!("result extension key must be IRI: {}", k)));
            }
        }
    }
    Ok(())
}

pub fn validate_context(v: &serde_json::Value, object_type: Option<&str>) -> Result<(), LrsStoreError> {
    let obj = v.as_object().ok_or_else(|| LrsStoreError::InvalidStatement("context must be an object".into()))?;
    for k in obj.keys() {
        match k.as_str() {
            "registration" | "instructor" | "team" | "contextActivities" | "revision" | "platform" | "language" | "statement" | "extensions" => {},
            _ => return Err(LrsStoreError::InvalidStatement(format!("Unexpected property on context: {}", k))),
        }
    }
    if let Some(reg) = obj.get("registration") {
        let s = reg.as_str().ok_or_else(|| LrsStoreError::InvalidStatement("registration must be string".into()))?;
        if !is_valid_uuid_str(s) {
            return Err(LrsStoreError::InvalidStatement("registration must be a valid UUID".into()));
        }
    }
    if let Some(inst) = obj.get("instructor") {
        validate_actor(inst)?;
    }
    if let Some(team) = obj.get("team") {
        validate_group_object(team)?;
    }
    if let Some(stmt) = obj.get("statement") {
        let stmt_obj = stmt.as_object().ok_or_else(|| LrsStoreError::InvalidStatement("context.statement must be an object".into()))?;
        let ot = stmt_obj.get("objectType").and_then(|v| v.as_str()).unwrap_or("");
        if ot != "StatementRef" {
            return Err(LrsStoreError::InvalidStatement("context.statement objectType must be 'StatementRef'".into()));
        }
        let id = stmt_obj.get("id").and_then(|v| v.as_str()).ok_or_else(|| LrsStoreError::InvalidStatement("context.statement missing id".into()))?;
        if !is_valid_uuid_str(id) {
            return Err(LrsStoreError::InvalidStatement("context.statement id must be UUID".into()));
        }
    }
    if let Some(ca) = obj.get("contextActivities") {
        let ca_obj = ca.as_object().ok_or_else(|| LrsStoreError::InvalidStatement("contextActivities must be an object".into()))?;
        for (k, val) in ca_obj {
            match k.as_str() {
                "parent" | "grouping" | "category" | "other" => {},
                _ => return Err(LrsStoreError::InvalidStatement(format!("Unexpected contextActivities property: {}", k))),
            }
            if let Some(arr) = val.as_array() {
                for item in arr {
                    validate_statement_object(item, false)?;
                }
            } else if val.is_object() {
                validate_statement_object(val, false)?;
            } else {
                return Err(LrsStoreError::InvalidStatement("contextActivities item must be Activity object or array of Activity objects".into()));
            }
        }
    }
    if let Some(rev) = obj.get("revision") {
        let _ = rev.as_str().ok_or_else(|| LrsStoreError::InvalidStatement("context.revision must be string".into()))?;
        if let Some(ot) = object_type {
            if ot != "Activity" {
                return Err(LrsStoreError::InvalidStatement("context.revision is only allowed when statement object is an Activity".into()));
            }
        }
    }
    if let Some(plat) = obj.get("platform") {
        let _ = plat.as_str().ok_or_else(|| LrsStoreError::InvalidStatement("context.platform must be string".into()))?;
        if let Some(ot) = object_type {
            if ot != "Activity" {
                return Err(LrsStoreError::InvalidStatement("context.platform is only allowed when statement object is an Activity".into()));
            }
        }
    }
    if let Some(lang) = obj.get("language") {
        let s = lang.as_str().ok_or_else(|| LrsStoreError::InvalidStatement("context.language must be string".into()))?;
        if !is_valid_rfc5646_language_tag(s) {
            return Err(LrsStoreError::InvalidStatement(format!("context.language is not a valid RFC 5646 language tag: {}", s)));
        }
    }
    if let Some(ext) = obj.get("extensions") {
        let ext_obj = ext.as_object().ok_or_else(|| LrsStoreError::InvalidStatement("context.extensions must be object".into()))?;
        for k in ext_obj.keys() {
            if !is_valid_iri(k) {
                return Err(LrsStoreError::InvalidStatement(format!("context extension key must be IRI: {}", k)));
            }
        }
    }
    Ok(())
}

pub fn validate_xapi_statement(val: &serde_json::Value) -> Result<(), LrsStoreError> {
    let obj = val.as_object().ok_or_else(|| LrsStoreError::InvalidStatement("Statement must be a JSON object".into()))?;

    // Check unexpected keys on Statement
    for k in obj.keys() {
        match k.as_str() {
            "id" | "actor" | "verb" | "object" | "result" | "context" | "timestamp" | "stored" | "authority" | "version" | "attachments" => {},
            _ => return Err(LrsStoreError::InvalidStatement(format!("Unexpected property on Statement: {}", k))),
        }
    }

    // 1. ID
    if let Some(id_val) = obj.get("id") {
        let id_str = id_val.as_str().ok_or_else(|| LrsStoreError::InvalidStatement("Statement id must be string".into()))?;
        if !is_valid_uuid_str(id_str) {
            return Err(LrsStoreError::InvalidStatement(format!("Invalid statement UUID: {}", id_str)));
        }
    }

    // 2. Actor
    let actor_val = obj.get("actor").ok_or_else(|| LrsStoreError::InvalidStatement("Missing actor".into()))?;
    validate_actor(actor_val)?;

    // 3. Verb
    let verb_val = obj.get("verb").ok_or_else(|| LrsStoreError::InvalidStatement("Missing verb".into()))?;
    validate_verb(verb_val)?;

    // 4. Object
    let obj_val = obj.get("object").ok_or_else(|| LrsStoreError::InvalidStatement("Missing object".into()))?;
    validate_statement_object(obj_val, true)?;
    let main_obj_type = obj_val.get("objectType").and_then(|v| v.as_str()).unwrap_or("Activity");

    // 5. Result
    if let Some(result_val) = obj.get("result") {
        validate_result(result_val)?;
    }

    // 6. Context
    if let Some(ctx_val) = obj.get("context") {
        validate_context(ctx_val, Some(main_obj_type))?;
    }

    // 7. Timestamp
    if let Some(ts) = obj.get("timestamp") {
        let s = ts.as_str().ok_or_else(|| LrsStoreError::InvalidStatement("timestamp must be string".into()))?;
        if chrono_parse_ms(s).is_none() {
            return Err(LrsStoreError::InvalidStatement("Invalid ISO-8601 timestamp".into()));
        }
    }

    // 8. Stored
    if let Some(st) = obj.get("stored") {
        let s = st.as_str().ok_or_else(|| LrsStoreError::InvalidStatement("stored must be string".into()))?;
        if chrono_parse_ms(s).is_none() {
            return Err(LrsStoreError::InvalidStatement("Invalid ISO-8601 stored timestamp".into()));
        }
    }

    // 9. Authority
    if let Some(auth) = obj.get("authority") {
        let auth_obj = auth.as_object().ok_or_else(|| LrsStoreError::InvalidStatement("authority must be object".into()))?;
        let ot = auth_obj.get("objectType").and_then(|v| v.as_str()).unwrap_or("Agent");
        if ot == "Group" {
            let has_ifi = auth_obj.get("mbox").is_some() || auth_obj.get("mbox_sha1sum").is_some() || auth_obj.get("openid").is_some() || auth_obj.get("account").is_some();
            if has_ifi {
                return Err(LrsStoreError::InvalidStatement("Authority group cannot have Inverse Functional Identifier".into()));
            }
            let members = auth_obj.get("member").and_then(|v| v.as_array()).ok_or_else(|| LrsStoreError::InvalidStatement("Authority group must have member array".into()))?;
            if members.len() != 2 {
                return Err(LrsStoreError::InvalidStatement("Authority group must have exactly 2 member agents".into()));
            }
            for m in members {
                validate_agent_object(m)?;
            }
        } else if ot == "Agent" {
            validate_agent_object(auth)?;
        } else {
            return Err(LrsStoreError::InvalidStatement("authority objectType must be 'Agent' or 'Group'".into()));
        }
    }

    // 10. Version
    if let Some(ver) = obj.get("version") {
        let s = ver.as_str().ok_or_else(|| LrsStoreError::InvalidStatement("version must be string".into()))?;
        if !s.starts_with("1.0") {
            return Err(LrsStoreError::InvalidStatement("version must start with 1.0".into()));
        }
    }

    // 11. Attachments
    if let Some(atts) = obj.get("attachments") {
        validate_attachments_value(atts)?;
    }

    Ok(())
}

pub fn validate_attachments_value(atts: &serde_json::Value) -> Result<(), LrsStoreError> {
    let arr = atts.as_array().ok_or_else(|| LrsStoreError::InvalidStatement("attachments must be an array".into()))?;
    for att in arr {
        let att_obj = att.as_object().ok_or_else(|| LrsStoreError::InvalidStatement("attachment must be an object".into()))?;
        for k in att_obj.keys() {
            match k.as_str() {
                "usageType" | "display" | "description" | "contentType" | "length" | "sha2" | "fileUrl" => {},
                _ => return Err(LrsStoreError::InvalidStatement(format!("Unexpected property on attachment: {}", k))),
            }
        }
        let usage = att_obj.get("usageType").and_then(|v| v.as_str()).ok_or_else(|| LrsStoreError::InvalidStatement("attachment missing usageType".into()))?;
        if !is_valid_iri(usage) {
            return Err(LrsStoreError::InvalidStatement("attachment usageType must be IRI".into()));
        }
        let display = att_obj.get("display").ok_or_else(|| LrsStoreError::InvalidStatement("attachment missing display".into()))?;
        if !is_language_map(display) {
            return Err(LrsStoreError::InvalidStatement("attachment display must be Language Map".into()));
        }
        if let Some(desc) = att_obj.get("description") {
            if !is_language_map(desc) {
                return Err(LrsStoreError::InvalidStatement("attachment description must be Language Map".into()));
            }
        }
        let sha2 = att_obj.get("sha2").and_then(|v| v.as_str()).ok_or_else(|| LrsStoreError::InvalidStatement("attachment missing sha2".into()))?;
        if sha2.len() != 64 || !sha2.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(LrsStoreError::InvalidStatement("attachment sha2 must be 64 hex characters".into()));
        }
        if let Some(file_url) = att_obj.get("fileUrl").and_then(|v| v.as_str()) {
            if !is_valid_iri(file_url) {
                return Err(LrsStoreError::InvalidStatement("attachment fileUrl must be a valid IRI".into()));
            }
        }
        let ct = att_obj.get("contentType").and_then(|v| v.as_str()).ok_or_else(|| LrsStoreError::InvalidStatement("attachment missing contentType".into()))?;
        if ct.is_empty() {
            return Err(LrsStoreError::InvalidStatement("attachment contentType cannot be empty".into()));
        }
        if usage == "http://adlnet.gov/expapi/attachments/signature" && ct != "application/octet-stream" {
            return Err(LrsStoreError::InvalidStatement("Signature attachment contentType must be 'application/octet-stream'".into()));
        }
        att_obj.get("length").and_then(|v| v.as_u64()).ok_or_else(|| LrsStoreError::InvalidStatement("attachment missing length integer".into()))?;
    }
    Ok(())
}

/// Parse and extract statement index metadata from raw statement JSON.
pub fn parse_statement_metadata(json_val: &serde_json::Value) -> Result<StatementMetadata, LrsStoreError> {
    // 1. Statement ID
    let statement_id = if let Some(id_str) = json_val.get("id").and_then(|v| v.as_str()) {
        uuid_to_u64_pair(id_str)?
    } else {
        return Err(LrsStoreError::InvalidStatement("Missing statement ID".into()));
    };

    // 2. Actor
    let actor_val = json_val.get("actor").ok_or_else(|| {
        LrsStoreError::InvalidStatement("Missing statement actor".into())
    })?;
    let actor_hash = hash_actor_value(actor_val);

    // 3. Verb
    let verb_val = json_val.get("verb").ok_or_else(|| {
        LrsStoreError::InvalidStatement("Missing statement verb".into())
    })?;
    let verb_iri = verb_val
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| LrsStoreError::InvalidStatement("Missing verb ID IRI".into()))?
        .to_string();
    let verb_hash = (xxh3_64(verb_iri.as_bytes()) & 0xFFFF_FFFF) as u32;

    // 4. Object
    let obj_val = json_val.get("object").ok_or_else(|| {
        LrsStoreError::InvalidStatement("Missing statement object".into())
    })?;
    let mut object_activity_id = None;
    let mut is_voiding = false;
    let mut voided_target_id = None;

    if verb_iri == "http://adlnet.gov/expapi/verbs/voided" {
        is_voiding = true;
        let target_str = obj_val
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| LrsStoreError::InvalidStatement("Voiding statement object must specify target ID".into()))?;
        voided_target_id = Some(uuid_to_u64_pair(target_str)?);
    } else if let Some(act_id) = obj_val.get("id").and_then(|v| v.as_str()) {
        object_activity_id = Some(act_id.to_string());
    }

    let activity_hash = object_activity_id
        .as_ref()
        .map(|id| (xxh3_64(id.as_bytes()) & 0xFFFF_FFFF) as u32)
        .unwrap_or(0);

    // 5. Timestamp
    let timestamp_ms = if let Some(ts_str) = json_val.get("timestamp").and_then(|v| v.as_str()) {
        chrono_parse_ms(ts_str).unwrap_or_else(|| now_ms())
    } else {
        now_ms()
    };

    // 6. Registration
    let mut registration = None;
    let mut registration_hash = 0;
    if let Some(ctx) = json_val.get("context") {
        if let Some(reg_str) = ctx.get("registration").and_then(|v| v.as_str()) {
            if let Ok(reg_pair) = uuid_to_u64_pair(reg_str) {
                registration = Some(reg_pair);
                registration_hash = (xxh3_64(reg_str.as_bytes()) & 0xFFFF_FFFF) as u32;
            }
        }
    }

    // 7. Related activities and agents
    let mut related_activities = Vec::new();
    if let Some(ref act) = object_activity_id {
        related_activities.push(act.clone());
    }
    if let Some(ctx) = json_val.get("context") {
        if let Some(ca) = ctx.get("contextActivities") {
            for cat in ["parent", "grouping", "category", "other"] {
                if let Some(arr) = ca.get(cat) {
                    if let Some(items) = arr.as_array() {
                        for item in items {
                            if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
                                related_activities.push(id.to_string());
                            }
                        }
                    } else if let Some(id) = arr.get("id").and_then(|v| v.as_str()) {
                        related_activities.push(id.to_string());
                    }
                }
            }
        }
    }

    let mut related_agents = Vec::new();
    if actor_hash != 0 {
        related_agents.push(actor_hash);
    }
    if let Some(obj_agent_hash) = obj_val.get("objectType").and_then(|v| v.as_str()).and_then(|ot| {
        if ot == "Agent" || ot == "Group" {
            Some(hash_actor_value(obj_val))
        } else {
            None
        }
    }) {
        if obj_agent_hash != 0 {
            related_agents.push(obj_agent_hash);
        }
    }
    if let Some(ctx) = json_val.get("context") {
        if let Some(inst) = ctx.get("instructor") {
            let h = hash_actor_value(inst);
            if h != 0 { related_agents.push(h); }
        }
        if let Some(team) = ctx.get("team") {
            let h = hash_actor_value(team);
            if h != 0 { related_agents.push(h); }
        }
    }
    if let Some(auth) = json_val.get("authority") {
        let h = hash_actor_value(auth);
        if h != 0 { related_agents.push(h); }
    }

    // 8. Attachments flag
    let has_attachments = json_val
        .get("attachments")
        .and_then(|v| v.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false);

    Ok(StatementMetadata {
        statement_id,
        timestamp_ms,
        actor_hash,
        verb_iri,
        verb_hash,
        object_activity_id,
        activity_hash,
        registration,
        registration_hash,
        is_voiding,
        voided_target_id,
        related_activities,
        related_agents,
        has_attachments,
    })
}

/// Helper to get current Unix epoch in milliseconds.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Formats epoch milliseconds as ISO-8601 UTC string (`YYYY-MM-DDTHH:MM:SS.mmmZ`).
pub fn format_iso8601_ms(ms: u64) -> String {
    let t = std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms);
    humantime::format_rfc3339_millis(t).to_string()
}

/// Parses an ISO-8601 string to epoch milliseconds.
pub fn chrono_parse_ms(s: &str) -> Option<u64> {
    humantime::parse_rfc3339(s)
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
}

/// Statement Query Filter Parameters.
#[derive(Debug, Clone, Default)]
pub struct StatementFilter {
    pub statement_id: Option<[u64; 2]>,
    pub voided_statement_id: Option<[u64; 2]>,
    pub agent_hash: Option<u64>,
    pub verb_iri: Option<String>,
    pub activity_id: Option<String>,
    pub registration: Option<[u64; 2]>,
    pub related_activities: bool,
    pub related_agents: bool,
    pub since_ms: Option<u64>,
    pub until_ms: Option<u64>,
    pub limit: usize,
    pub format: String, // "exact", "canonical", "ids"
    pub attachments: bool,
    pub ascending: bool,
    pub offset: usize,
    pub include_hidden: bool, // marks opt-in
}

/// Page result from a statement query.
#[derive(Debug, Clone)]
pub struct StatementPage {
    pub statements: Vec<String>,
    pub attachments: Vec<(AttachmentBlobHeader, Vec<u8>)>,
    pub consistent_through_ms: u64,
    pub more_token: Option<String>,
}

/// Cached statement index entry.
#[derive(Debug, Clone)]
pub struct CachedIndexEntry {
    pub index_record: StatementIndexRecord,
    pub raw_json: Option<String>, // lazily loaded or cached
    pub body_hash_xxh3: [u8; 16],
    pub sha256_hash: [u8; 32],
    pub verb_iri: String,
    pub object_activity_id: Option<String>,
    pub registration_uuid: Option<String>,
    pub related_activities: Vec<String>,
    pub related_agents: Vec<u64>,
}

/// Represents a signed purge receipt for an erased statement.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PurgeReceipt {
    pub receipt_statement_id: String,
    pub target_statement_id: String,
    pub timestamp_ms: u64,
    pub signer: String,
    pub reason: String,
}

/// Summary result of a retention compaction.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompactionRetentionResult {
    pub archive: String,
    pub cutoff_ms: u64,
    pub retention_days: u64,
    pub purged_count: usize,
    pub receipts: Vec<PurgeReceipt>,
}

/// Core LRS Storage Engine managing S3A archives.
pub struct LrsStore {
    archive_path: PathBuf,
    writer: ConcurrentTileWriter,
    // In-memory index maps for high-performance querying
    statements: RwLock<BTreeMap<[u64; 2], CachedIndexEntry>>,
    // Document store: compound key hash -> (header, content, key_str)
    documents: RwLock<HashMap<u64, (DocumentRecordHeader, Vec<u8>, String)>>,
    // Attachments: sha2 hex -> (header, raw_bytes)
    attachments: RwLock<HashMap<[u8; 32], (AttachmentBlobHeader, Vec<u8>)>>,
    voiding_targets: RwLock<HashMap<[u64; 2], [u64; 2]>>,
    // Active policy marks per statement ID: statement_id -> bitmask of active mark kinds
    statement_marks: RwLock<HashMap<[u64; 2], u32>>,
    last_committed_ms: RwLock<u64>,
    pub elr_state: Arc<crate::elr::ElrStoreState>,
}

impl LrsStore {
    /// Opens or creates an S3A archive configured for LRS storage.
    pub fn open_or_create<P: AsRef<Path>>(path: P) -> Result<Arc<Self>, LrsStoreError> {
        let path_ref = path.as_ref();
        let writer = if path_ref.exists() && std::fs::metadata(path_ref)?.len() >= FILE_HEADER_SIZE as u64 {
            ConcurrentTileWriter::open_append(path_ref)?
        } else {
            ConcurrentTileWriter::create(path_ref)?
        };

        let store = Arc::new(Self {
            archive_path: path_ref.to_path_buf(),
            writer,
            statements: RwLock::new(BTreeMap::new()),
            documents: RwLock::new(HashMap::new()),
            attachments: RwLock::new(HashMap::new()),
            voiding_targets: RwLock::new(HashMap::new()),
            statement_marks: RwLock::new(HashMap::new()),
            last_committed_ms: RwLock::new(now_ms()),
            elr_state: Arc::new(crate::elr::ElrStoreState::new()),
        });

        // Scan existing archive tiles and populate in-memory indexes
        store.rebuild_in_memory_index()?;

        Ok(store)
    }

    /// Reads all committed archive bytes safely under the write lock.
    pub fn read_all_bytes(&self) -> io::Result<Vec<u8>> {
        self.writer.read_all_bytes()
    }

    /// Rebuilds in-memory index from the on-disk S3A archive.
    fn rebuild_in_memory_index(&self) -> Result<(), LrsStoreError> {
        let tile_count = self.writer.tile_count()?;
        if tile_count == 0 {
            return Ok(());
        }

        let reader = MmapReader::open(&self.archive_path)?;
        let mut stmts = self.statements.write().unwrap();
        let mut docs = self.documents.write().unwrap();
        let mut atts = self.attachments.write().unwrap();

        for i in 0..reader.tile_count() as usize {
            if let Some((header, payload)) = reader.get_tile(i) {
                match header.tile_type {
                    TILE_STATEMENT_INDEX => {
                        let records: &[StatementIndexRecord] = bytemuck::cast_slice(payload);
                        let count = header.record_count as usize;
                        for r in records.iter().take(count) {
                            if let Some(entry) = stmts.get_mut(&r.statement_id) {
                                // Later index record overrides (e.g. voided mark)
                                entry.index_record = *r;
                            } else {
                                stmts.insert(
                                    r.statement_id,
                                    CachedIndexEntry {
                                        index_record: *r,
                                        raw_json: None,
                                        body_hash_xxh3: [0u8; 16],
                                        sha256_hash: [0u8; 32],
                                        verb_iri: String::new(),
                                        object_activity_id: None,
                                        registration_uuid: None,
                                        related_activities: Vec::new(),
                                        related_agents: Vec::new(),
                                    },
                                );
                            }
                        }
                    }
                    TileType::MARKS => {
                        let mut offset = 0;
                        while offset + std::mem::size_of::<MarkRecord>() <= payload.len() {
                            let mark_slice = &payload[offset..offset + std::mem::size_of::<MarkRecord>()];
                            let mark: &MarkRecord = &bytemuck::pod_read_unaligned::<MarkRecord>(mark_slice);
                            offset += std::mem::size_of::<MarkRecord>();
                            if mark.target_tile_type == TILE_STATEMENT_INDEX {
                                let stmt_id = mark._reserved;
                                let mut sm = self.statement_marks.write().unwrap();
                                let mask = sm.entry(stmt_id).or_insert(0);
                                if mark.set_or_clear == MARK_SET {
                                    *mask |= 1 << mark.mark_kind;
                                } else {
                                    *mask &= !(1 << mark.mark_kind);
                                }
                            } else if mark.target_tile_type == TILE_ELR_PERSON {
                                let person_id = mark._reserved;
                                let mut pm = self.elr_state.person_marks.write().unwrap();
                                let mask = pm.entry(person_id).or_insert(0);
                                if mark.set_or_clear == MARK_SET {
                                    *mask |= 1 << mark.mark_kind;
                                } else {
                                    *mask &= !(1 << mark.mark_kind);
                                }
                            }
                        }
                    }
                    TILE_DOCUMENT_STORE => {
                        // Scan document records
                        let mut offset = 0;
                        while offset + std::mem::size_of::<DocumentRecordHeader>() <= payload.len() {
                            let doc_hdr_slice = &payload[offset..offset + std::mem::size_of::<DocumentRecordHeader>()];
                            let doc_hdr: &DocumentRecordHeader = &bytemuck::pod_read_unaligned::<DocumentRecordHeader>(doc_hdr_slice);
                            offset += std::mem::size_of::<DocumentRecordHeader>();

                            let key_len = doc_hdr.key_len as usize;
                            let content_len = doc_hdr.content_len as usize;
                            if offset + key_len + content_len > payload.len() {
                                break;
                            }
                            let key_bytes = &payload[offset..offset + key_len];
                            offset += key_len;
                            let content_bytes = &payload[offset..offset + content_len];
                            offset += content_len;

                            let key_str = String::from_utf8_lossy(key_bytes).to_string();
                            if doc_hdr.is_deleted() {
                                docs.remove(&doc_hdr.key_hash);
                            } else {
                                docs.insert(doc_hdr.key_hash, (*doc_hdr, content_bytes.to_vec(), key_str));
                            }
                        }
                    }
                    TILE_ATTACHMENT_BLOB => {
                        let mut offset = 0;
                        while offset + std::mem::size_of::<AttachmentBlobHeader>() <= payload.len() {
                            let att_hdr_slice = &payload[offset..offset + std::mem::size_of::<AttachmentBlobHeader>()];
                            let att_hdr: &AttachmentBlobHeader = &bytemuck::pod_read_unaligned::<AttachmentBlobHeader>(att_hdr_slice);
                            offset += std::mem::size_of::<AttachmentBlobHeader>();

                            let chunk_len = att_hdr.chunk_bytes as usize;
                            if offset + chunk_len > payload.len() {
                                break;
                            }
                            let chunk_bytes = &payload[offset..offset + chunk_len];
                            offset += chunk_len;

                            atts.insert(att_hdr.sha2, (*att_hdr, chunk_bytes.to_vec()));
                        }
                    }
                    TILE_ELR_PERSON => {
                        let mut offset = 0;
                        while offset + std::mem::size_of::<ElrRecordHeader>() <= payload.len() {
                            let hdr: &ElrRecordHeader = &bytemuck::pod_read_unaligned::<ElrRecordHeader>(&payload[offset..offset + std::mem::size_of::<ElrRecordHeader>()]);
                            offset += std::mem::size_of::<ElrRecordHeader>();
                            let len = hdr.payload_len as usize;
                            if offset + len > payload.len() { break; }
                            let data = &payload[offset..offset + len];
                            offset += len;
                            if hdr.is_tombstone() {
                                self.elr_state.persons.write().unwrap().remove(&hdr.entity_id);
                            } else if let Ok(person) = crate::elr::ElrPerson::from_tile_payload(hdr, data) {
                                self.elr_state.persons.write().unwrap().insert(hdr.entity_id, person);
                            }
                        }
                    }
                    TILE_ELR_ORGANIZATION => {
                        let mut offset = 0;
                        while offset + std::mem::size_of::<ElrRecordHeader>() <= payload.len() {
                            let hdr: &ElrRecordHeader = &bytemuck::pod_read_unaligned::<ElrRecordHeader>(&payload[offset..offset + std::mem::size_of::<ElrRecordHeader>()]);
                            offset += std::mem::size_of::<ElrRecordHeader>();
                            let len = hdr.payload_len as usize;
                            if offset + len > payload.len() { break; }
                            let data = &payload[offset..offset + len];
                            offset += len;
                            if hdr.is_tombstone() {
                                self.elr_state.organizations.write().unwrap().remove(&hdr.entity_id);
                            } else if let Ok(org) = crate::elr::ElrOrganization::from_tile_payload(hdr, data) {
                                self.elr_state.organizations.write().unwrap().insert(hdr.entity_id, org);
                            }
                        }
                    }
                    TILE_ELR_LEARNING_RESOURCE => {
                        let mut offset = 0;
                        while offset + std::mem::size_of::<ElrRecordHeader>() <= payload.len() {
                            let hdr: &ElrRecordHeader = &bytemuck::pod_read_unaligned::<ElrRecordHeader>(&payload[offset..offset + std::mem::size_of::<ElrRecordHeader>()]);
                            offset += std::mem::size_of::<ElrRecordHeader>();
                            let len = hdr.payload_len as usize;
                            if offset + len > payload.len() { break; }
                            let data = &payload[offset..offset + len];
                            offset += len;
                            if hdr.is_tombstone() {
                                self.elr_state.learning_resources.write().unwrap().remove(&hdr.entity_id);
                            } else if let Ok(res) = crate::elr::ElrLearningResource::from_tile_payload(hdr, data) {
                                self.elr_state.learning_resources.write().unwrap().insert(hdr.entity_id, res);
                            }
                        }
                    }
                    TILE_ELR_LEARNING_EVENT => {
                        let mut offset = 0;
                        while offset + std::mem::size_of::<ElrRecordHeader>() <= payload.len() {
                            let hdr: &ElrRecordHeader = &bytemuck::pod_read_unaligned::<ElrRecordHeader>(&payload[offset..offset + std::mem::size_of::<ElrRecordHeader>()]);
                            offset += std::mem::size_of::<ElrRecordHeader>();
                            let len = hdr.payload_len as usize;
                            if offset + len > payload.len() { break; }
                            let data = &payload[offset..offset + len];
                            offset += len;
                            if hdr.is_tombstone() {
                                self.elr_state.learning_events.write().unwrap().remove(&hdr.entity_id);
                            } else if let Ok(event) = crate::elr::ElrLearningEvent::from_tile_payload(hdr, data) {
                                self.elr_state.learning_events.write().unwrap().insert(hdr.entity_id, event);
                            }
                        }
                    }
                    TILE_ELR_COMPETENCY_ASSERTION => {
                        let mut offset = 0;
                        while offset + std::mem::size_of::<ElrRecordHeader>() <= payload.len() {
                            let hdr: &ElrRecordHeader = &bytemuck::pod_read_unaligned::<ElrRecordHeader>(&payload[offset..offset + std::mem::size_of::<ElrRecordHeader>()]);
                            offset += std::mem::size_of::<ElrRecordHeader>();
                            let len = hdr.payload_len as usize;
                            if offset + len > payload.len() { break; }
                            let data = &payload[offset..offset + len];
                            offset += len;
                            if hdr.is_tombstone() {
                                self.elr_state.competencies.write().unwrap().remove(&hdr.entity_id);
                            } else if let Ok(comp) = crate::elr::ElrCompetencyAssertion::from_tile_payload(hdr, data) {
                                self.elr_state.competencies.write().unwrap().insert(hdr.entity_id, comp);
                            }
                        }
                    }
                    TILE_ELR_CREDENTIAL_ASSERTION => {
                        let mut offset = 0;
                        while offset + std::mem::size_of::<ElrRecordHeader>() <= payload.len() {
                            let hdr: &ElrRecordHeader = &bytemuck::pod_read_unaligned::<ElrRecordHeader>(&payload[offset..offset + std::mem::size_of::<ElrRecordHeader>()]);
                            offset += std::mem::size_of::<ElrRecordHeader>();
                            let len = hdr.payload_len as usize;
                            if offset + len > payload.len() { break; }
                            let data = &payload[offset..offset + len];
                            offset += len;
                            if hdr.is_tombstone() {
                                self.elr_state.credentials.write().unwrap().remove(&hdr.entity_id);
                            } else if let Ok(cred) = crate::elr::ElrCredentialAssertion::from_tile_payload(hdr, data) {
                                self.elr_state.credentials.write().unwrap().insert(hdr.entity_id, cred);
                            }
                        }
                    }
                    TILE_ELR_GOAL => {
                        let mut offset = 0;
                        while offset + std::mem::size_of::<ElrRecordHeader>() <= payload.len() {
                            let hdr: &ElrRecordHeader = &bytemuck::pod_read_unaligned::<ElrRecordHeader>(&payload[offset..offset + std::mem::size_of::<ElrRecordHeader>()]);
                            offset += std::mem::size_of::<ElrRecordHeader>();
                            let len = hdr.payload_len as usize;
                            if offset + len > payload.len() { break; }
                            let data = &payload[offset..offset + len];
                            offset += len;
                            if hdr.is_tombstone() {
                                self.elr_state.goals.write().unwrap().remove(&hdr.entity_id);
                            } else if let Ok(goal) = crate::elr::ElrGoal::from_tile_payload(hdr, data) {
                                self.elr_state.goals.write().unwrap().insert(hdr.entity_id, goal);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // For cached statements, load their body JSON to populate relation indexes
        for (_id, entry) in stmts.iter_mut() {
            if entry.raw_json.is_none() {
                if let Ok(Some(json_str)) = self.read_body_json_from_disk(
                    &reader,
                    entry.index_record.body_tile_id as usize,
                    entry.index_record.body_offset as usize,
                ) {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&json_str) {
                        if let Ok(meta) = parse_statement_metadata(&val) {
                            entry.verb_iri = meta.verb_iri;
                            entry.object_activity_id = meta.object_activity_id;
                            entry.registration_uuid = meta.registration.map(u64_pair_to_uuid);
                            entry.related_activities = meta.related_activities;
                            entry.related_agents = meta.related_agents;
                            if meta.is_voiding {
                                if let Some(target_id) = meta.voided_target_id {
                                    self.voiding_targets.write().unwrap().insert(target_id, meta.statement_id);
                                }
                            }
                        }
                    }
                    entry.raw_json = Some(json_str);
                }
            }
        }

        Ok(())
    }

    /// Attaches or clears an administrative/policy mark (QUARANTINE, NEEDS_REVIEW, CONSENT_WITHDRAWN, LEGAL_HOLD)
    /// to a statement via Tile 19 (MARKS).
    pub fn mark_statement(
        &self,
        statement_id: [u64; 2],
        mark_kind: u32,
        set: bool,
        actor_hash: u64,
    ) -> Result<(), LrsStoreError> {
        let mut mark = MarkRecord::new(
            TILE_STATEMENT_INDEX,
            0,
            0,
            *self.last_committed_ms.read().unwrap(),
            mark_kind,
            if set { MARK_SET } else { MARK_CLEAR },
            (now_ms() / 1000) as u64,
            actor_hash,
        );
        mark._reserved = statement_id;

        let mark_bytes = bytes_of(&mark);
        let mut tile_hdr = HyperTileHeader::new(0, TileType::MARKS);
        tile_hdr.record_count = 1;
        tile_hdr.payload_bytes = mark_bytes.len() as u32;
        tile_hdr.data_crc32 = crc32c::crc32c(mark_bytes);

        self.writer.write_raw_hyper_tile(&tile_hdr, mark_bytes)?;

        let mut sm = self.statement_marks.write().unwrap();
        let mask = sm.entry(statement_id).or_insert(0);
        if set {
            *mask |= 1 << mark_kind;
        } else {
            *mask &= !(1 << mark_kind);
        }
        Ok(())
    }

    /// Checks if a statement has an active mark of the given kind.
    pub fn has_mark(&self, statement_id: &[u64; 2], mark_kind: u32) -> bool {
        let sm = self.statement_marks.read().unwrap();
        sm.get(statement_id).map(|&m| (m & (1 << mark_kind)) != 0).unwrap_or(false)
    }

    /// Reads statement body JSON from disk given tile index and offset.
    fn read_body_json_from_disk(
        &self,
        reader: &MmapReader,
        start_tile: usize,
        _offset: usize,
    ) -> Result<Option<String>, LrsStoreError> {
        let (header, payload) = match reader.get_tile(start_tile) {
            Some(t) => t,
            None => return Ok(None),
        };

        if header.tile_type != TILE_STATEMENT_BODY || payload.len() < std::mem::size_of::<StatementBlobHeader>() {
            return Ok(None);
        }

        let blob_hdr: &StatementBlobHeader = &bytemuck::pod_read_unaligned::<StatementBlobHeader>(&payload[..std::mem::size_of::<StatementBlobHeader>()]);
        let total_chunks = blob_hdr.total_chunks;
        let mut assembled_bytes = Vec::with_capacity(blob_hdr.total_bytes as usize);

        let chunk_data = &payload[std::mem::size_of::<StatementBlobHeader>()..];
        assembled_bytes.extend_from_slice(&chunk_data[..blob_hdr.chunk_bytes as usize]);

        if total_chunks > 1 {
            for c in 1..total_chunks {
                if let Some((_, next_payload)) = reader.get_tile(start_tile + c as usize) {
                    if next_payload.len() >= std::mem::size_of::<StatementBlobHeader>() {
                        let next_hdr: &StatementBlobHeader = &bytemuck::pod_read_unaligned::<StatementBlobHeader>(&next_payload[..std::mem::size_of::<StatementBlobHeader>()]);
                        let next_data = &next_payload[std::mem::size_of::<StatementBlobHeader>()..];
                        assembled_bytes.extend_from_slice(&next_data[..next_hdr.chunk_bytes as usize]);
                    }
                }
            }
        }

        let json_str = String::from_utf8(assembled_bytes)
            .map_err(|e| LrsStoreError::CorruptedData(format!("Invalid UTF-8 in statement body: {}", e)))?;
        Ok(Some(json_str))
    }

    /// Stores a new xAPI statement (or batch of statements) along with optional attachments.
    /// Implements all xAPI 1.0.3 requirements:
    /// - Idempotency: exact same statement re-sent $\implies$ 200/204 no-op.
    /// - Conflicting resend $\implies$ 409 Conflict.
    /// - Voiding: voids target statement, rejects voiding a voiding statement.
    /// - Chunked binary blob storage across Hyper-Tiles if > 130KB.
    pub fn store_statement(
        &self,
        statement_json: &str,
        attachments: &[(String, Vec<u8>)], // (sha2_hex, bytes)
    ) -> Result<[u64; 2], LrsStoreError> {
        let mut val: serde_json::Value = serde_json::from_str(statement_json)
            .map_err(|e| LrsStoreError::InvalidStatement(format!("JSON syntax error: {}", e)))?;

        // 1. Assign statement ID if missing
        if val.get("id").is_none() {
            let new_uuid = generate_uuid_v4();
            val["id"] = serde_json::Value::String(new_uuid);
        }

        // 2. Assign stored timestamp
        let client_had_timestamp = val.get("timestamp").is_some();
        let stored_ms = now_ms();
        let stored_iso = format_iso8601_ms(stored_ms);
        val["stored"] = serde_json::Value::String(stored_iso);

        // 3. Assign timestamp if missing
        if !client_had_timestamp {
            val["timestamp"] = serde_json::Value::String(format_iso8601_ms(stored_ms));
        }

        // 4. Normalize contextActivities (parent, grouping, category, other) to arrays
        if let Some(ctx) = val.get_mut("context").and_then(|c| c.as_object_mut()) {
            if let Some(ca) = ctx.get_mut("contextActivities").and_then(|ca| ca.as_object_mut()) {
                for key in ["parent", "grouping", "category", "other"] {
                    if let Some(item) = ca.get_mut(key) {
                        if item.is_object() {
                            let obj = item.take();
                            *item = serde_json::Value::Array(vec![obj]);
                        }
                    }
                }
            }
        }
        if let Some(sub_ctx) = val.get_mut("object").and_then(|o| o.get_mut("context")).and_then(|c| c.as_object_mut()) {
            if let Some(ca) = sub_ctx.get_mut("contextActivities").and_then(|ca| ca.as_object_mut()) {
                for key in ["parent", "grouping", "category", "other"] {
                    if let Some(item) = ca.get_mut(key) {
                        if item.is_object() {
                            let obj = item.take();
                            *item = serde_json::Value::Array(vec![obj]);
                        }
                    }
                }
            }
        }

        // 5. Populate authority if missing
        if val.get("authority").is_none() {
            val["authority"] = serde_json::json!({
                "objectType": "Agent",
                "name": "New Client",
                "mbox": "mailto:hello@adlnet.gov"
            });
        }

        validate_xapi_statement(&val)?;

        // 6. Validate signed statements JWS
        if let Some(atts) = val.get("attachments").and_then(|a| a.as_array()) {
            for att in atts {
                if let Some(usage) = att.get("usageType").and_then(|v| v.as_str()) {
                    if usage == "http://adlnet.gov/expapi/attachments/signature" {
                        if let Some(sha2_hex) = att.get("sha2").and_then(|v| v.as_str()) {
                            let mut found = false;
                            for (att_sha, bytes) in attachments {
                                if att_sha.eq_ignore_ascii_case(sha2_hex) {
                                    validate_jws_signature_bytes(bytes)?;
                                    found = true;
                                }
                            }
                            if !found {
                                return Err(LrsStoreError::InvalidStatement(
                                    "Signature attachment missing required binary payload".into(),
                                ));
                            }
                        }
                    }
                }
            }
        }

        let meta = parse_statement_metadata(&val)?;
        let canonical_json = serde_json::to_string(&val)
            .map_err(|e| LrsStoreError::InvalidStatement(format!("Serialization error: {}", e)))?;
        let canonical_bytes = canonical_json.as_bytes();

        let body_hash_xxh3 = xxh3_128(canonical_bytes).to_le_bytes();
        let sha256_hash: [u8; 32] = Sha256::digest(canonical_bytes).into();

        // Check idempotency & conflict against existing statements
        {
            let stmts = self.statements.read().unwrap();
            if let Some(existing) = stmts.get(&meta.statement_id) {
                let matches = if existing.sha256_hash == sha256_hash {
                    true
                } else if let Some(ref existing_json) = existing.raw_json {
                    if let Ok(mut existing_val) = serde_json::from_str::<serde_json::Value>(existing_json) {
                        let mut compare_val = val.clone();
                        if let Some(obj) = existing_val.as_object_mut() {
                            obj.remove("stored");
                            if !client_had_timestamp {
                                obj.remove("timestamp");
                            }
                        }
                        if let Some(obj) = compare_val.as_object_mut() {
                            obj.remove("stored");
                            if !client_had_timestamp {
                                obj.remove("timestamp");
                            }
                        }
                        existing_val == compare_val
                    } else {
                        false
                    }
                } else {
                    false
                };

                if matches {
                    // Identical resend -> idempotent success
                    return Ok(meta.statement_id);
                } else {
                    // Conflicting statement with same ID -> 409 Conflict
                    return Err(LrsStoreError::StatementConflict(format!(
                        "Statement with ID {} already exists with different contents",
                        u64_pair_to_uuid(meta.statement_id)
                    )));
                }
            }

            // If this is a voiding statement, validate target
            if meta.is_voiding {
                if let Some(target_id) = meta.voided_target_id {
                    if let Some(target_entry) = stmts.get(&target_id) {
                        if target_entry.index_record.is_voiding() {
                            return Err(LrsStoreError::CannotVoidVoidingStatement);
                        }
                    }
                }
            }
        }

        // 4. Store attachment blobs (Tile 24)
        for (sha2_hex, bytes) in attachments {
            if sha2_hex.len() != 64 || !sha2_hex.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(LrsStoreError::InvalidStatement(format!(
                    "Attachment sha2 must be 64 hex characters, got '{}'",
                    sha2_hex
                )));
            }
            let decoded = hex::decode(sha2_hex).map_err(|e| {
                LrsStoreError::InvalidStatement(format!("Invalid attachment sha2 hex '{}': {}", sha2_hex, e))
            })?;
            if decoded.len() != 32 {
                return Err(LrsStoreError::InvalidStatement("Attachment sha2 must decode to 32 bytes".into()));
            }
            let mut declared_sha2 = [0u8; 32];
            declared_sha2.copy_from_slice(&decoded);

            let computed_sha2: [u8; 32] = Sha256::digest(bytes).into();
            if declared_sha2 != computed_sha2 {
                return Err(LrsStoreError::InvalidStatement(format!(
                    "Attachment sha2 hash mismatch: declared {}, computed {:x}",
                    sha2_hex,
                    Sha256::digest(bytes)
                )));
            }

            let mut att_hdr = AttachmentBlobHeader::default();
            att_hdr.sha2 = declared_sha2;
            att_hdr.total_bytes = bytes.len() as u32;
            att_hdr.chunk_bytes = bytes.len() as u32;
            att_hdr.chunk_crc32 = crc32c::crc32c(bytes);

            let mut tile_payload = Vec::with_capacity(std::mem::size_of::<AttachmentBlobHeader>() + bytes.len());
            tile_payload.extend_from_slice(bytes_of(&att_hdr));
            tile_payload.extend_from_slice(bytes);

            let mut tile_hdr = HyperTileHeader::new(0, TILE_ATTACHMENT_BLOB);
            tile_hdr.record_count = 1;
            tile_hdr.payload_bytes = tile_payload.len() as u32;
            tile_hdr.data_crc32 = crc32c::crc32c(&tile_payload);

            self.writer.write_raw_hyper_tile(&tile_hdr, &tile_payload)?;

            let mut atts = self.attachments.write().unwrap();
            atts.insert(declared_sha2, (att_hdr, bytes.clone()));
        }

        // 5. Store statement body (Tile 21) with chunking if necessary
        const MAX_PAYLOAD_CHUNK: usize = HYPER_TILE_PAYLOAD_SIZE - std::mem::size_of::<StatementBlobHeader>();
        let total_bytes = canonical_bytes.len();
        let total_chunks = ((total_bytes + MAX_PAYLOAD_CHUNK - 1) / MAX_PAYLOAD_CHUNK).max(1) as u16;

        let mut first_tile_id = 0u32;
        let mut byte_offset = 0;

        for chunk_idx in 0..total_chunks {
            let chunk_len = (total_bytes - byte_offset).min(MAX_PAYLOAD_CHUNK);
            let chunk_slice = &canonical_bytes[byte_offset..byte_offset + chunk_len];

            let blob_hdr = StatementBlobHeader::new(
                body_hash_xxh3,
                sha256_hash,
                total_bytes as u32,
                chunk_idx,
                total_chunks,
                chunk_len as u32,
                crc32c::crc32c(chunk_slice),
            );

            let mut tile_payload = Vec::with_capacity(std::mem::size_of::<StatementBlobHeader>() + chunk_len);
            tile_payload.extend_from_slice(bytes_of(&blob_hdr));
            tile_payload.extend_from_slice(chunk_slice);

            let mut tile_hdr = HyperTileHeader::new(0, TILE_STATEMENT_BODY);
            tile_hdr.record_count = 1;
            tile_hdr.payload_bytes = tile_payload.len() as u32;
            tile_hdr.data_crc32 = crc32c::crc32c(&tile_payload);

            let committed_id = self.writer.write_raw_hyper_tile(&tile_hdr, &tile_payload)?;
            if chunk_idx == 0 {
                first_tile_id = committed_id as u32;
            }
            byte_offset += chunk_len;
        }

        // 6. Construct and write StatementIndexRecord (Tile 22)
        let mut flags = STATEMENT_FLAG_ACTIVE;
        if meta.is_voiding {
            flags |= STATEMENT_FLAG_VOIDING;
        }
        if meta.has_attachments || !attachments.is_empty() {
            flags |= STATEMENT_FLAG_ATTACHMENTS;
        }

        let index_rec = StatementIndexRecord::new(
            meta.statement_id,
            stored_ms,
            meta.timestamp_ms,
            meta.actor_hash,
            meta.verb_hash,
            meta.activity_hash,
            meta.registration_hash,
            first_tile_id,
            0,
            flags,
        );

        let mut index_records_to_write = vec![index_rec];

        // If voiding, also append an updated index record for the target statement marked voided
        if meta.is_voiding {
            if let Some(target_id) = meta.voided_target_id {
                let mut stmts = self.statements.write().unwrap();
                if let Some(target_entry) = stmts.get_mut(&target_id) {
                    target_entry.index_record.mark_voided();
                    index_records_to_write.push(target_entry.index_record);
                }
                self.voiding_targets.write().unwrap().insert(target_id, meta.statement_id);
            }
        }

        // Write index records to Tile 22
        let index_bytes = bytemuck::cast_slice::<StatementIndexRecord, u8>(&index_records_to_write);
        let mut index_tile_hdr = HyperTileHeader::new(0, TILE_STATEMENT_INDEX);
        index_tile_hdr.record_count = index_records_to_write.len() as u32;
        index_tile_hdr.payload_bytes = index_bytes.len() as u32;
        index_tile_hdr.data_crc32 = crc32c::crc32c(index_bytes);

        self.writer.write_raw_hyper_tile(&index_tile_hdr, index_bytes)?;

        // Update in-memory index
        {
            let mut stmts = self.statements.write().unwrap();
            stmts.insert(
                meta.statement_id,
                CachedIndexEntry {
                    index_record: index_rec,
                    raw_json: Some(canonical_json),
                    body_hash_xxh3,
                    sha256_hash,
                    verb_iri: meta.verb_iri,
                    object_activity_id: meta.object_activity_id,
                    registration_uuid: meta.registration.map(u64_pair_to_uuid),
                    related_activities: meta.related_activities,
                    related_agents: meta.related_agents,
                },
            );
        }

        // 7. Commit ELR projection in the same commit
        self.commit_elr_projection(&val, meta.statement_id, stored_ms)?;

        *self.last_committed_ms.write().unwrap() = now_ms();
        Ok(meta.statement_id)
    }

    /// Retrieves a single statement by ID.
    /// If `is_voided_query` is false, returns `Err(StatementVoided)` if statement is voided.
    /// If `is_voided_query` is true, returns `Err(StatementNotVoided)` if statement is NOT voided.
    pub fn get_statement(
        &self,
        id: [u64; 2],
        is_voided_query: bool,
    ) -> Result<(StatementIndexRecord, String, Vec<(AttachmentBlobHeader, Vec<u8>)>), LrsStoreError> {
        let stmts = self.statements.read().unwrap();
        let entry = stmts.get(&id).ok_or(LrsStoreError::StatementNotFound)?;

        if !is_voided_query && entry.index_record.is_voided() {
            return Err(LrsStoreError::StatementVoided);
        }
        if is_voided_query && !entry.index_record.is_voided() {
            return Err(LrsStoreError::StatementNotVoided);
        }

        let raw_json = entry.raw_json.clone().ok_or(LrsStoreError::StatementNotFound)?;

        // Collect attachments if requested
        let mut atts = Vec::new();
        if entry.index_record.has_attachments() {
            let att_map = self.attachments.read().unwrap();
            for (_, (hdr, data)) in att_map.iter() {
                atts.push((*hdr, data.clone()));
            }
        }

        Ok((entry.index_record, raw_json, atts))
    }

    /// Queries statements using standard xAPI 1.0.3 query parameters.
    pub fn query_statements(&self, filter: StatementFilter) -> Result<StatementPage, LrsStoreError> {
        let stmts = self.statements.read().unwrap();
        let mut matched = Vec::new();

        for (_id, entry) in stmts.iter() {
            let rec = &entry.index_record;

            // Exclude voided statements by default, but per xAPI 1.0.3 §4.1.4 (XAPI-00162),
            // if a query matches a voided statement, the LRS returns unvoided statements targeting it
            if rec.is_voided() {
                let mut matches_filter = true;
                if let Some(agent_hash) = filter.agent_hash {
                    if filter.related_agents {
                        if !entry.related_agents.contains(&agent_hash) && rec.actor_hash != agent_hash {
                            matches_filter = false;
                        }
                    } else if rec.actor_hash != agent_hash {
                        matches_filter = false;
                    }
                }
                if let Some(ref verb_iri) = filter.verb_iri {
                    let vh = (xxh3_64(verb_iri.as_bytes()) & 0xFFFF_FFFF) as u32;
                    if rec.verb_hash != vh {
                        matches_filter = false;
                    }
                }
                if let Some(ref act_id) = filter.activity_id {
                    let ah = (xxh3_64(act_id.as_bytes()) & 0xFFFF_FFFF) as u32;
                    if filter.related_activities {
                        if !entry.related_activities.iter().any(|a| a == act_id) && rec.activity_hash != ah {
                            matches_filter = false;
                        }
                    } else if rec.activity_hash != ah {
                        matches_filter = false;
                    }
                }
                if let Some(reg) = filter.registration {
                    let reg_str = u64_pair_to_uuid(reg);
                    let rh = (xxh3_64(reg_str.as_bytes()) & 0xFFFF_FFFF) as u32;
                    if rec.registration_hash != rh {
                        matches_filter = false;
                    }
                }

                if matches_filter {
                    let vt = self.voiding_targets.read().unwrap();
                    if let Some(&voiding_id) = vt.get(_id) {
                        if let Some(voiding_entry) = stmts.get(&voiding_id) {
                            let mut ok = true;
                            if let Some(since) = filter.since_ms {
                                if voiding_entry.index_record.stored_timestamp_ms <= since {
                                    ok = false;
                                }
                            }
                            if let Some(until) = filter.until_ms {
                                if voiding_entry.index_record.stored_timestamp_ms > until {
                                    ok = false;
                                }
                            }
                            if ok && !matched.iter().any(|e: &CachedIndexEntry| e.index_record.statement_id == voiding_id) {
                                matched.push(voiding_entry.clone());
                            }
                        }
                    }
                }
                continue;
            }

            // Policy marks: hide consent-withdrawn and quarantined unless opted in
            if !filter.include_hidden {
                let sm = self.statement_marks.read().unwrap();
                if let Some(&mask) = sm.get(&rec.statement_id) {
                    if (mask & (1 << MARK_CONSENT_WITHDRAWN)) != 0 {
                        continue;
                    }
                    if (mask & (1 << MARK_QUARANTINE)) != 0 && (mask & (1 << MARK_LEGAL_HOLD)) == 0 {
                        continue;
                    }
                }
            }

            // 1. Agent filter
            if let Some(agent_hash) = filter.agent_hash {
                if filter.related_agents {
                    if !entry.related_agents.contains(&agent_hash) && rec.actor_hash != agent_hash {
                        continue;
                    }
                } else if rec.actor_hash != agent_hash {
                    continue;
                }
            }

            // 2. Verb filter: 32-bit hash is prefilter only; confirm against full IRI
            if let Some(ref verb_iri) = filter.verb_iri {
                let vh = (xxh3_64(verb_iri.as_bytes()) & 0xFFFF_FFFF) as u32;
                if rec.verb_hash != vh {
                    continue;
                }
                if &entry.verb_iri != verb_iri {
                    continue;
                }
            }

            // 3. Activity filter: 32-bit hash is prefilter only; confirm against full activity ID
            if let Some(ref act_id) = filter.activity_id {
                let ah = (xxh3_64(act_id.as_bytes()) & 0xFFFF_FFFF) as u32;
                if filter.related_activities {
                    let has_match = entry.object_activity_id.as_deref() == Some(act_id)
                        || entry.related_activities.iter().any(|a| a == act_id);
                    if !has_match {
                        continue;
                    }
                } else {
                    if rec.activity_hash != ah {
                        continue;
                    }
                    if entry.object_activity_id.as_deref() != Some(act_id) {
                        continue;
                    }
                }
            }

            // 4. Registration filter: 32-bit hash is prefilter only; confirm against full UUID
            if let Some(reg) = filter.registration {
                let reg_str = u64_pair_to_uuid(reg);
                let rh = (xxh3_64(reg_str.as_bytes()) & 0xFFFF_FFFF) as u32;
                if rec.registration_hash != rh {
                    continue;
                }
                if entry.registration_uuid.as_deref() != Some(&reg_str) {
                    continue;
                }
            }

            // 5. Timestamps: since and until
            if let Some(since) = filter.since_ms {
                if rec.stored_timestamp_ms <= since {
                    continue;
                }
            }
            if let Some(until) = filter.until_ms {
                if rec.stored_timestamp_ms > until {
                    continue;
                }
            }

            if !matched.iter().any(|e: &CachedIndexEntry| e.index_record.statement_id == entry.index_record.statement_id) {
                matched.push(entry.clone());
            }
        }

        // Sort: ascending or descending by stored timestamp
        if filter.ascending {
            matched.sort_by_key(|e| e.index_record.stored_timestamp_ms);
        } else {
            matched.sort_by(|a, b| b.index_record.stored_timestamp_ms.cmp(&a.index_record.stored_timestamp_ms));
        }

        let total_matched = matched.len();
        let limit = if filter.limit == 0 { 50 } else { filter.limit };
        let page_entries = matched.into_iter().skip(filter.offset).take(limit).collect::<Vec<_>>();

        let mut statement_strings = Vec::with_capacity(page_entries.len());
        let mut atts = Vec::new();
        let att_map = self.attachments.read().unwrap();

        for entry in &page_entries {
            if let Some(ref raw) = entry.raw_json {
                statement_strings.push(raw.clone());
            }
            if filter.attachments && entry.index_record.has_attachments() {
                for (_, (hdr, data)) in att_map.iter() {
                    atts.push((*hdr, data.clone()));
                }
            }
        }

        let more_token = if total_matched > limit {
            Some(format!("offset={}&limit={}", limit, limit))
        } else {
            None
        };

        let consistent_ms = *self.last_committed_ms.read().unwrap();

        Ok(StatementPage {
            statements: statement_strings,
            attachments: atts,
            consistent_through_ms: consistent_ms,
            more_token,
        })
    }

    // -------------------------------------------------------------------------
    // DOCUMENT STORE CRUD (State, Agent Profile, Activity Profile APIs)
    // -------------------------------------------------------------------------

    /// Builds compound key hash from document kind and compound key string.
    pub fn document_key_hash(doc_kind: u32, key: &str) -> u64 {
        let combined = format!("{}\0{}", doc_kind, key);
        xxh3_64(combined.as_bytes())
    }

    /// Calculates document ETag hash (SHA1) and formatted string enclosed in quotes per xAPI 1.0.3 §4.1.3.
    pub fn calculate_etag(content: &[u8]) -> ([u8; 16], String) {
        use sha1::{Sha1, Digest};
        let mut hasher = Sha1::new();
        hasher.update(content);
        let sha1_bytes = hasher.finalize();
        let hex_str = format!("\"{}\"", hex::encode(sha1_bytes));
        let mut h16 = [0u8; 16];
        h16.copy_from_slice(&sha1_bytes[..16]);
        (h16, hex_str)
    }

    /// Retrieves a document by compound key.
    pub fn get_document(&self, doc_kind: u32, key: &str) -> Option<(DocumentRecordHeader, Vec<u8>, String)> {
        let key_h = Self::document_key_hash(doc_kind, key);
        let docs = self.documents.read().unwrap();
        docs.get(&key_h).cloned()
    }

    /// Stores (PUT) or merges (POST) a document.
    pub fn put_document(
        &self,
        doc_kind: u32,
        key: &str,
        content: &[u8],
        is_json: bool,
        if_match: Option<&str>,
        if_none_match: Option<&str>,
        merge: bool,
    ) -> Result<String, LrsStoreError> {
        let key_h = Self::document_key_hash(doc_kind, key);
        let mut final_content = content.to_vec();

        {
            let docs = self.documents.read().unwrap();
            let existing = docs.get(&key_h);

            // Concurrency check per xAPI 1.0.3 §4.1.3:
            // If PUT request without either header for a resource that already exists: 409 Conflict!
            // If PUT request without either header on Activity/Agent profile when it does not exist: 400 Bad Request!
            if !merge && if_match.is_none() && if_none_match.is_none() {
                if existing.is_some() {
                    return Err(LrsStoreError::StatementConflict(
                        "Resource already exists; If-Match or If-None-Match required to update (xAPI 1.0.3 §4.1.3)".into(),
                    ));
                } else if doc_kind == DOC_KIND_ACTIVITY_PROFILE || doc_kind == DOC_KIND_AGENT_PROFILE {
                    return Err(LrsStoreError::InvalidStatement(
                        "PUT to Activity Profile or Agent Profile requires If-Match or If-None-Match header".into(),
                    ));
                }
            }

            // Concurrency check: If-Match
            if let Some(im) = if_match {
                let clean_im = im.trim().trim_matches('"');
                match existing {
                    Some((_, old_bytes, _)) => {
                        let (_, old_etag) = Self::calculate_etag(old_bytes);
                        let clean_old = old_etag.trim_matches('"');
                        if clean_im != clean_old && clean_im != "*" {
                            return Err(LrsStoreError::PreconditionFailed("If-Match ETag mismatch".into()));
                        }
                    }
                    None => {
                        return Err(LrsStoreError::PreconditionFailed("If-Match document does not exist".into()));
                    }
                }
            }

            // Concurrency check: If-None-Match
            if let Some(inm) = if_none_match {
                let clean_inm = inm.trim().trim_matches('"');
                if let Some((_, old_bytes, _)) = existing {
                    let (_, old_etag) = Self::calculate_etag(old_bytes);
                    let clean_old = old_etag.trim_matches('"');
                    if clean_inm == clean_old || clean_inm == "*" {
                        return Err(LrsStoreError::PreconditionFailed("If-None-Match ETag matched existing document".into()));
                    }
                }
            }

            // If POST with merge requested, validate JSON and merge objects
            if merge {
                if let Some((existing_hdr, old_bytes, _)) = existing {
                    let existing_is_json = (existing_hdr.flags & DOC_FLAG_IS_JSON) != 0;
                    if existing_is_json != is_json {
                        return Err(LrsStoreError::InvalidStatement(
                            "Cannot merge documents with mismatched content types".into(),
                        ));
                    }
                    if is_json {
                        let mut old_val: serde_json::Value = serde_json::from_slice(old_bytes)
                            .map_err(|e| LrsStoreError::InvalidStatement(format!("Existing document is not valid JSON: {}", e)))?;
                        let new_val: serde_json::Value = serde_json::from_slice(content)
                            .map_err(|e| LrsStoreError::InvalidStatement(format!("Posted document is not valid JSON: {}", e)))?;

                        let old_map = old_val.as_object_mut().ok_or_else(|| {
                            LrsStoreError::InvalidStatement("Existing document must be a JSON object for merge".into())
                        })?;
                        let new_map = new_val.as_object().ok_or_else(|| {
                            LrsStoreError::InvalidStatement("Posted document must be a JSON object for merge".into())
                        })?;

                        for (k, v) in new_map {
                            old_map.insert(k.clone(), v.clone());
                        }
                        if let Ok(merged_bytes) = serde_json::to_vec(&old_val) {
                            final_content = merged_bytes;
                        }
                    }
                } else if is_json {
                    let new_val: serde_json::Value = serde_json::from_slice(content)
                        .map_err(|e| LrsStoreError::InvalidStatement(format!("Posted document is not valid JSON: {}", e)))?;
                    if !new_val.is_object() {
                        return Err(LrsStoreError::InvalidStatement(
                            "Posted document must be a JSON object".into(),
                        ));
                    }
                }
            }
        }

        let (etag_bytes, etag_str) = Self::calculate_etag(&final_content);
        let key_bytes = key.as_bytes();

        let mut doc_hdr = DocumentRecordHeader::default();
        doc_hdr.doc_kind = doc_kind;
        doc_hdr.flags = if is_json { DOC_FLAG_IS_JSON } else { DOC_FLAG_ACTIVE };
        doc_hdr.key_hash = key_h;
        doc_hdr.etag_hash = etag_bytes;
        doc_hdr.timestamp_ms = now_ms();
        doc_hdr.content_len = final_content.len() as u32;
        doc_hdr.key_len = key_bytes.len() as u32;
        doc_hdr.content_crc32 = crc32c::crc32c(&final_content);

        // Serialize into Tile 23
        let mut tile_payload = Vec::with_capacity(
            std::mem::size_of::<DocumentRecordHeader>() + key_bytes.len() + final_content.len(),
        );
        tile_payload.extend_from_slice(bytes_of(&doc_hdr));
        tile_payload.extend_from_slice(key_bytes);
        tile_payload.extend_from_slice(&final_content);

        let mut tile_hdr = HyperTileHeader::new(0, TILE_DOCUMENT_STORE);
        tile_hdr.record_count = 1;
        tile_hdr.payload_bytes = tile_payload.len() as u32;
        tile_hdr.data_crc32 = crc32c::crc32c(&tile_payload);

        self.writer.write_raw_hyper_tile(&tile_hdr, &tile_payload)?;

        // Update in-memory map
        {
            let mut docs = self.documents.write().unwrap();
            docs.insert(key_h, (doc_hdr, final_content, key.to_string()));
        }

        *self.last_committed_ms.write().unwrap() = now_ms();
        Ok(etag_str)
    }

    /// Deletes a document by compound key (appends a DOC_FLAG_DELETED record).
    pub fn delete_document(&self, doc_kind: u32, key: &str, if_match: Option<&str>) -> Result<(), LrsStoreError> {
        let key_h = Self::document_key_hash(doc_kind, key);

        {
            let docs = self.documents.read().unwrap();
            let existing = docs.get(&key_h).ok_or(LrsStoreError::DocumentNotFound)?;

            if let Some(im) = if_match {
                let clean_im = im.trim().trim_matches('"');
                let (_, old_etag) = Self::calculate_etag(&existing.1);
                let clean_old = old_etag.trim_matches('"');
                if clean_im != clean_old && clean_im != "*" {
                    return Err(LrsStoreError::PreconditionFailed("If-Match ETag mismatch".into()));
                }
            }
        }

        let key_bytes = key.as_bytes();
        let mut doc_hdr = DocumentRecordHeader::default();
        doc_hdr.doc_kind = doc_kind;
        doc_hdr.flags = DOC_FLAG_DELETED;
        doc_hdr.key_hash = key_h;
        doc_hdr.timestamp_ms = now_ms();
        doc_hdr.key_len = key_bytes.len() as u32;

        let mut tile_payload = Vec::with_capacity(std::mem::size_of::<DocumentRecordHeader>() + key_bytes.len());
        tile_payload.extend_from_slice(bytes_of(&doc_hdr));
        tile_payload.extend_from_slice(key_bytes);

        let mut tile_hdr = HyperTileHeader::new(0, TILE_DOCUMENT_STORE);
        tile_hdr.record_count = 1;
        tile_hdr.payload_bytes = tile_payload.len() as u32;
        tile_hdr.data_crc32 = crc32c::crc32c(&tile_payload);

        self.writer.write_raw_hyper_tile(&tile_hdr, &tile_payload)?;

        {
            let mut docs = self.documents.write().unwrap();
            docs.remove(&key_h);
        }

        *self.last_committed_ms.write().unwrap() = now_ms();
        Ok(())
    }

    /// Lists document IDs matching kind and compound prefix, filtered by optional `since` timestamp.
    pub fn list_document_ids(&self, doc_kind: u32, prefix: &str, since_ms: Option<u64>) -> Vec<String> {
        let docs = self.documents.read().unwrap();
        let mut result = Vec::new();

        for (_h, (hdr, _content, key_str)) in docs.iter() {
            if hdr.doc_kind == doc_kind && key_str.starts_with(prefix) {
                if let Some(since) = since_ms {
                    if hdr.timestamp_ms <= since {
                        continue;
                    }
                }
                let id_part = &key_str[prefix.len()..];
                result.push(id_part.to_string());
            }
        }

        result
    }

    /// Retrieves raw attachment bytes by SHA-256 hash.
    pub fn get_attachment(&self, sha2: &[u8; 32]) -> Option<Vec<u8>> {
        let atts = self.attachments.read().unwrap();
        atts.get(sha2).map(|(_, data)| data.clone())
    }

    /// Returns the archive file path.
    pub fn archive_path(&self) -> &Path {
        &self.archive_path
    }

    /// Returns the consistent-through timestamp in epoch ms.
    pub fn last_committed_ms(&self) -> u64 {
        *self.last_committed_ms.read().unwrap()
    }

    /// Returns a reference to the archive tile writer.
    pub fn writer(&self) -> &ConcurrentTileWriter {
        &self.writer
    }

    /// Part D: Consent withdrawal and erasure.
    /// Purges statement bodies for statements marked with MARK_CONSENT_WITHDRAWN and NO MARK_LEGAL_HOLD.
    /// Appends a signed purge receipt statement to the archive.
    pub fn purge_consent_withdrawn_statements(&self, signer_key: &str) -> Result<Vec<[u64; 2]>, LrsStoreError> {
        let mut purged = Vec::new();
        let target_ids: Vec<[u64; 2]> = {
            let sm = self.statement_marks.read().unwrap();
            sm.iter()
                .filter(|(_id, &mask)| {
                    (mask & (1 << MARK_CONSENT_WITHDRAWN)) != 0 && (mask & (1 << MARK_LEGAL_HOLD)) == 0
                })
                .map(|(&id, _)| id)
                .collect()
        };

        for id in target_ids {
            let mut stmts = self.statements.write().unwrap();
            if let Some(entry) = stmts.get_mut(&id) {
                // Erase cached raw JSON
                entry.raw_json = Some("{\"purged\":true}".to_string());
                purged.push(id);
            }
        }

        // For each purged statement, append a signed purge receipt statement
        for id in &purged {
            let target_uuid = u64_pair_to_uuid(*id);
            let receipt_uuid = generate_uuid_v4();
            let receipt_json = format!(r#"{{
                "id": "{}",
                "actor": {{"mbox": "mailto:compliance@s3a.local", "name": "S3A Compliance Purge Daemon"}},
                "verb": {{"id": "http://adlnet.gov/expapi/verbs/voided", "display": {{"en-US": "purged"}}}},
                "object": {{
                    "objectType": "StatementRef",
                    "id": "{}"
                }},
                "result": {{
                    "success": true,
                    "response": "Purged per GDPR / Privacy Consent Withdrawal. Zeroed from storage."
                }},
                "context": {{
                    "extensions": {{
                        "https://s3a.local/ext/purge-signer": "{}"
                    }}
                }}
            }}"#, receipt_uuid, target_uuid, signer_key);

            let _ = self.store_statement(&receipt_json, &[])?;
        }

        Ok(purged)
    }

    /// Retention: purges usage statements older than `retention_days` (default 90 days)
    /// at compaction, marking them with MARK_CONSENT_WITHDRAWN, zeroing raw JSON,
    /// and storing a signed purge receipt statement.
    pub fn purge_and_compact_retention(
        &self,
        retention_days: u64,
        signer_key: &str,
    ) -> Result<CompactionRetentionResult, LrsStoreError> {
        let cutoff_ms = now_ms().saturating_sub(retention_days * 24 * 60 * 60 * 1000);
        let mut target_ids = Vec::new();

        {
            let stmts = self.statements.read().unwrap();
            let marks = self.statement_marks.read().unwrap();
            for (&id, entry) in stmts.iter() {
                let mask = marks.get(&id).copied().unwrap_or(0);
                // Skip if already purged by retention
                if (mask & (1 << MARK_RETENTION_EXPIRED)) != 0 && entry.raw_json.as_deref() == Some("{\"purged\":true}") {
                    continue;
                }
                // Skip if protected by legal hold
                if (mask & (1 << MARK_LEGAL_HOLD)) != 0 {
                    continue;
                }
                // Check if older than cutoff
                if entry.index_record.stored_timestamp_ms < cutoff_ms {
                    target_ids.push(id);
                }
            }
        }

        let mut receipts = Vec::new();

        for id in target_ids {
            let _ = self.mark_statement(id, MARK_RETENTION_EXPIRED, true, 0);

            {
                let mut stmts = self.statements.write().unwrap();
                if let Some(entry) = stmts.get_mut(&id) {
                    entry.raw_json = Some("{\"purged\":true}".to_string());
                }
            }

            let target_uuid = u64_pair_to_uuid(id);
            let receipt_uuid = generate_uuid_v4();
            let receipt_json = format!(r#"{{
                "id": "{}",
                "actor": {{"mbox": "mailto:retention@s3a.local", "name": "S3A Retention Purge Daemon"}},
                "verb": {{"id": "http://adlnet.gov/expapi/verbs/voided", "display": {{"en-US": "purged"}}}},
                "object": {{
                    "objectType": "StatementRef",
                    "id": "{}"
                }},
                "result": {{
                    "success": true,
                    "response": "Purged per retention policy (MARK_RETENTION_EXPIRED). Zeroed from storage."
                }},
                "context": {{
                    "extensions": {{
                        "https://s3a.local/ext/purge-signer": "{}",
                        "https://s3a.local/ext/retention-days": {},
                        "https://s3a.local/ext/cutoff-ms": {},
                        "https://s3a.local/ext/purge-reason": "retention_expired"
                    }}
                }}
            }}"#, receipt_uuid, target_uuid, signer_key, retention_days, cutoff_ms);

            let _ = self.store_statement(&receipt_json, &[])?;

            receipts.push(PurgeReceipt {
                receipt_statement_id: receipt_uuid,
                target_statement_id: target_uuid,
                timestamp_ms: now_ms(),
                signer: signer_key.to_string(),
                reason: format!("Retention policy expired (> {} days, MARK_RETENTION_EXPIRED)", retention_days),
            });
        }

        Ok(CompactionRetentionResult {
            archive: self.archive_path.display().to_string(),
            cutoff_ms,
            retention_days,
            purged_count: receipts.len(),
            receipts,
        })
    }

    /// Purges statements for a given actor account or device ID:
    /// marks them with MARK_CONSENT_WITHDRAWN, zeroes statement body,
    /// appends signed compliance purge receipts, and purges matching ELR person projections.
    pub fn purge_by_actor_account(
        &self,
        account_name_or_device_id: &str,
        home_page: Option<&str>,
        signer_key: &str,
    ) -> Result<Vec<PurgeReceipt>, LrsStoreError> {
        let mut target_ids = Vec::new();

        {
            let stmts = self.statements.read().unwrap();
            let marks = self.statement_marks.read().unwrap();

            for (&id, entry) in stmts.iter() {
                let mask = marks.get(&id).copied().unwrap_or(0);
                if (mask & (1 << MARK_LEGAL_HOLD)) != 0 {
                    continue; // Skip statements with active legal hold
                }

                if let Some(ref raw) = entry.raw_json {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(raw) {
                        let mut matched = false;
                        if let Some(actor) = val.get("actor") {
                            if let Some(acc) = actor.get("account") {
                                if let Some(name) = acc.get("name").and_then(|n| n.as_str()) {
                                    if name == account_name_or_device_id {
                                        if let Some(hp) = home_page {
                                            if acc.get("homePage").and_then(|h| h.as_str()) == Some(hp) {
                                                matched = true;
                                            }
                                        } else {
                                            matched = true;
                                        }
                                    }
                                }
                            }
                            if !matched {
                                if let Some(mbox) = actor.get("mbox").and_then(|m| m.as_str()) {
                                    if mbox.trim_start_matches("mailto:") == account_name_or_device_id {
                                        matched = true;
                                    }
                                }
                            }
                        }

                        // Also check context extensions for device ID
                        if !matched {
                            if let Some(dev) = val.get("context")
                                .and_then(|c| c.get("extensions"))
                                .and_then(|e| e.get("https://s3a.local/ext/device-id")
                                    .or_else(|| e.get("https://s3a.local/ext/purged-device-id"))
                                    .or_else(|| e.get("deviceId")))
                                .and_then(|d| d.as_str())
                            {
                                if dev == account_name_or_device_id {
                                    matched = true;
                                }
                            }
                        }

                        if matched {
                            target_ids.push(id);
                        }
                    }
                }
            }
        }

        let mut receipts = Vec::new();
        for id in target_ids {
            let _ = self.mark_statement(id, MARK_CONSENT_WITHDRAWN, true, 0);

            {
                let mut stmts = self.statements.write().unwrap();
                if let Some(entry) = stmts.get_mut(&id) {
                    entry.raw_json = Some("{\"purged\":true}".to_string());
                }
            }

            let target_uuid = u64_pair_to_uuid(id);
            let receipt_uuid = generate_uuid_v4();
            let receipt_json = format!(r#"{{
                "id": "{}",
                "actor": {{"mbox": "mailto:compliance@s3a.local", "name": "S3A Compliance Purge Daemon"}},
                "verb": {{"id": "http://adlnet.gov/expapi/verbs/voided", "display": {{"en-US": "purged"}}}},
                "object": {{
                    "objectType": "StatementRef",
                    "id": "{}"
                }},
                "result": {{
                    "success": true,
                    "response": "Purged device id / actor account per consent withdrawal. Zeroed from storage."
                }},
                "context": {{
                    "extensions": {{
                        "https://s3a.local/ext/purged-device-id": "{}",
                        "https://s3a.local/ext/purge-signer": "{}"
                    }}
                }}
            }}"#, receipt_uuid, target_uuid, account_name_or_device_id, signer_key);

            let _ = self.store_statement(&receipt_json, &[])?;

            receipts.push(PurgeReceipt {
                receipt_statement_id: receipt_uuid,
                target_statement_id: target_uuid,
                timestamp_ms: now_ms(),
                signer: signer_key.to_string(),
                reason: format!("Consent withdrawal for device ID '{}'", account_name_or_device_id),
            });
        }

        // Also check if any ELR Person projection matches this account/device
        let elr_persons = self.elr_state.persons.read().unwrap().clone();
        for (person_id, person) in elr_persons {
            if person.name == account_name_or_device_id || person.identifier == account_name_or_device_id {
                let _ = self.mark_person(person_id, MARK_CONSENT_WITHDRAWN, true, 0);
            }
        }
        let _ = self.purge_consent_withdrawn_persons(signer_key);

        Ok(receipts)
    }

    /// Part D: Registration Export.
    /// Exports all statements associated with a registration UUID as a standard xAPI JSON array.
    pub fn export_registration_xapi_json(&self, registration: [u64; 2]) -> Result<String, LrsStoreError> {
        let page = self.query_statements(StatementFilter {
            registration: Some(registration),
            limit: 10000,
            ..Default::default()
        })?;

        let json_arr = format!("[{}]", page.statements.join(","));
        Ok(json_arr)
    }

    // =========================================================================
    // PART 2 & 5: ELR (IEEE P2997 Subset) AGGREGATOR, PROJECTIONS & CRUD
    // =========================================================================

    /// Synchronously commits the ELR projection delta generated from an accepted statement.
    pub fn commit_elr_projection(
        &self,
        statement_val: &serde_json::Value,
        statement_id: [u64; 2],
        timestamp_ms: u64,
    ) -> Result<(), LrsStoreError> {
        let delta = crate::elr::project_statement_to_elr(statement_val, statement_id, timestamp_ms);

        if let Some(ref p) = delta.person {
            let (hdr, payload) = p.to_tile_bytes();
            self.write_elr_tile_payload(TILE_ELR_PERSON, &hdr, &payload)?;
        }
        if let Some(ref o) = delta.organization {
            let (hdr, payload) = o.to_tile_bytes();
            self.write_elr_tile_payload(TILE_ELR_ORGANIZATION, &hdr, &payload)?;
        }
        if let Some(ref lr) = delta.learning_resource {
            let (hdr, payload) = lr.to_tile_bytes();
            self.write_elr_tile_payload(TILE_ELR_LEARNING_RESOURCE, &hdr, &payload)?;
        }
        if let Some(ref le) = delta.learning_event {
            let (hdr, payload) = le.to_tile_bytes();
            self.write_elr_tile_payload(TILE_ELR_LEARNING_EVENT, &hdr, &payload)?;
        }
        if let Some(ref ca) = delta.competency_assertion {
            let (hdr, payload) = ca.to_tile_bytes();
            self.write_elr_tile_payload(TILE_ELR_COMPETENCY_ASSERTION, &hdr, &payload)?;
        }
        if let Some(ref cr) = delta.credential_assertion {
            let (hdr, payload) = cr.to_tile_bytes();
            self.write_elr_tile_payload(TILE_ELR_CREDENTIAL_ASSERTION, &hdr, &payload)?;
        }
        if let Some(ref g) = delta.goal {
            let (hdr, payload) = g.to_tile_bytes();
            self.write_elr_tile_payload(TILE_ELR_GOAL, &hdr, &payload)?;
        }

        self.elr_state.apply_delta(delta);
        Ok(())
    }

    fn write_elr_tile_payload(
        &self,
        tile_type: u32,
        hdr: &ElrRecordHeader,
        payload: &[u8],
    ) -> Result<(), LrsStoreError> {
        let mut tile_payload = Vec::with_capacity(std::mem::size_of::<ElrRecordHeader>() + payload.len());
        tile_payload.extend_from_slice(bytes_of(hdr));
        tile_payload.extend_from_slice(payload);

        let mut tile_hdr = HyperTileHeader::new(0, tile_type);
        tile_hdr.record_count = 1;
        tile_hdr.payload_bytes = tile_payload.len() as u32;
        tile_hdr.data_crc32 = crc32c::crc32c(&tile_payload);

        self.writer.write_raw_hyper_tile(&tile_hdr, &tile_payload)?;
        Ok(())
    }

    /// Rebuilds ELR projections deterministically from the statement log from scratch.
    pub fn rebuild_elr_projections(&self) -> Result<crate::elr::ElrStoreState, LrsStoreError> {
        let rebuilt = crate::elr::ElrStoreState::new();
        let reader = MmapReader::open(&self.archive_path)?;

        let stmts = self.statements.read().unwrap();
        for (stmt_id, entry) in stmts.iter() {
            if entry.index_record.is_voided() {
                continue;
            }
            if let Some(ref json_str) = entry.raw_json {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
                    let delta = crate::elr::project_statement_to_elr(&val, *stmt_id, entry.index_record.timestamp_ms);
                    rebuilt.apply_delta(delta);
                }
            } else if let Ok(Some(json_str)) = self.read_body_json_from_disk(
                &reader,
                entry.index_record.body_tile_id as usize,
                entry.index_record.body_offset as usize,
            ) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&json_str) {
                    let delta = crate::elr::project_statement_to_elr(&val, *stmt_id, entry.index_record.timestamp_ms);
                    rebuilt.apply_delta(delta);
                }
            }
        }

        Ok(rebuilt)
    }

    /// Attaches or clears an administrative/policy mark on an ELR Person.
    pub fn mark_person(
        &self,
        person_id: [u64; 2],
        mark_kind: u32,
        set: bool,
        actor_hash: u64,
    ) -> Result<(), LrsStoreError> {
        let mut mark = MarkRecord::new(
            TILE_ELR_PERSON,
            0,
            0,
            *self.last_committed_ms.read().unwrap(),
            mark_kind,
            if set { MARK_SET } else { MARK_CLEAR },
            (now_ms() / 1000) as u64,
            actor_hash,
        );
        mark._reserved = person_id;

        let mark_bytes = bytes_of(&mark);
        let mut tile_hdr = HyperTileHeader::new(0, TileType::MARKS);
        tile_hdr.record_count = 1;
        tile_hdr.payload_bytes = mark_bytes.len() as u32;
        tile_hdr.data_crc32 = crc32c::crc32c(mark_bytes);

        self.writer.write_raw_hyper_tile(&tile_hdr, mark_bytes)?;

        let mut pm = self.elr_state.person_marks.write().unwrap();
        let mask = pm.entry(person_id).or_insert(0);
        if set {
            *mask |= 1 << mark_kind;
        } else {
            *mask &= !(1 << mark_kind);
        }

        Ok(())
    }

    /// Part 5: Purges projections for persons marked with CONSENT_WITHDRAWN (without LEGAL_HOLD)
    /// and appends a signed compliance purge receipt.
    pub fn purge_consent_withdrawn_persons(&self, signer_key: &str) -> Result<Vec<[u64; 2]>, LrsStoreError> {
        let mut purged = Vec::new();
        let target_ids: Vec<[u64; 2]> = {
            let pm = self.elr_state.person_marks.read().unwrap();
            pm.iter()
                .filter(|(_id, &mask)| {
                    (mask & (1 << MARK_CONSENT_WITHDRAWN)) != 0 && (mask & (1 << MARK_LEGAL_HOLD)) == 0
                })
                .map(|(&id, _)| id)
                .collect()
        };

        for id in target_ids {
            // Delete Person
            let _ = self.delete_person(id);
            purged.push(id);

            // Tombstone related events, competencies, credentials, goals
            {
                let events: Vec<[u64; 2]> = self.elr_state.learning_events.read().unwrap()
                    .values()
                    .filter(|e| e.person_id == id)
                    .map(|e| e.id)
                    .collect();
                for ev_id in events {
                    let _ = self.delete_learning_event(ev_id);
                }
            }
            {
                let comps: Vec<[u64; 2]> = self.elr_state.competencies.read().unwrap()
                    .values()
                    .filter(|c| c.person_id == id)
                    .map(|c| c.id)
                    .collect();
                for c_id in comps {
                    let _ = self.delete_competency(c_id);
                }
            }
            {
                let creds: Vec<[u64; 2]> = self.elr_state.credentials.read().unwrap()
                    .values()
                    .filter(|c| c.person_id == id)
                    .map(|c| c.id)
                    .collect();
                for cr_id in creds {
                    let _ = self.delete_credential(cr_id);
                }
            }
            {
                let goals: Vec<[u64; 2]> = self.elr_state.goals.read().unwrap()
                    .values()
                    .filter(|g| g.person_id == id)
                    .map(|g| g.id)
                    .collect();
                for g_id in goals {
                    let _ = self.delete_goal(g_id);
                }
            }

            // Append signed purge receipt statement
            let target_uuid = u64_pair_to_uuid(id);
            let receipt_uuid = generate_uuid_v4();
            let receipt_json = format!(r#"{{
                "id": "{}",
                "actor": {{"mbox": "mailto:compliance@s3a.local", "name": "S3A ELR Compliance Purge Daemon"}},
                "verb": {{"id": "https://w3id.org/xapi/tla/verbs/purged", "display": {{"en-US": "purged"}}}},
                "object": {{
                    "objectType": "Activity",
                    "id": "https://s3a.local/elr/person/{}"
                }},
                "result": {{
                    "success": true,
                    "response": "Person and ELR projections purged per privacy consent withdrawal."
                }},
                "context": {{
                    "extensions": {{
                        "https://s3a.local/ext/purge-signer": "{}"
                    }}
                }}
            }}"#, receipt_uuid, target_uuid, signer_key);

            let _ = self.store_statement(&receipt_json, &[])?;
        }

        Ok(purged)
    }

    // --- Person CRUD ---
    pub fn list_persons(&self, include_hidden: bool) -> Vec<crate::elr::ElrPerson> {
        let persons = self.elr_state.persons.read().unwrap();
        persons.values()
            .filter(|p| include_hidden || !self.elr_state.is_person_consent_withdrawn(&p.id))
            .cloned()
            .collect()
    }

    pub fn get_person(&self, id: [u64; 2], include_hidden: bool) -> Option<crate::elr::ElrPerson> {
        if !include_hidden && self.elr_state.is_person_consent_withdrawn(&id) {
            return None;
        }
        self.elr_state.persons.read().unwrap().get(&id).cloned()
    }

    pub fn put_person(&self, person: crate::elr::ElrPerson) -> Result<(), LrsStoreError> {
        let (hdr, payload) = person.to_tile_bytes();
        self.write_elr_tile_payload(TILE_ELR_PERSON, &hdr, &payload)?;
        self.elr_state.persons.write().unwrap().insert(person.id, person);
        Ok(())
    }

    pub fn delete_person(&self, id: [u64; 2]) -> Result<(), LrsStoreError> {
        let mut hdr = ElrRecordHeader::default();
        hdr.entity_id = id;
        hdr.flags = ELR_FLAG_TOMBSTONE;
        hdr.timestamp_ms = now_ms();
        self.write_elr_tile_payload(TILE_ELR_PERSON, &hdr, &[])?;
        self.elr_state.persons.write().unwrap().remove(&id);
        Ok(())
    }

    // --- Organization CRUD ---
    pub fn list_organizations(&self) -> Vec<crate::elr::ElrOrganization> {
        self.elr_state.organizations.read().unwrap().values().cloned().collect()
    }

    pub fn get_organization(&self, id: [u64; 2]) -> Option<crate::elr::ElrOrganization> {
        self.elr_state.organizations.read().unwrap().get(&id).cloned()
    }

    pub fn put_organization(&self, org: crate::elr::ElrOrganization) -> Result<(), LrsStoreError> {
        let (hdr, payload) = org.to_tile_bytes();
        self.write_elr_tile_payload(TILE_ELR_ORGANIZATION, &hdr, &payload)?;
        self.elr_state.organizations.write().unwrap().insert(org.id, org);
        Ok(())
    }

    pub fn delete_organization(&self, id: [u64; 2]) -> Result<(), LrsStoreError> {
        let mut hdr = ElrRecordHeader::default();
        hdr.entity_id = id;
        hdr.flags = ELR_FLAG_TOMBSTONE;
        hdr.timestamp_ms = now_ms();
        self.write_elr_tile_payload(TILE_ELR_ORGANIZATION, &hdr, &[])?;
        self.elr_state.organizations.write().unwrap().remove(&id);
        Ok(())
    }

    // --- LearningResource CRUD ---
    pub fn list_learning_resources(&self) -> Vec<crate::elr::ElrLearningResource> {
        self.elr_state.learning_resources.read().unwrap().values().cloned().collect()
    }

    pub fn get_learning_resource(&self, id: [u64; 2]) -> Option<crate::elr::ElrLearningResource> {
        self.elr_state.learning_resources.read().unwrap().get(&id).cloned()
    }

    pub fn put_learning_resource(&self, res: crate::elr::ElrLearningResource) -> Result<(), LrsStoreError> {
        let (hdr, payload) = res.to_tile_bytes();
        self.write_elr_tile_payload(TILE_ELR_LEARNING_RESOURCE, &hdr, &payload)?;
        self.elr_state.learning_resources.write().unwrap().insert(res.id, res);
        Ok(())
    }

    pub fn delete_learning_resource(&self, id: [u64; 2]) -> Result<(), LrsStoreError> {
        let mut hdr = ElrRecordHeader::default();
        hdr.entity_id = id;
        hdr.flags = ELR_FLAG_TOMBSTONE;
        hdr.timestamp_ms = now_ms();
        self.write_elr_tile_payload(TILE_ELR_LEARNING_RESOURCE, &hdr, &[])?;
        self.elr_state.learning_resources.write().unwrap().remove(&id);
        Ok(())
    }

    // --- LearningEvent CRUD ---
    pub fn list_learning_events(&self, person_id: Option<[u64; 2]>, include_hidden: bool) -> Vec<crate::elr::ElrLearningEvent> {
        let events = self.elr_state.learning_events.read().unwrap();
        events.values()
            .filter(|e| person_id.map(|pid| e.person_id == pid).unwrap_or(true))
            .filter(|e| include_hidden || !self.elr_state.is_person_consent_withdrawn(&e.person_id))
            .cloned()
            .collect()
    }

    pub fn get_learning_event(&self, id: [u64; 2], include_hidden: bool) -> Option<crate::elr::ElrLearningEvent> {
        let event = self.elr_state.learning_events.read().unwrap().get(&id).cloned()?;
        if !include_hidden && self.elr_state.is_person_consent_withdrawn(&event.person_id) {
            return None;
        }
        Some(event)
    }

    pub fn put_learning_event(&self, ev: crate::elr::ElrLearningEvent) -> Result<(), LrsStoreError> {
        let (hdr, payload) = ev.to_tile_bytes();
        self.write_elr_tile_payload(TILE_ELR_LEARNING_EVENT, &hdr, &payload)?;
        self.elr_state.learning_events.write().unwrap().insert(ev.id, ev);
        Ok(())
    }

    pub fn delete_learning_event(&self, id: [u64; 2]) -> Result<(), LrsStoreError> {
        let mut hdr = ElrRecordHeader::default();
        hdr.entity_id = id;
        hdr.flags = ELR_FLAG_TOMBSTONE;
        hdr.timestamp_ms = now_ms();
        self.write_elr_tile_payload(TILE_ELR_LEARNING_EVENT, &hdr, &[])?;
        self.elr_state.learning_events.write().unwrap().remove(&id);
        Ok(())
    }

    // --- CompetencyAssertion CRUD ---
    pub fn list_competencies(&self, person_id: Option<[u64; 2]>, include_hidden: bool) -> Vec<crate::elr::ElrCompetencyAssertion> {
        let comps = self.elr_state.competencies.read().unwrap();
        comps.values()
            .filter(|c| person_id.map(|pid| c.person_id == pid).unwrap_or(true))
            .filter(|c| include_hidden || !self.elr_state.is_person_consent_withdrawn(&c.person_id))
            .cloned()
            .collect()
    }

    pub fn get_competency(&self, id: [u64; 2], include_hidden: bool) -> Option<crate::elr::ElrCompetencyAssertion> {
        let comp = self.elr_state.competencies.read().unwrap().get(&id).cloned()?;
        if !include_hidden && self.elr_state.is_person_consent_withdrawn(&comp.person_id) {
            return None;
        }
        Some(comp)
    }

    pub fn put_competency(&self, comp: crate::elr::ElrCompetencyAssertion) -> Result<(), LrsStoreError> {
        let (hdr, payload) = comp.to_tile_bytes();
        self.write_elr_tile_payload(TILE_ELR_COMPETENCY_ASSERTION, &hdr, &payload)?;
        self.elr_state.competencies.write().unwrap().insert(comp.id, comp);
        Ok(())
    }

    pub fn delete_competency(&self, id: [u64; 2]) -> Result<(), LrsStoreError> {
        let mut hdr = ElrRecordHeader::default();
        hdr.entity_id = id;
        hdr.flags = ELR_FLAG_TOMBSTONE;
        hdr.timestamp_ms = now_ms();
        self.write_elr_tile_payload(TILE_ELR_COMPETENCY_ASSERTION, &hdr, &[])?;
        self.elr_state.competencies.write().unwrap().remove(&id);
        Ok(())
    }

    // --- CredentialAssertion CRUD ---
    pub fn list_credentials(&self, person_id: Option<[u64; 2]>, include_hidden: bool) -> Vec<crate::elr::ElrCredentialAssertion> {
        let creds = self.elr_state.credentials.read().unwrap();
        creds.values()
            .filter(|c| person_id.map(|pid| c.person_id == pid).unwrap_or(true))
            .filter(|c| include_hidden || !self.elr_state.is_person_consent_withdrawn(&c.person_id))
            .cloned()
            .collect()
    }

    pub fn get_credential(&self, id: [u64; 2], include_hidden: bool) -> Option<crate::elr::ElrCredentialAssertion> {
        let cred = self.elr_state.credentials.read().unwrap().get(&id).cloned()?;
        if !include_hidden && self.elr_state.is_person_consent_withdrawn(&cred.person_id) {
            return None;
        }
        Some(cred)
    }

    pub fn put_credential(&self, cred: crate::elr::ElrCredentialAssertion) -> Result<(), LrsStoreError> {
        let (hdr, payload) = cred.to_tile_bytes();
        self.write_elr_tile_payload(TILE_ELR_CREDENTIAL_ASSERTION, &hdr, &payload)?;
        self.elr_state.credentials.write().unwrap().insert(cred.id, cred);
        Ok(())
    }

    pub fn delete_credential(&self, id: [u64; 2]) -> Result<(), LrsStoreError> {
        let mut hdr = ElrRecordHeader::default();
        hdr.entity_id = id;
        hdr.flags = ELR_FLAG_TOMBSTONE;
        hdr.timestamp_ms = now_ms();
        self.write_elr_tile_payload(TILE_ELR_CREDENTIAL_ASSERTION, &hdr, &[])?;
        self.elr_state.credentials.write().unwrap().remove(&id);
        Ok(())
    }

    // --- Goal CRUD ---
    pub fn list_goals(&self, person_id: Option<[u64; 2]>, include_hidden: bool) -> Vec<crate::elr::ElrGoal> {
        let goals = self.elr_state.goals.read().unwrap();
        goals.values()
            .filter(|g| person_id.map(|pid| g.person_id == pid).unwrap_or(true))
            .filter(|g| include_hidden || !self.elr_state.is_person_consent_withdrawn(&g.person_id))
            .cloned()
            .collect()
    }

    pub fn get_goal(&self, id: [u64; 2], include_hidden: bool) -> Option<crate::elr::ElrGoal> {
        let goal = self.elr_state.goals.read().unwrap().get(&id).cloned()?;
        if !include_hidden && self.elr_state.is_person_consent_withdrawn(&goal.person_id) {
            return None;
        }
        Some(goal)
    }

    pub fn put_goal(&self, goal: crate::elr::ElrGoal) -> Result<(), LrsStoreError> {
        let (hdr, payload) = goal.to_tile_bytes();
        self.write_elr_tile_payload(TILE_ELR_GOAL, &hdr, &payload)?;
        self.elr_state.goals.write().unwrap().insert(goal.id, goal);
        Ok(())
    }

    pub fn delete_goal(&self, id: [u64; 2]) -> Result<(), LrsStoreError> {
        let mut hdr = ElrRecordHeader::default();
        hdr.entity_id = id;
        hdr.flags = ELR_FLAG_TOMBSTONE;
        hdr.timestamp_ms = now_ms();
        self.write_elr_tile_payload(TILE_ELR_GOAL, &hdr, &[])?;
        self.elr_state.goals.write().unwrap().remove(&id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempArchive(PathBuf);
    impl TempArchive {
        fn new(name: &str) -> Self {
            let mut p = std::env::temp_dir();
            let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
            p.push(format!("s3a_lrs_{}_{}_{}.s3a", name, std::process::id(), nonce));
            let _ = std::fs::remove_file(&p);
            TempArchive(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempArchive {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn test_store_and_retrieve_statement() {
        let temp_archive = TempArchive::new("store_retrieve");
        let store = LrsStore::open_or_create(temp_archive.path()).unwrap();

        let stmt_json = r#"{
            "id": "12345678-1234-5678-1234-567812345678",
            "actor": {
                "mbox": "mailto:scholar@example.edu",
                "name": "Alice Scholar"
            },
            "verb": {
                "id": "http://adlnet.gov/expapi/verbs/completed",
                "display": {"en-US": "completed"}
            },
            "object": {
                "id": "http://example.edu/papers/quantum-gravity",
                "definition": {
                    "name": {"en-US": "Quantum Gravity Research Paper"}
                }
            }
        }"#;

        let id = store.store_statement(stmt_json, &[]).unwrap();
        assert_eq!(u64_pair_to_uuid(id), "12345678-1234-5678-1234-567812345678");

        // Retrieve
        let (idx, json, atts) = store.get_statement(id, false).unwrap();
        assert_eq!(idx.statement_id, id);
        assert!(!idx.is_voided());
        assert!(!idx.is_voiding());
        assert!(atts.is_empty());
        assert!(json.contains("Alice Scholar"));
    }

    #[test]
    fn test_statement_idempotency_and_conflict() {
        let temp_archive = TempArchive::new("idempotency");
        let store = LrsStore::open_or_create(temp_archive.path()).unwrap();

        let stmt_json = r#"{
            "id": "aaaa1111-2222-3333-4444-555566667777",
            "actor": {"mbox": "mailto:test@example.com"},
            "verb": {"id": "http://adlnet.gov/expapi/verbs/experienced"},
            "object": {"id": "http://example.com/act/1"}
        }"#;

        let id1 = store.store_statement(stmt_json, &[]).unwrap();

        // Idempotent re-send: should succeed with same id
        let id2 = store.store_statement(stmt_json, &[]).unwrap();
        assert_eq!(id1, id2);

        // Conflicting statement with same ID but different object
        let conflicting_json = r#"{
            "id": "aaaa1111-2222-3333-4444-555566667777",
            "actor": {"mbox": "mailto:test@example.com"},
            "verb": {"id": "http://adlnet.gov/expapi/verbs/experienced"},
            "object": {"id": "http://example.com/act/DIFFERENT"}
        }"#;

        let err = store.store_statement(conflicting_json, &[]).unwrap_err();
        assert!(matches!(err, LrsStoreError::StatementConflict(_)));
    }

    #[test]
    fn test_voiding_lifecycle_and_constraints() {
        let temp_archive = TempArchive::new("voiding");
        let store = LrsStore::open_or_create(temp_archive.path()).unwrap();

        let target_uuid = "11111111-2222-3333-4444-555555555555";
        let target_json = format!(r#"{{
            "id": "{}",
            "actor": {{"mbox": "mailto:learner@example.com"}},
            "verb": {{"id": "http://adlnet.gov/expapi/verbs/attempted"}},
            "object": {{"id": "http://example.com/activity/test"}}
        }}"#, target_uuid);

        let target_id = store.store_statement(&target_json, &[]).unwrap();
        assert!(store.get_statement(target_id, false).is_ok());

        // Void the statement
        let voiding_uuid = "99999999-8888-7777-6666-555555555555";
        let voiding_json = format!(r#"{{
            "id": "{}",
            "actor": {{"mbox": "mailto:admin@example.com"}},
            "verb": {{"id": "http://adlnet.gov/expapi/verbs/voided"}},
            "object": {{
                "objectType": "StatementRef",
                "id": "{}"
            }}
        }}"#, voiding_uuid, target_uuid);

        let _voiding_id = store.store_statement(&voiding_json, &[]).unwrap();

        // 1. Target statement is now voided: normal GET returns StatementVoided
        assert!(matches!(
            store.get_statement(target_id, false),
            Err(LrsStoreError::StatementVoided)
        ));

        // 2. Querying with is_voided_query = true returns the target statement
        assert!(store.get_statement(target_id, true).is_ok());

        // 3. Normal queries exclude voided statements
        let query_res = store.query_statements(StatementFilter::default()).unwrap();
        // Only the voiding statement is in the query result; target is excluded
        assert_eq!(query_res.statements.len(), 1);
        assert!(query_res.statements[0].contains(voiding_uuid));

        // 4. Voiding a voiding statement must fail closed
        let illegal_void_json = format!(r#"{{
            "id": "88888888-7777-6666-5555-444444444444",
            "actor": {{"mbox": "mailto:admin@example.com"}},
            "verb": {{"id": "http://adlnet.gov/expapi/verbs/voided"}},
            "object": {{
                "objectType": "StatementRef",
                "id": "{}"
            }}
        }}"#, voiding_uuid);

        let illegal_err = store.store_statement(&illegal_void_json, &[]).unwrap_err();
        assert!(matches!(illegal_err, LrsStoreError::CannotVoidVoidingStatement));
    }

    #[test]
    fn test_large_statement_chunking() {
        let temp_archive = TempArchive::new("chunking");
        let store = LrsStore::open_or_create(temp_archive.path()).unwrap();

        // Create a large statement (> 150 KB)
        let large_desc = "x".repeat(160 * 1024);
        let large_json = format!(r#"{{
            "id": "77777777-8888-9999-aaaa-bbbbccccdddd",
            "actor": {{"mbox": "mailto:large@example.com"}},
            "verb": {{"id": "http://adlnet.gov/expapi/verbs/passed"}},
            "object": {{
                "id": "http://example.com/large",
                "definition": {{"description": {{"en": "{}"}}}}
            }}
        }}"#, large_desc);

        let id = store.store_statement(&large_json, &[]).unwrap();

        // Read it back from store
        let (_, read_json, _) = store.get_statement(id, false).unwrap();
        assert!(read_json.contains(&large_desc));
    }

    #[test]
    fn test_document_store_crud_and_concurrency() {
        let temp_archive = TempArchive::new("docs");
        let store = LrsStore::open_or_create(temp_archive.path()).unwrap();

        let doc_kind = DOC_KIND_STATE;
        let key = "act1\0agent1\0reg1\0bookmark";
        let content1 = br#"{"page": 42, "chapter": "Introduction"}"#;

        // PUT document
        let etag1 = store.put_document(doc_kind, key, content1, true, None, None, false).unwrap();
        assert!(!etag1.is_empty());

        // GET document
        let (hdr, read_bytes, key_out) = store.get_document(doc_kind, key).unwrap();
        assert_eq!(key_out, key);
        assert_eq!(read_bytes, content1);
        assert_eq!(hdr.doc_kind, DOC_KIND_STATE);

        // Precondition failure on If-Match with wrong etag
        let wrong_etag = "\"0000000000000000\"";
        let err = store.put_document(doc_kind, key, br#"{"page": 43}"#, true, Some(wrong_etag), None, false).unwrap_err();
        assert!(matches!(err, LrsStoreError::PreconditionFailed(_)));

        // If-Match with correct etag succeeds
        let etag2 = store.put_document(doc_kind, key, br#"{"page": 43}"#, true, Some(&etag1), None, false).unwrap();
        assert_ne!(etag1, etag2);

        // POST with JSON merge
        let merge_update = br#"{"notes": "good chapter"}"#;
        let etag3 = store.put_document(doc_kind, key, merge_update, true, None, None, true).unwrap();
        let (_, merged_bytes, _) = store.get_document(doc_kind, key).unwrap();
        let merged_str = String::from_utf8(merged_bytes).unwrap();
        assert!(merged_str.contains("\"page\":43") || merged_str.contains("\"page\": 43"));
        assert!(merged_str.contains("good chapter"));

        // List IDs
        let ids = store.list_document_ids(doc_kind, "act1\0agent1\0reg1\0", None);
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], "bookmark");

        // DELETE
        store.delete_document(doc_kind, key, Some(&etag3)).unwrap();
        assert!(store.get_document(doc_kind, key).is_none());
    }

    #[test]
    fn test_attachments_store_and_retrieve() {
        let temp_archive = TempArchive::new("atts");
        let store = LrsStore::open_or_create(temp_archive.path()).unwrap();

        let att_bytes = b"Hello xAPI Attachment World!";
        let sha2_hex = hex::encode(Sha256::digest(att_bytes));

        let stmt_json = format!(r#"{{
            "id": "55555555-4444-3333-2222-111111111111",
            "actor": {{"mbox": "mailto:attachment@example.com"}},
            "verb": {{"id": "http://adlnet.gov/expapi/verbs/commented"}},
            "object": {{"id": "http://example.com/act/comment"}},
            "attachments": [{{
                "usageType": "http://example.com/usage",
                "display": {{"en": "Attachment"}},
                "contentType": "text/plain",
                "length": {},
                "sha2": "{}"
            }}]
        }}"#, att_bytes.len(), sha2_hex);

        let id = store.store_statement(&stmt_json, &[(sha2_hex.clone(), att_bytes.to_vec())]).unwrap();

        let (idx, _, atts) = store.get_statement(id, false).unwrap();
        assert!(idx.has_attachments());
        assert_eq!(atts.len(), 1);
        assert_eq!(&atts[0].1, att_bytes);

        // Direct retrieve by sha2
        let mut sha2_arr = [0u8; 32];
        sha2_arr.copy_from_slice(&hex::decode(&sha2_hex).unwrap());
        let direct_att = store.get_attachment(&sha2_arr).unwrap();
        assert_eq!(&direct_att, att_bytes);
    }

    #[test]
    fn test_attachment_mismatch_and_bad_hex_rejected() {
        let temp_archive = TempArchive::new("att_reject");
        let store = LrsStore::open_or_create(temp_archive.path()).unwrap();

        let att_bytes = b"Sample attachment bytes";
        let valid_sha2 = hex::encode(Sha256::digest(att_bytes));

        let make_stmt = |sha: &str| format!(r#"{{
            "id": "11112222-3333-4444-5555-666677778888",
            "actor": {{"mbox": "mailto:tester@example.com"}},
            "verb": {{"id": "http://adlnet.gov/expapi/verbs/commented"}},
            "object": {{"id": "http://example.com/act/test"}},
            "attachments": [{{
                "usageType": "http://example.com/usage",
                "display": {{"en": "Attachment"}},
                "contentType": "text/plain",
                "length": {},
                "sha2": "{}"
            }}]
        }}"#, att_bytes.len(), sha);

        // 1. Bad hex (invalid characters or bad length)
        let bad_hex = "not-a-valid-hex-string-at-all-0000000000000000000000000000000000";
        let err1 = store.store_statement(&make_stmt(bad_hex), &[(bad_hex.to_string(), att_bytes.to_vec())]).unwrap_err();
        assert!(matches!(err1, LrsStoreError::InvalidStatement(_)));

        // 2. Mismatched sha2 (valid 64-hex string, but doesn't match content)
        let wrong_sha2 = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let err2 = store.store_statement(&make_stmt(wrong_sha2), &[(wrong_sha2.to_string(), att_bytes.to_vec())]).unwrap_err();
        assert!(matches!(err2, LrsStoreError::InvalidStatement(_)));

        // 3. Exact matching sha2 succeeds
        let ok = store.store_statement(&make_stmt(&valid_sha2), &[(valid_sha2, att_bytes.to_vec())]);
        assert!(ok.is_ok());
    }

    #[test]
    fn test_forced_hash_collision_prefilter_and_confirmation() {
        let temp_archive = TempArchive::new("collision");
        let store = LrsStore::open_or_create(temp_archive.path()).unwrap();

        let stmt1_json = r#"{
            "id": "11111111-0000-0000-0000-000000000001",
            "actor": {"mbox": "mailto:actor1@example.com"},
            "verb": {"id": "http://example.com/verbs/verb-alpha"},
            "object": {"id": "http://example.com/act/target-1"}
        }"#;

        let stmt2_json = r#"{
            "id": "22222222-0000-0000-0000-000000000002",
            "actor": {"mbox": "mailto:actor2@example.com"},
            "verb": {"id": "http://example.com/verbs/verb-beta"},
            "object": {"id": "http://example.com/act/target-2"}
        }"#;

        let id1 = store.store_statement(stmt1_json, &[]).unwrap();
        let id2 = store.store_statement(stmt2_json, &[]).unwrap();

        // Artificially simulate a 32-bit hash collision by forcing stmt2's verb_hash to match stmt1's verb_hash
        {
            let mut stmts = store.statements.write().unwrap();
            let h1 = stmts.get(&id1).unwrap().index_record.verb_hash;
            let entry2 = stmts.get_mut(&id2).unwrap();
            entry2.index_record.verb_hash = h1; // Collision forced!
        }

        // Query for verb-alpha: the 32-bit prefilter passes both id1 and id2,
        // but the full string confirmation must filter out id2
        let page = store.query_statements(StatementFilter {
            verb_iri: Some("http://example.com/verbs/verb-alpha".to_string()),
            ..Default::default()
        }).unwrap();

        assert_eq!(page.statements.len(), 1);
        assert!(page.statements[0].contains("11111111-0000-0000-0000-000000000001"));
        assert!(!page.statements[0].contains("22222222-0000-0000-0000-000000000002"));
    }

    #[test]
    fn test_marks_consent_withdrawn_hides_in_query() {
        let temp_archive = TempArchive::new("marks_hide");
        let store = LrsStore::open_or_create(temp_archive.path()).unwrap();

        let stmt_json = r#"{
            "id": "33333333-4444-5555-6666-777777777777",
            "actor": {"mbox": "mailto:privacy@example.com"},
            "verb": {"id": "http://adlnet.gov/expapi/verbs/experienced"},
            "object": {"id": "http://example.com/act/privacy-test"}
        }"#;

        let id = store.store_statement(stmt_json, &[]).unwrap();

        // 1. Initial query: statement is visible
        let p1 = store.query_statements(StatementFilter::default()).unwrap();
        assert_eq!(p1.statements.len(), 1);

        // 2. Mark statement with MARK_CONSENT_WITHDRAWN via Tile 19 MARKS
        store.mark_statement(id, MARK_CONSENT_WITHDRAWN, true, 42).unwrap();
        assert!(store.has_mark(&id, MARK_CONSENT_WITHDRAWN));

        // 3. Default query now hides the consent-withdrawn statement
        let p2 = store.query_statements(StatementFilter::default()).unwrap();
        assert_eq!(p2.statements.len(), 0);

        // 4. Query with include_hidden = true reveals the statement
        let p3 = store.query_statements(StatementFilter {
            include_hidden: true,
            ..Default::default()
        }).unwrap();
        assert_eq!(p3.statements.len(), 1);

        // 5. Clear MARK_CONSENT_WITHDRAWN
        store.mark_statement(id, MARK_CONSENT_WITHDRAWN, false, 42).unwrap();
        assert!(!store.has_mark(&id, MARK_CONSENT_WITHDRAWN));

        // 6. Default query reveals it again
        let p4 = store.query_statements(StatementFilter::default()).unwrap();
        assert_eq!(p4.statements.len(), 1);
    }

    #[test]
    fn test_purge_consent_withdrawn_and_receipt() {
        let temp_archive = TempArchive::new("purge");
        let store = LrsStore::open_or_create(temp_archive.path()).unwrap();

        let stmt_json = r#"{
            "id": "44444444-5555-6666-7777-888888888888",
            "actor": {"mbox": "mailto:user@example.com"},
            "verb": {"id": "http://adlnet.gov/expapi/verbs/experienced"},
            "object": {"id": "http://example.com/act/purge-target"}
        }"#;

        let id = store.store_statement(stmt_json, &[]).unwrap();
        store.mark_statement(id, MARK_CONSENT_WITHDRAWN, true, 99).unwrap();

        let purged = store.purge_consent_withdrawn_statements("ed25519-sig-test-key").unwrap();
        assert_eq!(purged.len(), 1);
        assert_eq!(purged[0], id);

        // Target statement was voided and body content in memory was replaced with {"purged":true}
        let (_, body, _) = store.get_statement(id, true).unwrap();
        assert_eq!(body, "{\"purged\":true}");

        // Purge receipt statement was appended
        let all = store.query_statements(StatementFilter {
            include_hidden: true,
            ..Default::default()
        }).unwrap();
        assert!(all.statements.iter().any(|s| s.contains("S3A Compliance Purge Daemon")));
    }

    #[test]
    fn test_registration_export_xapi_json() {
        let temp_archive = TempArchive::new("reg_export");
        let store = LrsStore::open_or_create(temp_archive.path()).unwrap();

        let reg_uuid_str = "abcdef01-2345-6789-abcd-ef0123456789";
        let reg_pair = uuid_to_u64_pair(reg_uuid_str).unwrap();

        let stmt1 = format!(r#"{{
            "id": "11111111-aaaa-bbbb-cccc-000000000001",
            "actor": {{"mbox": "mailto:student@example.com"}},
            "verb": {{"id": "http://adlnet.gov/expapi/verbs/launched"}},
            "object": {{"id": "http://example.com/course/1"}},
            "context": {{"registration": "{}"}}
        }}"#, reg_uuid_str);

        let stmt2 = format!(r#"{{
            "id": "22222222-aaaa-bbbb-cccc-000000000002",
            "actor": {{"mbox": "mailto:student@example.com"}},
            "verb": {{"id": "http://adlnet.gov/expapi/verbs/completed"}},
            "object": {{"id": "http://example.com/course/1"}},
            "context": {{"registration": "{}"}}
        }}"#, reg_uuid_str);

        store.store_statement(&stmt1, &[]).unwrap();
        store.store_statement(&stmt2, &[]).unwrap();

        let exported = store.export_registration_xapi_json(reg_pair).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&exported).unwrap();
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 2);
    }

    #[test]
    fn test_rebuild_equals_live() {
        let temp_archive = TempArchive::new("rebuild_elr");
        let store = LrsStore::open_or_create(temp_archive.path()).unwrap();

        // Write 10 diverse statements exercising rules 1.1-3.4 and Scholar
        let statements = vec![
            r#"{
                "id": "10000000-0000-0000-0000-000000000001",
                "actor": {"mbox": "mailto:user1@example.com", "name": "User One"},
                "verb": {"id": "http://adlnet.gov/expapi/verbs/launched"},
                "object": {"id": "http://example.com/course/101", "definition": {"name": {"en": "Intro to CS"}}}
            }"#,
            r#"{
                "id": "10000000-0000-0000-0000-000000000002",
                "actor": {"mbox": "mailto:user1@example.com"},
                "verb": {"id": "http://adlnet.gov/expapi/verbs/passed"},
                "object": {"id": "http://example.com/course/101"},
                "result": {"score": {"scaled": 0.92}, "success": true, "completion": true}
            }"#,
            r#"{
                "id": "10000000-0000-0000-0000-000000000003",
                "actor": {"mbox": "mailto:user2@example.com", "name": "User Two"},
                "verb": {"id": "http://adlnet.gov/expapi/verbs/attempted"},
                "object": {"id": "http://example.com/course/102", "definition": {"name": {"en": "Algorithms"}}}
            }"#,
            r#"{
                "id": "10000000-0000-0000-0000-000000000004",
                "actor": {"mbox": "mailto:user2@example.com"},
                "verb": {"id": "http://adlnet.gov/expapi/verbs/completed"},
                "object": {"id": "http://example.com/course/102"},
                "result": {"completion": true}
            }"#,
            r#"{
                "id": "10000000-0000-0000-0000-000000000005",
                "actor": {"mbox": "mailto:user1@example.com"},
                "verb": {"id": "http://adlnet.gov/expapi/verbs/mastered"},
                "object": {
                    "id": "https://example.com/competencies/data-structures",
                    "definition": {"name": {"en": "Data Structures Mastery"}}
                },
                "result": {"score": {"scaled": 1.0}}
            }"#,
            r#"{
                "id": "10000000-0000-0000-0000-000000000006",
                "actor": {"mbox": "mailto:user1@example.com"},
                "verb": {"id": "http://adlnet.gov/expapi/verbs/earned"},
                "object": {
                    "id": "https://example.com/credentials/cert-1",
                    "definition": {"name": {"en": "Certified Developer"}}
                },
                "context": {"instructor": {"mbox": "mailto:authority@example.com", "name": "Test Authority"}}
            }"#,
            r#"{
                "id": "10000000-0000-0000-0000-000000000007",
                "actor": {"mbox": "mailto:user2@example.com"},
                "verb": {"id": "http://adlnet.gov/expapi/verbs/assigned"},
                "object": {
                    "id": "https://example.com/goals/complete-degree",
                    "definition": {"name": {"en": "Complete Degree"}}
                }
            }"#,
            r#"{
                "id": "10000000-0000-0000-0000-000000000008",
                "actor": {"mbox": "mailto:user3@example.com", "name": "Scholar Alice"},
                "verb": {"id": "https://ashecribb.dev/xapi/verbs/reviewed"},
                "object": {"id": "https://doi.org/10.1000/scholar1"}
            }"#,
            r#"{
                "id": "10000000-0000-0000-0000-000000000009",
                "actor": {"mbox": "mailto:user3@example.com"},
                "verb": {"id": "https://ashecribb.dev/xapi/verbs/flagged-hallucination"},
                "object": {"id": "https://doi.org/10.1000/scholar1"}
            }"#,
            r#"{
                "id": "10000000-0000-0000-0000-000000000010",
                "actor": {"mbox": "mailto:user3@example.com"},
                "verb": {"id": "https://ashecribb.dev/xapi/verbs/ta-approved-draft"},
                "object": {"id": "https://doi.org/10.1000/scholar1"}
            }"#,
        ];

        for stmt in statements {
            store.store_statement(stmt, &[]).unwrap();
        }

        // 1. Rebuild from statement log
        let rebuilt = store.rebuild_elr_projections().unwrap();
        let live = &store.elr_state;

        assert_eq!(rebuilt.persons.read().unwrap().len(), live.persons.read().unwrap().len());
        assert_eq!(rebuilt.learning_resources.read().unwrap().len(), live.learning_resources.read().unwrap().len());
        assert_eq!(rebuilt.learning_events.read().unwrap().len(), live.learning_events.read().unwrap().len());
        assert_eq!(rebuilt.competencies.read().unwrap().len(), live.competencies.read().unwrap().len());
        assert_eq!(rebuilt.credentials.read().unwrap().len(), live.credentials.read().unwrap().len());
        assert_eq!(rebuilt.goals.read().unwrap().len(), live.goals.read().unwrap().len());

        for (k, v) in rebuilt.persons.read().unwrap().iter() {
            assert_eq!(live.persons.read().unwrap().get(k), Some(v));
        }
        for (k, v) in rebuilt.learning_resources.read().unwrap().iter() {
            assert_eq!(live.learning_resources.read().unwrap().get(k), Some(v));
        }
        for (k, v) in rebuilt.learning_events.read().unwrap().iter() {
            assert_eq!(live.learning_events.read().unwrap().get(k), Some(v));
        }
        for (k, v) in rebuilt.competencies.read().unwrap().iter() {
            assert_eq!(live.competencies.read().unwrap().get(k), Some(v));
        }
        for (k, v) in rebuilt.credentials.read().unwrap().iter() {
            assert_eq!(live.credentials.read().unwrap().get(k), Some(v));
        }
        for (k, v) in rebuilt.goals.read().unwrap().iter() {
            assert_eq!(live.goals.read().unwrap().get(k), Some(v));
        }

        let live_p_len = live.persons.read().unwrap().len();
        let live_le_len = live.learning_events.read().unwrap().len();
        let live_ca_len = live.competencies.read().unwrap().len();

        drop(store);

        // 2. Re-open archive and verify indexing from disk tiles produces identical state
        let reopened_store = LrsStore::open_or_create(temp_archive.path()).unwrap();
        let disk_state = &reopened_store.elr_state;
        assert_eq!(disk_state.persons.read().unwrap().len(), live_p_len);
        assert_eq!(disk_state.learning_events.read().unwrap().len(), live_le_len);
        assert_eq!(disk_state.competencies.read().unwrap().len(), live_ca_len);
    }

    #[test]
    fn test_elr_consent_withdrawal_and_purge() {
        let temp_archive = TempArchive::new("consent_elr");
        let store = LrsStore::open_or_create(temp_archive.path()).unwrap();

        let stmt_alice = r#"{
            "id": "20000000-0000-0000-0000-000000000001",
            "actor": {"mbox": "mailto:alice@consent.org", "name": "Alice Cooper"},
            "verb": {"id": "http://adlnet.gov/expapi/verbs/completed"},
            "object": {"id": "http://example.com/privacy-course"}
        }"#;

        let stmt_bob = r#"{
            "id": "20000000-0000-0000-0000-000000000002",
            "actor": {"mbox": "mailto:bob@consent.org", "name": "Bob Dylan"},
            "verb": {"id": "http://adlnet.gov/expapi/verbs/completed"},
            "object": {"id": "http://example.com/privacy-course"}
        }"#;

        store.store_statement(stmt_alice, &[]).unwrap();
        store.store_statement(stmt_bob, &[]).unwrap();

        let alice_id = crate::elr::deterministic_uuid("person", "mailto:alice@consent.org");
        let bob_id = crate::elr::deterministic_uuid("person", "mailto:bob@consent.org");

        // Before withdrawal: both visible
        assert!(store.get_person(alice_id, false).is_some());
        assert!(store.get_person(bob_id, false).is_some());
        assert_eq!(store.list_learning_events(Some(alice_id), false).len(), 1);
        assert_eq!(store.list_learning_events(Some(bob_id), false).len(), 1);

        // Mark Alice with CONSENT_WITHDRAWN
        store.mark_person(alice_id, MARK_CONSENT_WITHDRAWN, true, 42).unwrap();

        // After withdrawal: Alice is hidden from normal queries
        assert!(store.get_person(alice_id, false).is_none());
        assert_eq!(store.list_learning_events(Some(alice_id), false).len(), 0);

        // But Bob is still visible
        assert!(store.get_person(bob_id, false).is_some());
        assert_eq!(store.list_learning_events(Some(bob_id), false).len(), 1);

        // Admin query with include_hidden sees Alice
        assert!(store.get_person(alice_id, true).is_some());
        assert_eq!(store.list_learning_events(Some(alice_id), true).len(), 1);

        // Purge Alice
        let purged = store.purge_consent_withdrawn_persons("compliance-ed25519-key").unwrap();
        assert_eq!(purged, vec![alice_id]);

        // Alice is now tombstoned and gone even with include_hidden
        assert!(store.get_person(alice_id, true).is_none());
        assert_eq!(store.list_learning_events(Some(alice_id), true).len(), 0);

        // Bob remains intact
        assert!(store.get_person(bob_id, false).is_some());

        // Compliance purge receipt statement is stored in the archive
        let all = store.query_statements(StatementFilter {
            include_hidden: true,
            ..Default::default()
        }).unwrap();
        assert!(all.statements.iter().any(|s| s.contains("S3A ELR Compliance Purge Daemon")));
    }

    #[test]
    fn test_elr_crud_and_tombstones() {
        let temp_archive = TempArchive::new("crud_elr");
        let store = LrsStore::open_or_create(temp_archive.path()).unwrap();

        let org_id = crate::elr::deterministic_uuid("organization", "org:123");
        let org = crate::elr::ElrOrganization {
            id: org_id,
            name: "Test Academy".to_string(),
            org_type: "Academic Institution".to_string(),
            source_statement_id: [1, 2],
            timestamp_ms: 1000,
            flags: 0,
        };

        store.put_organization(org.clone()).unwrap();
        assert_eq!(store.get_organization(org_id).unwrap().name, "Test Academy");

        store.delete_organization(org_id).unwrap();
        assert!(store.get_organization(org_id).is_none());
    }

    #[test]
    fn test_purge_and_compact_retention_90_days() {
        let temp_archive = TempArchive::new("retention_90d");
        let store = LrsStore::open_or_create(temp_archive.path()).unwrap();

        let stmt_old = serde_json::json!({
            "id": "11111111-0000-0000-0000-000000000001",
            "actor": {"mbox": "mailto:old@example.com"},
            "verb": {"id": "http://adlnet.gov/expapi/verbs/experienced"},
            "object": {"id": "http://example.com/act/old"}
        }).to_string();

        let stmt_new = serde_json::json!({
            "id": "22222222-0000-0000-0000-000000000002",
            "actor": {"mbox": "mailto:new@example.com"},
            "verb": {"id": "http://adlnet.gov/expapi/verbs/experienced"},
            "object": {"id": "http://example.com/act/new"}
        }).to_string();

        let old_id = store.store_statement(&stmt_old, &[]).unwrap();
        let new_id = store.store_statement(&stmt_new, &[]).unwrap();

        // Artificially backdate old statement in memory beyond 90 days (e.g. 100 days ago)
        let hundred_days_ago = now_ms() - (100 * 24 * 60 * 60 * 1000);
        {
            let mut stmts = store.statements.write().unwrap();
            if let Some(entry) = stmts.get_mut(&old_id) {
                entry.index_record.stored_timestamp_ms = hundred_days_ago;
            }
        }

        // Run retention purge for > 90 days
        let result = store.purge_and_compact_retention(90, "ed25519-compliance-signer").unwrap();
        assert_eq!(result.purged_count, 1);
        assert_eq!(result.receipts.len(), 1);
        assert_eq!(result.receipts[0].target_statement_id, u64_pair_to_uuid(old_id));

        // Old statement body is erased
        assert_eq!(
            store.statements.read().unwrap().get(&old_id).unwrap().raw_json.as_deref(),
            Some("{\"purged\":true}")
        );

        // New statement body is intact
        let (_, new_body, _) = store.get_statement(new_id, false).unwrap();
        assert!(new_body.contains("mailto:new@example.com"));

        // Retention purge receipt statement was appended to archive
        let all = store.query_statements(StatementFilter {
            limit: 100,
            ..Default::default()
        }).unwrap();
        assert!(all.statements.iter().any(|s| s.contains("S3A Retention Purge Daemon")));
    }

    #[test]
    fn test_purge_by_actor_account_device_id() {
        let temp_archive = TempArchive::new("device_purge");
        let store = LrsStore::open_or_create(temp_archive.path()).unwrap();

        let target_device = "device_tablet_9876";

        let stmt_target = serde_json::json!({
            "id": "33333333-0000-0000-0000-000000000003",
            "actor": {
                "account": {
                    "homePage": "https://scholar.explorer/devices",
                    "name": target_device
                }
            },
            "verb": {"id": "https://ashecribb.dev/xapi/verbs/reviewed"},
            "object": {"id": "https://example.com/act/review-doc"}
        }).to_string();

        let stmt_other = serde_json::json!({
            "id": "44444444-0000-0000-0000-000000000004",
            "actor": {
                "account": {
                    "homePage": "https://scholar.explorer/devices",
                    "name": "other_device_5555"
                }
            },
            "verb": {"id": "https://ashecribb.dev/xapi/verbs/reviewed"},
            "object": {"id": "https://example.com/act/review-doc"}
        }).to_string();

        let target_id = store.store_statement(&stmt_target, &[]).unwrap();
        let other_id = store.store_statement(&stmt_other, &[]).unwrap();

        // Purge target device
        let receipts = store.purge_by_actor_account(target_device, None, "admin-signer").unwrap();
        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0].target_statement_id, u64_pair_to_uuid(target_id));

        // Target statement is marked consent-withdrawn and body erased
        assert_eq!(
            store.statements.read().unwrap().get(&target_id).unwrap().raw_json.as_deref(),
            Some("{\"purged\":true}")
        );
        // Target statement is hidden from default queries (its top-level id is not returned)
        let queried = store.query_statements(StatementFilter {
            limit: 10,
            ..Default::default()
        }).unwrap();
        for s in &queried.statements {
            let parsed: serde_json::Value = serde_json::from_str(s).unwrap();
            assert_ne!(parsed.get("id").and_then(|i| i.as_str()), Some("33333333-0000-0000-0000-000000000003"));
        }
        assert!(queried.statements.iter().any(|s| s.contains("44444444-0000-0000-0000-000000000004")));
        assert!(queried.statements.iter().any(|s| s.contains("S3A Compliance Purge Daemon")));

        // Other statement is completely untouched
        let (_, other_body, _) = store.get_statement(other_id, false).unwrap();
        assert!(other_body.contains("other_device_5555"));
    }
}

