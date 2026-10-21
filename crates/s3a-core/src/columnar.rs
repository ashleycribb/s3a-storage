//! S3A Columnar Tile (Tile 65 / 0x0041) Layout and Serialization
//!
//! Provides vertically partitioned column-oriented storage for SIMD analytics,
//! min/max statistics pruning, null bitmaps, and zero-copy Apache Arrow projection.

use bytemuck::{Pod, Zeroable};
use sha2::{Digest, Sha256};
use std::io;

/// Primitive data types supported in columnar storage.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnDataType {
    UInt8 = 1,
    UInt16 = 2,
    UInt32 = 3,
    UInt64 = 4,
    Int32 = 5,
    Int64 = 6,
    Float32 = 7,
    Float64 = 8,
}

impl ColumnDataType {
    pub fn byte_size(&self) -> usize {
        match self {
            ColumnDataType::UInt8 => 1,
            ColumnDataType::UInt16 => 2,
            ColumnDataType::UInt32 | ColumnDataType::Int32 | ColumnDataType::Float32 => 4,
            ColumnDataType::UInt64 | ColumnDataType::Int64 | ColumnDataType::Float64 => 8,
        }
    }
}

/// Header preceding the columnar payload (32 bytes, 8-byte aligned, Pod/Zeroable).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct ColumnarTileHeader {
    pub magic: [u8; 4],                 // *b"S3AC"
    pub version: u16,                   // Version 1
    pub column_count: u16,              // Number of columns
    pub record_count: u32,              // Total rows/records
    pub compression_codec: u16,         // 0 = Raw
    pub _reserved16: u16,               // Alignment padding
    pub column_descriptors_offset: u32, // Offset from payload start
    pub null_bitmap_offset: u32,        // Offset from payload start
    pub source_tile_type: u32,          // Source row tile type (e.g. TileType::TELEMETRY)
    pub _reserved: [u8; 4],             // Padding to 32 bytes
}

impl ColumnarTileHeader {
    pub const MAGIC: [u8; 4] = *b"S3AC";
    pub const SIZE: usize = 32;

    pub fn new(column_count: u16, record_count: u32, source_tile_type: u32) -> Self {
        Self {
            magic: Self::MAGIC,
            version: 1,
            column_count,
            record_count,
            compression_codec: 0,
            _reserved16: 0,
            column_descriptors_offset: Self::SIZE as u32,
            null_bitmap_offset: 0,
            source_tile_type,
            _reserved: [0u8; 4],
        }
    }

    pub fn is_valid(&self) -> bool {
        self.magic == Self::MAGIC && self.version == 1
    }
}

/// Metadata descriptor for an individual column (96 bytes, 8-byte aligned, Pod/Zeroable).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq)]
pub struct ColumnDescriptor {
    pub name: [u8; 32],          // Null-terminated UTF-8 column name (offset 0..32)
    pub data_type: u16,          // ColumnDataType enum (offset 32..34)
    pub nullable: u16,           // 1 if column allows nulls, 0 otherwise (offset 34..36)
    pub data_offset: u32,        // Offset in payload where raw values start (offset 36..40)
    pub data_bytes: u32,         // Total byte length of column data (offset 40..44)
    pub null_bitmap_offset: u32, // Offset in payload for this column's null bitmap (offset 44..48)
    pub null_count: u32,         // Number of null values in column (offset 48..52)
    pub _padding: [u8; 4],       // Explicit padding before 8-byte aligned f64 (offset 52..56)
    pub min_value: f64,          // Min observation for statistics pruning (offset 56..64)
    pub max_value: f64,          // Max observation for statistics pruning (offset 64..72)
    pub column_hash: [u8; 32],   // SHA-256 over raw data + null bitmap (offset 72..104)
}

impl ColumnDescriptor {
    pub const SIZE: usize = 104;

    pub fn new(name_str: &str, data_type: ColumnDataType, nullable: bool) -> Self {
        let mut name = [0u8; 32];
        let bytes = name_str.as_bytes();
        let len = bytes.len().min(31);
        name[..len].copy_from_slice(&bytes[..len]);

        Self {
            name,
            data_type: data_type as u16,
            nullable: if nullable { 1 } else { 0 },
            data_offset: 0,
            data_bytes: 0,
            null_bitmap_offset: 0,
            null_count: 0,
            _padding: [0u8; 4],
            min_value: f64::MAX,
            max_value: f64::MIN,
            column_hash: [0u8; 32],
        }
    }

