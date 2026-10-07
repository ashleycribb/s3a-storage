# S3A Conformance Test Vectors (`spec-vectors/`)

This directory contains official conformance test vectors for the **S3A Hyper-Tile Storage Specification (v1.1-draft)**. Each test case consists of a binary `.s3a` archive and a companion `.expected.json` document specifying expected reader resolution, checksum verification, record counts, and failure classes.

---

## 1. Conformance Matrix & Expected Results

| Case Name | Category | Expected Outcome | Error Class | Description |
|:---|:---:|:---:|:---:|:---|
| `valid_empty` | **Valid** | `ok` | *None* | Minimal valid S3A file (512B DualFileHeader, 0 tiles, generation 1). |
| `valid_single_tile_core` | **Valid** | `ok` | *None* | Single 128KB Hyper-Tile containing 3 core Telemetry records. |
| `valid_multi_tile` | **Valid** | `ok` | *None* | Multi-tile archive containing Telemetry (0), Embedding (1), and Statement Index (22). |
| `valid_after_compaction` | **Valid** | `ok` | *None* | Archive after compaction: soft-deleted tombstones purged, live records preserved. |
| `valid_dual_header_slots` | **Valid** | `ok` | *None* | Both Slot A and Slot B valid; reader must arbitrate to Slot B (higher generation). |
| `valid_unknown_tile_type` | **Valid** | `ok` | *None* | Archive containing unknown tile type 9999; readers must skip during domain queries and verify CRC. |
| `valid_snapshot_container` | **Valid** | `ok` | *None* | Portable research snapshot (.snapshot.s3a) containing Tile 15 SRS_MANIFEST and ADDON_EDU. |
| `invalid_bad_magic` | **Invalid** | `error` | `InvalidMagic` | Header magic corrupted to `b"BAD!"`. Reader must fail immediately. |
| `invalid_both_slots_corrupt` | **Invalid** | `error` | `CorruptedHeader` | Both Slot A and Slot B have corrupted CRC32C. Reader must fail closed. |
| `invalid_torn_write_fallback` | **Invalid** | `fallback_recovered` | `TornWriteFallback` | Newest Slot B has bad CRC; reader must recover by falling back to valid Slot A. |
| `invalid_tile_crc_mismatch` | **Invalid** | `error` | `ChecksumMismatch` | Tile payload bytes modified without updating `data_crc32`. Verification must fail. |
| `invalid_truncated_tile` | **Invalid** | `error` | `TruncatedTile` | File truncated mid-tile; reader must detect file boundary violation. |
| `invalid_unsupported_layout_version` | **Invalid** | `error` | `UnsupportedLayoutVersion` | Tile header declares `record_layout_version = 99`; reader must reject record read. |
| `valid_protected_mark_hidden` | **Valid** | `ok` | *None* | Record masked by Tile 19 `MARK_CONSENT_WITHDRAWN`; hidden by default, visible with override. |
| `sealed_ok` | **Valid** | `ok` | *None* | Sealed archive with directory, K=16/M=2 Reed-Solomon parity, and valid 64-byte end cap. |
| `sealed_footer_missing` | **Valid** | `ok_with_warning` | *None* | Sealed archive with footer missing; reader rebuilds directory by scanning tiles. |
| `sealed_endcap_corrupt` | **Valid** | `ok_with_warning` | *None* | Sealed archive with corrupt end cap; reader rebuilds directory by scanning tiles. |
| `sealed_truncated_mid_tile` | **Invalid** | `error` | `SealedTruncated` | Sealed archive truncated mid-tile; reader must reject with SealedTruncated error. |
| `one_tile_corrupt_repaired` | **Valid** | `repaired` | *None* | 1 corrupted data tile in group; Reed-Solomon parity repairs it in memory. |
| `three_tiles_corrupt_in_one_group` | **Invalid** | `error` | `UnrepairableCorruption` | 3 corrupted tiles in group with M=2; exceeds parity capacity and fails closed. |
| `tiles_out_of_order` | **Valid** | `ok_with_warning` | *None* | Tiles out of order on disk; reader reassembles them by tile_index. |
| `foreign_tile` | **Invalid** | `error` | `FileUuidMismatch` | Tile header carrying foreign file_uuid; reader rejects with FileUuidMismatch error. |
| `legacy_unaddressed_tiles` | **Valid** | `ok` | *None* | Pre-v1.1 unaddressed tiles (record_layout_version<=1, file_uuid=[0,0]); accepted via legacy path. |

---

## 2. Test Runner Execution Contract

An independent implementation runner MUST execute the test suite adhering to the following protocol:

1. **Header Resolution**:
   - Parse `DualFileHeader` from bytes 0..512.
   - Evaluate CRC32C of Slot A (offset 0) and Slot B (offset 64).
   - Verify active slot matches `header.active_slot` and `header.generation`.
   - If `expected_outcome == "fallback_recovered"`, verify reader successfully selects the older valid slot without crashing.
   - If `error_class` is `InvalidMagic` or `CorruptedHeader`, verify reader aborts opening with the corresponding error.
2. **Tile & Checksum Verification**:
   - For all valid cases, invoke the implementation's checksum verification over all committed tiles.
   - For `invalid_tile_crc_mismatch`, verify checksum verification detects mismatch and returns error.
   - For `invalid_truncated_tile`, verify reader rejects reading tiles beyond physical EOF.
3. **Record Extraction & Layout Enforcement**:
   - Verify record counts per tile type match `records_per_tile_type`.
   - For `invalid_unsupported_layout_version`, verify reader returns an unsupported layout error when accessing records in that tile.
4. **Policy Mark Evaluation**:
   - For `valid_protected_mark_hidden`:
     - Querying without administrative override MUST return 0 records (`visible_records == 0`).
     - Querying with administrative override (`include_hidden = true`) MUST reveal the record (`visible_records + hidden_records == 1`).
5. **Sealed File & Self-Addressing Verification (v1.1)**:
   - For `sealed_ok`, inspect 64-byte end cap (`b"S3AEND\0\x01"`), verify directory, parity, and tile checksums.
   - For `sealed_footer_missing` and `sealed_endcap_corrupt`, detect missing/corrupted footer, emit warning, and rebuild directory by scanning self-addressed tiles.
   - For `sealed_truncated_mid_tile`, reject with `SealedTruncated`.
   - For `one_tile_corrupt_repaired`, repair damaged data tile in memory using Reed-Solomon parity and report as repaired.
   - For `three_tiles_corrupt_in_one_group`, reject with `UnrepairableCorruption`.
   - For `tiles_out_of_order`, detect out-of-order storage and reassemble into logical sequence by `tile_index`.
   - For `foreign_tile`, compare tile `file_uuid` against file header `file_uuid`; reject with `FileUuidMismatch`.
   - For `legacy_unaddressed_tiles`, verify unaddressed tiles (`record_layout_version <= 1 && file_uuid == [0,0]`) are accepted through the legacy path.

---

## 3. Regenerating Test Vectors

To regenerate these test vectors using the reference implementation:
```bash
cargo run -p s3a-spec-vectors
```
