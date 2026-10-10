#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
extern crate std as alloc;

#[cfg(test)]
extern crate std;

use core::fmt;
use core::str::FromStr;
use bytemuck::{Pod, Zeroable};

pub mod hilbert;
pub mod dop;
pub mod registry;
pub mod lrs;
pub mod consent;
pub mod schema;
pub mod erasure;
pub mod columnar;
pub mod array;

pub use hilbert::{point_to_hilbert_3d, hilbert_to_point_3d, compare_hilbert_vs_morton_locality, compare_hilbert_vs_morton_curve_continuity, morton_to_point_3d, s3a_morton_3d};
pub use dop::{Dop14Hull, toroidal_distance_3d, toroidal_interval_contains};
pub use registry::{
    TileTypeFlags, TileTypeDescriptor, TileLifecycle, RegistryError, TileTypeRegistry,
    register_core_types, register_standard_pack_types,
    TilePayloadMigrator, migrate_tile_payload,
};
pub use lrs::*;
pub use consent::*;
pub use schema::*;
pub use erasure::*;
pub use columnar::*;
pub use array::*;
#[cfg(feature = "std")]
pub use registry::{global_registry, with_global_registry_mut};

/// Magic bytes at the start of an S3A file (`S3A1`).
pub const S3A_MAGIC: [u8; 4] = *b"S3A1";

/// S3A file header total size (512 bytes page/sector-aligned dual-slot block).
pub const FILE_HEADER_SIZE: usize = 512;

/// S3A file header individual slot size (64 bytes).
pub const FILE_HEADER_SLOT_SIZE: usize = 64;

/// S3A file header Slot A offset (0 bytes).
pub const FILE_HEADER_SLOT_A_OFFSET: usize = 0;

/// S3A file header Slot B offset (64 bytes).
pub const FILE_HEADER_SLOT_B_OFFSET: usize = 64;


/// Standard Hyper-Tile block size (128 KB page-aligned block for servers/edge servers/robotics/GIS/DA).
pub const HYPER_TILE_SIZE: usize = 128 * 1024; // 131,072 bytes

/// Standard Hyper-Tile block header size (512 bytes).
pub const HYPER_TILE_HEADER_SIZE: usize = 512;

/// Standard Hyper-Tile payload capacity in bytes (130,560 bytes).
pub const HYPER_TILE_PAYLOAD_SIZE: usize = HYPER_TILE_SIZE - HYPER_TILE_HEADER_SIZE;

/// WEARABLE MICRO-TILE PROFILE: 4 KB SPI NOR Flash Sector Aligned (Earbuds, Wristbands, Smart Glasses).
pub const MICRO_TILE_SIZE: usize = 4 * 1024; // 4,096 bytes (matches typical SPI NOR flash sector erase size)

/// Micro-Tile block header size (128 bytes).
pub const MICRO_TILE_HEADER_SIZE: usize = 128;

/// Micro-Tile payload capacity in bytes (3,968 bytes).
pub const MICRO_TILE_PAYLOAD_SIZE: usize = MICRO_TILE_SIZE - MICRO_TILE_HEADER_SIZE;

/// Simplex hull dimension capacity for standard tile rejection indexing.
pub const MAX_HULL_DIMENSIONS: usize = 16;

/// Compact micro-hull dimension capacity for wearable tile rejection indexing.
pub const MICRO_HULL_DIMENSIONS: usize = 4;

/// Record flag bits for CRUD lifecycle management.
pub const FLAG_ACTIVE: u64 = 0;
pub const FLAG_TOMBSTONE: u64 = 1 << 0;

/// Error types returned by S3A core functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum S3AError {
    InvalidMagic,
    UnsupportedVersion(u16),
    ChecksumMismatch { expected: u32, actual: u32 },
    InvalidTileSize(usize),
    BufferTooSmall { required: usize, provided: usize },
    OutOfBounds,
    InvalidFormat,
    CorruptedHeader,
    PayloadOverflow,
    RecordNotFound,
    InvalidCoordinate,
    UnsupportedLayoutVersion(u32),
    FileUuidMismatch { expected: [u64; 2], actual: [u64; 2] },
    SealedFooterCorrupt,
    SealedEndCapCorrupt,
    SealedTruncated,
    UnrepairableCorruption { group_idx: usize, corrupted_count: usize, max_repairable: usize },
    SealedIndexHashMismatch,
    SealedChainBreak,
    SealedSwappedGeneration,
}

impl fmt::Display for S3AError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            S3AError::InvalidMagic => write!(f, "Invalid S3A file magic"),
            S3AError::UnsupportedVersion(v) => write!(f, "Unsupported file version: {}", v),
            S3AError::UnsupportedLayoutVersion(v) => write!(f, "Unsupported record layout version: {}", v),
            S3AError::ChecksumMismatch { expected, actual } => {
                write!(f, "Checksum mismatch: expected 0x{:08x}, got 0x{:08x}", expected, actual)
            }
            S3AError::InvalidTileSize(sz) => write!(f, "Invalid tile size: {}", sz),
            S3AError::BufferTooSmall { required, provided } => {
                write!(f, "Buffer too small: required {}, provided {}", required, provided)
            }
            S3AError::OutOfBounds => write!(f, "Index or offset out of bounds"),
            S3AError::InvalidFormat => write!(f, "Invalid S3A file format"),
            S3AError::CorruptedHeader => write!(f, "Corrupted block or file header"),
            S3AError::PayloadOverflow => write!(f, "Hyper-Tile payload capacity exceeded"),
            S3AError::RecordNotFound => write!(f, "Requested record was not found"),
            S3AError::InvalidCoordinate => write!(f, "Invalid coordinate address format (expected L<level>:T<tile>:R<offset>)"),
            S3AError::FileUuidMismatch { expected, actual } => {
                write!(f, "Tile file_uuid mismatch: expected {:016x}{:016x}, got {:016x}{:016x}", expected[0], expected[1], actual[0], actual[1])
            }
            S3AError::SealedFooterCorrupt => write!(f, "Sealed container footer corrupted"),
            S3AError::SealedEndCapCorrupt => write!(f, "Sealed container end cap corrupted"),
            S3AError::SealedTruncated => write!(f, "Sealed container truncated"),
            S3AError::UnrepairableCorruption { group_idx, corrupted_count, max_repairable } => {
                write!(f, "Group {} has {} corrupted tiles, exceeding max repairable {}", group_idx, corrupted_count, max_repairable)
            }
            S3AError::SealedIndexHashMismatch => write!(f, "Sealed footer index hash mismatch"),
            S3AError::SealedChainBreak => write!(f, "Sealed hash chain break detected"),
            S3AError::SealedSwappedGeneration => write!(f, "Sealed generation reordering or swapped seal detected"),
        }
    }
}

