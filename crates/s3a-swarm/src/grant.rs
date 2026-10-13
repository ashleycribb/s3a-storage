//! Scoped delegation grants and governance enforcement for simulated agents.

use std::sync::Mutex;
use ed25519_dalek::{SigningKey, VerifyingKey, Signature, Signer, Verifier};
use s3a_vault::records::{DelegationGrantRecord, ACTION_WRITE, ACTION_READ};

/// Rejection reason when a commit violates delegation grants.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GrantRejectionReason {
    InvalidGranterSignature,
    ExpiredGrant,
    MaxOperationsExhausted,
    ActionNotPermitted(u32),
    UnauthorizedPath {
        path: String,
        allowed_prefix: String,
    },
}

impl std::fmt::Display for GrantRejectionReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidGranterSignature => write!(f, "Invalid cryptographic granter signature"),
            Self::ExpiredGrant => write!(f, "Delegation grant has expired"),
            Self::MaxOperationsExhausted => write!(f, "Delegation grant operation count exhausted"),
            Self::ActionNotPermitted(act) => write!(f, "Requested action {:#x} not permitted", act),
            Self::UnauthorizedPath { path, allowed_prefix } => {
                write!(f, "Path '{}' outside allowed grant prefix '{}'", path, allowed_prefix)
            }
        }
    }
}

/// Audit log record for rejected commit attempts.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GrantRejectionEvent {
    pub agent_id: String,
    pub gate_address: String,
    pub reason: GrantRejectionReason,
    pub timestamp: u64,
    pub attempted_paths: Vec<String>,
}

/// Thread-safe in-memory audit log of grant rejections.
pub static REJECTION_LOG: Mutex<Vec<GrantRejectionEvent>> = Mutex::new(Vec::new());

/// Issues a valid signed delegation grant from root authority to an agent.
pub fn issue_agent_grant(
    root_key: &SigningKey,
    agent_id: [u8; 16],
    agent_pubkey: [u8; 32],
    max_operations: u32,
    scope_mask: u32,
    ttl_secs: u64,
) -> DelegationGrantRecord {
    let granter_pubkey = root_key.verifying_key().to_bytes();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let expires_at = now + ttl_secs;

    let mut grant_id = [0u8; 16];
    let mut h = sha2::Sha256::default();
    use sha2::Digest;
    h.update(b"S3A_AGENT_GRANT");
    h.update(&agent_id);
    h.update(&now.to_le_bytes());
    grant_id.copy_from_slice(&h.finalize()[..16]);

    let mut record = DelegationGrantRecord {
        grant_id,
        granter_pubkey,
        agent_id,
        agent_pubkey,
        allowed_actions: ACTION_WRITE | ACTION_READ,
        allowed_scopes: scope_mask,
        max_operations,
        _pad: [0u8; 4],
        issued_at: now.to_le_bytes(),
        expires_at: expires_at.to_le_bytes(),
        granter_signature: [0u8; 64],
    };

    let sig: Signature = root_key.sign(record.signed_bytes());
    record.granter_signature = sig.to_bytes();
    record
}

/// Validates whether a commit operation is permitted under an agent's grant.
pub fn validate_commit_grant(
    grant: &DelegationGrantRecord,
    allowed_prefix: &str,
    paths: &[String],
    current_time: u64,
    operation_index: u32,
) -> Result<(), GrantRejectionReason> {
    // 1. Verify granter signature
    let verifying_key = VerifyingKey::from_bytes(&grant.granter_pubkey)
        .map_err(|_| GrantRejectionReason::InvalidGranterSignature)?;
    let Ok(sig) = Signature::from_slice(&grant.granter_signature) else {
        return Err(GrantRejectionReason::InvalidGranterSignature);
    };
    verifying_key
        .verify(grant.signed_bytes(), &sig)
        .map_err(|_| GrantRejectionReason::InvalidGranterSignature)?;

    // 2. Check expiration
    if current_time > grant.get_expires_at() {
        return Err(GrantRejectionReason::ExpiredGrant);
    }

    // 3. Check operation limit
    if operation_index >= grant.max_operations {
        return Err(GrantRejectionReason::MaxOperationsExhausted);
    }

    // 4. Check action mask (must have ACTION_WRITE)
    if (grant.allowed_actions & ACTION_WRITE) == 0 {
        return Err(GrantRejectionReason::ActionNotPermitted(ACTION_WRITE));
    }

    // 5. Scoped path boundaries check (fail-safe default)
    for path in paths {
        if !path.starts_with(allowed_prefix) {
            return Err(GrantRejectionReason::UnauthorizedPath {
                path: path.clone(),
                allowed_prefix: allowed_prefix.to_string(),
            });
        }
    }

    Ok(())
}

/// Logs a grant rejection event to global audit history.
pub fn record_grant_rejection(
    agent_id: &str,
    gate_address: &str,
    reason: GrantRejectionReason,
    attempted_paths: Vec<String>,
) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut log = REJECTION_LOG.lock().unwrap();
    log.push(GrantRejectionEvent {
        agent_id: agent_id.to_string(),
        gate_address: gate_address.to_string(),
        reason,
        timestamp: now,
        attempted_paths,
    });
}
