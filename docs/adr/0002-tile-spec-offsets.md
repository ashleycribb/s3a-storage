# ADR-0002: Tile spec offsets follow the on-disk layout

Status: Accepted (Ashley Cribb, Oct 2026)

## Context
`S3A-TILE-SPEC.md` §3.1 placed `filter` at 256 and `provenance` at 384..416, while §3.2.2–§3.2.7 placed them 8 bytes later (264, 392..424, …, learned index at 480..488, colliding with `generation`).

## Decision
The reference code's on-disk layout is authoritative; the spec is edited to match and file bytes never change. Verified with `core::mem::offset_of!` (`crates/s3a-engine/tests/header_layout.rs`) and against tile headers in a generated `spec-vectors/valid_multi_tile.s3a` (filter_type at +256, hash_seed `0x533A` at +258, tile_index at +488, file_uuid at +496). §3.1 was correct; §3.2 was wrong.

Also: all fields little-endian; writers MUST zero reserved bytes, readers MUST ignore them. Hash rules documented in §3.4 with vectors in `spec-vectors/hashes/`.

## Consequences
No format or API change. Layout drift now fails CI. Spec version 1.2-draft → 1.3-draft.