/// Precise S3A Hyper-Tile Coordinate Address (`L<level>:T<tile_id>:R<record_offset>`), analogous to Excel's `A1`/`B2` cell references.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct S3ACoordinate {
    pub level: u16,        // Stratum level (0 = Wearable Micro-Tile / GIS Mesh Tile, 1 = Hot L1 cache tile, 2 = Stratified)
    pub _reserved: u16,
    pub tile_id: u32,      // Tile index within archive (0-based)
    pub record_offset: u32,// Record index within tile payload (0-based)
    pub _padding: u32,
}

impl S3ACoordinate {
    pub fn new(level: u16, tile_id: u32, record_offset: u32) -> Self {
        Self {
            level,
            _reserved: 0,
            tile_id,
            record_offset,
            _padding: 0,
        }
    }
}

impl fmt::Display for S3ACoordinate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "L{}:T{}:R{}", self.level, self.tile_id, self.record_offset)
    }
}

impl FromStr for S3ACoordinate {
    type Err = S3AError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let mut parts = s.split(':');

        let p1 = parts.next().ok_or(S3AError::InvalidCoordinate)?;
        let p2 = parts.next().ok_or(S3AError::InvalidCoordinate)?;
        let p3 = parts.next().ok_or(S3AError::InvalidCoordinate)?;

        if parts.next().is_some() {
            return Err(S3AError::InvalidCoordinate);
        }

        if !p1.starts_with('L') || !p2.starts_with('T') || !p3.starts_with('R') {
            return Err(S3AError::InvalidCoordinate);
        }

        let level: u16 = p1[1..].parse().map_err(|_| S3AError::InvalidCoordinate)?;
        let tile_id: u32 = p2[1..].parse().map_err(|_| S3AError::InvalidCoordinate)?;
        let record_offset: u32 = p3[1..].parse().map_err(|_| S3AError::InvalidCoordinate)?;

        Ok(S3ACoordinate::new(level, tile_id, record_offset))
    }
}

/// Addon identifier bitmasks for modular domain extensions.
pub const ADDON_NONE: u32 = 0;
pub const ADDON_EDU: u32 = 1 << 0;       // 0x0001: Scholar Explorer Educational & Cognitive Provenance Addon
pub const ADDON_FINTECH: u32 = 1 << 1;   // 0x0002: High-Frequency Financial Tick Addon (reserved)
pub const ADDON_ROBOTICS: u32 = 1 << 2;  // 0x0004: Kinematic Trajectory & Robotics Pack
pub const ADDON_WEARABLE: u32 = 1 << 3;  // 0x0008: Wearable / Micro-Telemetry Pack
pub const ADDON_GIS: u32 = 1 << 4;       // 0x0010: GIS Topographic Point Cloud & Mesh Pack
pub const ADDON_DA: u32 = 1 << 5;        // 0x0020: Decentralized Data Availability / State Commitments Pack
pub const ADDON_ONTOLOGY: u32 = 1 << 6;  // 0x0040: Formal Ontology & Axiom Registry Pack

/// Resolves the required addon capability bitmask and pack name for a given tile type.
/// Returns None for domain-agnostic core tiles (0, 1, 2, 7, 8, 9).
pub fn required_addon_for_tile(tile_type: u32) -> Option<(&'static str, u32)> {
    #[cfg(feature = "std")]
    {
        let reg = global_registry();
        if let Some(desc) = reg.get(tile_type) {
            return desc.required_addon.map(|mask| (desc.owner_pack, mask));
        }
        None
    }
    #[cfg(not(feature = "std"))]
    {
        match tile_type {
            3 => Some(("s3a-pack-wearable", ADDON_WEARABLE)),
            4 => Some(("s3a-pack-robotics", ADDON_ROBOTICS)),
            5 => Some(("s3a-pack-gis", ADDON_GIS)),
            6 => Some(("s3a-pack-da", ADDON_DA)),
            10..=15 | 18 | 20 => Some(("s3a-pack-scholar", ADDON_EDU)),
            16..=17 => Some(("s3a-pack-ontology", ADDON_ONTOLOGY)),
            _ => None,
        }
    }
}

/// Generic S3A Addon Trait defining domain extensions.
pub trait S3AAddon {
    fn addon_id(&self) -> u32;
    fn name(&self) -> &'static str;
    fn version(&self) -> &'static str;
    fn tile_range(&self) -> core::ops::RangeInclusive<u32>;
    fn descriptors(&self) -> &'static [TileTypeDescriptor] {
        &[]
    }
}

/// S3APack is an alias trait for modular domain packs implementing the S3A addon contract.
pub trait S3APack: S3AAddon {}
impl<T: S3AAddon + ?Sized> S3APack for T {}

/// S3A File Header Slot (64 bytes, 8-byte aligned).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct FileHeader {
    pub magic: [u8; 4],
    pub version: u16,
    pub flags: u16,
    pub generation: u64,
    pub tile_count: u32,
    pub _reserved1: u32,
    pub created_timestamp: u64,
    pub commit_timestamp: u64,
    pub header_crc32: u32,
    pub addon_mask: u32,
    pub file_uuid: [u64; 2],
}

