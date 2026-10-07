use bytemuck::{Pod, Zeroable};

/// DECENTRALIZED DATA AVAILABILITY (DA) & BLOCKCHAIN INDEXER RECORD (64 bytes, 8-byte aligned).
/// Designed for L2 Rollups, State Channels, and Decentralized AI Model Weight Commitment Verification.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Default)]
pub struct DaCommitmentRecord {
    pub block_height: u64,          // Block height / L2 slot index
    pub state_root_hash: [u8; 32],   // 256-bit State Root or Polynomial KZG commitment
    pub timestamp_sec: u32,         // Block / Slot timestamp
    pub quorum_bitmask: u32,        // Validator availability quorum signoff bitmask
    pub transaction_count: u32,     // Number of bundled L2 / AI transactions
    pub _reserved: [u32; 3],
}

pub type DACommitmentRecord = DaCommitmentRecord;

impl DaCommitmentRecord {
    pub fn new(
        block_height: u64,
        state_root_hash: [u8; 32],
        timestamp_sec: u32,
        quorum_bitmask: u32,
        transaction_count: u32,
    ) -> Self {
        Self {
            block_height,
            state_root_hash,
            timestamp_sec,
            quorum_bitmask,
            transaction_count,
            _reserved: [0u32; 3],
        }
    }
}
