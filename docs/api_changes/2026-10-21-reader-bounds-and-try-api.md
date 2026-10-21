# Reader bounds checks and try_* API (A3, ADR-0004)

- No on-disk change. Existing `MmapReader::get_tile` / `get_edge_tile` / `get_promoted_tile` signatures unchanged; they now return `None` instead of panicking on out-of-bounds or overflowing lengths.
- Added: `MmapReader::try_get_tile`, `try_get_edge_tile`, `try_get_promoted_tile`, `validate_file_size`; types `Corrupt { tile, reason }` and `CorruptReason`.
- Behavior: `get_edge_tile` now rejects tiles with `TILE_FLAG_CRC_CHAIN` whose `header_crc32`/`data_crc32` don't match; `verify_checksums` fails on files shorter than `tile_count` × stride; `open` rejects tile stride < tile header size and overflowing `tile_count × stride`.
- New tests: `s3a-engine/tests/reader_bounds_a3.rs`.
