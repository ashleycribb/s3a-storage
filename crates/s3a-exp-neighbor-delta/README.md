# s3a-exp-neighbor-delta (EXPERIMENT)

Lossless neighbour-residual coding of Hyper-Tile payloads, then zstd level 3. Inspired by CliffordNet's differential context (C = C_loc - H, arXiv 2601.06793 §3.4). Does **not** touch s3a-core or the S3A format; it only reads core constants. Current S3A codec is Raw (compression_codec = 0), so "raw bytes" is today's on-disk size and `None` is plain zstd.

Predictors: None; PrevTile (t - t-1); NeighborTileMean (t - mean(t-1, t-2), Laplacian-style); NeighborValueMean (x[i] - mean(x[i-1], x[i-2]) in-tile); ReferenceTile (t - tile[0]). Integers use wrapping subtraction; f32 uses XOR of bit patterns so NaN/inf/-0.0 are bit-exact.

Run: `cargo run --release -p s3a-exp-neighbor-delta --example bench [-- copy_of_store.s3a]`

## DNA/RNA mapping (analogies only, not biology claims)

| Biology | Here |
|---|---|
| Reference-based genome compression (CRAM / VCF variants vs a reference) | `ReferenceTile` and neighbour predictors store residuals vs a reference/neighbour tile |
| Transcription: DNA stays put, a working RNA copy is made | `encode` borrows tiles immutably; `decode` builds fresh working copies (test `originals_never_mutated`) |
| Complementary strand / mismatch repair | per-tile CRC32C of the original stored in the stream; decode rejects on mismatch (`ChecksumMismatch`) |
| Codon reading frame | fixed-width residuals (1 or 4 bytes) at fixed offsets, so decoding cannot drift out of frame |

## Results (box, one run each, release build, zstd level 3)

