use bytemuck::{bytes_of, cast_slice, Pod, Zeroable};

use crate::S3AError;
use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;
use crc32c::crc32c;

pub const SCHEMA_FORMAT_COMPACT_BINARY: u16 = 1;

/// EMBEDDED SCHEMA REGISTRY HEADER (32 bytes, 8-byte aligned, Pod/Zeroable).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct CompactSchemaHeader {
    pub schema_id: u32,      // Unique schema identifier (e.g. 1)
    pub schema_format: u16,  // 1 = S3ACompactBinary
    pub type_count: u16,     // Number of CompactSchemaTypeRecord entries
    pub total_fields: u32,   // Number of CompactSchemaFieldRecord entries
    pub schema_crc32: u32,   // CRC32C checksum over type records and field records
    pub _reserved: [u8; 16], // Reserved padding
}

/// EMBEDDED SCHEMA TYPE RECORD (80 bytes, 8-byte aligned, Pod/Zeroable).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct CompactSchemaTypeRecord {
    pub type_id: u32,          // Wire tile type identifier
    pub type_version: u32,     // Record layout version
    pub name: [u8; 32],        // UTF-8 canonical name string
    pub owning_pack: [u8; 32], // UTF-8 subsystem or pack name
    pub record_size: u32,      // Byte length per record (0 if variable)
    pub field_count: u16,      // Number of field records following
    pub _reserved: u16,        // Alignment padding
}

/// EMBEDDED SCHEMA FIELD RECORD (48 bytes, 8-byte aligned, Pod/Zeroable).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct CompactSchemaFieldRecord {
    pub field_id: u16,      // 1-based sequential field number
    pub field_type: u16, // 1=u8, 2=u16, 3=u32, 4=u64, 5=i8, 6=i16, 7=i32, 8=i64, 9=f32, 10=f64, 11=string, 12=uuid, 13=bytes
    pub byte_offset: u32, // Offset within record struct
    pub byte_len: u32,   // Byte length of field
    pub null_allowed: u8, // 1 = Nullable, 0 = Required
    pub _reserved: [u8; 3], // Alignment padding
    pub name: [u8; 32],  // UTF-8 attribute name string
}

/// High-level representation of a schema field definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaFieldDefinition {
    pub field_id: u16,
    pub field_type: u16,
    pub byte_offset: u32,
    pub byte_len: u32,
    pub null_allowed: bool,
    pub name: String,
}

/// High-level representation of a registered tile type schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaTypeDefinition {
    pub type_id: u32,
    pub type_version: u32,
    pub name: String,
    pub owning_pack: String,
    pub record_size: u32,
    pub fields: Vec<SchemaFieldDefinition>,
}

pub fn string_to_fixed_32(s: &str) -> [u8; 32] {
    let mut arr = [0u8; 32];
    let bytes = s.as_bytes();
    let len = bytes.len().min(32);
    arr[..len].copy_from_slice(&bytes[..len]);
    arr
}

pub fn fixed_32_to_string(arr: &[u8; 32]) -> String {
    let nul_pos = arr.iter().position(|&b| b == 0).unwrap_or(32);
    core::str::from_utf8(&arr[..nul_pos])
        .unwrap_or("")
        .to_string()
}

