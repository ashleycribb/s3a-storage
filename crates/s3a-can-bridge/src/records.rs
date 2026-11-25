//! Fixed-layout CAN bridge records.
//!
//! Every record starts with a 64-byte provenance prefix: `source_id` (16),
//! `content_hash` (32), `origin_ptr` (16), then a 32-byte `log_hash` and an
//! 8-byte `frame_offset`. Layout version 1. Padding bytes must be zero.

use bytemuck::{Pod, Zeroable};

/// J1939 DM1 diagnostic trouble code.
pub const DTC_KIND_J1939_DM1: u8 = 1;
/// OBD-II Mode 03 diagnostic trouble code.
pub const DTC_KIND_OBD_MODE03: u8 = 2;

/// Trigger kind: J1939 SPN/FMI mapping.
pub const TRIGGER_KIND_J1939: u8 = 1;
/// Trigger kind: OBD-II DTC mapping.
pub const TRIGGER_KIND_OBD: u8 = 2;
/// Trigger kind: out-of-range parameter mapping.
pub const TRIGGER_KIND_PARAM: u8 = 3;

/// Decoded diagnostic trouble code (tile 47, `can_dtc`, 128 B).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq, Default)]
pub struct CanDtcRecord {
    pub source_id: [u8; 16],
    pub content_hash: [u8; 32],
    pub origin_ptr: [u8; 16],
    pub log_hash: [u8; 32],
    pub frame_offset: u64,
    pub can_id: u32,
    pub pgn: u32,
    pub spn: u32,
    pub fmi: u8,
    pub occurrence_count: u8,
    pub conversion_method: u8,
    pub kind: u8,
    pub source_address: u8,
    pub _pad0: u8,
    pub obd_raw: u16,
    pub _pad: [u8; 4],
}

impl CanDtcRecord {
    /// Wire size of the record in bytes.
    pub const SIZE: usize = 128;
}

/// Fail-closed mapping from a decoded fault onto a pack-to node (tile 48, `can_trigger`, 128 B).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq, Default)]
pub struct CanTriggerRecord {
    pub source_id: [u8; 16],
    pub content_hash: [u8; 32],
    pub origin_ptr: [u8; 16],
    pub log_hash: [u8; 32],
    pub frame_offset: u64,
    pub fault_node_id: u64,
    pub spn: u32,
    pub observed_raw: u32,
    pub fmi: u16,
    pub obd_raw: u16,
    pub kind: u8,
    pub _pad: [u8; 3],
}

impl CanTriggerRecord {
    /// Wire size of the record in bytes.
    pub const SIZE: usize = 128;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_sizes_and_alignments() {
        assert_eq!(std::mem::size_of::<CanDtcRecord>(), 128);
        assert_eq!(std::mem::align_of::<CanDtcRecord>(), 8);
        assert_eq!(std::mem::size_of::<CanTriggerRecord>(), 128);
        assert_eq!(std::mem::align_of::<CanTriggerRecord>(), 8);
    }
}