impl FileHeader {
    pub fn new(tile_count: u32, created_timestamp: u64) -> Self {
        Self {
            magic: S3A_MAGIC,
            version: 1,
            flags: 0,
            generation: 1,
            tile_count,
            _reserved1: 0,
            created_timestamp,
            commit_timestamp: created_timestamp,
            header_crc32: 0,
            addon_mask: 0,
            file_uuid: [0, 0],
        }
    }

    pub fn with_commit(tile_count: u32, generation: u64, created_timestamp: u64, commit_timestamp: u64) -> Self {
        Self {
            magic: S3A_MAGIC,
            version: 1,
            flags: 0,
            generation,
            tile_count,
            _reserved1: 0,
            created_timestamp,
            commit_timestamp,
            header_crc32: 0,
            addon_mask: 0,
            file_uuid: [0, 0],
        }
    }

    pub fn with_file_uuid(mut self, file_uuid: [u64; 2]) -> Self {
        self.file_uuid = file_uuid;
        self
    }

    pub fn has_addon(&self, mask: u32) -> bool {
        (self.addon_mask & mask) != 0
    }

    pub fn enable_addon(&mut self, mask: u32) {
        self.addon_mask |= mask;
    }

    pub fn verify(&self) -> Result<(), S3AError> {
        if self.magic != S3A_MAGIC {
            return Err(S3AError::InvalidMagic);
        }
        if self.version != 1 {
            return Err(S3AError::UnsupportedVersion(self.version));
        }
        Ok(())
    }
}

/// Dual-Generation File Header Block (512 bytes, 8-byte aligned) containing two redundant metadata slots
/// (Slot A and Slot B) for crash-resilient atomic commits.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct DualFileHeader {
    pub slot_a: FileHeader,
    pub slot_b: FileHeader,
    pub _padding: [[u64; 16]; 3],
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


/// Simplex convex hull bounding manifold embedded in Hyper-Tile header (208 bytes, 8-byte aligned).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq)]
pub struct SimplexHull {
    pub dim: u32,
    pub radius: f32,
    pub min_bounds: [f32; MAX_HULL_DIMENSIONS],
    pub max_bounds: [f32; MAX_HULL_DIMENSIONS],
    pub centroid: [f32; MAX_HULL_DIMENSIONS],
    pub _reserved: [u32; 2],
}

impl SimplexHull {
    pub fn empty() -> Self {
        Self {
            dim: 0,
            radius: 0.0,
            min_bounds: [f32::INFINITY; MAX_HULL_DIMENSIONS],
            max_bounds: [f32::NEG_INFINITY; MAX_HULL_DIMENSIONS],
            centroid: [0.0; MAX_HULL_DIMENSIONS],
            _reserved: [0u32; 2],
        }
    }

    pub fn contains_point(&self, point: &[f32]) -> bool {
        let d = (self.dim as usize).min(MAX_HULL_DIMENSIONS).min(point.len());
        for i in 0..d {
            if point[i] < self.min_bounds[i] || point[i] > self.max_bounds[i] {
                return false;
            }
        }
        true
    }
}

/// Wearable Micro-Hull bounding manifold (36 bytes, 4-byte aligned).
#[repr(C, align(4))]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct MicroHull {
    pub dim: u32,
    pub min_bounds: [f32; MICRO_HULL_DIMENSIONS],
    pub max_bounds: [f32; MICRO_HULL_DIMENSIONS],
}

impl MicroHull {
    pub fn empty() -> Self {
        Self {
            dim: 0,
            min_bounds: [f32::INFINITY; MICRO_HULL_DIMENSIONS],
            max_bounds: [f32::NEG_INFINITY; MICRO_HULL_DIMENSIONS],
        }
    }
}

/// Tile Type enumeration values stored as u32 in header.
/// Core engine owns tiles 0..=8. Tile 9 is strictly reserved for boundary markers.
/// Domain packs (e.g. Wearable 3, Robotics 4, GIS 5, DA 6, Scholar 10..=15) define their tile types in their respective crates.
pub struct TileType;
impl TileType {
    pub const TELEMETRY: u32 = 0;
    pub const EMBEDDING: u32 = 1;
    pub const HYBRID: u32 = 2;
    pub const QUANTIZED_EMBEDDING_INT8: u32 = 7;
    pub const QUANTIZED_EMBEDDING_4BIT: u32 = 8;
    pub const RESERVED_BOUNDARY: u32 = 9;
    pub const RESERVED_PACK_SLOT: u32 = 9;
    /// Core changeable metadata marks tile (Tile 19)
    pub const MARKS: u32 = 19;
    /// Core xAPI 1.0.3 Statement Body blob tile (Tile 21)
    pub const STATEMENT_BODY: u32 = 21;
    /// Core xAPI 1.0.3 Statement Index tile (Tile 22)
    pub const STATEMENT_INDEX: u32 = 22;
    /// Core xAPI 1.0.3 Document Store tile (Tile 23: State / Profile APIs)
    pub const DOCUMENT_STORE: u32 = 23;
    /// Core xAPI 1.0.3 Attachment Blob tile (Tile 24)
    pub const ATTACHMENT_BLOB: u32 = 24;
    /// Core Columnar Partition Tile (Tile 65 / 0x0041)
    pub const COLUMNAR: u32 = 65;
    /// Core N-Dimensional Array Tile (Tile 66 / 0x0042)
    pub const ARRAY: u32 = 66;
    /// Core Participant Consent Tile (Tile 71 / 0x0047)
    pub const CONSENT: u32 = 71;
    /// Core Right-to-be-Forgotten Erasure Tile (Tile 73 / 0x0049)
    pub const ERASURE: u32 = 73;
    /// Core Embedded Schema Registry Tile (Tile 77 / 0x004D)
    pub const SCHEMA: u32 = 77;
    /// Deprecated: Tile 9 is strictly reserved for boundary markers (no user writes permitted).
    /// Greenfield LRS records belong in Scholar Pack (Tile 10, `HumanLrsRecord`).
    #[deprecated(note = "Tile 9 is RESERVED_BOUNDARY. Greenfield LRS belongs in Scholar Pack Tile 10.")]
    pub const LEARNING_RECORD_STORE: u32 = 9;
}