| dataset | raw bytes | predictor | zstd-3 bytes | ratio vs none | enc MB/s | dec MB/s |
|---|---|---|---|---|---|---|
| synthetic smooth f32 | 4177920 | None | 3469846 | 1.000 | 223 | 235 |
| synthetic smooth f32 | 4177920 | PrevTile | 3478972 | 1.003 | 216 | 230 |
| synthetic smooth f32 | 4177920 | NeighborTileMean | 3568724 | 1.028 | 241 | 233 |
| synthetic smooth f32 | 4177920 | NeighborValueMean | 2395477 | 0.690 | 140 | 227 |
| synthetic smooth f32 | 4177920 | ReferenceTile | 3555366 | 1.025 | 302 | 237 |
| synthetic noisy f32 | 4177920 | None | 3749964 | 1.000 | 378 | 250 |
| synthetic noisy f32 | 4177920 | PrevTile | 3803805 | 1.014 | 344 | 256 |
| synthetic noisy f32 | 4177920 | NeighborTileMean | 3842403 | 1.025 | 305 | 258 |
| synthetic noisy f32 | 4177920 | NeighborValueMean | 3844757 | 1.025 | 318 | 243 |
| synthetic noisy f32 | 4177920 | ReferenceTile | 3803549 | 1.014 | 386 | 240 |
| synthetic sensor f32 (slow drift + 1% noise, quantised 1/256) | 4177920 | None | 970788 | 1.000 | 224 | 242 |
| synthetic sensor f32 (slow drift + 1% noise, quantised 1/256) | 4177920 | PrevTile | 1219074 | 1.256 | 197 | 241 |
| synthetic sensor f32 (slow drift + 1% noise, quantised 1/256) | 4177920 | NeighborTileMean | 1338954 | 1.379 | 185 | 244 |
| synthetic sensor f32 (slow drift + 1% noise, quantised 1/256) | 4177920 | NeighborValueMean | 1125102 | 1.159 | 240 | 236 |
| synthetic sensor f32 (slow drift + 1% noise, quantised 1/256) | 4177920 | ReferenceTile | 1228957 | 1.266 | 193 | 248 |
| synthetic counter u32 | 4177920 | None | 2876279 | 1.000 | 419 | 276 |
| synthetic counter u32 | 4177920 | PrevTile | 87841 | 0.031 | 710 | 335 |
| synthetic counter u32 | 4177920 | NeighborTileMean | 87847 | 0.031 | 810 | 294 |
| synthetic counter u32 | 4177920 | NeighborValueMean | 741 | 0.000 | 753 | 330 |
| synthetic counter u32 | 4177920 | ReferenceTile | 88017 | 0.031 | 939 | 302 |
| legacy_v1_0_telemetry.s3a as U8 | 261120 | None | 144 | 1.000 | 356 | 144 |
| legacy_v1_0_telemetry.s3a as U8 | 261120 | PrevTile | 144 | 1.000 | 322 | 135 |
| legacy_v1_0_telemetry.s3a as U8 | 261120 | NeighborTileMean | 144 | 1.000 | 292 | 136 |
| legacy_v1_0_telemetry.s3a as U8 | 261120 | NeighborValueMean | 185 | 1.285 | 311 | 152 |
| legacy_v1_0_telemetry.s3a as U8 | 261120 | ReferenceTile | 144 | 1.000 | 306 | 157 |
| legacy_v1_0_telemetry.s3a as U32 | 261120 | None | 144 | 1.000 | 1103 | 630 |
| legacy_v1_0_telemetry.s3a as U32 | 261120 | PrevTile | 144 | 1.000 | 1075 | 523 |
| legacy_v1_0_telemetry.s3a as U32 | 261120 | NeighborTileMean | 144 | 1.000 | 965 | 548 |
| legacy_v1_0_telemetry.s3a as U32 | 261120 | NeighborValueMean | 259 | 1.799 | 856 | 472 |
| legacy_v1_0_telemetry.s3a as U32 | 261120 | ReferenceTile | 144 | 1.000 | 1023 | 558 |
| legacy_v1_0_telemetry.s3a as F32 | 261120 | None | 144 | 1.000 | 1104 | 542 |
| legacy_v1_0_telemetry.s3a as F32 | 261120 | PrevTile | 144 | 1.000 | 992 | 587 |
| legacy_v1_0_telemetry.s3a as F32 | 261120 | NeighborTileMean | 144 | 1.000 | 1099 | 577 |
| legacy_v1_0_telemetry.s3a as F32 | 261120 | NeighborValueMean | 241 | 1.674 | 1001 | 471 |
| legacy_v1_0_telemetry.s3a as F32 | 261120 | ReferenceTile | 144 | 1.000 | 1182 | 612 |
| pre_change_tile18_fixture.s3a as U8 | 130560 | None | 88 | 1.000 | 455 | 437 |
| pre_change_tile18_fixture.s3a as U8 | 130560 | PrevTile | 88 | 1.000 | 302 | 346 |
| pre_change_tile18_fixture.s3a as U8 | 130560 | NeighborTileMean | 88 | 1.000 | 342 | 367 |
| pre_change_tile18_fixture.s3a as U8 | 130560 | NeighborValueMean | 90 | 1.023 | 343 | 341 |
| pre_change_tile18_fixture.s3a as U8 | 130560 | ReferenceTile | 88 | 1.000 | 384 | 416 |
| pre_change_tile18_fixture.s3a as U32 | 130560 | None | 88 | 1.000 | 1025 | 1042 |
| pre_change_tile18_fixture.s3a as U32 | 130560 | PrevTile | 88 | 1.000 | 874 | 871 |
| pre_change_tile18_fixture.s3a as U32 | 130560 | NeighborTileMean | 88 | 1.000 | 915 | 881 |
| pre_change_tile18_fixture.s3a as U32 | 130560 | NeighborValueMean | 94 | 1.068 | 864 | 899 |
| pre_change_tile18_fixture.s3a as U32 | 130560 | ReferenceTile | 88 | 1.000 | 1054 | 1063 |
| pre_change_tile18_fixture.s3a as F32 | 130560 | None | 88 | 1.000 | 1067 | 1156 |
| pre_change_tile18_fixture.s3a as F32 | 130560 | PrevTile | 88 | 1.000 | 978 | 848 |
| pre_change_tile18_fixture.s3a as F32 | 130560 | NeighborTileMean | 88 | 1.000 | 945 | 953 |
| pre_change_tile18_fixture.s3a as F32 | 130560 | NeighborValueMean | 95 | 1.080 | 922 | 599 |
| pre_change_tile18_fixture.s3a as F32 | 130560 | ReferenceTile | 88 | 1.000 | 1049 | 1031 |
