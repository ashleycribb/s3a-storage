# Edge header fields documented (edge spec 1.1-draft, tile spec 1.4-draft)

- No API or on-disk change.
- `CommonTileHeader` bytes 12..16 / 56..60 / 60..64 are now normative as `data_crc32` / `header_crc32` / `generation` (ADR-0003).
- New test: `s3a-edge/tests/header_layout.rs`; new vectors `spec-vectors/hashes/edge_data_crc32c.json`, `edge_header_crc32c.json`.
