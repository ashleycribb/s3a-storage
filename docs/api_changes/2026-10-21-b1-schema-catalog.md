# 2026-10-21 — B1: schema tile catalog API

**New (additive):** module `s3a_engine::schema`
- `MmapReader::catalog() -> Result<Catalog, CatalogError>`: embedded Schema tile (77) if present, else `registry_catalog()`.
- `Catalog { source: CatalogSource::{Embedded{tile_index}, Registry}, types }`, `Catalog::get(type_id)`.
- `registry_catalog()`, `catalog_mismatches(&Catalog)`, `decode_schema_tile(payload, payload_bytes)`.

**Changed (stricter):** `s3a_core::schema::decode_schema_payload` now decodes unaligned input without panicking and returns `InvalidFormat` when per-type `field_count`s don't sum to `total_fields`.

**Unchanged:** wire format, `get_schema_definitions`, `resolve_tile_type_name`, writers. Files without a Schema tile read as before.

**Tests:** `crates/s3a-engine/tests/schema_catalog.rs`; vector `spec-vectors/valid_schema_catalog`; fuzz `fuzz/fuzz_targets/schema_tile.rs`. See ADR-0008.
