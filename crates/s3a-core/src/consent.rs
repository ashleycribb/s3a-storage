use bytemuck::{Pod, Zeroable};
use core::fmt;
use sha2::{Digest, Sha256};

/// Declarative research participant consent purpose enum.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ConsentPurpose {
    Research = 1,
    Analytics = 2,
    ModelTraining = 3,
    Sharing = 4,
}

impl ConsentPurpose {
    pub const fn from_u32(val: u32) -> Option<Self> {
        match val {
            1 => Some(Self::Research),
            2 => Some(Self::Analytics),
            3 => Some(Self::ModelTraining),
            4 => Some(Self::Sharing),
            _ => None,
        }
    }

    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Research => "research",
            Self::Analytics => "analytics",
            Self::ModelTraining => "model_training",
            Self::Sharing => "sharing",
        }
    }
}

impl fmt::Display for ConsentPurpose {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// SUBJECT CONSENT & GOVERNANCE RECORD (136 bytes, 8-byte aligned, Pod/Zeroable).
/// Represents an immutable consent event (grant or withdrawal) for a research participant.
/// Append-only semantics: the latest record per (subject_id, study_id, purpose) wins.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct ConsentRecord {
    pub subject_id: [u8; 16],        // 128-bit UUID (offset 0..16)
    pub study_id: u64,               // Study or protocol ID (offset 16..24)
    pub purpose: u32,                // ConsentPurpose enum (offset 24..28)
    pub scope: u32, // Tile type / record kind scope (0 = all / wildcard) (offset 28..32)
    pub granted_at: u64, // Timestamp granted (Unix epoch) (offset 32..40)
    pub expires_at: u64, // Expiration timestamp (0 = no expiry) (offset 40..48)
    pub withdrawn_at: u64, // Timestamp withdrawn (0 = active) (offset 48..56)
    pub legal_hold: u8, // 1 = active legal hold, 0 = standard (offset 56..57)
    pub _reserved: [u8; 7], // Alignment padding (offset 57..64)
    pub granted_by: u64, // Actor ID of authorized human/admin (offset 64..72)
    pub prev_consent_hash: [u8; 32], // Cryptographic chain hash per subject (offset 72..104)
    pub consent_hash: [u8; 32], // SHA-256 digest over preceding 104 bytes (offset 104..136)
}

impl ConsentRecord {
    pub const SIZE: usize = 136;

    pub fn new(
        subject_id: [u8; 16],
        study_id: u64,
        purpose: ConsentPurpose,
        scope: u32,
        granted_at: u64,
        expires_at: u64,
        legal_hold: bool,
        granted_by: u64,
        prev_consent_hash: [u8; 32],
    ) -> Self {
        let mut rec = Self {
            subject_id,
            study_id,
            purpose: purpose as u32,
            scope,
            granted_at,
            expires_at,
            withdrawn_at: 0,
            legal_hold: if legal_hold { 1 } else { 0 },
            _reserved: [0u8; 7],
            granted_by,
            prev_consent_hash,
            consent_hash: [0u8; 32],
        };
        rec.consent_hash = rec.compute_hash();
        rec
    }

    pub fn new_withdrawal(
        subject_id: [u8; 16],
        study_id: u64,
        purpose: ConsentPurpose,
        scope: u32,
        withdrawn_at: u64,
        granted_by: u64,
        prev_consent_hash: [u8; 32],
    ) -> Self {
        let mut rec = Self {
            subject_id,
            study_id,
            purpose: purpose as u32,
            scope,
            granted_at: 0,
            expires_at: 0,
            withdrawn_at,
            legal_hold: 0,
            _reserved: [0u8; 7],
            granted_by,
            prev_consent_hash,
            consent_hash: [0u8; 32],
        };
        rec.consent_hash = rec.compute_hash();
        rec
    }

    pub fn compute_hash(&self) -> [u8; 32] {
        let bytes = bytemuck::bytes_of(self);
        let mut hasher = Sha256::new();
        hasher.update(&bytes[..104]);
        hasher.finalize().into()
    }

    pub fn verify_hash(&self) -> bool {
        self.consent_hash == self.compute_hash()
    }

    pub fn is_active(&self, now: u64) -> bool {
        if self.withdrawn_at > 0 {
            return false;
        }
        if self.expires_at > 0 && now > self.expires_at {
            return false;
        }
        true
    }

    pub fn is_legal_hold(&self) -> bool {
        self.legal_hold != 0
    }

    pub fn parsed_purpose(&self) -> Option<ConsentPurpose> {
        ConsentPurpose::from_u32(self.purpose)
    }
}

/// Helper to convert a 64-bit actor ID to a 128-bit subject UUID slice.
pub fn actor_id_to_subject_id(actor_id: u64) -> [u8; 16] {
    let mut out = [0u8; 16];
    out[0..8].copy_from_slice(&actor_id.to_le_bytes());
    out
}