// Mark kinds for MarkRecord.mark_kind
pub const MARK_QUARANTINE: u32 = 1;
pub const MARK_NEEDS_REVIEW: u32 = 2;
pub const MARK_CONSENT_WITHDRAWN: u32 = 3;
pub const MARK_LEGAL_HOLD: u32 = 4;
pub const MARK_ACCESS_HEAT: u32 = 5;
pub const MARK_RETENTION_EXPIRED: u32 = 6;

// Mark actions for MarkRecord.set_or_clear
pub const MARK_CLEAR: u8 = 0;
pub const MARK_SET: u8 = 1;

// Target wildcards
pub const MARK_TARGET_ALL_RECORDS: u32 = u32::MAX;

/// CHANGEABLE METADATA MARK RECORD (64 bytes, 8-byte aligned, Pod/Zeroable).
/// Tracks mutable administrative, regulatory, and policy status (quarantine, review, consent withdrawal,
/// legal holds, access heat) separate from immutable primary records.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct MarkRecord {
    pub target_tile_type: u32,  // Tile type of target (e.g. TileType::TELEMETRY, ScholarTileType::HUMAN_LRS) (offset 0..4)
    pub record_index: u32,       // Record offset within tile, or MARK_TARGET_ALL_RECORDS (u32::MAX) (offset 4..8)
    pub target_tile_id: u64,     // Tile ID in archive (offset 8..16)
    pub generation: u64,         // Archive generation when mark was appended (offset 16..24)
    pub timestamp: u64,          // Timestamp of mark action in epoch seconds (offset 24..32)
    pub actor_hash: u64,         // Identity hash of actor asserting or clearing mark (offset 32..40)
    pub mark_kind: u32,          // MARK_QUARANTINE, MARK_NEEDS_REVIEW, etc. (offset 40..44)
    pub set_or_clear: u8,        // MARK_SET (1) or MARK_CLEAR (0) (offset 44)
    pub _padding: [u8; 3],       // Alignment padding (offset 45..48)
    pub _reserved: [u64; 2],     // Reserved for future extensions (all zeros) (offset 48..64)
}

impl MarkRecord {
    pub fn new(
        target_tile_type: u32,
        target_tile_id: u64,
        record_index: u32,
        generation: u64,
        mark_kind: u32,
        set_or_clear: u8,
        timestamp: u64,
        actor_hash: u64,
    ) -> Self {
        Self {
            target_tile_type,
            record_index,
            target_tile_id,
            generation,
            timestamp,
            actor_hash,
            mark_kind,
            set_or_clear,
            _padding: [0u8; 3],
            _reserved: [0u64; 2],
        }
    }

    pub fn is_set(&self) -> bool {
        self.set_or_clear == MARK_SET
    }

    pub fn is_cleared(&self) -> bool {
        self.set_or_clear == MARK_CLEAR
    }

    /// Returns true if this mark represents a durable policy mark (QUARANTINE, CONSENT_WITHDRAWN, LEGAL_HOLD)
    /// that must be preserved across snapshot exports.
    pub fn is_policy_mark(&self) -> bool {
        matches!(
            self.mark_kind,
            MARK_QUARANTINE | MARK_CONSENT_WITHDRAWN | MARK_LEGAL_HOLD
        )
    }

    pub fn is_whole_tile(&self) -> bool {
        self.record_index == MARK_TARGET_ALL_RECORDS
    }
}

/// Category 1: 960-bit SIMD Blocked-Bloom Filter Metadata (128 bytes, 8-byte aligned).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct TileFilterMetadata {
    pub filter_type: u16,        // 0=None, 1=BlockedBloom960
    pub hash_seed: u16,          // Seed for hash functions
    pub distinct_sensors: u32,   // Exact or estimated distinct sensor count
    pub bits: [u64; 15],         // 120 bytes (960-bit bitmask, total struct = 128 bytes)
}

impl Default for TileFilterMetadata {
    fn default() -> Self {
        Self {
            filter_type: 1,
            hash_seed: 0x533A,
            distinct_sensors: 0,
            bits: [0u64; 15],
        }
    }
}

/// Category 2: Cryptographic Provenance & Merkle Root (32 bytes, 8-byte aligned).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct TileProvenanceMetadata {
    pub merkle_root: [u8; 32],   // 256-bit BLAKE3/SHA-256 payload Merkle root
}

impl Default for TileProvenanceMetadata {
    fn default() -> Self {
        Self {
            merkle_root: [0u8; 32],
        }
    }
}

/// Category 3: Lifecycle & Certified Freshness Metadata (24 bytes, 8-byte aligned).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct TileLifecycleMetadata {
    pub tombstone_count: u32,       // Number of soft-deleted records in tile
    pub stratum_tier: u8,           // 0=L0 (memory), 1=L1 (hot), 2=L2 (warm), 3=L3 (cold)
    pub compaction_epoch: u8,       // Compaction generation count
    pub _reserved_tier: u16,
    pub min_expiry_timestamp: u64,  // Earliest record TTL in epoch seconds
    pub max_expiry_timestamp: u64,  // Latest record TTL in epoch seconds
}

impl Default for TileLifecycleMetadata {
    fn default() -> Self {
        Self {
            tombstone_count: 0,
            stratum_tier: 1,
            compaction_epoch: 0,
            _reserved_tier: 0,
            min_expiry_timestamp: 0,
            max_expiry_timestamp: u64::MAX,
        }
    }
}

/// Category 4: Multi-Tenant & Security Capability Metadata (16 bytes, 8-byte aligned).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct TileSecurityMetadata {
    pub tenant_id: u64,          // Tenant / Workspace / Organization ID
    pub security_tag_mask: u32,  // Bitmask tags (PUBLIC, CONFIDENTIAL, RESTRICTED)
    pub min_clearance_level: u8, // Required clearance level (0-255)
    pub _reserved: [u8; 3],
}

