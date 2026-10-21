# ADR-0008: B1 self-describing catalog uses the existing Schema tile (77)

**Status:** Proposed (2026-10-21)

## Context
Roadmap item B1 asks for an in-file catalog (like SQLite's `sqlite_schema` or Postgres `pg_class`/`pg_attribute`) and suggested taking "the next free core type_id" with a new `SchemaRecord` layout. On inspection, main already has Schema tile type 77 (spec §14.5.4), a compact binary codec in `s3a-core/src/schema.rs`, a writer (`append_schema_tile_if_missing`, compaction), and a reader (`get_schema_definitions`). Gaps found:
1. `decode_schema_payload` used `bytemuck::cast_slice`, which panics on a misaligned payload.
2. It did not check that per-type `field_count`s add up to `total_fields`.
3. `get_schema_definitions` silently ignored a corrupt Schema tile and returned an empty list.
4. No registry fallback API, no registry consistency check, no spec-vector, no fuzz target.
5. The spec text for the payload did not match the bytes the code writes. 77 was missing from `registry.md`.

## Decision
Keep type 77 and its existing wire format (conservative: files already written stay valid, no new type ID). Add `s3a_engine::schema` with `MmapReader::catalog()` (embedded first, registry fallback, hard error on a bad Schema tile), `registry_catalog()`, and `catalog_mismatches()`. Harden the decoder (unaligned reads, count check, overflow-safe sizes). Fix the spec text to match the code. Add 77 to `registry.md`.

## Alternatives
- New type ID plus the roadmap's `SchemaRecord` layout: two catalog formats for one job, and existing files with tile 77 would be orphaned. Rejected.
- Make the catalog required now: breaks every existing file. Deferred to v2.0 (roadmap Q12).

## Consequences
- No on-disk bytes change. Old files read identically and fall back to the registry.
- Decoding is stricter: a payload whose `field_count`s don't add up to `total_fields` used to decode partially and is now rejected. The reference writer never produced such payloads.
- Not done here: a TileWriter builder flag that auto-emits the catalog on every commit; registering 77 as a descriptor in `s3a-core/src/registry.rs`; signing the catalog beyond the existing seal path (Schema tiles are sealed like any tile when `seal_file` runs).
