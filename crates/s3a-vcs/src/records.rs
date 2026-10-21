//! Binary record definitions for S3A Git-Style Version Control (S3A-VCS).

use bytemuck::{Pod, Zeroable};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

/// Maximum message length stored inline in commit record (bytes).
pub const VCS_COMMIT_MESSAGE_MAX: usize = 256;

/// Version Control Commit Record (480 bytes, 8-byte aligned).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct VcsCommitRecord {
    pub commit_id: [u8; 32],               // SHA-256 digest of commit envelope
    pub seal_hash: [u8; 32],               // Authoritative sealed footer v2 seal hash
    pub author_pubkey: [u8; 32],           // Ed25519 public key of author
    pub author_address_prefix: [u8; 8],    // GateAddress prefix
    pub author_checksum: u8,               // GateAddress checksum
    pub parent_count: u8,                  // Number of parent commits (0=root, 1=linear, 2=merge)
    pub _pad: [u8; 6],                     // Alignment padding
    pub parent_seal_hashes: [[u8; 32]; 2], // Primary and secondary parent seal hashes
    pub committed_timestamp: u64,          // Unix epoch timestamp (seconds)
    pub signature: [u8; 64],               // Ed25519 signature over commit envelope
    pub message_bytes: [u8; VCS_COMMIT_MESSAGE_MAX], // UTF-8 commit message (null-padded)
}

impl VcsCommitRecord {
    /// Computes the envelope bytes that are signed by the author and hashed into `commit_id`.
    pub fn compute_envelope_bytes(
        seal_hash: &[u8; 32],
        parent_count: u8,
        parent_seal_hashes: &[[u8; 32]; 2],
        committed_timestamp: u64,
        message: &str,
    ) -> Vec<u8> {
        let mut env = Vec::with_capacity(384);
        env.extend_from_slice(b"S3ACOMMIT1");
        env.extend_from_slice(seal_hash);
        env.push(parent_count);
        env.extend_from_slice(&parent_seal_hashes[0]);
        env.extend_from_slice(&parent_seal_hashes[1]);
        env.extend_from_slice(&committed_timestamp.to_le_bytes());
        let msg_bytes = message.as_bytes();
        let len = msg_bytes.len().min(VCS_COMMIT_MESSAGE_MAX);
        env.extend_from_slice(&msg_bytes[..len]);
        env
    }

    /// Computes the authoritative `commit_id` digest.
    pub fn compute_commit_id(envelope: &[u8]) -> [u8; 32] {
        Sha256::digest(envelope).into()
    }

    /// Verifies the cryptographic signature of the commit record.
    pub fn verify_signature(&self) -> bool {
        let msg_len = self
            .message_bytes
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(VCS_COMMIT_MESSAGE_MAX);
        let msg_str = match std::str::from_utf8(&self.message_bytes[..msg_len]) {
            Ok(s) => s,
            Err(_) => return false,
        };

        let envelope = Self::compute_envelope_bytes(
            &self.seal_hash,
            self.parent_count,
            &self.parent_seal_hashes,
            self.committed_timestamp,
            msg_str,
        );

        let computed_id = Self::compute_commit_id(&envelope);
        if computed_id != self.commit_id {
            return false;
        }

        let Ok(verifying_key) = VerifyingKey::from_bytes(&self.author_pubkey) else {
            return false;
        };
        let Ok(signature) = Signature::from_slice(&self.signature) else {
            return false;
        };

        verifying_key.verify(&envelope, &signature).is_ok()
    }

    /// Extracts the human-readable commit message.
    pub fn message(&self) -> &str {
        let len = self
            .message_bytes
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(VCS_COMMIT_MESSAGE_MAX);
        std::str::from_utf8(&self.message_bytes[..len]).unwrap_or("")
    }
}

pub const REF_KIND_BRANCH: u16 = 1;
pub const REF_KIND_TAG: u16 = 2;
pub const VCS_REF_NAME_MAX: usize = 64;

