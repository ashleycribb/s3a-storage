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