    pub fn name_as_str(&self) -> &str {
        let len = self
            .name
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(self.name.len());
        std::str::from_utf8(&self.name[..len]).unwrap_or("")
    }
}

/// Builder and serializer for Columnar Tiles.
pub struct ColumnarTileBuilder {
    header: ColumnarTileHeader,
    descriptors: Vec<ColumnDescriptor>,
    null_bitmaps: Vec<Vec<u8>>,
    data_buffers: Vec<Vec<u8>>,
}

impl ColumnarTileBuilder {
    pub fn new(record_count: u32, source_tile_type: u32) -> Self {
        Self {
            header: ColumnarTileHeader::new(0, record_count, source_tile_type),
            descriptors: Vec::new(),
            null_bitmaps: Vec::new(),
            data_buffers: Vec::new(),
        }
    }

    /// Adds a column with typed values, optional null mask (1 = valid, 0 = null), and min/max stats.
    pub fn add_column<T: Pod>(
        &mut self,
        name: &str,
        data_type: ColumnDataType,
        values: &[T],
        validity: Option<&[bool]>,
        min_val: f64,
        max_val: f64,
    ) {
        let mut desc = ColumnDescriptor::new(name, data_type, validity.is_some());
        desc.min_value = min_val;
        desc.max_value = max_val;

        let data_raw = bytemuck::cast_slice(values).to_vec();
        desc.data_bytes = data_raw.len() as u32;

        let num_records = self.header.record_count as usize;
        let bitmap_bytes_len = (num_records + 7) / 8;
        let mut null_map = vec![0xFFu8; bitmap_bytes_len]; // Default all valid

        if let Some(v_mask) = validity {
            let mut null_count = 0u32;
            for (idx, &is_valid) in v_mask.iter().enumerate().take(num_records) {
                let byte_idx = idx / 8;
                let bit_idx = idx % 8;
                if !is_valid {
                    null_map[byte_idx] &= !(1 << bit_idx);
                    null_count += 1;
                }
            }
            desc.null_count = null_count;
        }

        // Compute column SHA-256 hash
        let mut hasher = Sha256::new();
        hasher.update(&data_raw);
        hasher.update(&null_map);
        desc.column_hash = hasher.finalize().into();

        self.descriptors.push(desc);
        self.null_bitmaps.push(null_map);
        self.data_buffers.push(data_raw);
        self.header.column_count = self.descriptors.len() as u16;
    }

    /// Serializes the complete Columnar Tile payload.
    pub fn build(mut self) -> Vec<u8> {
        let col_count = self.descriptors.len();
        let desc_array_bytes = col_count * ColumnDescriptor::SIZE;
        let mut current_offset = ColumnarTileHeader::SIZE + desc_array_bytes;

        // Assign null bitmap offsets
        self.header.null_bitmap_offset = current_offset as u32;
        for i in 0..col_count {
            self.descriptors[i].null_bitmap_offset = current_offset as u32;
            current_offset += self.null_bitmaps[i].len();
        }

        // 8-byte align before data buffers
        let align_pad = (8 - (current_offset % 8)) % 8;
        current_offset += align_pad;

        // Assign data buffer offsets
        for i in 0..col_count {
            self.descriptors[i].data_offset = current_offset as u32;
            current_offset += self.data_buffers[i].len();
            let pad = (8 - (current_offset % 8)) % 8;
            current_offset += pad;
        }

        let mut payload = Vec::with_capacity(current_offset);
        // 1. Header
        payload.extend_from_slice(bytemuck::bytes_of(&self.header));
        // 2. Descriptors
        for desc in &self.descriptors {
            payload.extend_from_slice(bytemuck::bytes_of(desc));
        }
        // 3. Null bitmaps
        for bitmap in &self.null_bitmaps {
            payload.extend_from_slice(bitmap);
        }
        // Align
        payload.resize(payload.len() + align_pad, 0);
        // 4. Data buffers
        for (i, data) in self.data_buffers.iter().enumerate() {
            let expected_start = self.descriptors[i].data_offset as usize;
            if payload.len() < expected_start {
                payload.resize(expected_start, 0);
            }
            payload.extend_from_slice(data);
        }

        payload
    }
}