/// Encodes a list of schema type definitions into a compact binary schema payload.
pub fn encode_schema_payload(types: &[SchemaTypeDefinition]) -> Vec<u8> {
    let total_fields: usize = types.iter().map(|t| t.fields.len()).sum();
    let mut type_records: Vec<CompactSchemaTypeRecord> = Vec::with_capacity(types.len());
    let mut field_records: Vec<CompactSchemaFieldRecord> = Vec::with_capacity(total_fields);

    for t in types {
        type_records.push(CompactSchemaTypeRecord {
            type_id: t.type_id,
            type_version: t.type_version,
            name: string_to_fixed_32(&t.name),
            owning_pack: string_to_fixed_32(&t.owning_pack),
            record_size: t.record_size,
            field_count: t.fields.len() as u16,
            _reserved: 0,
        });
        for f in &t.fields {
            field_records.push(CompactSchemaFieldRecord {
                field_id: f.field_id,
                field_type: f.field_type,
                byte_offset: f.byte_offset,
                byte_len: f.byte_len,
                null_allowed: if f.null_allowed { 1 } else { 0 },
                _reserved: [0u8; 3],
                name: string_to_fixed_32(&f.name),
            });
        }
    }

    let type_bytes = cast_slice::<CompactSchemaTypeRecord, u8>(&type_records);
    let field_bytes = cast_slice::<CompactSchemaFieldRecord, u8>(&field_records);

    let mut body_bytes = Vec::with_capacity(type_bytes.len() + field_bytes.len());
    body_bytes.extend_from_slice(type_bytes);
    body_bytes.extend_from_slice(field_bytes);

    let schema_crc32 = crc32c(&body_bytes);
    let header = CompactSchemaHeader {
        schema_id: 1,
        schema_format: SCHEMA_FORMAT_COMPACT_BINARY,
        type_count: types.len() as u16,
        total_fields: total_fields as u32,
        schema_crc32,
        _reserved: [0u8; 16],
    };

    let header_bytes = bytes_of(&header);
    let mut out = Vec::with_capacity(header_bytes.len() + body_bytes.len());
    out.extend_from_slice(header_bytes);
    out.extend_from_slice(&body_bytes);
    out
}

/// Decodes a compact binary schema payload into schema type definitions.
pub fn decode_schema_payload(payload: &[u8]) -> Result<Vec<SchemaTypeDefinition>, S3AError> {
    let header_size = core::mem::size_of::<CompactSchemaHeader>();
    if payload.len() < header_size {
        return Err(S3AError::BufferTooSmall {
            required: header_size,
            provided: payload.len(),
        });
    }

    let header: &CompactSchemaHeader =
        &bytemuck::pod_read_unaligned::<CompactSchemaHeader>(&payload[..header_size]);
    if header.schema_format != SCHEMA_FORMAT_COMPACT_BINARY {
        return Err(S3AError::InvalidFormat);
    }

    let type_size = core::mem::size_of::<CompactSchemaTypeRecord>();
    let field_size = core::mem::size_of::<CompactSchemaFieldRecord>();

    let types_len = header.type_count as usize * type_size;
    let fields_len = (header.total_fields as usize)
        .checked_mul(field_size)
        .ok_or(S3AError::OutOfBounds)?;
    let required_body = types_len
        .checked_add(fields_len)
        .ok_or(S3AError::OutOfBounds)?;
    let required_total = header_size
        .checked_add(required_body)
        .ok_or(S3AError::OutOfBounds)?;

    if payload.len() < required_total {
        return Err(S3AError::BufferTooSmall {
            required: required_total,
            provided: payload.len(),
        });
    }

    let body_bytes = &payload[header_size..required_total];
    let actual_crc = crc32c(body_bytes);
    if actual_crc != header.schema_crc32 {
        return Err(S3AError::ChecksumMismatch {
            expected: header.schema_crc32,
            actual: actual_crc,
        });
    }

    // B1: decode records with pod_read_unaligned so a payload at any alignment
    // (mmap offset, fuzz input, network buffer) cannot panic in cast_slice.
    let type_slice: Vec<CompactSchemaTypeRecord> = payload[header_size..header_size + types_len]
        .chunks_exact(type_size)
        .map(bytemuck::pod_read_unaligned::<CompactSchemaTypeRecord>)
        .collect();
    let field_slice: Vec<CompactSchemaFieldRecord> = payload
        [header_size + types_len..required_total]
        .chunks_exact(field_size)
        .map(bytemuck::pod_read_unaligned::<CompactSchemaFieldRecord>)
        .collect();
    let declared: usize = type_slice.iter().map(|t| t.field_count as usize).sum();
    if declared != field_slice.len() {
        return Err(S3AError::InvalidFormat);
    }

    let mut result = Vec::with_capacity(header.type_count as usize);
    let mut field_cursor = 0;

    for tr in &type_slice {
        let f_count = tr.field_count as usize;
        if field_cursor + f_count > field_slice.len() {
            return Err(S3AError::OutOfBounds);
        }
        let fields_for_type = &field_slice[field_cursor..field_cursor + f_count];
        field_cursor += f_count;

        let mut fields = Vec::with_capacity(f_count);
        for fr in fields_for_type {
            fields.push(SchemaFieldDefinition {
                field_id: fr.field_id,
                field_type: fr.field_type,
                byte_offset: fr.byte_offset,
                byte_len: fr.byte_len,
                null_allowed: fr.null_allowed != 0,
                name: fixed_32_to_string(&fr.name),
            });
        }

        result.push(SchemaTypeDefinition {
            type_id: tr.type_id,
            type_version: tr.type_version,
            name: fixed_32_to_string(&tr.name),
            owning_pack: fixed_32_to_string(&tr.owning_pack),
            record_size: tr.record_size,
            fields,
        });
    }

    Ok(result)
}

