//! Display layouts: read-only field maps for fixed-size record types, so a viewer can show named
//! columns instead of hex.
//!
//! This is metadata about structs that already exist. It is **not** used when sealing or
//! writing (that is `standard_schema_for_type` in the engine, whose output is on-disk bytes and
//! must not change), it adds nothing to any file, and it has no effect on the format. A layout
//! describes how to read a record; it never says how to write one.
//!
//! Layouts are built from the real Rust structs with [`display_layout!`], so offsets and sizes
//! cannot drift from the struct without the build or the tests noticing. They reuse
//! [`SchemaTypeDefinition`], the shape viewers already consume.
//!
//! Field type codes are those of the compact schema (`CompactSchemaFieldRecord`): see [`ft`].
//! A layout may skip padding and reserved bytes. It does not say how a NULL is represented:
//! fixed-size row records have no defined NULL encoding in the spec (only columnar tiles do), so
//! `null_allowed` is always `false` here.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use crate::schema::{SchemaFieldDefinition, SchemaTypeDefinition};

#[doc(hidden)]
pub use bytemuck::Zeroable as __Zeroable;

/// Field type codes (same numbering as `CompactSchemaFieldRecord::field_type`).
pub mod ft {
    pub const U8: u16 = 1;
    pub const U16: u16 = 2;
    pub const U32: u16 = 3;
    pub const U64: u16 = 4;
    pub const I8: u16 = 5;
    pub const I16: u16 = 6;
    pub const I32: u16 = 7;
    pub const I64: u16 = 8;
    pub const F32: u16 = 9;
    pub const F64: u16 = 10;
    /// NUL-padded UTF-8 text of `byte_len` bytes.
    pub const STRING: u16 = 11;
    /// 16 bytes in canonical (big-endian text) order, i.e. as `[u8; 16]`.
    pub const UUID: u16 = 12;
    /// Opaque bytes, shown as hex.
    pub const BYTES: u16 = 13;
}

/// Builds a [`SchemaTypeDefinition`] from a `#[repr(C)]` `Pod` struct. Offsets and field sizes
/// are measured from the struct itself.
///
/// ```text
/// display_layout!(MyRecord, 12, 1, "my_record", "my-pack", [
///     (paper_id, ft::U64),
///     (year, ft::U16),
/// ])
/// ```
/// Each `(field, code)` becomes one field (ids 1, 2, ... in the order written). Fields not
/// listed (padding, reserved bytes) are left out.
#[macro_export]
macro_rules! display_layout {
    ($ty:ty, $type_id:expr, $version:expr, $name:expr, $pack:expr, [ $( ($field:ident, $code:expr) ),* $(,)? ]) => {{
        let zeroed: $ty = <$ty as $crate::display_layout::__Zeroable>::zeroed();
        #[allow(unused_mut)]
        let mut fields: $crate::__alloc_vec::Vec<$crate::schema::SchemaFieldDefinition> =
            $crate::__alloc_vec::Vec::new();
        $(
            fields.push($crate::schema::SchemaFieldDefinition {
                field_id: (fields.len() + 1) as u16,
                field_type: $code,
                byte_offset: ::core::mem::offset_of!($ty, $field) as u32,
                byte_len: ::core::mem::size_of_val(&zeroed.$field) as u32,
                null_allowed: false,
                name: $crate::__alloc_string::ToString::to_string(stringify!($field)),
            });
        )*
        $crate::schema::SchemaTypeDefinition {
            type_id: $type_id,
            type_version: $version,
            name: $crate::__alloc_string::ToString::to_string($name),
            owning_pack: $crate::__alloc_string::ToString::to_string($pack),
            record_size: ::core::mem::size_of::<$ty>() as u32,
            fields,
        }
    }};
}

/// Why a layout was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplayLayoutError {
    ZeroRecordSize,
    NoFields,
    NameTooLong(String),
    FieldOutOfBounds(String),
    FieldsOverlap(String, String),
    BadFieldType(String),
    BadFieldLength(String),
    Duplicate {
        type_id: u32,
        type_version: u32,
        record_size: u32,
    },
}

impl fmt::Display for DisplayLayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroRecordSize => write!(f, "record size is zero"),
            Self::NoFields => write!(f, "layout has no fields"),
            Self::NameTooLong(n) => write!(f, "name '{n}' is longer than 32 bytes"),
            Self::FieldOutOfBounds(n) => write!(f, "field '{n}' runs past the record"),
            Self::FieldsOverlap(a, b) => write!(f, "fields '{a}' and '{b}' overlap"),
            Self::BadFieldType(n) => write!(f, "field '{n}' has an unknown type code"),
            Self::BadFieldLength(n) => write!(f, "field '{n}' length does not fit its type"),
            Self::Duplicate {
                type_id,
                type_version,
                record_size,
            } => write!(
                f,
                "a layout for type {type_id} v{type_version} with record size {record_size} is already registered"
            ),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for DisplayLayoutError {}