/// Reader view over an on-disk Columnar Tile payload.
pub struct ColumnarTileView<'a> {
    pub header: &'a ColumnarTileHeader,
    pub descriptors: &'a [ColumnDescriptor],
    payload: &'a [u8],
}

impl<'a> ColumnarTileView<'a> {
    pub fn parse(payload: &'a [u8]) -> io::Result<Self> {
        if payload.len() < ColumnarTileHeader::SIZE {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Payload too small for ColumnarTileHeader",
            ));
        }
        let header: &ColumnarTileHeader =
            bytemuck::try_from_bytes::<ColumnarTileHeader>(&payload[..ColumnarTileHeader::SIZE])
                // A4: view keeps a borrowed header (public API); misaligned input is an error, never a panic.
                .map_err(|_| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "misaligned tile header (A4: borrowed view requires 8-byte alignment)",
                    )
                })?;
        if !header.is_valid() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid ColumnarTileHeader magic/version",
            ));
        }

        let desc_start = header.column_descriptors_offset as usize;
        let desc_len = header.column_count as usize * ColumnDescriptor::SIZE;
        if payload.len() < desc_start + desc_len {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Payload truncated before column descriptors",
            ));
        }

        let descriptors: &[ColumnDescriptor] =
            bytemuck::cast_slice(&payload[desc_start..desc_start + desc_len]);
        Ok(Self {
            header,
            descriptors,
            payload,
        })
    }

    /// Finds descriptor for a given column name.
    pub fn find_column(&self, name: &str) -> Option<&'a ColumnDescriptor> {
        self.descriptors.iter().find(|d| d.name_as_str() == name)
    }

    /// Checks if a column's min/max bounds allow pruning for a target range `[query_min, query_max]`.
    /// Returns true if the column can be safely skipped (no possible matches).
    pub fn can_prune_column(&self, name: &str, query_min: f64, query_max: f64) -> bool {
        if let Some(desc) = self.find_column(name) {
            // Disjoint intervals: [min_val, max_val] does not overlap [query_min, query_max]
            desc.max_value < query_min || desc.min_value > query_max
        } else {
            false
        }
    }

    /// Checks if a value at `row_idx` is valid (not null) for column `desc`.
    pub fn is_valid(&self, desc: &ColumnDescriptor, row_idx: usize) -> bool {
        if desc.nullable == 0 || desc.null_count == 0 {
            return true;
        }
        let byte_offset = desc.null_bitmap_offset as usize + (row_idx / 8);
        if byte_offset >= self.payload.len() {
            return false;
        }
        let byte = self.payload[byte_offset];
        (byte & (1 << (row_idx % 8))) != 0
    }

    /// Zero-copy read of a column's raw data slice.
    pub fn read_column_data(&self, desc: &ColumnDescriptor) -> io::Result<&'a [u8]> {
        let start = desc.data_offset as usize;
        let end = start + desc.data_bytes as usize;
        if end > self.payload.len() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Column data truncated",
            ));
        }
        Ok(&self.payload[start..end])
    }

    /// Typed slice access for primitive column data.
    pub fn read_column_slice<T: Pod>(&self, desc: &ColumnDescriptor) -> io::Result<&'a [T]> {
        let bytes = self.read_column_data(desc)?;
        bytemuck::try_cast_slice(bytes).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Cast failed for column slice: {}", e),
            )
        })
    }

    /// Verifies the cryptographic SHA-256 hash of a column descriptor against on-disk bytes.
    pub fn verify_column_hash(&self, desc: &ColumnDescriptor) -> bool {
        let Ok(data) = self.read_column_data(desc) else {
            return false;
        };
        let num_records = self.header.record_count as usize;
        let bitmap_len = (num_records + 7) / 8;
        let bm_start = desc.null_bitmap_offset as usize;
        if bm_start + bitmap_len > self.payload.len() {
            return false;
        }
        let null_map = &self.payload[bm_start..bm_start + bitmap_len];

        let mut hasher = Sha256::new();
        hasher.update(data);
        hasher.update(null_map);
        let actual_hash: [u8; 32] = hasher.finalize().into();
        actual_hash == desc.column_hash
    }
}