/// Magic of the optional "about" trailer in a Schema tile payload (TILE-SPEC §14.5.4, item 6).
pub const SCHEMA_ABOUT_MAGIC: [u8; 8] = *b"S3AABOUT";
/// Trailer header size: magic (8) + text_len u32 + text_crc32 u32.
pub const SCHEMA_ABOUT_HEADER_SIZE: usize = 16;

/// Self-description of an archive: which spec it was written against, and by what.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SchemaAbout {
    pub spec_version: String,
    pub spec_url: String,
    /// Lowercase hex SHA-256 of the exact spec text the writer was built with.
    pub spec_sha256: String,
    pub writer: String,
    /// Declared seal hash algorithm ID (`algorithms::ALGORITHMS`); empty = not declared (SHA-256).
    pub seal_hash_alg: String,
    /// Declared signature algorithm ID; empty = not declared.
    pub sig_alg: String,
}

impl SchemaAbout {
    fn text(&self) -> String {
        let mut s = String::new();
        for (k, v) in [
            ("spec_version", &self.spec_version),
            ("spec_url", &self.spec_url),
            ("spec_sha256", &self.spec_sha256),
            ("writer", &self.writer),
            ("seal_hash_alg", &self.seal_hash_alg),
            ("sig_alg", &self.sig_alg),
        ] {
            // Optional keys are written only when set, so older entries keep their bytes.
            if v.is_empty() && (k == "seal_hash_alg" || k == "sig_alg") {
                continue;
            }
            s.push_str(k);
            s.push('=');
            s.push_str(&v.replace(['\n', '\r'], " "));
            s.push('\n');
        }
        s
    }
}

/// Encodes the about trailer: `S3AABOUT | text_len u32 | crc32c(text) u32 | text` (LE).
pub fn encode_schema_about(about: &SchemaAbout) -> Vec<u8> {
    let text = about.text();
    let mut out = Vec::with_capacity(SCHEMA_ABOUT_HEADER_SIZE + text.len());
    out.extend_from_slice(&SCHEMA_ABOUT_MAGIC);
    out.extend_from_slice(&(text.len() as u32).to_le_bytes());
    out.extend_from_slice(&crc32c(text.as_bytes()).to_le_bytes());
    out.extend_from_slice(text.as_bytes());
    out
}

/// Byte length of the schema records (header + types + fields) at the start of `payload`.
pub fn schema_records_len(payload: &[u8]) -> Result<usize, S3AError> {
    let hs = core::mem::size_of::<CompactSchemaHeader>();
    if payload.len() < hs {
        return Err(S3AError::BufferTooSmall {
            required: hs,
            provided: payload.len(),
        });
    }
    let h: CompactSchemaHeader = bytemuck::pod_read_unaligned(&payload[..hs]);
    let total = (h.type_count as usize)
        .checked_mul(core::mem::size_of::<CompactSchemaTypeRecord>())
        .and_then(|t| {
            (h.total_fields as usize)
                .checked_mul(core::mem::size_of::<CompactSchemaFieldRecord>())
                .and_then(|f| t.checked_add(f))
        })
        .and_then(|b| b.checked_add(hs))
        .ok_or(S3AError::OutOfBounds)?;
    if payload.len() < total {
        return Err(S3AError::BufferTooSmall {
            required: total,
            provided: payload.len(),
        });
    }
    Ok(total)
}

