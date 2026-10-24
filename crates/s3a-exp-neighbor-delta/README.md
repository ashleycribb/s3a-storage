# s3a-exp-neighbor-delta (EXPERIMENT)

Lossless neighbour-residual coding of Hyper-Tile payloads, then zstd level 3. Inspired by CliffordNet's differential context (C = C_loc - H, arXiv 2601.06793 §3.4). Does **not** touch s3a-core or the S3A format; it only reads core constants. Current S3A codec is Raw (compression_codec = 0), so "raw bytes" is today's on-disk size and `None` is plain zstd.

Predictors: None; PrevTile (t - t-1); NeighborTileMean (t - mean(t-1, t-2), Laplacian-style); NeighborValueMean (x[i] - mean(x[i-1], x[i-2]) in-tile). Integers use wrapping subtraction; f32 uses XOR of bit patterns so NaN/inf/-0.0 are bit-exact.

Run: `cargo run --release -p s3a-exp-neighbor-delta --example bench [-- copy_of_store.s3a]`

## Results (box, single run, debug-free release build)

| dataset | raw bytes | predictor | zstd-3 bytes | ratio vs none | enc MB/s | dec MB/s |
|---|---|---|---|---|---|---|
| synthetic smooth f32 | 4177920 | None | 3469718 | 1.000 | 290 | 279 |
| synthetic smooth f32 | 4177920 | PrevTile | 3478844 | 1.003 | 188 | 265 |
| synthetic smooth f32 | 4177920 | NeighborTileMean | 3568596 | 1.028 | 271 | 254 |
| synthetic smooth f32 | 4177920 | NeighborValueMean | 2395349 | 0.690 | 149 | 234 |
| synthetic noisy f32 | 4177920 | None | 3749836 | 1.000 | 428 | 310 |
| synthetic noisy f32 | 4177920 | PrevTile | 3803677 | 1.014 | 412 | 275 |
| synthetic noisy f32 | 4177920 | NeighborTileMean | 3842275 | 1.025 | 384 | 269 |
| synthetic noisy f32 | 4177920 | NeighborValueMean | 3844629 | 1.025 | 415 | 253 |
| synthetic sensor f32 (slow drift + 1% noise, quantised 1/256) | 4177920 | None | 970660 | 1.000 | 237 | 297 |
| synthetic sensor f32 (slow drift + 1% noise, quantised 1/256) | 4177920 | PrevTile | 1218946 | 1.256 | 195 | 278 |
| synthetic sensor f32 (slow drift + 1% noise, quantised 1/256) | 4177920 | NeighborTileMean | 1338826 | 1.379 | 197 | 258 |
| synthetic sensor f32 (slow drift + 1% noise, quantised 1/256) | 4177920 | NeighborValueMean | 1124974 | 1.159 | 235 | 282 |
| synthetic counter u32 | 4177920 | None | 2876151 | 1.000 | 378 | 277 |
| synthetic counter u32 | 4177920 | PrevTile | 87713 | 0.030 | 792 | 372 |
| synthetic counter u32 | 4177920 | NeighborTileMean | 87719 | 0.030 | 790 | 331 |
| synthetic counter u32 | 4177920 | NeighborValueMean | 613 | 0.000 | 768 | 311 |
| legacy_v1_0_telemetry.s3a as U8 | 261120 | None | 136 | 1.000 | 552 | 175 |
| legacy_v1_0_telemetry.s3a as U8 | 261120 | PrevTile | 136 | 1.000 | 469 | 171 |
| legacy_v1_0_telemetry.s3a as U8 | 261120 | NeighborTileMean | 136 | 1.000 | 433 | 162 |
| legacy_v1_0_telemetry.s3a as U8 | 261120 | NeighborValueMean | 177 | 1.301 | 453 | 161 |
| legacy_v1_0_telemetry.s3a as U32 | 261120 | None | 136 | 1.000 | 1545 | 742 |
| legacy_v1_0_telemetry.s3a as U32 | 261120 | PrevTile | 136 | 1.000 | 1542 | 691 |
| legacy_v1_0_telemetry.s3a as U32 | 261120 | NeighborTileMean | 136 | 1.000 | 1372 | 676 |
| legacy_v1_0_telemetry.s3a as U32 | 261120 | NeighborValueMean | 251 | 1.846 | 1286 | 606 |
| legacy_v1_0_telemetry.s3a as F32 | 261120 | None | 136 | 1.000 | 1510 | 737 |
| legacy_v1_0_telemetry.s3a as F32 | 261120 | PrevTile | 136 | 1.000 | 1315 | 683 |
| legacy_v1_0_telemetry.s3a as F32 | 261120 | NeighborTileMean | 136 | 1.000 | 1406 | 702 |
| legacy_v1_0_telemetry.s3a as F32 | 261120 | NeighborValueMean | 233 | 1.713 | 1303 | 539 |
| pre_change_tile18_fixture.s3a as U8 | 130560 | None | 84 | 1.000 | 593 | 573 |
| pre_change_tile18_fixture.s3a as U8 | 130560 | PrevTile | 84 | 1.000 | 524 | 509 |
| pre_change_tile18_fixture.s3a as U8 | 130560 | NeighborTileMean | 84 | 1.000 | 455 | 341 |
| pre_change_tile18_fixture.s3a as U8 | 130560 | NeighborValueMean | 86 | 1.024 | 368 | 327 |
| pre_change_tile18_fixture.s3a as U32 | 130560 | None | 84 | 1.000 | 1249 | 1051 |
| pre_change_tile18_fixture.s3a as U32 | 130560 | PrevTile | 84 | 1.000 | 812 | 936 |
| pre_change_tile18_fixture.s3a as U32 | 130560 | NeighborTileMean | 84 | 1.000 | 800 | 1131 |
| pre_change_tile18_fixture.s3a as U32 | 130560 | NeighborValueMean | 90 | 1.071 | 1081 | 1129 |
| pre_change_tile18_fixture.s3a as F32 | 130560 | None | 84 | 1.000 | 1322 | 1453 |
| pre_change_tile18_fixture.s3a as F32 | 130560 | PrevTile | 84 | 1.000 | 1248 | 1355 |
| pre_change_tile18_fixture.s3a as F32 | 130560 | NeighborTileMean | 84 | 1.000 | 1081 | 1144 |
| pre_change_tile18_fixture.s3a as F32 | 130560 | NeighborValueMean | 91 | 1.083 | 1096 | 616 |