/// Version Control Reference Record (184 bytes, 8-byte aligned).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct VcsRefRecord {
    pub ref_kind: u16,                // 1 = Branch (mutable), 2 = Tag (immutable)
    pub _pad: [u8; 6],                // Alignment padding
    pub target_seal_hash: [u8; 32],   // Seal hash pointed to by this ref
    pub author_pubkey: [u8; 32],      // Creator/updater public key
    pub updated_timestamp: u64,       // Unix epoch timestamp of ref update
    pub signature: [u8; 64],          // Ed25519 signature over ref envelope
    pub name: [u8; VCS_REF_NAME_MAX], // UTF-8 ref name (e.g. "main", "v1.0.0")
}

impl VcsRefRecord {
    pub fn compute_envelope_bytes(
        ref_kind: u16,
        name: &str,
        target_seal_hash: &[u8; 32],
        updated_timestamp: u64,
    ) -> Vec<u8> {
        let mut env = Vec::with_capacity(128);
        env.extend_from_slice(b"S3AREF01");
        env.extend_from_slice(&ref_kind.to_le_bytes());
        let name_bytes = name.as_bytes();
        let len = name_bytes.len().min(VCS_REF_NAME_MAX);
        env.extend_from_slice(&name_bytes[..len]);
        env.extend_from_slice(target_seal_hash);
        env.extend_from_slice(&updated_timestamp.to_le_bytes());
        env
    }

    pub fn verify_signature(&self) -> bool {
        let name_len = self
            .name
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(VCS_REF_NAME_MAX);
        let name_str = match std::str::from_utf8(&self.name[..name_len]) {
            Ok(s) => s,
            Err(_) => return false,
        };

        let env = Self::compute_envelope_bytes(
            self.ref_kind,
            name_str,
            &self.target_seal_hash,
            self.updated_timestamp,
        );

        let Ok(verifying_key) = VerifyingKey::from_bytes(&self.author_pubkey) else {
            return false;
        };
        let Ok(signature) = Signature::from_slice(&self.signature) else {
            return false;
        };

        verifying_key.verify(&env, &signature).is_ok()
    }

    pub fn ref_name(&self) -> &str {
        let len = self
            .name
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(VCS_REF_NAME_MAX);
        std::str::from_utf8(&self.name[..len]).unwrap_or("")
    }
}

pub const RESOLUTION_CHOICE_TARGET: u8 = 1;
pub const RESOLUTION_CHOICE_SOURCE: u8 = 2;
pub const RESOLUTION_CHOICE_SYNTHESIS: u8 = 3;

pub const VCS_RECORD_ID_MAX: usize = 64;
pub const VCS_RATIONALE_MAX: usize = 128;

/// Signed Human Resolution Record for VCS Merge Conflicts (352 bytes, 8-byte aligned).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct VcsResolutionRecord {
    pub resolution_id: [u8; 32],   // SHA-256 digest of resolution envelope
    pub resolver_pubkey: [u8; 32], // Human supervisor Ed25519 public key
    pub chosen_side: u8,           // 1 = Target, 2 = Source, 3 = Synthesis
    pub _pad: [u8; 7],
    pub resolved_timestamp: u64, // Unix epoch timestamp (seconds)
    pub signature: [u8; 64],     // Ed25519 signature over envelope
    pub record_id: [u8; VCS_RECORD_ID_MAX], // UTF-8 conflict record ID (null-padded)
    pub rationale: [u8; VCS_RATIONALE_MAX], // UTF-8 supervisor rationale (null-padded)
    pub _reserved: [u8; 16],
}

impl VcsResolutionRecord {
    pub fn compute_envelope_bytes(
        record_id: &str,
        chosen_side: u8,
        resolved_timestamp: u64,
        rationale: &str,
    ) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(256);
        bytes.extend_from_slice(b"S3AVCS_RES1");
        bytes.push(chosen_side);
        bytes.extend_from_slice(&resolved_timestamp.to_le_bytes());
        bytes.extend_from_slice(record_id.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(rationale.as_bytes());
        bytes
    }