/// Reads the optional about trailer that follows the schema records.
/// `Ok(None)` when absent; an error when present but malformed or its CRC fails.
pub fn decode_schema_about(payload: &[u8]) -> Result<Option<SchemaAbout>, S3AError> {
    let start = schema_records_len(payload)?;
    let rest = &payload[start..];
    if rest.len() < SCHEMA_ABOUT_HEADER_SIZE || rest[..8] != SCHEMA_ABOUT_MAGIC {
        return Ok(None);
    }
    let len = u32::from_le_bytes(rest[8..12].try_into().unwrap()) as usize;
    let crc = u32::from_le_bytes(rest[12..16].try_into().unwrap());
    let text = rest
        .get(SCHEMA_ABOUT_HEADER_SIZE..SCHEMA_ABOUT_HEADER_SIZE + len)
        .ok_or(S3AError::OutOfBounds)?;
    if crc32c(text) != crc {
        return Err(S3AError::ChecksumMismatch {
            expected: crc,
            actual: crc32c(text),
        });
    }
    let text = core::str::from_utf8(text).map_err(|_| S3AError::InvalidFormat)?;
    let mut a = SchemaAbout::default();
    for line in text.lines() {
        // Unknown keys are ignored so the entry can grow.
        if let Some((k, v)) = line.split_once('=') {
            let v = v.to_string();
            match k {
                "spec_version" => a.spec_version = v,
                "spec_url" => a.spec_url = v,
                "spec_sha256" => a.spec_sha256 = v,
                "writer" => a.writer = v,
                "seal_hash_alg" => a.seal_hash_alg = v,
                "sig_alg" => a.sig_alg = v,
                _ => {}
            }
        }
    }
    Ok(Some(a))
}

/// The raw, CRC-verified about trailer bytes of a Schema payload, for writers that rebuild
/// the Schema tile and must carry the entry through unchanged (TILE-SPEC §14.5.4).
pub fn schema_about_bytes(payload: &[u8]) -> Option<&[u8]> {
    decode_schema_about(payload).ok().flatten()?;
    let start = schema_records_len(payload).ok()?;
    let len = u32::from_le_bytes(payload[start + 8..start + 12].try_into().ok()?) as usize;
    payload.get(start..start + SCHEMA_ABOUT_HEADER_SIZE + len)
}

#[cfg(test)]
mod about_tests {
    use super::*;

    #[test]
    fn about_round_trip_and_absent() {
        let base = encode_schema_payload(&[]);
        assert_eq!(decode_schema_about(&base).unwrap(), None);
        let a = SchemaAbout {
            spec_version: "1.5-draft".into(),
            spec_url: "https://example.org/spec".into(),
            spec_sha256: "ab".repeat(32),
            writer: "s3a-engine 0.1.0".into(),
            ..Default::default()
        };
        let mut p = base.clone();
        p.extend_from_slice(&encode_schema_about(&a));
        assert_eq!(decode_schema_about(&p).unwrap(), Some(a));
        // Existing decoder still reads the records (trailer is ignored).
        assert!(decode_schema_payload(&p).unwrap().is_empty());
        let n = p.len();
        p[n - 1] ^= 1;
        assert!(decode_schema_about(&p).is_err());
    }
}


// -----------------------------------------------------------------------------
// Quantized Slab Size Classes & Addressing
// -----------------------------------------------------------------------------

/// The 10 quantized slab size classes in bytes (1.5x stepping):
/// [4, 8, 12, 16, 24, 32, 48, 64, 96, 128 KB]
pub const SLAB_CLASS_4KB: u32 = 4 * 1024; // 4,096
pub const SLAB_CLASS_8KB: u32 = 8 * 1024; // 8,192
pub const SLAB_CLASS_12KB: u32 = 12 * 1024; // 12,288
pub const SLAB_CLASS_16KB: u32 = 16 * 1024; // 16,384
pub const SLAB_CLASS_24KB: u32 = 24 * 1024; // 24,576
pub const SLAB_CLASS_32KB: u32 = 32 * 1024; // 32,768
pub const SLAB_CLASS_48KB: u32 = 48 * 1024; // 49,152
pub const SLAB_CLASS_64KB: u32 = 64 * 1024; // 65,536
pub const SLAB_CLASS_96KB: u32 = 96 * 1024; // 98,304
pub const SLAB_CLASS_128KB: u32 = 128 * 1024; // 131,072