impl Default for TileSecurityMetadata {
    fn default() -> Self {
        Self {
            tenant_id: 0,
            security_tag_mask: 0,
            min_clearance_level: 0,
            _reserved: [0u8; 3],
        }
    }
}

/// Category 5: Columnar Encoding & Frame-of-Reference (FoR) Baselines (16 bytes, 8-byte aligned).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq)]
pub struct TileEncodingMetadata {
    pub value_base: f64,         // Minimum baseline value in tile
    pub value_scale: f32,        // Delta quantization step
    pub encoding_mode: u16,      // 0=Raw, 1=Delta-FoR, 2=RunLength
    pub _reserved: u16,
}

impl Default for TileEncodingMetadata {
    fn default() -> Self {
        Self {
            value_base: 0.0,
            value_scale: 1.0,
            encoding_mode: 0,
            _reserved: 0,
        }
    }
}

/// Category 6: Adaptive Learned Index Spline Parameters (8 bytes, 4-byte aligned).
#[repr(C, align(4))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq)]
pub struct TileLearnedIndex {
    pub slope: f32,              // Slope of monotonic timestamp progression
    pub intercept: f32,          // Intercept / base offset
}

impl Default for TileLearnedIndex {
    fn default() -> Self {
        Self {
            slope: 1.0,
            intercept: 0.0,
        }
    }
}

/// Hyper-Tile Header (512 bytes, 8-byte aligned).
/// Exactly 512 bytes total:
/// - Base fields: 48 bytes (0..48)
/// - SimplexHull: 208 bytes (48..256)
/// - filter: 128 bytes (256..384)
/// - provenance: 32 bytes (384..416)
/// - lifecycle: 24 bytes (416..440)
/// - security: 16 bytes (440..456)
/// - encoding: 16 bytes (456..472)
/// - learned_index: 8 bytes (472..480)
/// - Self-addressing:
///   - generation: 8 bytes (480..488)
///   - tile_index: 4 bytes (488..492)
///   - _reserved_addr: 4 bytes (492..496)
///   - file_uuid: 16 bytes (496..512)
/// Total = 48 + 208 + 128 + 32 + 24 + 16 + 16 + 8 + 8 + 4 + 4 + 16 = 512 bytes.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq)]
pub struct HyperTileHeader {
    pub tile_id: u64,
    pub tile_type: u32,
    pub record_count: u32,
    pub min_timestamp: u64,
    pub max_timestamp: u64,
    pub data_crc32: u32,
    pub header_crc32: u32,
    pub payload_bytes: u32,
    pub record_layout_version: u32,
    pub hull: SimplexHull,
    pub filter: TileFilterMetadata,
    pub provenance: TileProvenanceMetadata,
    pub lifecycle: TileLifecycleMetadata,
    pub security: TileSecurityMetadata,
    pub encoding: TileEncodingMetadata,
    pub learned_index: TileLearnedIndex,
    pub generation: u64,
    pub tile_index: u32,
    pub _reserved_addr: u32,
    pub file_uuid: [u64; 2],
}

impl HyperTileHeader {
    pub fn new(tile_id: u64, tile_type: u32) -> Self {
        #[cfg(feature = "std")]
        let default_version = global_registry().get(tile_type).map(|d| d.current_layout_version).unwrap_or(1);
        #[cfg(not(feature = "std"))]
        let default_version = 1;

        Self {
            tile_id,
            tile_type,
            record_count: 0,
            min_timestamp: u64::MAX,
            max_timestamp: 0,
            data_crc32: 0,
            header_crc32: 0,
            payload_bytes: 0,
            record_layout_version: default_version,
            hull: SimplexHull::empty(),
            filter: TileFilterMetadata::default(),
            provenance: TileProvenanceMetadata::default(),
            lifecycle: TileLifecycleMetadata::default(),
            security: TileSecurityMetadata::default(),
            encoding: TileEncodingMetadata::default(),
            learned_index: TileLearnedIndex::default(),
            generation: 1,
            tile_index: 0,
            _reserved_addr: 0,
            file_uuid: [0, 0],
        }
    }
}

/// Wearable Micro-Tile Header (128 bytes, 8-byte aligned) for 4 KB Flash Page Blocks.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct MicroTileHeader {
    pub tile_id: u32,
    pub tile_type: u16,
    pub record_count: u16,
    pub min_timestamp: u32,
    pub max_timestamp: u32,
    pub data_crc32: u32,
    pub header_crc32: u32,
    pub payload_bytes: u16,
    pub record_layout_version: u16,
    pub hull: MicroHull,
    pub _reserved2: [u32; 16],
}

impl MicroTileHeader {
    pub fn new(tile_id: u32, tile_type: u16) -> Self {
        Self {
            tile_id,
            tile_type,
            record_count: 0,
            min_timestamp: u32::MAX,
            max_timestamp: 0,
            data_crc32: 0,
            header_crc32: 0,
            payload_bytes: 0,
            record_layout_version: 1,
            hull: MicroHull::empty(),
            _reserved2: [0u32; 16],
        }
    }
}

pub const SEALED_ENDCAP_MAGIC: [u8; 8] = *b"S3AEND\0\x01";
pub const SEALED_ENDCAP_MAGIC_V1: [u8; 8] = *b"S3AEND\0\x01";
pub const SEALED_ENDCAP_MAGIC_V2: [u8; 8] = *b"S3AEND\0\x02";
pub const SEALED_INDEX_MAGIC_V2: [u8; 8] = *b"S3AIDX02";
pub const SEALED_FLAG_PARITY_ENABLED: u16 = 1 << 0;
pub const SEALED_INDEX_HEADER_SIZE_V2: usize = 128;
pub const SEALED_INDEX_ENTRY_SIZE_V2: usize = 80;
pub const SEALED_ENDCAP_SIZE: usize = 64;
pub const SEALED_ANCHORS_MAGIC: [u8; 8] = *b"ANCHORS\0";

