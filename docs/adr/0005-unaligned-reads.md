# ADR-0005: Unaligned reads of on-disk structs (A4)

Status: Accepted (pending review by Ashley Cribb)

## Context
`bytemuck::from_bytes` / `cast_ref` panic with `TargetAlignmentGreaterAndInputNotAligned` when the
input slice is not aligned for the target `Pod` type. File, flash, stack and `Vec<u8>` buffers carry no
alignment guarantee, so a valid file (or a hostile one) could crash readers and the edge writer.
Fuzzing found this (`fuzz/regressions/tile_header/unaligned_from_bytes.bin`).

## Decision
- Structs read from file/flash/buffer bytes are **copied out** with `bytemuck::pod_read_unaligned`
  (no_std-compatible). Applied across the workspace (engine, edge writer, spec-vectors, core, vcs,
  vault, lrs-server, pack-scholar, cli, wasm-snapshot).
- Writes continue to use `bytes_of` on an owned value.
- Public APIs that return a header **borrowed** from the buffer keep their signatures and use
  `bytemuck::try_from_bytes`, turning misalignment into an error instead of a panic:
  - `MmapReader::try_get_tile` / `try_get_edge_tile` (and `get_*`): mmap bases are page-aligned and
    canonical strides are multiples of 8, so well-formed files are always aligned. A file-claimed
    non-multiple-of-8 edge stride now yields `CorruptReason::Misaligned { offset }`.
  - `ArrayTileView::parse` / `ColumnarTileView::parse` (public `header: &'a T` field): misaligned
    payload returns `Err`.
- `from_bytes_mut` in `sealed.rs` / `pack-scholar recheck.rs` mutate headers in owned `Vec<u8>` buffers
  at offset 0/fixed slot offsets (allocator-aligned); left unchanged, follow-up to convert to
  read-modify-`copy_from_slice` if ever needed.
- No on-disk format change. All multi-byte values remain little-endian (ADR-0002).
