#![no_std]

#[cfg(test)]
extern crate std;

use core::fmt;
use core::str::FromStr;
use bytemuck::{Pod, Zeroable};

pub mod hilbert;
pub mod dop;

pub use hilbert::{point_to_hilbert_3d, hilbert_to_point_3d, compare_hilbert_vs_morton_locality, compare_hilbert_vs_morton_curve_continuity, morton_to_point_3d, s3a_morton_3d};
pub use dop::{Dop14Hull, toroidal_distance_3d, toroidal_interval_contains};

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
}

impl fmt::Display for S3AError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            S3AError::InvalidMagic => write!(f, "Invalid S3A file magic"),
            S3AError::UnsupportedVersion(v) => write!(f, "Unsupported file version: {}", v),
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
    pub _reserved2: [u32; 5],
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
            _reserved2: [0u32; 5],
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
            _reserved2: [0u32; 5],
        }
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


/// Simplex convex hull bounding manifold embedded in Hyper-Tile header (216 bytes, 8-byte aligned).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct SimplexHull {
    pub dim: u32,
    pub radius: f32,
    pub min_bounds: [f32; MAX_HULL_DIMENSIONS],
    pub max_bounds: [f32; MAX_HULL_DIMENSIONS],
    pub centroid: [f32; MAX_HULL_DIMENSIONS],
    pub _reserved: [u32; 4],
}

