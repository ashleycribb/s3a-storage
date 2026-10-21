//! N-Dimensional Array Tile (Tile 66 / 0x0042)
//!
//! Native storage for dense multi-dimensional tensors, volumetric imaging cubes,
//! and numerical simulation grids with explicit spatial dimensionality, units,
//! and chunk coordinates.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
use bytemuck::{Pod, Zeroable};
use sha2::{Digest, Sha256};

/// Maximum number of tensor dimensions supported by S3A Array tiles.
pub const MAX_ARRAY_DIMENSIONS: usize = 8;

/// Primitive numerical data types supported by Array tiles.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrayDataType {
    Int8 = 1,
    UInt8 = 2,
    Int16 = 3,
    UInt16 = 4,
    Int32 = 5,
    UInt32 = 6,
    Float32 = 7,
    Float64 = 8,
    Complex64 = 9,
}

impl ArrayDataType {
    pub fn element_size(&self) -> usize {
        match self {
            ArrayDataType::Int8 | ArrayDataType::UInt8 => 1,
            ArrayDataType::Int16 | ArrayDataType::UInt16 => 2,
            ArrayDataType::Int32 | ArrayDataType::UInt32 | ArrayDataType::Float32 => 4,
            ArrayDataType::Float64 | ArrayDataType::Complex64 => 8,
        }
    }

    pub fn from_u16(val: u16) -> Option<Self> {
        match val {
            1 => Some(ArrayDataType::Int8),
            2 => Some(ArrayDataType::UInt8),
            3 => Some(ArrayDataType::Int16),
            4 => Some(ArrayDataType::UInt16),
            5 => Some(ArrayDataType::Int32),
            6 => Some(ArrayDataType::UInt32),
            7 => Some(ArrayDataType::Float32),
            8 => Some(ArrayDataType::Float64),
            9 => Some(ArrayDataType::Complex64),
            _ => None,
        }
    }
}

/// Header preceding the Array Tile payload (320 bytes, 8-byte aligned, Pod/Zeroable).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct ArrayTileHeader {
    pub magic: [u8; 4],                            // *b"S3AA" (offset 0..4)
    pub version: u16,                              // Version 1 (offset 4..6)
    pub ndim: u16,                                 // Dimensionality 1..8 (offset 6..8)
    pub dtype: u16,                                // ArrayDataType enum (offset 8..10)
    pub compression_codec: u16,                    // 0 = Raw (offset 10..12)
    pub _reserved16: u32,                          // Alignment padding (offset 12..16)
    pub shape: [u64; MAX_ARRAY_DIMENSIONS], // Tensor dimensions of this chunk (offset 16..80)
    pub chunk_shape: [u64; MAX_ARRAY_DIMENSIONS], // Standard chunk grid shape (offset 80..144)
    pub chunk_coords: [u64; MAX_ARRAY_DIMENSIONS], // Coordinate offset in global array (offset 144..208)
    pub fill_value: [u8; 8], // Default value for unwritten chunks (offset 208..216)
    pub chunk_hash: [u8; 32], // SHA-256 over active chunk payload bytes (offset 216..248)
    pub axis_names: [u8; 64], // Null-terminated comma-separated axis names (offset 248..312)
    pub _reserved: [u8; 8],  // Padding to 320 bytes (offset 312..320)
}

impl ArrayTileHeader {
    pub const MAGIC: [u8; 4] = *b"S3AA";
    pub const SIZE: usize = 320;

    pub fn new(
        ndim: usize,
        dtype: ArrayDataType,
        shape: &[u64],
        chunk_coords: &[u64],
    ) -> Result<Self, &'static str> {
        if ndim == 0 || ndim > MAX_ARRAY_DIMENSIONS {
            return Err("ndim must be between 1 and 8");
        }
        if shape.len() < ndim || chunk_coords.len() < ndim {
            return Err("shape and chunk_coords length must match ndim");
        }

        let mut fixed_shape = [0u64; MAX_ARRAY_DIMENSIONS];
        let mut fixed_chunk_coords = [0u64; MAX_ARRAY_DIMENSIONS];
        for i in 0..ndim {
            if shape[i] == 0 {
                return Err("shape dimensions must be greater than zero");
            }
            fixed_shape[i] = shape[i];
            fixed_chunk_coords[i] = chunk_coords[i];
        }

        Ok(Self {
            magic: Self::MAGIC,
            version: 1,
            ndim: ndim as u16,
            dtype: dtype as u16,
            compression_codec: 0,
            _reserved16: 0,
            shape: fixed_shape,
            chunk_shape: fixed_shape,
            chunk_coords: fixed_chunk_coords,
            fill_value: [0u8; 8],
            chunk_hash: [0u8; 32],
            axis_names: [0u8; 64],
            _reserved: [0u8; 8],
        })
    }

    pub fn is_valid(&self) -> bool {
        self.magic == Self::MAGIC
            && self.version == 1
            && self.ndim >= 1
            && (self.ndim as usize) <= MAX_ARRAY_DIMENSIONS
    }

    pub fn total_elements(&self) -> usize {
        let mut count = 1usize;
        for i in 0..self.ndim as usize {
            count = count.saturating_mul(self.shape[i] as usize);
        }
        count
    }

    pub fn element_size(&self) -> usize {
        ArrayDataType::from_u16(self.dtype)
            .map(|d| d.element_size())
            .unwrap_or(1)
    }

    pub fn expected_payload_bytes(&self) -> usize {
        self.total_elements() * self.element_size()
    }

    pub fn set_axis_names(&mut self, names: &str) {
        let bytes = names.as_bytes();
        let len = bytes.len().min(63);
        self.axis_names[..len].copy_from_slice(&bytes[..len]);
        self.axis_names[len] = 0;
    }

    pub fn axis_names_as_str(&self) -> &str {
        let len = self
            .axis_names
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(self.axis_names.len());
        core::str::from_utf8(&self.axis_names[..len]).unwrap_or("")
    }
}