    pub fn new_signed(
        record_id: &str,
        chosen_side: u8,
        rationale: &str,
        signing_key: &ed25519_dalek::SigningKey,
    ) -> Self {
        use ed25519_dalek::Signer;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let env = Self::compute_envelope_bytes(record_id, chosen_side, now, rationale);
        let mut hasher = Sha256::new();
        hasher.update(&env);
        let res_id: [u8; 32] = hasher.finalize().into();

        let sig: Signature = signing_key.sign(&env);
        let pubkey = signing_key.verifying_key().to_bytes();

        let mut rec_id_bytes = [0u8; VCS_RECORD_ID_MAX];
        let rb = record_id.as_bytes();
        let r_len = rb.len().min(VCS_RECORD_ID_MAX);
        rec_id_bytes[..r_len].copy_from_slice(&rb[..r_len]);

        let mut rat_bytes = [0u8; VCS_RATIONALE_MAX];
        let rat = rationale.as_bytes();
        let rat_len = rat.len().min(VCS_RATIONALE_MAX);
        rat_bytes[..rat_len].copy_from_slice(&rat[..rat_len]);

        Self {
            resolution_id: res_id,
            resolver_pubkey: pubkey,
            chosen_side,
            _pad: [0u8; 7],
            resolved_timestamp: now,
            signature: sig.to_bytes(),
            record_id: rec_id_bytes,
            rationale: rat_bytes,
            _reserved: [0u8; 16],
        }
    }

    pub fn conflict_record_id(&self) -> String {
        let end = self
            .record_id
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(VCS_RECORD_ID_MAX);
        String::from_utf8_lossy(&self.record_id[..end]).to_string()
    }

    pub fn rationale_text(&self) -> String {
        let end = self
            .rationale
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(VCS_RATIONALE_MAX);
        String::from_utf8_lossy(&self.rationale[..end]).to_string()
    }

    pub fn verify_signature(&self) -> bool {
        let Ok(vk) = VerifyingKey::from_bytes(&self.resolver_pubkey) else {
            return false;
        };
        let Ok(sig) = Signature::from_slice(&self.signature) else {
            return false;
        };
        let rec_id = self.conflict_record_id();
        let rat = self.rationale_text();
        let env =
            Self::compute_envelope_bytes(&rec_id, self.chosen_side, self.resolved_timestamp, &rat);
        vk.verify(&env, &sig).is_ok()
    }
}

/// Detailed information regarding a conflicting record during merge.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ConflictRecordDetail {
    pub record_id: String,
    pub tile_type: u32,
    pub type_name: String,
    pub target_version: String,
    pub source_version: String,
    pub requires_human_judgment: bool,
}

/// Result of an S3A VCS merge operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VcsMergeResult {
    Merged(VcsCommitRecord),
    ConflictNeedsHuman {
        conflicting_records: Vec<ConflictRecordDetail>,
        message: String,
    },
    Conflict {
        conflicting_record_ids: Vec<String>,
        message: String,
    },
}

impl VcsMergeResult {
    pub fn is_merged(&self) -> bool {
        matches!(self, VcsMergeResult::Merged(_))
    }

    pub fn commit(&self) -> Option<&VcsCommitRecord> {
        match self {
            VcsMergeResult::Merged(c) => Some(c),
            _ => None,
        }
    }
}

impl std::ops::Deref for VcsMergeResult {
    type Target = VcsCommitRecord;
    fn deref(&self) -> &Self::Target {
        match self {
            VcsMergeResult::Merged(c) => c,
            VcsMergeResult::ConflictNeedsHuman { message, .. } => {
                panic!(
                    "Merge did not produce a commit (requires human judgment): {}",
                    message
                )
            }
            VcsMergeResult::Conflict { message, .. } => {
                panic!("Merge did not produce a commit (conflict): {}", message)
            }
        }
    }
}
