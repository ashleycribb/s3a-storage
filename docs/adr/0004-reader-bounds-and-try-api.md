# ADR-0004: Reader bounds checks and `try_*` API (roadmap A3)

Status: Proposed (pending Ashley's review)

## Context
Fuzzing (`fuzz/regressions/tile_header/`) showed `MmapReader` could panic on malformed files:
the edge tile slot check used the file-claimed stride but sliced a fixed 64-byte header
(`slice_oob_lib861`), and `payload_start + payload_len` was unchecked (`add_overflow_lib865`).
The engine also accepted edge tiles without checking their CRCs.

## Decision
- Every tile slot is computed with checked arithmetic and bounds-checked against the real mapped
  file length; a stride smaller than the tile header is rejected; a payload length that does not
  fit inside its tile slot is rejected (`try_get_tile`, `try_get_edge_tile`).
- Existing `get_tile` / `get_edge_tile` / `get_promoted_tile` keep their signatures and return
  `None` on corruption. New `try_get_tile` / `try_get_edge_tile` / `try_get_promoted_tile` return
  `Result<_, Corrupt { tile, reason: CorruptReason }>`.
- When an edge tile has `TILE_FLAG_CRC_CHAIN` (2) set, `try_get_edge_tile` verifies
  `header_crc32` and `data_crc32` (CRC32C) and rejects mismatches.
- File size vs `tile_count`/stride: `open` rejects overflow and impossible strides, but a
  *truncated* file still opens so intact leading tiles stay readable (required by spec vector
  `invalid_truncated_tile`, and in line with "never reject older files"). The mismatch is
  reported by the new `validate_file_size()` and makes `verify_checksums()` fail
  (`S3AError::OutOfBounds`); tiles past EOF return `None` / `SlotPastEof`.

## Not in scope
Unaligned `bytemuck::from_bytes` casts (file header slots, tile headers) are A4. The
`unaligned_from_bytes` regression, and walking tiles of `add_overflow_lib865` (whose claimed edge
stride is not 8-aligned), are `#[ignore = "A4"]`.

## Consequences
No on-disk format change. Edge files with a CRC chain flag and wrong CRCs are now rejected by
`get_edge_tile` (previously accepted silently).