/// Builder for constructing an Array tile payload (Header + contiguous C-order element bytes).
pub struct ArrayTileBuilder {
    pub header: ArrayTileHeader,
    pub data: Vec<u8>,
}

impl ArrayTileBuilder {
    pub fn new(
        ndim: usize,
        dtype: ArrayDataType,
        shape: &[u64],
        chunk_coords: &[u64],
    ) -> Result<Self, &'static str> {
        let header = ArrayTileHeader::new(ndim, dtype, shape, chunk_coords)?;
        Ok(Self {
            header,
            data: Vec::new(),
        })
    }

    pub fn set_fill_value<T: Pod>(&mut self, fill: T) -> Result<(), &'static str> {
        let bytes = bytemuck::bytes_of(&fill);
        if bytes.len() > 8 {
            return Err("fill value exceeds 8 bytes");
        }
        self.header.fill_value = [0u8; 8];
        self.header.fill_value[..bytes.len()].copy_from_slice(bytes);
        Ok(())
    }

    pub fn set_axis_names(&mut self, names: &str) {
        self.header.set_axis_names(names);
    }

    pub fn set_data<T: Pod>(&mut self, elements: &[T]) -> Result<(), &'static str> {
        let expected_elems = self.header.total_elements();
        if elements.len() != expected_elems {
            return Err("element count does not match array shape");
        }
        let raw = bytemuck::cast_slice(elements);
        let expected_bytes = self.header.expected_payload_bytes();
        if raw.len() != expected_bytes {
            return Err("byte length mismatch for array dtype");
        }

        let mut hasher = Sha256::new();
        hasher.update(raw);
        self.header.chunk_hash = hasher.finalize().into();

        self.data = raw.to_vec();
        Ok(())
    }

    pub fn set_raw_data(&mut self, raw: &[u8]) -> Result<(), &'static str> {
        let expected_bytes = self.header.expected_payload_bytes();
        if raw.len() != expected_bytes {
            return Err("byte length mismatch for array shape and dtype");
        }
        let mut hasher = Sha256::new();
        hasher.update(raw);
        self.header.chunk_hash = hasher.finalize().into();
        self.data = raw.to_vec();
        Ok(())
    }

    pub fn build(self) -> Vec<u8> {
        let mut payload = Vec::with_capacity(ArrayTileHeader::SIZE + self.data.len());
        payload.extend_from_slice(bytemuck::bytes_of(&self.header));
        payload.extend_from_slice(&self.data);
        payload
    }
}

/// Reader view over an on-disk Array Tile payload.
pub struct ArrayTileView<'a> {
    pub header: &'a ArrayTileHeader,
    pub data: &'a [u8],
}

impl<'a> ArrayTileView<'a> {
    pub fn parse(payload: &'a [u8]) -> Result<Self, &'static str> {
        if payload.len() < ArrayTileHeader::SIZE {
            return Err("payload too small for ArrayTileHeader");
        }
        let header: &ArrayTileHeader =
            bytemuck::try_from_bytes::<ArrayTileHeader>(&payload[..ArrayTileHeader::SIZE])
                // A4: view keeps a borrowed header (public API); misaligned input is an error, never a panic.
                .map_err(|_| {
                    "misaligned tile header (A4: borrowed view requires 8-byte alignment)"
                })?;
        if !header.is_valid() {
            return Err("invalid ArrayTileHeader magic or version");
        }
        let expected_bytes = header.expected_payload_bytes();
        let data = &payload[ArrayTileHeader::SIZE..];
        if data.len() < expected_bytes {
            return Err("payload truncated: fewer data bytes than declared by array shape");
        }
        Ok(Self {
            header,
            data: &data[..expected_bytes],
        })
    }

    pub fn read_elements<T: Pod>(&self) -> Result<&'a [T], &'static str> {
        bytemuck::try_cast_slice(self.data).map_err(|_| "cast failed for array elements")
    }

    pub fn verify_chunk_hash(&self) -> bool {
        let mut hasher = Sha256::new();
        hasher.update(self.data);
        let actual_hash: [u8; 32] = hasher.finalize().into();
        actual_hash == self.header.chunk_hash
    }

    /// Linear index in row-major (C-order) for coordinates [c_0, c_1, ...].
    pub fn linear_index(&self, local_coords: &[u64]) -> Option<usize> {
        let ndim = self.header.ndim as usize;
        if local_coords.len() < ndim {
            return None;
        }
        let mut index = 0usize;
        for i in 0..ndim {
            let coord = local_coords[i];
            let dim_size = self.header.shape[i];
            if coord >= dim_size {
                return None;
            }
            index = index * (dim_size as usize) + (coord as usize);
        }
        Some(index)
    }

    /// Read an individual element at local chunk coordinates.
    pub fn get_element<T: Pod>(&self, local_coords: &[u64]) -> Option<T> {
        let idx = self.linear_index(local_coords)?;
        let slice: &[T] = self.read_elements().ok()?;
        slice.get(idx).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_array_tile_header_size_and_alignment() {
        assert_eq!(core::mem::size_of::<ArrayTileHeader>(), 320);
        assert_eq!(core::mem::align_of::<ArrayTileHeader>(), 8);
    }
}
