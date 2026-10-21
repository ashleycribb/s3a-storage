# API change: unaligned reads (A4, ADR-0005)

- `s3a_engine::CorruptReason` gains `Misaligned { offset: usize }`. Exhaustive `match`es on
  `CorruptReason` must add an arm. Returned by `try_get_tile` / `try_get_edge_tile` instead of panicking.
- `ArrayTileView::parse` / `ColumnarTileView::parse` return `Err` on misaligned input instead of panicking.
- No other signature changes. Internal reads now use `bytemuck::pod_read_unaligned`. No file format change.