/// Sealed file directory entry (40 bytes, 8-byte aligned).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq, Default)]
pub struct SealedDirectoryEntry {
    pub tile_index: u32,
    pub tile_type: u32,
    pub generation: u64,
    pub offset: u64,
    pub length: u64,
    pub xxh3_64: u64,
}

/// Fixed 64-byte end cap at the tail of a sealed S3A archive (8-byte aligned).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct SealedEndCap {
    pub magic: [u8; 8],
    pub footer_offset: u64,
    pub footer_length: u64,
    pub tile_count: u32,
    pub k: u16,
    pub m: u16,
    pub file_uuid: [u64; 2],
    pub footer_crc32: u32,
    pub endcap_crc32: u32,
    pub _reserved: [u8; 8],
}

impl Default for SealedEndCap {
    fn default() -> Self {
        Self {
            magic: SEALED_ENDCAP_MAGIC,
            footer_offset: 0,
            footer_length: 0,
            tile_count: 0,
            k: 16,
            m: 2,
            file_uuid: [0, 0],
            footer_crc32: 0,
            endcap_crc32: 0,
            _reserved: [0u8; 8],
        }
    }
}

/// Fixed 64-byte end cap at the tail of a v2 sealed S3A archive (8-byte aligned).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct SealedEndCapV2 {
    pub magic: [u8; 8],
    pub index_offset: u64,
    pub index_len: u64,
    pub index_hash: [u8; 32],
    pub footer_version: u16,
    pub flags: u16,
    pub endcap_crc32: u32,
}

impl Default for SealedEndCapV2 {
    fn default() -> Self {
        Self {
            magic: SEALED_ENDCAP_MAGIC_V2,
            index_offset: 0,
            index_len: 0,
            index_hash: [0u8; 32],
            footer_version: 2,
            flags: 0,
            endcap_crc32: 0,
        }
    }
}

/// Header preceding the tile index entries in the v2 sealed footer directory (128 bytes, 8-byte aligned).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct SealedIndexHeaderV2 {
    pub magic: [u8; 8],
    pub tile_count: u32,
    pub flags: u16,
    pub k: u16,
    pub m: u16,
    pub _padding: [u8; 6],
    pub file_uuid: [u64; 2],
    pub prev_seal_hash: [u8; 32],
    pub seal_generation: u64,
    pub sealed_timestamp: u64,
    pub seal_hash: [u8; 32],
    pub _reserved: [u8; 8],
}

impl Default for SealedIndexHeaderV2 {
    fn default() -> Self {
        Self {
            magic: SEALED_INDEX_MAGIC_V2,
            tile_count: 0,
            flags: 0,
            k: 16,
            m: 2,
            _padding: [0u8; 6],
            file_uuid: [0, 0],
            prev_seal_hash: [0u8; 32],
            seal_generation: 1,
            sealed_timestamp: 0,
            seal_hash: [0u8; 32],
            _reserved: [0u8; 8],
        }
    }
}

/// Individual tile entry in the v2 sealed footer directory (80 bytes, 8-byte aligned).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq, Default)]
pub struct SealedTileIndexEntry {
    pub offset: u64,
    pub length: u64,
    pub type_id: u32,
    pub type_version: u16,
    pub _pad: [u8; 2],
    pub record_count: u32,
    pub tile_index: u32,
    pub min_key: u64,
    pub max_key: u64,
    pub payload_hash: [u8; 32],
}

/// Telemetry record layout (32 bytes).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Default)]
pub struct TelemetryRecord {
    pub timestamp: u64,
    pub sensor_id: u32,
    pub metric_id: u32,
    pub value: f64,
    pub flags: u64,
}

impl TelemetryRecord {
    pub fn new(timestamp: u64, sensor_id: u32, metric_id: u32, value: f64) -> Self {
        Self {
            timestamp,
            sensor_id,
            metric_id,
            value,
            flags: FLAG_ACTIVE,
        }
    }

    pub fn is_tombstone(&self) -> bool {
        (self.flags & FLAG_TOMBSTONE) != 0
    }

    pub fn mark_tombstone(&mut self) {
        self.flags |= FLAG_TOMBSTONE;
    }
}

/// Compact 32-Dimensional INT8 Quantized Embedding Record (48 bytes) for On-Device Speech/Gesture Keyword Spotting on Glasses/Earbuds.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Default)]
pub struct CompactEmbeddingRecord32 {
    pub id: u32,
    pub timestamp_sec: u32,
    pub quantized_vector: [i8; 32], // INT8 quantized embedding features
    pub scale: f32,                // Quantization scale factor
    pub _reserved: u32,
}

impl CompactEmbeddingRecord32 {
    pub fn new(id: u32, timestamp_sec: u32, float_vector: &[f32; 32]) -> Self {
        let mut max_abs = 0.0f32;
        for &v in float_vector.iter() {
            let abs_v = if v < 0.0 { -v } else { v };
            if abs_v > max_abs {
                max_abs = abs_v;
            }
        }
        let scale = if max_abs > 0.0 { max_abs / 127.0 } else { 1.0 };
        let mut quantized_vector = [0i8; 32];
        for i in 0..32 {
            quantized_vector[i] = (float_vector[i] / scale) as i8;
        }

        Self {
            id,
            timestamp_sec,
            quantized_vector,
            scale,
            _reserved: 0,
        }
    }
}

/// Fixed-size 128-dimensional embedding record layout (528 bytes).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq)]
pub struct EmbeddingRecord128 {
    pub id: u64,
    pub timestamp: u64,
    pub vector: [f32; 128],
}

impl Default for EmbeddingRecord128 {
    fn default() -> Self {
        Self {
            id: 0,
            timestamp: 0,
            vector: [0.0; 128],
        }
    }
}

