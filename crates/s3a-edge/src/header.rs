//! S3A Edge Profile Header Definitions and Flag Constants.

use bytemuck::{bytes_of, Pod, Zeroable};
use crate::crc::{crc32c, crc32c_append};

/// Magic ASCII bytes identifying an S3A archive: `S3A1`.
pub const S3A_MAGIC: [u8; 4] = *b"S3A1";

/// Dual-slot file header block total size (512 bytes).
pub const FILE_HEADER_SIZE: usize = 512;

/// Individual file header slot size (64 bytes).
pub const FILE_HEADER_SLOT_SIZE: usize = 64;

/// Slot A byte offset in dual file header (0 bytes).
pub const FILE_HEADER_SLOT_A_OFFSET: usize = 0;

/// Slot B byte offset in dual file header (64 bytes).
pub const FILE_HEADER_SLOT_B_OFFSET: usize = 64;

/// Canonical 64-byte Common Tile Header size.
pub const COMMON_TILE_HEADER_SIZE: usize = 64;

/// Ultra-compact 32-byte tile header size.
pub const ULTRA_COMPACT_TILE_HEADER_SIZE: usize = 32;

/// Standard 4 KB Micro-Tile size (4,096 bytes).
pub const MICRO_TILE_4KB_SIZE: usize = 4096;

/// Standard 8 KB Micro-Tile size (8,192 bytes).
pub const MICRO_TILE_8KB_SIZE: usize = 8192;

// --- Archive Flags Bitmask (`FileHeader.flags`, §2.2.2) ---
pub const FLAG_PROFILE_EDGE: u16 = 1 << 0;       // 0x0001: Identifies archive as Edge Micro-Tile container
pub const FLAG_TILE_SIZE_4KB: u16 = 1 << 1;      // 0x0002: Explicit flag for 4,096-byte tile stride
pub const FLAG_TILE_SIZE_8KB: u16 = 1 << 2;      // 0x0004: Explicit flag for 8,192-byte tile stride
pub const FLAG_COMPACT_HEADER: u16 = 1 << 3;     // 0x0008: Flash-page aligned compact header variant
pub const FLAG_SEAL_CHAIN_ACTIVE: u16 = 1 << 4;  // 0x0010: Cryptographic seal hash chain active
pub const FLAG_ED25519_SIGNED: u16 = 1 << 5;     // 0x0020: Hardware Ed25519 signatures present

// --- Tile Flags Bitmask (`CommonTileHeader.flags`, §2.4.1) ---
pub const TILE_FLAG_ACTIVE: u16 = 0;
pub const TILE_FLAG_SEALED: u16 = 1;
pub const TILE_FLAG_CRC_CHAIN: u16 = 2;

// --- Addon Mask Constants ---
pub const ADDON_WEARABLE: u32 = 0x0008;

// --- Tile Type Wire Identifiers (§12.2) ---
pub const TILE_TYPE_RECORD: u16 = 64;        // 0x0040: Industrial equipment event log
pub const TILE_TYPE_SEAL: u16 = 70;          // 0x0046: Cryptographic seal
pub const TILE_TYPE_CONSENT: u16 = 71;       // 0x0047: Participant consent & ownership
pub const TILE_TYPE_PROVENANCE: u16 = 72;    // 0x0048: Merge provenance wrapper
pub const TILE_TYPE_TIME_SERIES: u16 = 75;   // 0x004B: Time-series telemetry
pub const TILE_TYPE_CONFIG: u16 = 77;        // 0x004D: Key-value configuration snapshot
pub const TILE_TYPE_WEARABLE_MICRO: u16 = 3; // 0x0003: Legacy Wearable micro

/// S3A File Header Slot (64 bytes, 8-byte aligned, Pod/Zeroable).
/// In Edge Profile (§2.2.1), `_reserved1` at offset 20..24 is specified as `tile_stride`.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct FileHeader {
    pub magic: [u8; 4],
    pub version: u16,
    pub flags: u16,
    pub generation: u64,
    pub tile_count: u32,
    pub tile_stride: u32,
    pub created_timestamp: u64,
    pub commit_timestamp: u64,
    pub header_crc32: u32,
    pub addon_mask: u32,
    pub file_uuid: [u64; 2],
}

impl FileHeader {
    pub fn new(tile_count: u32, tile_stride: u32, created_timestamp: u64) -> Self {
        let mut flags = FLAG_PROFILE_EDGE | FLAG_SEAL_CHAIN_ACTIVE;
        if tile_stride == 4096 {
            flags |= FLAG_TILE_SIZE_4KB;
        } else if tile_stride == 8192 {
            flags |= FLAG_TILE_SIZE_8KB;
        }

        Self {
            magic: S3A_MAGIC,
            version: 1,
            flags,
            generation: 1,
            tile_count,
            tile_stride,
            created_timestamp,
            commit_timestamp: created_timestamp,
            header_crc32: 0,
            addon_mask: ADDON_WEARABLE,
            file_uuid: [0, 0],
        }
    }