fn fixed_width(code: u16) -> Option<u32> {
    Some(match code {
        ft::U8 | ft::I8 => 1,
        ft::U16 | ft::I16 => 2,
        ft::U32 | ft::I32 | ft::F32 => 4,
        ft::U64 | ft::I64 | ft::F64 => 8,
        ft::UUID => 16,
        _ => return None,
    })
}

/// Checks that a layout is internally consistent: non-zero size, at least one field, every field
/// inside the record, no overlaps, a known type code, and a length that fits the type.
pub fn validate_layout(def: &SchemaTypeDefinition) -> Result<(), DisplayLayoutError> {
    if def.record_size == 0 {
        return Err(DisplayLayoutError::ZeroRecordSize);
    }
    if def.fields.is_empty() {
        return Err(DisplayLayoutError::NoFields);
    }
    if def.name.len() > 32 || def.owning_pack.len() > 32 {
        return Err(DisplayLayoutError::NameTooLong(def.name.clone()));
    }
    for f in &def.fields {
        if f.name.len() > 32 {
            return Err(DisplayLayoutError::NameTooLong(f.name.clone()));
        }
        let end = f.byte_offset.checked_add(f.byte_len);
        if f.byte_len == 0 || end.is_none_or(|e| e > def.record_size) {
            return Err(DisplayLayoutError::FieldOutOfBounds(f.name.clone()));
        }
        match f.field_type {
            ft::STRING | ft::BYTES => {}
            code => match fixed_width(code) {
                None => return Err(DisplayLayoutError::BadFieldType(f.name.clone())),
                Some(w) if w != f.byte_len => {
                    return Err(DisplayLayoutError::BadFieldLength(f.name.clone()))
                }
                Some(_) => {}
            },
        }
    }
    let mut sorted: Vec<&SchemaFieldDefinition> = def.fields.iter().collect();
    sorted.sort_by_key(|f| f.byte_offset);
    for w in sorted.windows(2) {
        if w[0].byte_offset + w[0].byte_len > w[1].byte_offset {
            return Err(DisplayLayoutError::FieldsOverlap(
                w[0].name.clone(),
                w[1].name.clone(),
            ));
        }
    }
    Ok(())
}

/// A set of display layouts, looked up by tile type (and optionally layout version and record
/// size, since one tile type can hold more than one record size, e.g. vault records in type 64).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DisplayLayoutRegistry {
    layouts: Vec<SchemaTypeDefinition>,
}

impl DisplayLayoutRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds a registry from layouts, validating each.
    pub fn from_layouts(
        layouts: impl IntoIterator<Item = SchemaTypeDefinition>,
    ) -> Result<Self, DisplayLayoutError> {
        let mut r = Self::new();
        for l in layouts {
            r.register(l)?;
        }
        Ok(r)
    }

    /// Adds a layout. Rejects an invalid one, and a second layout for the same
    /// (type, version, record size).
    pub fn register(&mut self, def: SchemaTypeDefinition) -> Result<(), DisplayLayoutError> {
        validate_layout(&def)?;
        if self.layouts.iter().any(|l| {
            l.type_id == def.type_id
                && l.type_version == def.type_version
                && l.record_size == def.record_size
        }) {
            return Err(DisplayLayoutError::Duplicate {
                type_id: def.type_id,
                type_version: def.type_version,
                record_size: def.record_size,
            });
        }
        self.layouts.push(def);
        Ok(())
    }

    pub fn layouts(&self) -> &[SchemaTypeDefinition] {
        &self.layouts
    }

    /// Finds a layout. `version`: exact match when given, otherwise the highest registered.
    /// `record_size`: exact match when given (derive it as `payload_len / record_count`).
    pub fn find(
        &self,
        type_id: u32,
        version: Option<u32>,
        record_size: Option<u32>,
    ) -> Option<&SchemaTypeDefinition> {
        self.layouts
            .iter()
            .filter(|l| {
                l.type_id == type_id
                    && version.is_none_or(|v| l.type_version == v)
                    && record_size.is_none_or(|s| l.record_size == s)
            })
            .max_by_key(|l| l.type_version)
    }
}

/// Display layouts for record types defined in `s3a-core`.
pub fn core_display_layouts() -> Vec<SchemaTypeDefinition> {
    use crate::lrs::StatementIndexRecord;
    alloc::vec![display_layout!(
        StatementIndexRecord,
        crate::TileType::STATEMENT_INDEX,
        1,
        "statement_index",
        "core",
        [
            (statement_id, ft::BYTES),
            (stored_timestamp_ms, ft::U64),
            (timestamp_ms, ft::U64),
            (actor_hash, ft::U64),
            (verb_hash, ft::U32),
            (activity_hash, ft::U32),
            (registration_hash, ft::U32),
            (body_tile_id, ft::U32),
            (body_offset, ft::U32),
            (flags, ft::U32),
        ]
    )]
}