impl EmbeddingRecord128 {
    pub fn new(id: u64, timestamp: u64, vector: [f32; 128]) -> Self {
        Self { id, timestamp, vector }
    }
}

/// Quantized 256-Dimensional INT8 Embedding Record (288 bytes, 8-byte aligned).
/// Provides ~4x higher embedding density per 128 KB Hyper-Tile (~453 embeddings/tile) with SIMD acceleration.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq)]
pub struct QuantizedEmbeddingRecord256 {
    pub id: u64,
    pub timestamp: u64,
    pub scale: f32,
    pub offset: f32,
    pub flags: u64,
    pub quantized_vector: [i8; 256],
}

impl Default for QuantizedEmbeddingRecord256 {
    fn default() -> Self {
        Self {
            id: 0,
            timestamp: 0,
            scale: 1.0,
            offset: 0.0,
            flags: FLAG_ACTIVE,
            quantized_vector: [0i8; 256],
        }
    }
}

impl QuantizedEmbeddingRecord256 {
    pub fn new(id: u64, timestamp: u64, float_vector: &[f32; 256]) -> Self {
        let mut min_val = f32::INFINITY;
        let mut max_val = f32::NEG_INFINITY;
        for &v in float_vector {
            if v < min_val { min_val = v; }
            if v > max_val { max_val = v; }
        }
        let range = (max_val - min_val).max(1e-7);
        let scale = range / 254.0;
        let offset = min_val;

        let mut quantized_vector = [0i8; 256];
        for i in 0..256 {
            let normalized = (float_vector[i] - offset) / scale;
            let val = (normalized - 127.0).clamp(-128.0, 127.0);
            quantized_vector[i] = val as i8;
        }

        Self {
            id,
            timestamp,
            scale,
            offset,
            flags: FLAG_ACTIVE,
            quantized_vector,
        }
    }

    pub fn dequantize(&self) -> [f32; 256] {
        let mut out = [0.0f32; 256];
        for i in 0..256 {
            let normalized = (self.quantized_vector[i] as f32) + 127.0;
            out[i] = normalized * self.scale + self.offset;
        }
        out
    }

    pub fn is_tombstone(&self) -> bool {
        (self.flags & FLAG_TOMBSTONE) != 0
    }

    pub fn mark_tombstone(&mut self) {
        self.flags |= FLAG_TOMBSTONE;
    }
}

/// Quantized 512-Dimensional 4-Bit Embedding Record (288 bytes, 8-byte aligned).
/// Stores 512 dimensions packed as 2 dimensions per byte (nibbles) for extreme embedding density.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq)]
pub struct QuantizedEmbeddingRecord512 {
    pub id: u64,
    pub timestamp: u64,
    pub scale: f32,
    pub offset: f32,
    pub flags: u64,
    pub packed_vector: [u8; 256], // 512 4-bit unsigned integers (0..15) packed into 256 bytes
}

impl Default for QuantizedEmbeddingRecord512 {
    fn default() -> Self {
        Self {
            id: 0,
            timestamp: 0,
            scale: 1.0,
            offset: 0.0,
            flags: FLAG_ACTIVE,
            packed_vector: [0u8; 256],
        }
    }
}

impl QuantizedEmbeddingRecord512 {
    pub fn new(id: u64, timestamp: u64, float_vector: &[f32; 512]) -> Self {
        let mut min_val = f32::INFINITY;
        let mut max_val = f32::NEG_INFINITY;
        for &v in float_vector {
            if v < min_val { min_val = v; }
            if v > max_val { max_val = v; }
        }
        let range = (max_val - min_val).max(1e-7);
        let scale = range / 15.0;
        let offset = min_val;

        let mut packed_vector = [0u8; 256];
        for i in 0..256 {
            let v0 = (((float_vector[2 * i] - offset) / scale).clamp(0.0, 15.0)) as u8;
            let v1 = (((float_vector[2 * i + 1] - offset) / scale).clamp(0.0, 15.0)) as u8;
            packed_vector[i] = (v0 & 0x0F) | ((v1 & 0x0F) << 4);
        }

        Self {
            id,
            timestamp,
            scale,
            offset,
            flags: FLAG_ACTIVE,
            packed_vector,
        }
    }

    pub fn dequantize(&self) -> [f32; 512] {
        let mut out = [0.0f32; 512];
        for i in 0..256 {
            let byte = self.packed_vector[i];
            let v0 = (byte & 0x0F) as f32;
            let v1 = ((byte >> 4) & 0x0F) as f32;
            out[2 * i] = v0 * self.scale + self.offset;
            out[2 * i + 1] = v1 * self.scale + self.offset;
        }
        out
    }

    pub fn is_tombstone(&self) -> bool {
        (self.flags & FLAG_TOMBSTONE) != 0
    }

    pub fn mark_tombstone(&mut self) {
        self.flags |= FLAG_TOMBSTONE;
    }
}

/// LEARNING RECORD STORE (LRS) & AI AGENT TRAJECTORY RECORD (64 bytes, 8-byte aligned).
/// Supports both xAPI/ADL standards (Actor-Verb-Object-Result) and
/// AI Agent Reinforcement Learning & Tool Calling Experience Replays (Agent-Tool-State-Reward).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Default)]
pub struct LearningActivityRecord {
    pub actor_id: u64,          // User ID / Student ID / AI Agent ID (offset 0)
    pub timestamp_sec: u64,     // Timestamp of learning interaction or tool execution (offset 8)
    pub verb_id: u32,           // Action / xAPI Verb (offset 16)
    pub object_id: u32,         // Activity / Course / Tool ID / State ID (offset 20)
    pub result_score: f32,      // Grade / Scaled Reward / Accuracy (offset 24)
    pub duration_ms: u32,       // Execution latency / time spent in milliseconds (offset 28)
    pub success_flag: u32,      // 1 = Passed / Succeeded, 0 = Failed (offset 32)
    pub _reserved32: u32,       // Padding to align next u64 (offset 36)
    pub flags: u64,             // FLAG_ACTIVE, FLAG_TOMBSTONE (offset 40)
    pub _reserved: [u64; 2],    // Padding to exactly 64 bytes (offset 48..64)
}