    pub fn with_commit(
        tile_count: u32,
        tile_stride: u32,
        generation: u64,
        created_timestamp: u64,
        commit_timestamp: u64,
    ) -> Self {
        let mut h = Self::new(tile_count, tile_stride, created_timestamp);
        h.generation = generation;
        h.commit_timestamp = commit_timestamp;
        h
    }

    pub fn compute_crc(&self) -> u32 {
        compute_file_header_crc32(self)
    }

    pub fn verify(&self) -> bool {
        if self.magic != S3A_MAGIC || self.version != 1 {
            return false;
        }
        if self.header_crc32 == 0 {
            return false;
        }
        self.header_crc32 == self.compute_crc()
    }
}

/// Dual-Slot File Header Block (512 bytes, 8-byte aligned) containing Slot A and Slot B
/// with 384 bytes of trailing zero padding.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct DualFileHeader {
    pub slot_a: FileHeader,
    pub slot_b: FileHeader,
    pub _padding: [[u64; 16]; 3], // 3 * 128 = 384 bytes
}

impl DualFileHeader {
    pub fn new(slot_a: FileHeader) -> Self {
        Self {
            slot_a,
            slot_b: FileHeader::zeroed(),
            _padding: [[0u64; 16]; 3],
        }
    }
}

/// Computes Castagnoli CRC32C over a FileHeader excluding the header_crc32 field itself (offset 40..44).
pub fn compute_file_header_crc32(header: &FileHeader) -> u32 {
    let bytes = bytes_of(header);
    let mut crc = crc32c(&bytes[..40]);
    crc = crc32c_append(crc, &bytes[44..64]);
    crc
}

/// Canonical 64-Byte Common Tile Header (§2.4.1).
///
/// Clarification / Audit Note:
/// To satisfy power-loss safety (§4.2, §4.4) while preserving strict 64-byte natural alignment
/// with `S3A-TILE-SPEC.md` §14.1, `data_crc32` is positioned at offset 12..16 (which corresponds
/// to `_reserved2` in §14.1, explicitly declared as ignored by standard readers), and
/// `header_crc32` is stored at offset 56..60 (the lower 4 bytes of `_reserved3`).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct CommonTileHeader {
    pub type_id: u16,          // Canonical wire tile type ID (offset 0..2)
    pub type_version: u16,     // Record layout version (offset 2..4)
    pub flags: u16,            // Tile flags (offset 4..6)
    pub _reserved1: u16,       // Explicit alignment padding (offset 6..8)
    pub record_count: u32,     // Number of logical records (offset 8..12)
    pub data_crc32: u32,       // Castagnoli CRC32C over payload bytes (offset 12..16)
    pub payload_len: u64,      // Byte length of active payload (offset 16..24)
    pub payload_hash: [u8; 32],// SHA-256 digest over active payload bytes (offset 24..56)
    pub header_crc32: u32,     // Castagnoli CRC32C over bytes 0..56 and 60..64 (offset 56..60)
    pub generation: u32,       // Monotonic commit generation counter (offset 60..64)
}

impl CommonTileHeader {
    pub fn new(type_id: u16, record_count: u32, payload_len: u64) -> Self {
        Self {
            type_id,
            type_version: 1,
            flags: TILE_FLAG_SEALED | TILE_FLAG_CRC_CHAIN,
            _reserved1: 0,
            record_count,
            data_crc32: 0,
            payload_len,
            payload_hash: [0u8; 32],
            header_crc32: 0,
            generation: 1,
        }
    }

    pub fn compute_crc(&self) -> u32 {
        compute_common_tile_header_crc32(self)
    }

    pub fn verify(&self) -> bool {
        if self.header_crc32 == 0 {
            return false;
        }
        self.header_crc32 == self.compute_crc()
    }
}

/// Computes Castagnoli CRC32C over a CommonTileHeader excluding header_crc32 (offset 56..60).
pub fn compute_common_tile_header_crc32(header: &CommonTileHeader) -> u32 {
    let bytes = bytes_of(header);
    let mut crc = crc32c(&bytes[..56]);
    crc = crc32c_append(crc, &bytes[60..64]);
    crc
}

/// Ultra-Compact 32-Byte Tile Header Variant (§2.4.2) for RAM-constrained microcontrollers.
#[repr(C, align(4))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct UltraCompactTileHeader {
    pub type_id: u16,
    pub type_version: u8,
    pub flags: u8,
    pub record_count: u16,
    pub payload_len: u16,
    pub data_crc32: u32,
    pub header_crc32: u32,
    pub payload_hash_prefix: [u8; 16],
}

impl UltraCompactTileHeader {
    pub fn compute_crc(&self) -> u32 {
        let bytes = bytes_of(self);
        let mut crc = crc32c(&bytes[..12]);
        crc = crc32c_append(crc, &bytes[16..32]);
        crc
    }

    pub fn verify(&self) -> bool {
        if self.header_crc32 == 0 {
            return false;
        }
        self.header_crc32 == self.compute_crc()
    }
}