pub const NUM_SLAB_CLASSES: usize = 10;
pub const MAX_SLAB_PAYLOAD_SIZE: usize = SLAB_CLASS_128KB as usize;

/// Array of all 10 quantized size classes sorted ascending.
pub const SLAB_SIZE_CLASSES: [u32; NUM_SLAB_CLASSES] = [
    SLAB_CLASS_4KB,
    SLAB_CLASS_8KB,
    SLAB_CLASS_12KB,
    SLAB_CLASS_16KB,
    SLAB_CLASS_24KB,
    SLAB_CLASS_32KB,
    SLAB_CLASS_48KB,
    SLAB_CLASS_64KB,
    SLAB_CLASS_96KB,
    SLAB_CLASS_128KB,
];

/// Selects the smallest quantized size class (0..9) that fits `payload_len` bytes.
/// Returns `None` if `payload_len` exceeds 128 KB (131,072 bytes).
pub fn select_size_class(payload_len: usize) -> Option<u8> {
    for (idx, &class_bytes) in SLAB_SIZE_CLASSES.iter().enumerate() {
        if payload_len <= class_bytes as usize {
            return Some(idx as u8);
        }
    }
    None
}

/// Returns the byte capacity for a given size class ID (0..9),
/// or `None` if `class_id >= 10`.
pub fn size_class_bytes(class_id: u8) -> Option<u32> {
    SLAB_SIZE_CLASSES.get(class_id as usize).copied()
}

pub const MAX_SIZE_CLASS_ID: u8 = (NUM_SLAB_CLASSES - 1) as u8; // 9
pub const MAX_SLAB_EXTENT_ID: u32 = 0x0FFF_FFFF; // 28 bits (268,435,455)
pub const MAX_SLOT_INDEX: u32 = 0x00FF_FFFF; // 24 bits (16,777,215)

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileAddressError {
    InvalidSizeClass(u8),
    ExtentIdOverflow(u32),
    SlotIndexOverflow(u32),
    ReservedNonZero(u8),
}

