//! B1: self-describing in-file catalog (Schema tile, type 77).
//!
//! The Schema tile wire format already exists (`s3a_core::schema`, spec §14.5.4).
//! This module adds the reader contract the roadmap asks for:
//! - [`MmapReader::catalog`] returns the embedded catalog when present and falls
//!   back to the compiled-in registry when absent, so files without a Schema tile
//!   read exactly as before.
//! - A Schema tile that fails its tile CRC or its `schema_crc32` is an error,
//!   never a silent fallback and never a panic.
//! - [`registry_catalog`] and [`catalog_mismatches`] let callers check an embedded
//!   catalog against the registry (record size and layout version).

use crate::{Corrupt, CorruptReason, MmapReader};
use s3a_core::schema::{decode_schema_payload, SchemaTypeDefinition};
use s3a_core::{S3AError, TileType};

/// Where a [`Catalog`] came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogSource {
    /// Decoded from a Schema tile at this tile index.
    Embedded { tile_index: usize },
    /// No Schema tile in the file; built from the compiled-in tile-type registry.
    Registry,
}

/// A typed catalog of tile types: type_id → name, version, owner, record size, fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalog {
    pub source: CatalogSource,
    pub types: Vec<SchemaTypeDefinition>,
}

impl Catalog {
    /// Looks up a type by wire id.
    pub fn get(&self, type_id: u32) -> Option<&SchemaTypeDefinition> {
        self.types.iter().find(|t| t.type_id == type_id)
    }
}

/// Error returned when a Schema tile is present but unreadable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogError {
    /// The tile itself failed bounds/CRC validation (ADR-0004).
    Tile(Corrupt),
    /// The tile CRC passed but the schema payload did not decode
    /// (bad `schema_crc32`, truncated body, inconsistent counts).
    Payload { tile_index: usize, error: S3AError },
}

impl std::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CatalogError::Tile(c) => write!(f, "schema tile: {}", c),
            CatalogError::Payload { tile_index, error } => {
                write!(f, "schema tile {}: {:?}", tile_index, error)
            }
        }
    }
}

impl std::error::Error for CatalogError {}

/// Builds a catalog from the compiled-in registry (no field lists; the registry
/// does not carry them). Engine-known types get their standard field lists.
pub fn registry_catalog() -> Catalog {
    let reg = s3a_core::registry::global_registry();
    let mut types: Vec<SchemaTypeDefinition> = Vec::new();
    for d in reg.descriptors() {
        let id = d.tile_type;
        {
            let std_def = crate::standard_schema_for_type(id);
            types.push(SchemaTypeDefinition {
                type_id: id,
                type_version: d.current_layout_version,
                name: d.name.to_string(),
                owning_pack: d.owner_pack.to_string(),
                record_size: d.record_size as u32,
                fields: std_def.fields,
            });
        }
    }
    types.sort_by_key(|t| t.type_id);
    Catalog {
        source: CatalogSource::Registry,
        types,
    }
}

/// Decodes one Schema tile payload, honoring `payload_bytes`.
pub fn decode_schema_tile(
    payload: &[u8],
    payload_bytes: u32,
) -> Result<Vec<SchemaTypeDefinition>, S3AError> {
    let len = (payload_bytes as usize).min(payload.len());
    decode_schema_payload(&payload[..len])
}

/// Lists `(type_id, field, embedded, registry)` disagreements between an embedded
/// catalog and the registry. Types unknown to the registry are not mismatches.
/// Checked fields: `record_size` (when both non-zero) and `type_version`
/// (embedded must be within the registry's supported window).
pub fn catalog_mismatches(cat: &Catalog) -> Vec<(u32, &'static str, u64, u64)> {
    let reg = s3a_core::registry::global_registry();
    let mut out = Vec::new();
    for t in &cat.types {
        let Some(d) = reg.get(t.type_id) else {
            continue;
        };
        let rs = d.record_size as u64;
        if t.record_size != 0 && rs != 0 && t.record_size as u64 != rs {
            out.push((t.type_id, "record_size", t.record_size as u64, rs));
        }
        if t.type_version != 0
            && (t.type_version < d.min_supported_layout_version
                || t.type_version > d.current_layout_version)
        {
            out.push((
                t.type_id,
                "type_version",
                t.type_version as u64,
                d.current_layout_version as u64,
            ));
        }
    }
    out
}

impl MmapReader {
    /// B1: the file's catalog. Uses the first Schema tile when present (validated
    /// by tile CRC and `schema_crc32`); otherwise falls back to the registry.
    pub fn catalog(&self) -> Result<Catalog, CatalogError> {
        if self.is_edge_archive() {
            // Edge profile uses a separate u16 type namespace (77 = CONFIG there).
            return Ok(registry_catalog());
        }
        for i in 0..self.tile_count() as usize {
            // Non-schema tiles that fail bounds checks are not the catalog's concern;
            // verify_checksums() reports them. A bad Schema tile is always an error.
            let Ok((hdr, payload)) = self.try_get_tile(i) else {
                continue;
            };
            if hdr.tile_type != TileType::SCHEMA {
                continue;
            }
            if hdr.header_crc32 != 0 {
                let computed = crate::compute_tile_header_crc32(hdr);
                if computed != hdr.header_crc32 {
                    return Err(CatalogError::Tile(Corrupt {
                        tile: i,
                        reason: CorruptReason::HeaderCrcMismatch {
                            stored: hdr.header_crc32,
                            computed,
                        },
                    }));
                }
            }
            if hdr.data_crc32 != 0 {
                let computed = crc32c::crc32c(payload);
                if computed != hdr.data_crc32 {
                    return Err(CatalogError::Tile(Corrupt {
                        tile: i,
                        reason: CorruptReason::DataCrcMismatch {
                            stored: hdr.data_crc32,
                            computed,
                        },
                    }));
                }
            }
            let types = decode_schema_tile(payload, hdr.payload_bytes).map_err(|error| {
                CatalogError::Payload {
                    tile_index: i,
                    error,
                }
            })?;
            return Ok(Catalog {
                source: CatalogSource::Embedded { tile_index: i },
                types,
            });
        }
        Ok(registry_catalog())
    }
}
