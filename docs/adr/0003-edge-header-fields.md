# ADR-0003: Edge CommonTileHeader integrity fields are normative

Status: Accepted (Ashley Cribb, 2026-10-21)

## Context
S3A-EDGE-PROFILE §2.4.1 and S3A-TILE-SPEC §14.1 listed header bytes 12..16 and 56..64 as reserved (MUST be 0). The reference code (`crates/s3a-edge/src/header.rs`, `CommonTileHeader`) has always stored `data_crc32` (12..16), `header_crc32` (56..60) and `generation` (60..64) there, and every edge file ever written contains non-zero values. A reader written only from the spec would reject or misread every edge tile.

## Decision
Code wins; the spec is edited to match. No on-disk byte changes.
- `data_crc32`: CRC32C over active payload. `header_crc32`: CRC32C over bytes 0..56 ‖ 60..64. `generation`: u32, low 32 bits of the archive generation at seal time. All little-endian.
- When `TILE_FLAG_CRC_CHAIN` (= 2, `header.rs`) is set, desktop/full-engine readers SHOULD verify both CRCs.
- Remaining reserved bytes (`_reserved1`, 6..8): writers MUST zero, readers MUST ignore.

## Consequences
- Edge spec → 1.1-draft, tile spec → 1.4-draft.
- `crates/s3a-edge/tests/header_layout.rs` pins offsets and checks real writer bytes; vectors `spec-vectors/hashes/edge_data_crc32c.json` and `edge_header_crc32c.json` (independently cross-checked with a separate Python CRC32C).
- Gap: `s3a-engine::get_edge_tile` does not verify these CRCs yet (to address with A3 reader hardening).