impl core::fmt::Display for TileAddressError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidSizeClass(c) => write!(f, "Invalid size class ID {c} (max {MAX_SIZE_CLASS_ID})"),
            Self::ExtentIdOverflow(e) => {
                write!(f, "Extent ID {e:#x} overflows 28 bits (max {MAX_SLAB_EXTENT_ID:#x})")
            }
            Self::SlotIndexOverflow(s) => {
                write!(f, "Slot index {s:#x} overflows 24 bits (max {MAX_SLOT_INDEX:#x})")
            }
            Self::ReservedNonZero(r) => write!(f, "Reserved field must be 0, got {r:#x}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for TileAddressError {}

/// 64-bit hardware-aligned global tile address referencing a quantized slab slot:
/// - Bits 63..60 (4 bits): SizeClassId (0..=9)
/// - Bits 59..32 (28 bits): SlabExtentId (0..=0x0FFF_FFFF)
/// - Bits 31..8  (24 bits): SlotIndexInExtent (0..=0x00FF_FFFF)
/// - Bits 7..0   (8 bits): Reserved (must be 0)
#[repr(transparent)]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct TileAddress(pub u64);

impl TileAddress {
    pub const NULL: Self = Self(0);

    /// Packs size class ID, extent ID, and slot index into a TileAddress with bounds checking.
    pub fn new(size_class_id: u8, extent_id: u32, slot_index: u32) -> Result<Self, TileAddressError> {
        if size_class_id > MAX_SIZE_CLASS_ID {
            return Err(TileAddressError::InvalidSizeClass(size_class_id));
        }
        if extent_id > MAX_SLAB_EXTENT_ID {
            return Err(TileAddressError::ExtentIdOverflow(extent_id));
        }
        if slot_index > MAX_SLOT_INDEX {
            return Err(TileAddressError::SlotIndexOverflow(slot_index));
        }
        let raw = ((size_class_id as u64) << 60)
            | ((extent_id as u64) << 32)
            | ((slot_index as u64) << 8);
        Ok(Self(raw))
    }

    /// Unpacks and validates a TileAddress from a raw u64.
    pub fn from_u64(raw: u64) -> Result<Self, TileAddressError> {
        let addr = Self(raw);
        if addr.size_class_id() > MAX_SIZE_CLASS_ID {
            return Err(TileAddressError::InvalidSizeClass(addr.size_class_id()));
        }
        if addr.extent_id() > MAX_SLAB_EXTENT_ID {
            return Err(TileAddressError::ExtentIdOverflow(addr.extent_id()));
        }
        if addr.slot_index() > MAX_SLOT_INDEX {
            return Err(TileAddressError::SlotIndexOverflow(addr.slot_index()));
        }
        if addr.reserved() != 0 {
            return Err(TileAddressError::ReservedNonZero(addr.reserved()));
        }
        Ok(addr)
    }

    #[inline]
    pub const fn as_u64(&self) -> u64 {
        self.0
    }

    #[inline]
    pub const fn size_class_id(&self) -> u8 {
        ((self.0 >> 60) & 0x0F) as u8
    }

    #[inline]
    pub const fn extent_id(&self) -> u32 {
        ((self.0 >> 32) & 0x0FFF_FFFF) as u32
    }

    #[inline]
    pub const fn slot_index(&self) -> u32 {
        ((self.0 >> 8) & 0x00FF_FFFF) as u32
    }

    #[inline]
    pub const fn reserved(&self) -> u8 {
        (self.0 & 0xFF) as u8
    }

    /// Computes the deterministic physical byte offset within the extent file:
    /// PhysicalOffset = 4096 + (slot_index * slot_size_bytes)
    pub fn physical_offset(&self) -> Result<u64, TileAddressError> {
        let class_bytes = size_class_bytes(self.size_class_id())
            .ok_or(TileAddressError::InvalidSizeClass(self.size_class_id()))?;
        Ok(4096 + (self.slot_index() as u64 * class_bytes as u64))
    }
}

#[cfg(test)]
mod slab_address_tests {
    use super::*;

    #[test]
    fn test_tile_address_roundtrip() {
        let addr = TileAddress::new(3, 42, 100).expect("valid address");
        assert_eq!(addr.size_class_id(), 3);
        assert_eq!(addr.extent_id(), 42);
        assert_eq!(addr.slot_index(), 100);
        assert_eq!(addr.reserved(), 0);

        let parsed = TileAddress::from_u64(addr.as_u64()).expect("valid parse");
        assert_eq!(addr, parsed);
    }

    #[test]
    fn test_tile_address_limits() {
        // Max values
        let addr = TileAddress::new(MAX_SIZE_CLASS_ID, MAX_SLAB_EXTENT_ID, MAX_SLOT_INDEX).unwrap();
        assert_eq!(addr.size_class_id(), MAX_SIZE_CLASS_ID);
        assert_eq!(addr.extent_id(), MAX_SLAB_EXTENT_ID);
        assert_eq!(addr.slot_index(), MAX_SLOT_INDEX);

        // Class overflow
        assert!(TileAddress::new(10, 0, 0).is_err());
        // Extent overflow
        assert!(TileAddress::new(0, MAX_SLAB_EXTENT_ID + 1, 0).is_err());
        // Slot overflow
        assert!(TileAddress::new(0, 0, MAX_SLOT_INDEX + 1).is_err());
        // Reserved non-zero
        assert!(TileAddress::from_u64(addr.as_u64() | 0x01).is_err());
    }

    #[test]
    fn test_size_class_selection() {
        assert_eq!(select_size_class(0), Some(0));
        assert_eq!(select_size_class(4096), Some(0));
        assert_eq!(select_size_class(4097), Some(1));
        assert_eq!(select_size_class(8192), Some(1));
        assert_eq!(select_size_class(8193), Some(2));
        assert_eq!(select_size_class(131072), Some(9));
        assert_eq!(select_size_class(131073), None);
    }

    #[test]
    fn test_physical_offset() {
        let a0 = TileAddress::new(0, 0, 0).unwrap();
        assert_eq!(a0.physical_offset().unwrap(), 4096);
        let a1 = TileAddress::new(0, 0, 1).unwrap();
        assert_eq!(a1.physical_offset().unwrap(), 4096 + 4096);
        let a_128k = TileAddress::new(9, 5, 2).unwrap();
        assert_eq!(a_128k.physical_offset().unwrap(), 4096 + 2 * 131072);
    }
}
