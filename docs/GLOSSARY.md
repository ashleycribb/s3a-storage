# S3A glossary

Short definitions with a pointer to the normative text. "TILE-SPEC" is [`docs/spec/S3A-TILE-SPEC.md`](spec/S3A-TILE-SPEC.md).

## File and header

| Term | Meaning | Source |
|---|---|---|
| **Archive / `.s3a` file** | One file made of a 512-byte file header and then fixed-size tiles, optionally followed by a sealed footer. | TILE-SPEC §2 |
| **DualFileHeader, slot A / slot B** | The first 512 bytes. Two 64-byte `FileHeader` copies sit at 0..64 and 64..128, and the rest is padding that readers ignore. Each slot has its own CRC32C. A reader uses the valid slot with the higher generation, so a torn commit falls back to the older slot. | TILE-SPEC §2.2, §5, [`durability.md`](durability.md), ADR-0001 |
| **Generation** | Commit counter in the file header. Tiles carry the generation they were committed in. | TILE-SPEC §2.2, §3.1 |
| **`file_uuid`, self-addressing** | 128-bit file id stored in the header and in every tile header (with `tile_index`). A tile from another file is rejected. Legacy v1.0 tiles with a zero UUID are accepted. | TILE-SPEC §3.6 |
| **Header flags, tile stride** | Header fields that change how a file is parsed (edge profile, 4/8 KB stride, compact header). An unknown value means the file is rejected. | TILE-SPEC §2.2, ADR-0013 |
| **Addon mask** | Header bitmask of the domain capabilities enabled on the archive (`s3a-cli addon`). | TILE-SPEC §2.2, [`pack-contract.md`](pack-contract.md) |

## Tiles

| Term | Meaning | Source |
|---|---|---|
| **Hyper-Tile** | A 131,072-byte block: a 512-byte `HyperTileHeader` followed by up to 130,560 payload bytes. | TILE-SPEC §3 |
| **Tile type** | `u32` id saying what the payload holds (0 telemetry, 77 schema, 78 manifest, 79 embedded spec, 83 record roots, …). | [`registry.md`](spec/registry.md), TILE-SPEC §14 |
| **Unknown tile** | A type the reader doesn't know. Readers skip it after checking its CRCs, and rewriters copy it through unchanged. | TILE-SPEC §7.2, ADR-0011, ADR-0013 |
| **`record_layout_version`** | Per-tile record layout version. 0 is an alias for 1, and anything newer than the reader supports is rejected. | TILE-SPEC §12, [`schema-evolution.md`](schema-evolution.md) |
| **CRC32C** | Castagnoli checksum used for header slots and tile headers and payloads. | TILE-SPEC §3.4, §3.5 |
| **Schema tile (77)** | An embedded catalog of the tile types and fields in the file. It may carry an **about** entry naming the spec version, spec URL, spec SHA-256 and the writing tool. | TILE-SPEC §14.5.4, ADR-0008 |
| **Manifest tile (78)** | A plain-text UTF-8 summary at tile 0 of a sealed file, readable with `head` or `less`. | TILE-SPEC §14.5.5 |
| **Embedded spec tile (79)** | The spec text, zlib-compressed, written by `seal --embed-spec`. | TILE-SPEC §14.5.6 |
| **Record-root tile (83)** | Per-tile RFC 6962 Merkle roots over records, which enable per-record proofs (`s3a-cli prove`). It is derived data, so rewriters drop it and regenerate it. | TILE-SPEC Appendix, ADR-0012 |
| **Edge profile, micro-tile** | 4 KB or 8 KB tiles with a 64-byte common tile header, for microcontrollers. | [`S3A-EDGE-PROFILE.md`](spec/S3A-EDGE-PROFILE.md) |

## Sealing

| Term | Meaning | Source |
|---|---|---|
| **Seal, sealed file** | An immutable export. `s3a-cli seal` adds a v2 footer: a tile directory index (80 bytes per tile, with a payload SHA-256) and a 64-byte end cap at EOF. Live archives aren't sealed. | TILE-SPEC §10.1–10.4 |
| **Seal hash, seal chain** | SHA-256 over the index and the previous seal's hash, so successive seals form a chain. Anchors commit to it. | TILE-SPEC §10.5 |
| **Reed-Solomon parity** | Optional (`--parity`). There are `m` parity tiles per group of `k` data tiles over GF(2^8), so up to `m` damaged tiles per group can be rebuilt. | TILE-SPEC §10.6, §10.9 |
| **Anchor** | The seal hash timestamped with OpenTimestamps or an RFC 3161 TSA (`s3a-cli anchor`). | TILE-SPEC §11 |

## Packs and tools

| Term | Meaning | Source |
|---|---|---|
| **Domain pack** | A crate (`s3a-pack-scholar`, `-gis`, `-robotics`, `-da`, `-wearable`, `-ontology`) that owns a band of tile type ids and their record layouts. | [`pack-contract.md`](pack-contract.md), TILE-SPEC §14.2 |
| **Compaction / fusion** | Rewrites that merge tiles into a new file (`s3a-cli compact`, `s3a-cli fuse`). | TILE-SPEC §6, [`durability.md`](durability.md) |
| **Conformance level** | *Reader*: passes every `level: reader` vector. *Writer*: also passes the rewrite cases, and its output reads `ok` under the reference reader. | [`conformance/README.md`](../conformance/README.md) |
| **Reference reader / Python reader** | The Rust engine (`s3a-conformance read`) and the independent stdlib reader in `readers/python`. | [`readers/python/README.md`](../readers/python/README.md) |
