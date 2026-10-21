# Tile spec offset correction (spec 1.3-draft)

- No API or on-disk change.
- Spec §3.2 sub-structure offsets corrected to match §3.1 and code (ADR-0002).
- New normative §3.3 (little-endian; reserved bytes: writers zero, readers ignore) and §3.4 (hash rules).
- New tests: `s3a-engine/tests/header_layout.rs`, `s3a-vault/tests/hash_vectors.rs`; vectors in `spec-vectors/hashes/`.
