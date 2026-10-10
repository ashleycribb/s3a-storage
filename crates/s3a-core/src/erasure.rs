//! Right-to-be-Forgotten Erasure Tile Record Layout and Operations
//!
//! Provides verifiable cryptographic audit receipts proving permanent data destruction
//! under GDPR Article 17, HIPAA, or IRB mandates.

use bytemuck::{Pod, Zeroable};
use sha2::{Digest, Sha256};

/// Regulatory / compliance reason for data erasure.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErasureReason {
    Withdrawal = 1,
    RetentionExpiry = 2,
    LegalRequest = 3,
    Correction = 4,
}

impl ErasureReason {
    pub fn from_u32(val: u32) -> Option<Self> {
        match val {
            1 => Some(ErasureReason::Withdrawal),
            2 => Some(ErasureReason::RetentionExpiry),
            3 => Some(ErasureReason::LegalRequest),
            4 => Some(ErasureReason::Correction),
            _ => None,
        }
    }
}

/// Cryptographic audit receipt for permanent record erasure (144 bytes, 8-byte aligned, Pod/Zeroable).
///
/// Documents permanent destruction of a target record without retaining any of the erased data.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct ErasureRecord {
    pub erased_record_hash: [u8; 32], // SHA-256 digest of original record (offset 0..32)
    pub target_tile_type: u32,       // Tile type of target (offset 32..36)
    pub record_index: u32,           // Record index within target tile (offset 36..40)
    pub target_tile_id: u64,         // Tile ID in archive (offset 40..48)
    pub reason: u32,                 // ErasureReason enum (offset 48..52)
    pub _reserved_u32: u32,          // Alignment padding (offset 52..56)
    pub authority: u64,              // Actor ID of authorized human/admin (offset 56..64)
    pub requested_at: u64,           // Unix epoch timestamp requested (offset 64..72)
    pub executed_at: u64,            // Unix epoch timestamp executed (offset 72..80)
    pub prev_erasure_hash: [u8; 32], // Cryptographic chain hash (offset 80..112)
    pub erasure_hash: [u8; 32],      // SHA-256 digest over preceding 112 bytes (offset 112..144)
}

impl ErasureRecord {
    pub const SIZE: usize = 144;

    pub fn new(
        erased_record_hash: [u8; 32],
        target_tile_type: u32,
        target_tile_id: u64,
        record_index: u32,
        reason: ErasureReason,
        authority: u64,
        requested_at: u64,
        executed_at: u64,
        prev_erasure_hash: [u8; 32],
    ) -> Self {
        let mut rec = Self {
            erased_record_hash,
            target_tile_type,
            record_index,
            target_tile_id,
            reason: reason as u32,
            _reserved_u32: 0,
            authority,
            requested_at,
            executed_at,
            prev_erasure_hash,
            erasure_hash: [0u8; 32],
        };
        rec.erasure_hash = rec.compute_hash();
        rec
    }

    /// Computes SHA-256 cryptographic digest over the first 112 bytes of the record.
    pub fn compute_hash(&self) -> [u8; 32] {
        let bytes = bytemuck::bytes_of(self);
        let mut hasher = Sha256::new();
        hasher.update(&bytes[..112]);
        hasher.finalize().into()
    }

    /// Verifies that `erasure_hash` matches the SHA-256 digest of record metadata.
    pub fn verify_hash(&self) -> bool {
        self.erasure_hash == self.compute_hash()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_erasure_record_size_and_alignment() {
        assert_eq!(core::mem::size_of::<ErasureRecord>(), 144);
        assert_eq!(core::mem::align_of::<ErasureRecord>(), 8);
    }

    #[test]
    fn test_erasure_record_hash_verification() {
        let rec = ErasureRecord::new(
            [0xAA; 32],
            0,
            1,
            0,
            ErasureReason::Withdrawal,
            999,
            1000,
            1050,
            [0u8; 32],
        );
        assert!(rec.verify_hash());

        let mut tampered = rec;
        tampered.authority = 1234;
        assert!(!tampered.verify_hash());
    }
}