impl LearningActivityRecord {
    pub fn new(
        actor_id: u64,
        timestamp_sec: u64,
        verb_id: u32,
        object_id: u32,
        result_score: f32,
        duration_ms: u32,
        success_flag: u32,
    ) -> Self {
        Self {
            actor_id,
            timestamp_sec,
            verb_id,
            object_id,
            result_score,
            duration_ms,
            success_flag,
            _reserved32: 0,
            flags: FLAG_ACTIVE,
            _reserved: [0u64; 2],
        }
    }

    pub fn is_tombstone(&self) -> bool {
        (self.flags & FLAG_TOMBSTONE) != 0
    }

    pub fn mark_tombstone(&mut self) {
        self.flags |= FLAG_TOMBSTONE;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::size_of;
    use std::format;

    #[test]
    fn test_micro_tile_profile_sizes() {
        assert_eq!(MICRO_TILE_SIZE, 4096);
        assert_eq!(size_of::<MicroTileHeader>(), MICRO_TILE_HEADER_SIZE);
        assert_eq!(size_of::<CompactEmbeddingRecord32>(), 48);
        assert_eq!(MICRO_TILE_PAYLOAD_SIZE, 3968);
    }

    #[test]
    fn test_coordinate_formatting_and_parsing() {
        let coord = S3ACoordinate::new(0, 5, 42);
        let formatted = format!("{}", coord);
        assert_eq!(formatted, "L0:T5:R42");

        let parsed: S3ACoordinate = formatted.parse().unwrap();
        assert_eq!(parsed, coord);
        assert_eq!(parsed.level, 0);
        assert_eq!(parsed.tile_id, 5);
        assert_eq!(parsed.record_offset, 42);
    }

    #[test]
    fn test_header_sizes_and_alignments() {
        assert_eq!(size_of::<FileHeader>(), FILE_HEADER_SLOT_SIZE);
        assert_eq!(size_of::<DualFileHeader>(), FILE_HEADER_SIZE);
        assert_eq!(size_of::<HyperTileHeader>(), HYPER_TILE_HEADER_SIZE);
        assert_eq!(size_of::<TelemetryRecord>(), 32);
        assert_eq!(size_of::<EmbeddingRecord128>(), 528);
        assert_eq!(size_of::<S3ACoordinate>(), 16);
        assert_eq!(size_of::<TileFilterMetadata>(), 128);
        assert_eq!(size_of::<TileProvenanceMetadata>(), 32);
        assert_eq!(size_of::<TileLifecycleMetadata>(), 24);
        assert_eq!(size_of::<TileSecurityMetadata>(), 16);
        assert_eq!(size_of::<TileEncodingMetadata>(), 16);
        assert_eq!(size_of::<TileLearnedIndex>(), 8);
        assert_eq!(size_of::<LearningActivityRecord>(), 64);
        assert_eq!(size_of::<MarkRecord>(), 64);
        assert_eq!(core::mem::align_of::<MarkRecord>(), 8);
        assert_eq!(size_of::<SealedDirectoryEntry>(), 40);
        assert_eq!(size_of::<SealedEndCap>(), SEALED_ENDCAP_SIZE);
        assert_eq!(core::mem::align_of::<SealedEndCap>(), 8);
        assert_eq!(size_of::<ConsentRecord>(), 136);
        assert_eq!(core::mem::align_of::<ConsentRecord>(), 8);
        assert_eq!(size_of::<CompactSchemaHeader>(), 32);
        assert_eq!(core::mem::align_of::<CompactSchemaHeader>(), 8);
        assert_eq!(size_of::<CompactSchemaTypeRecord>(), 80);
        assert_eq!(core::mem::align_of::<CompactSchemaTypeRecord>(), 8);
        assert_eq!(size_of::<CompactSchemaFieldRecord>(), 48);
        assert_eq!(core::mem::align_of::<CompactSchemaFieldRecord>(), 8);
        assert_eq!(size_of::<ColumnarTileHeader>(), 32);
        assert_eq!(core::mem::align_of::<ColumnarTileHeader>(), 8);
        assert_eq!(size_of::<ColumnDescriptor>(), 104);
        assert_eq!(core::mem::align_of::<ColumnDescriptor>(), 8);
        assert_eq!(size_of::<ArrayTileHeader>(), 320);
        assert_eq!(core::mem::align_of::<ArrayTileHeader>(), 8);
        assert_eq!(size_of::<SealedEndCapV2>(), 64);
        assert_eq!(core::mem::align_of::<SealedEndCapV2>(), 8);
        assert_eq!(size_of::<SealedIndexHeaderV2>(), 128);
        assert_eq!(core::mem::align_of::<SealedIndexHeaderV2>(), 8);
        assert_eq!(size_of::<SealedTileIndexEntry>(), 80);
        assert_eq!(core::mem::align_of::<SealedTileIndexEntry>(), 8);
    }

    #[test]
    fn test_file_header_verification() {
        let header = FileHeader::new(10, 1000);
        assert!(header.verify().is_ok());

        let mut invalid_header = header;
        invalid_header.magic = *b"BAD1";
        assert_eq!(invalid_header.verify(), Err(S3AError::InvalidMagic));
    }

    #[test]
    fn test_simplex_hull_bounds() {
        let mut hull = SimplexHull::empty();
        hull.dim = 3;
        hull.min_bounds[0] = 0.0;
        hull.max_bounds[0] = 10.0;
        hull.min_bounds[1] = 0.0;
        hull.max_bounds[1] = 10.0;
        hull.min_bounds[2] = 0.0;
        hull.max_bounds[2] = 10.0;

        assert!(hull.contains_point(&[5.0, 5.0, 5.0]));
        assert!(!hull.contains_point(&[15.0, 5.0, 5.0]));
    }
}

