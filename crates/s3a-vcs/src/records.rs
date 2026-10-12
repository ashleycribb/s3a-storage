//! Binary record definitions for S3A Git-Style Version Control (S3A-VCS).

use bytemuck::{Pod, Zeroable};
use sha2::{Digest, Sha256};
use ed25519_dalek::{Signature, VerifyingKey, Verifier};

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
        let msg_len = self.message_bytes.iter().position(|&b| b == 0).unwrap_or(VCS_COMMIT_MESSAGE_MAX);
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
        let len = self.message_bytes.iter().position(|&b| b == 0).unwrap_or(VCS_COMMIT_MESSAGE_MAX);
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
    pub ref_kind: u16,                   // 1 = Branch (mutable), 2 = Tag (immutable)
    pub _pad: [u8; 6],                   // Alignment padding
    pub target_seal_hash: [u8; 32],      // Seal hash pointed to by this ref
    pub author_pubkey: [u8; 32],         // Creator/updater public key
    pub updated_timestamp: u64,          // Unix epoch timestamp of ref update
    pub signature: [u8; 64],             // Ed25519 signature over ref envelope
    pub name: [u8; VCS_REF_NAME_MAX],    // UTF-8 ref name (e.g. "main", "v1.0.0")
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
        let name_len = self.name.iter().position(|&b| b == 0).unwrap_or(VCS_REF_NAME_MAX);
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
        let len = self.name.iter().position(|&b| b == 0).unwrap_or(VCS_REF_NAME_MAX);
        std::str::from_utf8(&self.name[..len]).unwrap_or("")
    }
}
