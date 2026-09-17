#![no_std]

#[cfg(test)]
extern crate std;

use core::fmt;
use core::str::FromStr;
use bytemuck::{Pod, Zeroable};

/// Magic bytes at the start of an S3A file (`S3A1`).
pub const S3A_MAGIC: [u8; 4] = *b"S3A1";

/// S3A file header size (64 bytes).
pub const FILE_HEADER_SIZE: usize = 64;

/// Standard Hyper-Tile block size (128 KB page-aligned block for servers/edge servers).
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
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq, PartialOrd, Ord)]
pub struct S3ACoordinate {
    pub level: u16,        // Stratum level (0 = Wearable Micro-Tile, 1 = Hot L1 cache tile, 2 = Stratified)
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

/// S3A File Header (64 bytes, 8-byte aligned, zero-padded).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct FileHeader {
    pub magic: [u8; 4],
    pub version: u16,
    pub flags: u16,
    pub tile_count: u32,
    pub _reserved1: u32,
    pub created_timestamp: u64,
    pub _reserved2: [u64; 5],
}

impl FileHeader {
    pub fn new(tile_count: u32, created_timestamp: u64) -> Self {
        Self {
            magic: S3A_MAGIC,
            version: 1,
            flags: 0,
            tile_count,
            _reserved1: 0,
            created_timestamp,
            _reserved2: [0u64; 5],
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

/// Simplex convex hull bounding manifold embedded in Hyper-Tile header (212 bytes, 4-byte aligned).
#[repr(C, align(4))]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct SimplexHull {
    pub dim: u32,
    pub radius: f32,
    pub min_bounds: [f32; MAX_HULL_DIMENSIONS],
    pub max_bounds: [f32; MAX_HULL_DIMENSIONS],
    pub centroid: [f32; MAX_HULL_DIMENSIONS],
    pub _reserved: [u32; 3],
}

impl SimplexHull {
    pub fn empty() -> Self {
        Self {
            dim: 0,
            radius: 0.0,
            min_bounds: [f32::INFINITY; MAX_HULL_DIMENSIONS],
            max_bounds: [f32::NEG_INFINITY; MAX_HULL_DIMENSIONS],
            centroid: [0.0; MAX_HULL_DIMENSIONS],
            _reserved: [0u32; 3],
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
}

/// Hyper-Tile Header (512 bytes, 8-byte aligned).
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
    pub _reserved2: [u32; 32],
    pub _reserved3: [u32; 31],
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
            _reserved2: [0u32; 32],
            _reserved3: [0u32; 31],
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
    pub min_timestamp: u32, // Delta-compressed 32-bit seconds timestamp
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
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq)]
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

/// Ultra-Compact Quantized Wearable Telemetry Record (16 bytes) for Heart Rate / IMU / PPG / Gyro in Earbuds & Smart Rings.
#[repr(C, align(4))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct CompactWearableRecord {
    pub timestamp_sec: u32, // Relative seconds timestamp (fits >130 years)
    pub sensor_id: u8,      // Sensor ID (0=PPG HeartRate, 1=Accel, 2=Gyro, 3=Temp, 4=SpO2)
    pub metric_id: u8,      // Metric ID
    pub quantized_val: i16, // Q8.8 fixed-point quantized measurement (value * 256)
    pub flags: u16,         // Compact status flags
    pub _reserved: u16,
    pub _padding: u32,
}

impl CompactWearableRecord {
    pub fn new(timestamp_sec: u32, sensor_id: u8, metric_id: u8, value: f32) -> Self {
        let quantized_val = (value * 256.0) as i16;
        Self {
            timestamp_sec,
            sensor_id,
            metric_id,
            quantized_val,
            flags: 0,
            _reserved: 0,
            _padding: 0,
        }
    }

    pub fn float_value(&self) -> f32 {
        (self.quantized_val as f32) / 256.0
    }
}

/// Compact 32-Dimensional INT8 Quantized Embedding Record (48 bytes) for On-Device Speech/Gesture Keyword Spotting on Glasses/Earbuds.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq)]
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

impl EmbeddingRecord128 {
    pub fn new(id: u64, timestamp: u64, vector: [f32; 128]) -> Self {
        Self { id, timestamp, vector }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::size_of;
    use std::format;

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
        assert_eq!(size_of::<FileHeader>(), FILE_HEADER_SIZE);
        assert_eq!(size_of::<HyperTileHeader>(), HYPER_TILE_HEADER_SIZE);
        assert_eq!(size_of::<TelemetryRecord>(), 32);
        assert_eq!(size_of::<EmbeddingRecord128>(), 528);
        assert_eq!(size_of::<S3ACoordinate>(), 16);
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