impl SimplexHull {
    pub fn empty() -> Self {
        Self {
            dim: 0,
            radius: 0.0,
            min_bounds: [f32::INFINITY; MAX_HULL_DIMENSIONS],
            max_bounds: [f32::NEG_INFINITY; MAX_HULL_DIMENSIONS],
            centroid: [0.0; MAX_HULL_DIMENSIONS],
            _reserved: [0u32; 4],
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
pub struct TileType;
impl TileType {
    pub const TELEMETRY: u32 = 0;
    pub const EMBEDDING: u32 = 1;
    pub const HYBRID: u32 = 2;
    pub const WEARABLE_MICRO: u32 = 3;
    pub const ROBOTICS_KINEMATIC: u32 = 4;
    pub const GIS_SURVEY_MESH: u32 = 5;
    pub const DATA_AVAILABILITY: u32 = 6;
    pub const QUANTIZED_EMBEDDING_INT8: u32 = 7;
    pub const QUANTIZED_EMBEDDING_4BIT: u32 = 8;
    pub const LEARNING_RECORD_STORE: u32 = 9;
    pub const HUMAN_LRS: u32 = 10;
    pub const AI_TRACE_LOG: u32 = 11;
    pub const ACADEMIC_PAPERS: u32 = 12;
    pub const RESEARCH_GRAPH: u32 = 13;
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
/// - Base fields: 48 bytes
/// - SimplexHull: 212 bytes
/// - filter: 128 bytes
/// - provenance: 32 bytes
/// - lifecycle: 24 bytes
/// - security: 16 bytes
/// - encoding: 16 bytes
/// - learned_index: 8 bytes
/// - _padding: 16 bytes
/// Total = 56 + 216 + 128 + 32 + 24 + 16 + 16 + 8 + 16 = 512 bytes.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct HyperTileHeader {
    pub tile_id: u64,
    pub tile_type: u32,
    pub record_count: u32,
    pub min_timestamp: u64,
    pub max_timestamp: u64,
    pub data_crc32: u32,
    pub header_crc32: u32,
    pub payload_bytes: u32,
    pub _reserved1: u32,
    pub hull: SimplexHull,
    pub filter: TileFilterMetadata,
    pub provenance: TileProvenanceMetadata,
    pub lifecycle: TileLifecycleMetadata,
    pub security: TileSecurityMetadata,
    pub encoding: TileEncodingMetadata,
    pub learned_index: TileLearnedIndex,
    pub _padding: [u32; 6],
}

impl HyperTileHeader {
    pub fn new(tile_id: u64, tile_type: u32) -> Self {
        Self {
            tile_id,
            tile_type,
            record_count: 0,
            min_timestamp: u64::MAX,
            max_timestamp: 0,
            data_crc32: 0,
            header_crc32: 0,
            payload_bytes: 0,
            _reserved1: 0,
            hull: SimplexHull::empty(),
            filter: TileFilterMetadata::default(),
            provenance: TileProvenanceMetadata::default(),
            lifecycle: TileLifecycleMetadata::default(),
            security: TileSecurityMetadata::default(),
            encoding: TileEncodingMetadata::default(),
            learned_index: TileLearnedIndex::default(),
            _padding: [0u32; 6],
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
    pub _reserved1: u16,
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
            _reserved1: 0,
            hull: MicroHull::empty(),
            _reserved2: [0u32; 16],
        }
    }
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

/// DECENTRALIZED DATA AVAILABILITY (DA) & BLOCKCHAIN INDEXER RECORD (64 bytes, 8-byte aligned).
/// Designed for L2 Rollups, State Channels, and Decentralized AI Model Weight Commitment Verification.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Default)]
pub struct DACommitmentRecord {
    pub block_height: u64,          // Block height / L2 slot index
    pub state_root_hash: [u8; 32],   // 256-bit State Root or Polynomial KZG commitment
    pub timestamp_sec: u32,         // Block / Slot timestamp
    pub quorum_bitmask: u32,        // Validator availability quorum signoff bitmask
    pub transaction_count: u32,     // Number of bundled L2 / AI transactions
    pub _reserved: [u32; 3],
}

impl DACommitmentRecord {
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

/// GIS SURVEYING & SUBSURFACE POINT CLOUD / MESH RECORD (32 bytes, 8-byte aligned).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Default)]
pub struct GISSurveyPointRecord {
    pub latitude_microdeg: i32,  // Latitude in micro-degrees
    pub longitude_microdeg: i32, // Longitude in micro-degrees
    pub elevation_mm: i32,       // Elevation/Depth above/below sea level in millimeters
    pub point_class: u16,        // Classification (0=Ground, 1=Subsurface Strata, 2=Vegetation, 3=Structure)
    pub intensity: u16,          // LiDAR/Radar return intensity
    pub color_rgb: u32,          // Packed 24-bit RGB point color
    pub timestamp_sec: u32,      // Survey timestamp
    pub _reserved: u32,
    pub _padding: u32,
}

impl GISSurveyPointRecord {
    pub fn new(
        latitude_deg: f64,
        longitude_deg: f64,
        elevation_m: f64,
        point_class: u16,
        intensity: u16,
        timestamp_sec: u32,
    ) -> Self {
        Self {
            latitude_microdeg: (latitude_deg * 1_000_000.0) as i32,
            longitude_microdeg: (longitude_deg * 1_000_000.0) as i32,
            elevation_mm: (elevation_m * 1000.0) as i32,
            point_class,
            intensity,
            color_rgb: 0x00FFFFFF,
            timestamp_sec,
            _reserved: 0,
            _padding: 0,
        }
    }

    pub fn latitude_deg(&self) -> f64 {
        (self.latitude_microdeg as f64) / 1_000_000.0
    }

    pub fn longitude_deg(&self) -> f64 {
        (self.longitude_microdeg as f64) / 1_000_000.0
    }

    pub fn elevation_m(&self) -> f64 {
        (self.elevation_mm as f64) / 1000.0
    }
}

/// ROBOTICS PLATFORM KINEMATIC & IMU STATE RECORD (72 bytes, 8-byte aligned).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Default)]
pub struct RoboticsKinematicRecord {
    pub timestamp_us: u64, // Microsecond-precision timestamp
    pub robot_id: u32,      // Robot / Joint / Actuator ID
    pub joint_mask: u32,    // Joint bitmask
    pub position_xyz: [f32; 3], // 3D Position [X, Y, Z] in meters
    pub orientation_quat: [f32; 4], // Quaternion [W, X, Y, Z]
    pub linear_velocity: [f32; 3], // Linear Velocity [Vx, Vy, Vz]
    pub angular_velocity: [f32; 3], // Angular Velocity [Wx, Wy, Wz]
    pub _padding: u32,
}

impl RoboticsKinematicRecord {
    pub fn new(
        timestamp_us: u64,
        robot_id: u32,
        position_xyz: [f32; 3],
        orientation_quat: [f32; 4],
        linear_velocity: [f32; 3],
        angular_velocity: [f32; 3],
    ) -> Self {
        Self {
            timestamp_us,
            robot_id,
            joint_mask: 0xFFFF,
            position_xyz,
            orientation_quat,
            linear_velocity,
            angular_velocity,
            _padding: 0,
        }
    }
}

/// Ultra-Compact Quantized Wearable Telemetry Record (16 bytes) for Heart Rate / IMU / PPG / Gyro in Earbuds & Smart Rings.
#[repr(C, align(4))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq, Default)]
pub struct CompactWearableRecord {
    pub timestamp_sec: u32, // Relative seconds timestamp (fits >130 years)
    pub sensor_id: u8,      // Sensor ID (0=PPG HeartRate, 1=Accel, 2=Gyro, 3=Temp, 4=SpO2)
    pub metric_id: u8,      // Metric ID
    pub quantized_val: i16, // INT16 scaled value (e.g. HeartRate * 256.0, Temp * 100.0)
    pub quality_flags: u16, // Signal quality, Motion artifact bits
    pub _reserved: [u8; 6],
}

impl CompactWearableRecord {
    pub fn new(timestamp_sec: u32, sensor_id: u8, metric_id: u8, float_value: f32) -> Self {
        Self {
            timestamp_sec,
            sensor_id,
            metric_id,
            quantized_val: (float_value * 256.0) as i16,
            quality_flags: 0x0001,
            _reserved: [0u8; 6],
        }
    }

    pub fn float_value(&self) -> f32 {
        (self.quantized_val as f32) / 256.0
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

/// HUMAN ACTIVITY SQL LRS RECORD (64 bytes, 8-byte aligned).
/// Captures human researcher reviews, approvals, rework decisions, and xAPI activity statements.
/// Contains a shared 128-bit `session_uuid` and direct O(1) coordinate pointer to the corresponding AI trace.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Default)]
pub struct HumanLrsRecord {
    pub session_uuid: [u64; 2],        // 128-bit shared session/registration UUID (offset 0..16)
    pub timestamp_sec: u64,            // Recorded timestamp (offset 16..24)
    pub actor_hash: u64,               // Hash of researcher / human actor (offset 24..32)
    pub verb_id: u32,                  // xAPI verb ID (e.g. accepted, reworked, decided) (offset 32..36)
    pub decision_score: f32,           // Feedback / rework score (offset 36..40)
    pub ai_trace_coord: S3ACoordinate, // Direct O(1) pointer to AI agent trace step (offset 40..56)
    pub object_hash: u32,              // Hash of target artifact / paper / claim (offset 56..60)
    pub flags: u32,                    // FLAG_ACTIVE, FLAG_TOMBSTONE, etc. (offset 60..64)
}

impl HumanLrsRecord {
    pub fn new(
        session_uuid: [u64; 2],
        timestamp_sec: u64,
        actor_hash: u64,
        verb_id: u32,
        decision_score: f32,
        ai_trace_coord: S3ACoordinate,
        object_hash: u32,
    ) -> Self {
        Self {
            session_uuid,
            timestamp_sec,
            actor_hash,
            verb_id,
            decision_score,
            ai_trace_coord,
            object_hash,
            flags: FLAG_ACTIVE as u32,
        }
    }

    pub fn is_tombstone(&self) -> bool {
        (self.flags & (FLAG_TOMBSTONE as u32)) != 0
    }
}

/// AI AGENT TRACEABLE LOG METADATA RECORD (64 bytes, 8-byte aligned).
/// Captures agent reasoning steps, tool calls, hallucination events, and claim verifications.
/// Matches the human SQL LRS via shared 128-bit `session_uuid` and direct O(1) coordinate pointer.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Default)]
pub struct AiTraceRecord {
    pub session_uuid: [u64; 2],        // 128-bit shared session/registration UUID (offset 0..16)
    pub timestamp_sec: u64,            // Execution timestamp (offset 16..24)
    pub agent_id: u64,                 // AI Agent hash (offset 24..32)
    pub step_type: u32,                // Step category (reasoning, tool_call, claim, hallucination) (offset 32..36)
    pub confidence: f32,               // Model confidence / reward score (offset 36..40)
    pub human_coord: S3ACoordinate,    // Direct O(1) pointer to corresponding human decision (offset 40..56)
    pub hilbert_index: u32,            // 3D spatial cluster coordinate (offset 56..60)
    pub flags: u32,                    // Active, tombstone, warning flags (offset 60..64)
}

impl AiTraceRecord {
    pub fn new(
        session_uuid: [u64; 2],
        timestamp_sec: u64,
        agent_id: u64,
        step_type: u32,
        confidence: f32,
        human_coord: S3ACoordinate,
        hilbert_index: u32,
    ) -> Self {
        Self {
            session_uuid,
            timestamp_sec,
            agent_id,
            step_type,
            confidence,
            human_coord,
            hilbert_index,
            flags: FLAG_ACTIVE as u32,
        }
    }

    pub fn is_tombstone(&self) -> bool {
        (self.flags & (FLAG_TOMBSTONE as u32)) != 0
    }
}

/// ACADEMIC PAPER & RESEARCH METADATA RECORD (64 bytes, 8-byte aligned).
/// Captures literature metadata with 3D Skilling Hilbert curve spatial coordinates.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Default)]
pub struct AcademicPaperRecord {
    pub paper_id: u64,                 // 64-bit hash of DOI / URI (offset 0..8)
    pub timestamp_sec: u64,            // Publication timestamp (offset 8..16)
    pub hilbert_index: u64,            // 3D Skilling Hilbert curve index (offset 16..24)
    pub topic_x: f32,                  // 3D coordinate X (topic/cluster) (offset 24..28)
    pub topic_y: f32,                  // 3D coordinate Y (methodology/field) (offset 28..32)
    pub topic_z: f32,                  // 3D coordinate Z (year/recency) (offset 32..36)
    pub citation_count: u32,           // Citation count (offset 36..40)
    pub year: u16,                     // Publication year (offset 40..42)
    pub venue_id: u16,                 // Venue / service ID (offset 42..44)
    pub open_access_flag: u16,         // 1 = OA PDF, 0 = Paywalled (offset 44..46)
    pub warning_count: u16,            // Provenance warning count (offset 46..48)
    pub doi_prefix_hash: u64,          // Hash of publisher DOI prefix (offset 48..56)
    pub flags: u64,                    // Active, tombstone, reviewed (offset 56..64)
}

impl AcademicPaperRecord {
    pub fn new(
        paper_id: u64,
        timestamp_sec: u64,
        hilbert_index: u64,
        topic_x: f32,
        topic_y: f32,
        topic_z: f32,
        citation_count: u32,
        year: u16,
        venue_id: u16,
        open_access_flag: u16,
        warning_count: u16,
        doi_prefix_hash: u64,
    ) -> Self {
        Self {
            paper_id,
            timestamp_sec,
            hilbert_index,
            topic_x,
            topic_y,
            topic_z,
            citation_count,
            year,
            venue_id,
            open_access_flag,
            warning_count,
            doi_prefix_hash,
            flags: FLAG_ACTIVE,
        }
    }
}

/// RESEARCH KNOWLEDGE GRAPH EDGE RECORD (64 bytes, 8-byte aligned).
/// Captures subject-predicate-object knowledge graph relations.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Default)]
pub struct ResearchGraphEdgeRecord {
    pub subject_hash: u64,             // Node A identifier hash (offset 0..8)
    pub object_hash: u64,              // Node B identifier hash (offset 8..16)
    pub timestamp_sec: u64,            // Extraction timestamp (offset 16..24)
    pub hilbert_coord: u64,            // Spatial cluster coordinate (offset 24..32)
    pub predicate_id: u32,             // Predicate type (contains, authored_by, cites) (offset 32..36)
    pub weight: f32,                   // Edge weight / confidence (offset 36..40)
    pub flags: u64,                    // Active, tombstone (offset 40..48)
    pub _reserved: [u64; 2],           // Padding to 64 bytes (offset 48..64)
}

impl ResearchGraphEdgeRecord {
    pub fn new(
        subject_hash: u64,
        object_hash: u64,
        timestamp_sec: u64,
        hilbert_coord: u64,
        predicate_id: u32,
        weight: f32,
    ) -> Self {
        Self {
            subject_hash,
            object_hash,
            timestamp_sec,
            hilbert_coord,
            predicate_id,
            weight,
            flags: FLAG_ACTIVE,
            _reserved: [0u64; 2],
        }
    }
}

/// SCHOLAR RESEARCH SNAPSHOT (SRS) MANIFEST HEADER (512 bytes, exact sector-aligned, Pod/Zeroable).
/// Encapsulates portable research object identity, Project UUID, Snapshot UUID, parent lineage,
/// and hardware CRC32C checksums as defined in SECS-SRS-1.0.0.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct SrsManifestHeader {
    pub magic: [u8; 8],                // b"S3ASTOR1" (offset 0..8)
    pub schema_version: u32,           // SRS specification version (e.g. 1) (offset 8..12)
    pub archive_flags: u32,            // Bitmask: IS_IMMUTABLE, IS_SNAPSHOT, etc. (offset 12..16)
    pub project_uuid: [u64; 2],        // 128-bit persistent project identifier (offset 16..32)
    pub snapshot_uuid: [u64; 2],       // 128-bit unique checkpoint identifier (offset 32..48)
    pub parent_snapshot_uuid: [u64; 2],// 128-bit parent checkpoint (0 if initial root) (offset 48..64)
    pub origin_session_uuid: [u64; 2], // 128-bit active session identifier (offset 64..80)
    pub created_at_sec: u64,           // Unix epoch seconds (offset 80..88)
    pub tile_count: u32,               // Number of Hyper-Tiles in container (offset 88..92)
    pub crc32c_checksum: u32,          // Hardware CRC32C of entire payload (offset 92..96)
    pub total_records: u64,            // Total record count across all tiles (offset 96..104)
    pub _reserved1: [u64; 32],         // Zero-padded: 256 bytes (offset 104..360)
    pub _reserved2: [u64; 19],         // Zero-padded: 152 bytes (offset 360..512)
}

impl SrsManifestHeader {
    pub const MAGIC: [u8; 8] = *b"S3ASTOR1";
    pub const FLAG_IMMUTABLE: u32 = 1 << 0;
    pub const FLAG_SNAPSHOT: u32 = 1 << 1;
    pub const FLAG_WASM_EXPORTED: u32 = 1 << 2;

    pub fn new(
        project_uuid: [u64; 2],
        snapshot_uuid: [u64; 2],
        parent_snapshot_uuid: [u64; 2],
        origin_session_uuid: [u64; 2],
        created_at_sec: u64,
        tile_count: u32,
        total_records: u64,
    ) -> Self {
        Self {
            magic: Self::MAGIC,
            schema_version: 1,
            archive_flags: Self::FLAG_IMMUTABLE | Self::FLAG_SNAPSHOT,
            project_uuid,
            snapshot_uuid,
            parent_snapshot_uuid,
            origin_session_uuid,
            created_at_sec,
            tile_count,
            crc32c_checksum: 0,
            total_records,
            _reserved1: [0u64; 32],
            _reserved2: [0u64; 19],
        }
    }
}

/// SCHOLAR RESEARCH WORKSPACE RECORD (64 bytes, 8-byte aligned, Pod/Zeroable).
/// Encapsulates research question hash, collection masks, and topic centroid.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Default)]
pub struct SrsWorkspaceRecord {
    pub project_uuid: [u64; 2],        // 128-bit project UUID (offset 0..16)
    pub question_hash: u64,            // Hash of current research question (offset 16..24)
    pub collection_mask: u64,          // Active collection bitmask (offset 24..32)
    pub topic_centroid_xyz: [f32; 3],  // Topic cluster centroid coordinates (offset 32..44)
    pub status: u32,                   // Workspace status (open, investigating, synthesized) (offset 44..48)
    pub pinned_works_count: u32,       // Number of pinned works (offset 48..52)
    pub claims_count: u32,             // Number of extracted claims (offset 52..56)
    pub flags: u64,                    // Flags (offset 56..64)
}

impl SrsWorkspaceRecord {
    pub fn new(
        project_uuid: [u64; 2],
        question_hash: u64,
        collection_mask: u64,
        topic_centroid_xyz: [f32; 3],
        status: u32,
    ) -> Self {
        Self {
            project_uuid,
            question_hash,
            collection_mask,
            topic_centroid_xyz,
            status,
            pinned_works_count: 0,
            claims_count: 0,
            flags: FLAG_ACTIVE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::size_of;
    use std::format;

    #[test]
    fn test_da_commitment_record_layout() {
        assert_eq!(size_of::<DACommitmentRecord>(), 64);
        let rec = DACommitmentRecord::new(100_000, [0xAA; 32], 1600000000, 0xFF, 500);
        assert_eq!(rec.block_height, 100_000);
        assert_eq!(rec.transaction_count, 500);
    }

    #[test]
    fn test_gis_survey_point_record() {
        assert_eq!(size_of::<GISSurveyPointRecord>(), 32);
        let point = GISSurveyPointRecord::new(37.774929, -122.419416, 125.5, 0, 100, 1600000000);

        assert!((point.latitude_deg() - 37.774929).abs() < 1e-5);
        assert!((point.longitude_deg() - (-122.419416)).abs() < 1e-5);
        assert!((point.elevation_m() - 125.5).abs() < 1e-3);
    }

    #[test]
    fn test_robotics_kinematic_record_layout() {
        assert_eq!(size_of::<RoboticsKinematicRecord>(), 72);
        let rec = RoboticsKinematicRecord::new(
            1_000_000,
            42,
            [1.0, 2.0, 3.0],
            [1.0, 0.0, 0.0, 0.0],
            [0.5, 0.0, 0.0],
            [0.0, 0.1, 0.0],
        );
        assert_eq!(rec.timestamp_us, 1_000_000);
        assert_eq!(rec.position_xyz, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn test_wearable_micro_tile_sizes() {
        assert_eq!(MICRO_TILE_SIZE, 4096);
        assert_eq!(size_of::<MicroTileHeader>(), MICRO_TILE_HEADER_SIZE);
        assert_eq!(size_of::<CompactWearableRecord>(), 16);
        assert_eq!(size_of::<CompactEmbeddingRecord32>(), 48);
        assert_eq!(MICRO_TILE_PAYLOAD_SIZE, 3968);

        // Capacity: 3,968 / 16 = 248 wearable records in a single 4 KB flash page!
        assert_eq!(MICRO_TILE_PAYLOAD_SIZE / size_of::<CompactWearableRecord>(), 248);
    }

    #[test]
    fn test_compact_wearable_record_quantization() {
        let rec = CompactWearableRecord::new(100, 1, 10, 72.5);
        assert_eq!(rec.sensor_id, 1);
        assert!((rec.float_value() - 72.5).abs() < 0.01);
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
        assert_eq!(size_of::<HumanLrsRecord>(), 64);
        assert_eq!(size_of::<AiTraceRecord>(), 64);
        assert_eq!(size_of::<AcademicPaperRecord>(), 64);
        assert_eq!(size_of::<ResearchGraphEdgeRecord>(), 64);
    }

    #[test]
    fn test_cross_trace_human_and_ai_with_shared_uuid() {
        let session_uuid = [0x1234_5678_9ABC_DEF0, 0x0FED_CBA9_8765_4321];
        let ai_coord = S3ACoordinate::new(0, 1, 5);
        let human_coord = S3ACoordinate::new(0, 2, 8);

        let human = HumanLrsRecord::new(session_uuid, 1600000000, 42, 1, 1.0, ai_coord, 100);
        let ai = AiTraceRecord::new(session_uuid, 1600000005, 99, 2, 0.95, human_coord, 500);

        assert_eq!(human.session_uuid, ai.session_uuid);
        assert_eq!(human.ai_trace_coord, ai_coord);
        assert_eq!(ai.human_coord, human_coord);
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

    #[test]
    fn test_srs_manifest_header_layout() {
        assert_eq!(size_of::<SrsManifestHeader>(), 512);
        let project_uuid = [0x123456789abcdef0, 0x0fedcba987654321];
        let snapshot_uuid = [0x1111222233334444, 0x5555666677778888];
        let parent_uuid = [0, 0];
        let session_uuid = [0xaaaabbbbccccdddd, 0xeeeeffff00001111];

        let manifest = SrsManifestHeader::new(
            project_uuid,
            snapshot_uuid,
            parent_uuid,
            session_uuid,
            1600000000,
            5,
            1250,
        );

        assert_eq!(manifest.magic, SrsManifestHeader::MAGIC);
        assert_eq!(manifest.project_uuid, project_uuid);
        assert_eq!(manifest.snapshot_uuid, snapshot_uuid);
        assert_eq!(manifest.tile_count, 5);
        assert_eq!(manifest.total_records, 1250);
        assert_eq!(manifest.archive_flags & SrsManifestHeader::FLAG_SNAPSHOT, SrsManifestHeader::FLAG_SNAPSHOT);
    }

    #[test]
    fn test_srs_workspace_record_layout() {
        assert_eq!(size_of::<SrsWorkspaceRecord>(), 64);
        let project_uuid = [0x123456789abcdef0, 0x0fedcba987654321];
        let ws = SrsWorkspaceRecord::new(
            project_uuid,
            999888,
            0b1011,
            [0.25, 0.5, 0.75],
            1,
        );

        assert_eq!(ws.project_uuid, project_uuid);
        assert_eq!(ws.question_hash, 999888);
        assert_eq!(ws.collection_mask, 0b1011);
        assert_eq!(ws.topic_centroid_xyz, [0.25, 0.5, 0.75]);
        assert_eq!(ws.status, 1);
    }
}

