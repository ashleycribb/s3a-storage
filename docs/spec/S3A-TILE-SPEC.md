# S3A Hyper-Tile Storage Format Specification

**Document Version:** 1.2-draft  
**Status:** Implementable Standard  
**Date:** October 2026  
**License:** [Creative Commons Attribution 4.0 International (CC-BY-4.0)](LICENSE)  
**Patents:** [Royalty-Free Patent Non-Assertion Pledge](PATENTS.md)

### Specification Changelog

- **v1.2-draft (October 2026)**:
  - **Snapshot Anchoring (§11 Anchoring)**: Cryptographic commitment of sealed archives into public OpenTimestamps (OTS) calendars and RFC 3161 Timestamp Authorities (TSA).
  - **Archive Fingerprint**: Canonical 32-byte SHA-256 digest calculated over `file_uuid`, `tile_count`, and all `SealedDirectoryEntry` structs.
  - **ANCHORS Footer Extension**: Added tamper-evident `ANCHORS` section (`b"ANCHORS\0"`, 8-byte aligned) appended after parity in the sealed footer, preserving fingerprint invariance upon anchor attachment and upgrades.
  - **Proof Verification & Upgrades**: Defined reader rules for validating OTS Merkle paths to Bitcoin block headers and TSA timestamp signatures.
- **v1.1-draft (October 2026)**:
  - **Self-Addressing Tiles**: `HyperTileHeader` now stamps `generation` (`u64`), `tile_index` (`u32`), and `file_uuid` (`[u64; 2]`) within the CRC32C-covered header range. Added reader enforcement rule to reject foreign tiles (`FileUuidMismatch`).
  - **Legacy Compatibility Path**: Preserved non-breaking access for legacy pre-v1.1 unaddressed tiles keyed on `record_layout_version <= 1 && file_uuid == [0, 0]`.
  - **Sealed File Container Format**: Added tamper-evident footer specification for cold archives and exports (`.snapshot.s3a`, exports, backups), consisting of an index directory, Reed-Solomon $K+M$ erasure parity, and a fixed 64-byte end cap (`b"S3AEND\0\x01"`).
  - **In-Memory Non-Destructive Repair**: Reader recovery rules defined: transparent memory repair without silent disk writes; scanner fallback on damaged or missing footers.
  - **CLI & Conformance**: Specified `s3a-cli seal` and `s3a-cli verify [--repair <out>]` contracts, plus 9 new conformance vectors (23 total).
- **v1.0-draft (October 2026)**:
  - Initial open standard specification for the S3A Hyper-Tile Storage Format (DualFileHeader, HyperTileHeader, core Pod layouts, A/B atomic commit protocol, MARKS semantics, research snapshot container).

---

## 1. Conformance and Terminology

The key words **"MUST"**, **"MUST NOT"**, **"REQUIRED"**, **"SHALL"**, **"SHALL NOT"**, **"SHOULD"**, **"SHOULD NOT"**, **"RECOMMENDED"**, **"NOT RECOMMENDED"**, **"MAY"**, and **"OPTIONAL"** in this document are to be interpreted as described in BCP 14 [RFC 2119] [RFC 8174] when, and only when, they appear in all capitals, as shown here.

An implementation is not compliant if it fails to satisfy one or more of the **MUST** or **REQUIRED** level requirements for the protocols or formats it implements.

---

## 2. Architecture & File Layout

An S3A archive is a single-file, page-aligned binary storage container designed for high-performance zero-copy memory mapping (`mmap`), SIMD-accelerated filtering, and crash-resilient atomic commits.

### 2.1 File Structure Overview

All binary data in an S3A archive MUST be encoded in **Little-Endian** byte order.

```
+---------------------------------------------------------------+
| Dual-Slot File Header (512 Bytes, Sector-Aligned)             |
|   +---------------------------------------------------------+ |
|   | Slot A: FileHeader (64 Bytes, Offset 0..64)             | |
|   +---------------------------------------------------------+ |
|   | Slot B: FileHeader (64 Bytes, Offset 64..128)           | |
|   +---------------------------------------------------------+ |
|   | Reserved Zero Padding (384 Bytes, Offset 128..512)      | |
|   +---------------------------------------------------------+ |
+---------------------------------------------------------------+
| Hyper-Tile 0 (128 KB = 131,072 Bytes)                         |
|   +---------------------------------------------------------+ |
|   | HyperTileHeader (512 Bytes)                             | |
|   +---------------------------------------------------------+ |
|   | Payload Records / Blobs (payload_bytes <= 130,560 Bytes)| |
|   +---------------------------------------------------------+ |
|   | Zero Padding to 128 KB (131,072 - 512 - payload_bytes)  | |
|   +---------------------------------------------------------+ |
+---------------------------------------------------------------+
| Hyper-Tile 1 (128 KB = 131,072 Bytes)                         |
+---------------------------------------------------------------+
| ...                                                           |
+---------------------------------------------------------------+
| Hyper-Tile N-1 (128 KB = 131,072 Bytes)                       |
+---------------------------------------------------------------+
```

The total file size of a cleanly committed archive MUST equal:
$$\text{Total File Size} = 512 + (\text{tile\_count} \times 131,072) \text{ bytes}$$

---

### 2.2 Dual-Slot File Header (DualFileHeader)

The first 512 bytes of an S3A file comprise the `DualFileHeader`, consisting of two redundant 64-byte metadata slots (Slot A and Slot B) followed by 384 bytes of zero padding.

#### DualFileHeader Memory Layout (512 Bytes, 8-Byte Aligned)
| Offset (Bytes) | Size | Field Name | Description |
|:---:|:---:|:---|:---|
| `0 .. 64` | 64 B | `slot_a` | Primary Header Slot (Offset 0) |
| `64 .. 128` | 64 B | `slot_b` | Secondary Header Slot (Offset 64) |
| `128 .. 512` | 384 B | `_padding` | Reserved zero padding (`[[u64; 16]; 3]`) |

#### FileHeader Field Layout (64 Bytes, 8-Byte Aligned)
Each slot is an instance of `FileHeader`:

| Byte Offset | Type | Field Name | Semantics |
|:---:|:---:|:---|:---|
| `0 .. 4` | `[u8; 4]` | `magic` | Magic identifier. MUST be ASCII bytes `b"S3A1"` (`0x53, 0x33, 0x41, 0x31`). |
| `4 .. 6` | `u16` | `version` | Specification version. MUST be `1`. |
| `6 .. 8` | `u16` | `flags` | Archive flags (reserved; MUST be `0`). |
| `8 .. 16` | `u64` | `generation` | Monotonically increasing commit counter. |
| `16 .. 20` | `u32` | `tile_count` | Number of committed Hyper-Tiles in the archive. |
| `20 .. 24` | `u32` | `_reserved1` | Reserved padding. MUST be `0`. |
| `24 .. 32` | `u64` | `created_timestamp` | Archive creation timestamp in Unix epoch seconds. |
| `32 .. 40` | `u64` | `commit_timestamp` | Timestamp of latest commit in Unix epoch seconds. |
| `40 .. 44` | `u32` | `header_crc32` | Hardware CRC32C checksum of the slot. |
| `44 .. 48` | `u32` | `addon_mask` | Enabled domain capability bitmask. |
| `48 .. 64` | `[u64; 2]` | `file_uuid` | 128-bit unique file UUID for tile self-addressing validation. |

#### Checksum Calculation for FileHeader
The `header_crc32` field protects the slot against bit-rot or torn writes. When computing or verifying `header_crc32`:
1. The 4 bytes of `header_crc32` itself (offset 40..44) MUST be excluded from the calculation.
2. The checksum MUST be computed as CRC32C over bytes `0..40`, concatenated with bytes `44..64`.
3. The generator polynomial MUST be the Castagnoli polynomial ($0x1EDC6F41$).

#### Reader Resolution Rule (A/B Arbitration)
To open an S3A archive, a compliant reader MUST evaluate both Slot A and Slot B:
1. **Verification**: A slot is **VALID** if and only if:
   - `magic == b"S3A1"`, and
   - `version == 1`, and
   - `header_crc32 != 0`, and
   - `header_crc32 == compute_header_crc32(slot)`.
2. **Selection**:
   - If both Slot A and Slot B are valid: The reader MUST select the slot with the strictly greater `generation` value. If both generations are equal, Slot A MUST be selected.
   - If only Slot A is valid: The reader MUST select Slot A.
   - If only Slot B is valid: The reader MUST select Slot B.
   - If neither slot is valid:
     - **Legacy Compatibility Fallback**: If Slot A has `magic == b"S3A1"`, `version == 1`, and `header_crc32 == 0` (unhashed legacy format), Slot A MAY be accepted.
     - Otherwise: The archive is corrupted. The reader MUST fail closed and return an error (`CorruptedHeader`).

---

## 3. Hyper-Tile Specification

A **Hyper-Tile** is a self-contained, 128 KB (131,072 bytes) block aligned to 128 KB boundaries.

### 3.1 HyperTileHeader Layout (512 Bytes, 8-Byte Aligned)

The first 512 bytes of every Hyper-Tile contain its header metadata:

| Byte Range | Size | Field Name | Type | Description |
|:---:|:---:|:---|:---:|:---|
| `0 .. 8` | 8 B | `tile_id` | `u64` | 1-based sequential tile identifier within the archive. |
| `8 .. 12` | 4 B | `tile_type` | `u32` | Wire tile type identifier from the Tile Registry. |
| `12 .. 16` | 4 B | `record_count` | `u32` | Number of logical records contained in payload. |
| `16 .. 24` | 8 B | `min_timestamp` | `u64` | Earliest record timestamp in epoch seconds or milliseconds. |
| `24 .. 32` | 8 B | `max_timestamp` | `u64` | Latest record timestamp in epoch seconds or milliseconds. |
| `32 .. 36` | 4 B | `data_crc32` | `u32` | CRC32C checksum of the active payload data. |
| `36 .. 40` | 4 B | `header_crc32` | `u32` | CRC32C checksum of this 512-byte header. |
| `40 .. 44` | 4 B | `payload_bytes` | `u32` | Exact byte length of active payload data ($0 \le \text{len} \le 130,560$). |
| `44 .. 48` | 4 B | `record_layout_version` | `u32` | Layout schema version for records in this tile ($1$-based). |
| `48 .. 256` | 208 B | `hull` | `SimplexHull` | Geometric bounding manifold for spatial/vector pruning. |
| `256 .. 384` | 128 B | `filter` | `TileFilterMetadata` | 960-bit Blocked-Bloom filter for discrete key sieving. |
| `384 .. 416` | 32 B | `provenance` | `TileProvenanceMetadata`| Cryptographic payload digest / Merkle root. |
| `416 .. 440` | 24 B | `lifecycle` | `TileLifecycleMetadata` | Tombstone count, stratum tier, and compaction epoch. |
| `440 .. 456` | 16 B | `security` | `TileSecurityMetadata`  | Multi-tenant ID and security clearance mask. |
| `456 .. 472` | 16 B | `encoding` | `TileEncodingMetadata`  | Delta Frame-of-Reference base and scaling step. |
| `472 .. 480` | 8 B | `learned_index` | `TileLearnedIndex` | Monotonic timestamp index slope and intercept. |
| `480 .. 488` | 8 B | `generation` | `u64` | Archive generation when this tile was written. |
| `488 .. 492` | 4 B | `tile_index` | `u32` | 0-based logical index of this tile in the archive sequence. |
| `492 .. 496` | 4 B | `_reserved_addr`| `u32` | Reserved alignment padding (MUST be zeroed). |
| `496 .. 512` | 16 B | `file_uuid` | `[u64; 2]` | 128-bit file UUID matching containing archive's `FileHeader.file_uuid`. |

### 3.2 Sub-Header Structures

#### 3.2.1 SimplexHull (208 Bytes, 8-Byte Aligned, Offset 48..256)
- `dim`: `u32` (Offset 48..52) - Active dimensionality ($0 \le \text{dim} \le 16$).
- `radius`: `f32` (Offset 52..56) - Bounding hypersphere radius.
- `min_bounds`: `[f32; 16]` (Offset 56..120) - Coordinate minimums along each axis.
- `max_bounds`: `[f32; 16]` (Offset 120..184) - Coordinate maximums along each axis.
- `centroid`: `[f32; 16]` (Offset 184..248) - Mean cluster centroid coordinates.
- `_reserved`: `[u32; 2]` (Offset 248..256) - Padding to 208 bytes.

#### 3.2.2 TileFilterMetadata (128 Bytes, 8-Byte Aligned, Offset 264..392)
- `filter_type`: `u16` (Offset 264..266) - Filter algorithm ($0 = \text{None}$, $1 = \text{BlockedBloom960}$).
- `hash_seed`: `u16` (Offset 266..268) - Seed for hash functions (default `0x533A`).
- `distinct_sensors`: `u32` (Offset 268..272) - Estimated or exact count of distinct keys.
- `bits`: `[u64; 15]` (Offset 272..392) - 960-bit SIMD bitmask for Bloom membership testing.

#### 3.2.3 TileProvenanceMetadata (32 Bytes, 8-Byte Aligned, Offset 392..424)
- `merkle_root`: `[u8; 32]` (Offset 392..424) - 256-bit cryptographic payload digest or Merkle root.

#### 3.2.4 TileLifecycleMetadata (24 Bytes, 8-Byte Aligned, Offset 424..448)
- `tombstone_count`: `u32` (Offset 424..428) - Number of soft-deleted records in this tile.
- `stratum_tier`: `u8` (Offset 428..429) - Storage tier ($0 = \text{L0 memory}$, $1 = \text{L1 hot}$, $2 = \text{L2 warm}$, $3 = \text{L3 cold}$).
- `compaction_epoch`: `u8` (Offset 429..430) - Number of compaction passes this tile has undergone.
- `_reserved_tier`: `u16` (Offset 430..432) - Reserved padding.
- `min_expiry_timestamp`: `u64` (Offset 432..440) - Earliest TTL expiry timestamp.
- `max_expiry_timestamp`: `u64` (Offset 440..448) - Latest TTL expiry timestamp.

#### 3.2.5 TileSecurityMetadata (16 Bytes, 8-Byte Aligned, Offset 448..464)
- `tenant_id`: `u64` (Offset 448..456) - Organization / Workspace identifier.
- `security_tag_mask`: `u32` (Offset 456..460) - Security label bitmask.
- `min_clearance_level`: `u8` (Offset 460..461) - Required user clearance ($0..255$).
- `_reserved`: `[u8; 3]` (Offset 461..464) - Alignment padding.

#### 3.2.6 TileEncodingMetadata (16 Bytes, 8-Byte Aligned, Offset 464..480)
- `value_base`: `f64` (Offset 464..472) - Baseline offset for delta encoding.
- `value_scale`: `f32` (Offset 472..476) - Quantization scaling factor.
- `encoding_mode`: `u16` (Offset 476..478) - Encoding mode ($0 = \text{Raw}$, $1 = \text{Delta-FoR}$, $2 = \text{RunLength}$).
- `_reserved`: `u16` (Offset 478..480) - Alignment padding.

#### 3.2.7 TileLearnedIndex (8 Bytes, 4-Byte Aligned, Offset 480..488)
- `slope`: `f32` (Offset 480..484) - Monotonic record rate per time unit ($\Delta \text{records} / \Delta \text{time}$).
- `intercept`: `f32` (Offset 484..488) - Monotonic offset at `min_timestamp`.

### 3.3 CRC32C Covered Ranges & Padding Rules

1. **Header CRC (`header_crc32`)**:
   - `header_crc32` resides at offset `36..40` of `HyperTileHeader`.
   - When computing `header_crc32`, bytes `36..40` MUST be skipped. The calculation MUST cover bytes `0..36` concatenated with bytes `40..512`.
2. **Payload CRC (`data_crc32`)**:
   - `data_crc32` resides at offset `32..36` of `HyperTileHeader`.
   - `data_crc32` MUST cover exactly `payload_bytes` bytes starting at offset 512 of the tile.
   - Any trailing padding bytes up to 131,072 MUST NOT be included in the `data_crc32` calculation.
3. **Tile Padding**:
   - Every Hyper-Tile MUST occupy exactly 131,072 bytes on disk.
   - If `payload_bytes < 130,560`, the writer MUST write $131,072 - (512 + \text{payload\_bytes})$ zero bytes (`0x00`) following the active payload.

### 3.4 Tile Self-Addressing & Reader Verification Rules

In specification v1.1-draft, every Hyper-Tile is self-addressing:
1. **Protected Address Range**: The `file_uuid` (128-bit), `tile_index` (`u32`), and `generation` (`u64`) fields reside inside `HyperTileHeader` (offsets 480..512) and are strictly covered by `header_crc32`.
2. **Foreign Tile Rejection**: When opening an archive or verifying checksums, a compliant reader MUST compare each tile's `file_uuid` against the active `FileHeader.file_uuid`. If `header.file_uuid != active_header.file_uuid`, the reader MUST reject the tile with a `FileUuidMismatch` error.
3. **Legacy Unaddressed Tiles**: To preserve backwards compatibility with archives written under specification v1.0, readers MUST provide a legacy compatibility path:
   - A tile is evaluated as **Legacy Unaddressed** if and only if `record_layout_version <= 1` and `file_uuid == [0, 0]`.
   - Compliant readers MUST permit legacy unaddressed tiles to pass verification through the legacy path without failing closed.

---

## 4. Record Binary Layouts

All primary records stored in S3A Hyper-Tiles are plain-old-data (Pod) structures. They MUST be 8-byte aligned and encoded in Little-Endian format.

### 4.1 Record Flags & Lifecycle Conventions

Every record type containing a `flags` field MUST adhere to the following conventions:
- **`FLAG_ACTIVE = 0`**: Active, live record.
- **`FLAG_TOMBSTONE = 1 << 0` (1)**: Soft-deleted record.
- Any reader encountering `flags & FLAG_TOMBSTONE != 0` MUST treat the record as deleted and exclude it from query results unless tombstoned records are explicitly requested.

---

### 4.2 Core 64-Byte Pod Records

#### 4.2.1 LearningActivityRecord (64 Bytes, 8-Byte Aligned)
Used for standard educational interaction and AI agent experience replays:

| Offset | Type | Field Name | Description |
|:---:|:---:|:---|:---|
| `0 .. 8` | `u64` | `actor_id` | User ID, Student ID, or AI Agent ID |
| `8 .. 16` | `u64` | `timestamp_sec` | Epoch seconds of interaction or tool step |
| `16 .. 20` | `u32` | `verb_id` | xAPI verb ID or action identifier |
| `20 .. 24` | `u32` | `object_id` | Activity ID, course ID, or state/tool ID |
| `24 .. 28` | `f32` | `result_score` | Scaled grade, reward, or accuracy |
| `28 .. 32` | `u32` | `duration_ms` | Execution latency / time spent in ms |
| `32 .. 36` | `u32` | `success_flag` | `1` = Passed/Succeeded, `0` = Failed |
| `36 .. 40` | `u32` | `_reserved32` | Alignment padding |
| `40 .. 48` | `u64` | `flags` | `FLAG_ACTIVE = 0`, `FLAG_TOMBSTONE = 1` |
| `48 .. 64` | `[u64; 2]` | `_reserved` | Padding to exactly 64 bytes |

#### 4.2.2 StatementIndexRecord (Tile 22, 64 Bytes, 8-Byte Aligned)
Used for high-throughput multi-attribute indexing of xAPI 1.0.3 statements:

| Offset | Type | Field Name | Description |
|:---:|:---:|:---|:---|
| `0 .. 16` | `[u64; 2]` | `statement_id` | 128-bit statement UUID |
| `16 .. 24` | `u64` | `stored_timestamp_ms` | Epoch milliseconds when stored |
| `24 .. 32` | `u64` | `timestamp_ms` | Statement timestamp in epoch ms |
| `32 .. 40` | `u64` | `actor_hash` | 64-bit hash of actor IFI |
| `40 .. 44` | `u32` | `verb_hash` | 32-bit hash or ID of verb IRI |
| `44 .. 48` | `u32` | `activity_hash` | 32-bit hash of activity ID |
| `48 .. 52` | `u32` | `registration_hash` | 32-bit hash of registration UUID (0 if none) |
| `52 .. 56` | `u32` | `body_tile_id` | Hyper-Tile ID holding statement body (Tile 21) |
| `56 .. 60` | `u32` | `body_offset` | Byte offset within Tile 21 payload |
| `60 .. 64` | `u32` | `flags` | `0` = Active, `1` = Voided, `2` = Voiding, `4` = Attachments |

#### 4.2.3 MarkRecord (Tile 19, 64 Bytes, 8-Byte Aligned)
Used for tracking mutable policy, privacy, and integrity states:

| Offset | Type | Field Name | Description |
|:---:|:---:|:---|:---|
| `0 .. 4` | `u32` | `target_tile_type` | Wire tile type ID of targeted record |
| `4 .. 8` | `u32` | `record_index` | Record offset within tile (`u32::MAX` for whole tile) |
| `8 .. 16` | `u64` | `target_tile_id` | Tile ID in archive |
| `16 .. 24` | `u64` | `generation` | Archive generation when mark was appended |
| `24 .. 32` | `u64` | `timestamp` | Epoch seconds when mark was created |
| `32 .. 40` | `u64` | `actor_hash` | Identity hash of actor asserting/clearing mark |
| `40 .. 44` | `u32` | `mark_kind` | Mark type ($1=$ Quarantine, $3=$ Consent Withdrawn, $4=$ Legal Hold) |
| `44` | `u8` | `set_or_clear` | `1` = MARK_SET, `0` = MARK_CLEAR |
| `45 .. 48` | `[u8; 3]` | `_padding` | Alignment padding |
| `48 .. 64` | `[u64; 2]` | `_reserved` | Reserved for future expansion (MUST be zero) |

#### 4.2.4 StatementBlobHeader (Tile 21 Prefix, 64 Bytes, 8-Byte Aligned)
Precedes statement JSON payloads in Tile 21:

| Offset | Type | Field Name | Description |
|:---:|:---:|:---|:---|
| `0 .. 16` | `[u8; 16]` | `body_hash_xxh3` | 128-bit XXH3 hash of canonical statement JSON |
| `16 .. 48` | `[u8; 32]` | `sha256_hash` | 256-bit SHA-256 hash of canonical statement JSON |
| `48 .. 52` | `u32` | `total_bytes` | Total uncompressed byte length of statement JSON |
| `52 .. 54` | `u16` | `chunk_index` | Chunk offset ($0$ if single chunk) |
| `54 .. 56` | `u16` | `total_chunks` | Total chunks ($1$ if single chunk) |
| `56 .. 60` | `u32` | `chunk_bytes` | Byte length of JSON payload following this header |
| `60 .. 64` | `u32` | `chunk_crc32` | CRC32C checksum of data payload following this header |

#### 4.2.5 DocumentRecordHeader (Tile 23 Prefix, 64 Bytes, 8-Byte Aligned)
Precedes state/profile documents in Tile 23:

| Offset | Type | Field Name | Description |
|:---:|:---:|:---|:---|
| `0 .. 4` | `u32` | `doc_kind` | $1$ = State, $2$ = Agent Profile, $3$ = Activity Profile |
| `4 .. 8` | `u32` | `flags` | `0` = Active, `1` = Deleted, `2` = Is JSON |
| `8 .. 16` | `u64` | `key_hash` | 64-bit hash of compound key |
| `16 .. 32` | `[u8; 16]` | `etag_hash` | 128-bit hash representing HTTP ETag |
| `32 .. 40` | `u64` | `timestamp_ms` | Last-Modified timestamp in epoch ms |
| `40 .. 44` | `u32` | `content_len` | Byte length of document payload |
| `44 .. 48` | `u32` | `key_len` | Byte length of UTF-8 key string |
| `48 .. 52` | `u32` | `content_crc32` | CRC32C checksum of document content |
| `52 .. 64` | `[u32; 3]` | `_reserved` | Zero padding |

---

### 4.3 IEEE P2997 ELR Record Header (Tiles 25..=31, 96 Bytes, 8-Byte Aligned)

Every Enterprise Learner Record projection begins with `ElrRecordHeader`:

| Offset | Type | Field Name | Description |
|:---:|:---:|:---|:---|
| `0 .. 16` | `[u64; 2]` | `entity_id` | 128-bit entity UUID |
| `16 .. 32` | `[u64; 2]` | `source_statement_id` | 128-bit source statement UUID (provenance) |
| `32 .. 48` | `[u64; 2]` | `related_id` | 128-bit parent entity UUID (person ID, resource ID) |
| `48 .. 56` | `u64` | `timestamp_ms` | Timestamp in epoch milliseconds |
| `56 .. 60` | `u32` | `flags` | `ELR_FLAG_ACTIVE = 0`, `ELR_FLAG_TOMBSTONE = 1` |
| `60 .. 64` | `u32` | `status` | Learning status ($1=$ Attempted, $2=$ Completed, $3=$ Passed, $4=$ Failed, $5=$ Satisfied) |
| `64 .. 68` | `f32` | `numeric_value` | Scaled score ($0.0 .. 1.0$) or confidence |
| `68 .. 72` | `u32` | `payload_len` | Byte length of following UTF-8 JSON payload |
| `72 .. 76` | `u32` | `payload_crc32` | CRC32C checksum of JSON payload |
| `76 .. 80` | `u32` | `sub_type` | Goal subtype ($1=$ Self, $2=$ Assigned, $3=$ Recommended) |
| `80 .. 96` | `[u64; 2]` | `_reserved` | Zero padding to 96 bytes |

Immediately following `ElrRecordHeader` are `payload_len` bytes of UTF-8 encoded JSON containing the entity's attributes.

---

## 5. Atomic Commit Protocol & Crash Recovery

S3A guarantees atomic writes and full crash resilience without write-ahead logging (WAL) by using an alternating dual-slot header commit protocol.

### 5.1 Write Sequence & Fsync Points

To append a new Hyper-Tile to an existing S3A archive:
1. **Prepare Header**: Formulate the 512-byte `HyperTileHeader`, compute `data_crc32` over the payload, and compute `header_crc32`.
2. **Append Tile Data**: Seek to the end of the committed archive ($512 + \text{tile\_count} \times 131,072$) and write:
   - 512 bytes of `HyperTileHeader`.
   - `payload_bytes` of record data.
   - $131,072 - (512 + \text{payload\_bytes})$ zero bytes of padding.
3. **Data Flush Barrier (`fsync`)**:
   - The writer MUST invoke `flush()` and `sync_data()` (or OS equivalent, e.g., `fdatasync`) to ensure all tile bytes are committed to non-volatile storage.
4. **Post-Write Disk Verification**:
   - The writer SHOULD re-read the tile bytes from disk and verify both `header_crc32` and `data_crc32` match.
5. **Prepare Next Header Slot**:
   - Let `target_slot = 1 - active_slot` (Slot B if Slot A is active; Slot A if Slot B is active).
   - Increment `generation = generation + 1`.
   - Set `tile_count = tile_count + 1`.
   - Compute `header_crc32 = compute_header_crc32(new_slot)`.
6. **Commit Switch (`fsync`)**:
   - Seek to the target slot offset (Offset 0 for Slot A, Offset 64 for Slot B).
   - Write the 64 bytes of `new_slot`.
   - The writer MUST invoke `flush()` and `sync_all()` (or OS `fsync`) to persist the new metadata slot.
7. **Complete Transition**:
   - Update in-memory `active_slot = target_slot`, `generation`, and `tile_count`.

```
Timeline of Atomic Tile Commit:
[Writer] Write 128KB Tile -> flush() -> sync_data() [DATA PERSISTED]
                           -> Compute Slot CRC
                           -> Write 64B Target Slot -> flush() -> sync_all() [METADATA SWITCH]
```

### 5.2 Crash Recovery

If power loss, process termination, or hardware failure occurs at any point during writing:
1. **Crash During Step 2 or 3 (Before Metadata Switch)**:
   - On reopen, the reader inspects Slots A and B. The target slot was either never written or contains invalid data/checksum.
   - The reader resolves the previously committed slot, which still reflects the old `tile_count`.
   - Any trailing uncommitted bytes beyond $512 + \text{committed\_tile\_count} \times 131,072$ are ignored.
   - The append writer MUST truncate the file to $512 + \text{committed\_tile\_count} \times 131,072$ using `file.set_len()`.
2. **Crash During Step 6 (Torn Slot Write)**:
   - The target slot will have an invalid CRC32C.
   - Reader resolution ignores the corrupted slot and falls back to the surviving valid slot.
3. **Crash After Step 6**:
   - The target slot is fully synced and valid.
   - Its higher `generation` counter makes it authoritative immediately upon reboot.

---

## 6. Compaction & Historical Preservation

Compaction merges fragmented tiles, reclaims space occupied by soft-deleted records, and preserves policy history.

### 6.1 Compaction Execution Contract

1. **Out-of-Place Compaction**: Compaction MUST write to an independent temporary file (e.g., `{stem}.compact.{uuid}.tmp`).
2. **Preflight Verification**: Every input tile MUST pass CRC32C validation before processing. Corrupt tiles MUST NOT be compacted.
3. **Tombstone Purging**: Active records are retained; records with `FLAG_TOMBSTONE` are dropped, unless under `MARK_LEGAL_HOLD`.
4. **Addon Capability Union**: The compacted archive's `addon_mask` MUST equal the bitwise OR of all input archives' `addon_mask` fields.
5. **Tile ID Remapping**: Compacted tiles receive new sequential tile IDs ($1..\text{new\_count}$). A remap table `(input_idx, old_tile_id) -> new_tile_id` MUST be maintained.
6. **Mark Preservation**: Active marks (`MARK_SET`) MUST be carried forward to the compacted archive. Their `target_tile_id` fields MUST be remapped to the corresponding new tile IDs.
7. **Conservation Invariant**: The total count of live, non-tombstoned records in MUST equal the count of live records out.
8. **Atomic Replacement**: Once the output file is fully synced and verified, it replaces the target archive via an atomic rename operation (`atomic_replace`).

---

## 7. Tile-Type Registry & Allocation Policy

### 7.1 Allocation Bands

The `tile_type` field is partitioned into distinct ranges:
- **0 .. 9**: Core Storage Engine Primitives.
- **3 .. 6**: Standard Hardware & Domain Packs (Legacy frozen assignments inside core band; Wearable 3, Robotics 4, GIS 5, DA 6)¹.
- **10 .. 18**: Scholar Explorer & Ontology Packs.
- **19**: Core Changeable Metadata Marks.
- **20**: Reserved Domain Pack Slot.
- **21 .. 24**: Core Learning Record Store (LRS).
- **25 .. 31**: Core Enterprise Learner Record (ELR) Projections.
- **32 .. 63**: Reserved for standard domain packs (allocated via spec amendment).
- **64 .. $2^{32}-1$**: Open allocation for third-party, private, or application packs.

> ¹ **Note on IDs 3–6**: Tile IDs 3–6 are legacy pack assignments inside the core band (0–9) and are now frozen. New standard packs MUST use IDs in the **32–63** band (via specification amendment), and third-party packs MUST use **64+**.

### 7.2 Unknown Tile Types: Reader Handling

Compliant readers and tools MUST adhere to the following rules when encountering a tile type identifier they do not recognize:
1. **Queries**: A query searching for specific tile types MUST skip unknown tile types without failing.
2. **Compaction**: A compactor encountering an unknown tile type with `TileTypeFlags::COPY_THROUGH_COMPACT` MUST copy the tile through intact (`write_raw_hyper_tile`), preserving its header and payload bytes verbatim.
3. **Integrity Verification**: Checksum verification tools MUST verify the CRC32C of all tiles, regardless of whether their tile types are recognized.

---

## 8. Changeable Metadata Marks (Tile 19) Semantics

Tile 19 contains `MarkRecord` entries that attach mutable policy, governance, or diagnostic status to immutable records.

### 8.1 Mark Kinds

| Kind | Name | Semantics |
|:---:|:---|:---|
| **1** | `MARK_QUARANTINE` | Integrity or corruption quarantine. Readers MUST hide quarantined records from query results. |
| **2** | `MARK_NEEDS_REVIEW` | Supervisory flag requiring human or TA review. |
| **3** | `MARK_CONSENT_WITHDRAWN` | Privacy consent revoked. Readers MUST hide the record unless explicitly authorized with `include_hidden = true`. |
| **4** | `MARK_LEGAL_HOLD` | Regulatory preservation. Purge processes MUST NOT delete or tombstone records under active legal hold. |
| **5** | `MARK_ACCESS_HEAT` | Ephemeral caching heat metric. Dropped during snapshot export. |
| **6** | `MARK_RETENTION_EXPIRED` | Retention window elapsed. Applied by retention purge before compaction; readers MUST hide the record unless explicitly authorized with `include_hidden = true`. |

### 8.2 Mark Resolution Semantics

1. **Append-Only Wins**: Multiple marks MAY target the same record or whole tile. The latest mark (highest timestamp / latest tile offset) determines the active state.
2. **Whole Tile vs. Single Record**:
   - A mark with `record_index == MARK_TARGET_ALL_RECORDS` (`u32::MAX`) applies to all records in the tile.
   - An explicit per-record mark overrides a whole-tile mark for that specific record offset.
3. **Reader Query Filtering**: When evaluating `is_record_hidden`, if `MARK_CONSENT_WITHDRAWN`, `MARK_RETENTION_EXPIRED`, or `MARK_QUARANTINE` is set to `MARK_SET`, the record MUST be excluded from query results unless `include_hidden` is active.
4. **Purge Protocol**: Retention purge routines MUST mark records with `MARK_RETENTION_EXPIRED` (never conflated with consent withdrawal), and the purge receipt MUST record the expiration reason. User/actor deletion requests MUST continue to use `MARK_CONSENT_WITHDRAWN`.

---

## 9. Snapshot Container (.snapshot.s3a) Manifest

A Scholar Research Snapshot container (`.snapshot.s3a`) is an immutable, self-contained S3A archive capturing a reproducible research project milestone.

### 9.1 Container Requirements

1. **Capability Mask**: The container's `FileHeader.addon_mask` MUST have the `ADDON_EDU` bit (`0x0001`) enabled.
2. **Authoritative Manifest Tile**: The final tile in the container MUST be Tile 15 (`ScholarTileType::SRS_MANIFEST`), containing an `SrsManifestHeader`.

### 9.2 SrsManifestHeader Layout (512 Bytes, 8-Byte Aligned)

| Offset | Type | Field Name | Description |
|:---:|:---:|:---|:---|
| `0 .. 8` | `[u8; 8]` | `magic` | Container magic. MUST be ASCII bytes `b"S3ASTOR1"`. |
| `8 .. 12` | `u32` | `schema_version` | Snapshot specification version (`SCHOLAR_LAYOUT_VERSION_V1 = 1`). |
| `12 .. 16` | `u32` | `archive_flags` | Bitmask (`1` = Immutable, `2` = Snapshot, `4` = WASM Exported). |
| `16 .. 32` | `[u64; 2]` | `project_uuid` | 128-bit persistent project UUID. |
| `32 .. 48` | `[u64; 2]` | `snapshot_uuid` | 128-bit unique checkpoint UUID. |
| `48 .. 64` | `[u64; 2]` | `parent_snapshot_uuid` | 128-bit parent checkpoint UUID (all zeroes for root). |
| `64 .. 80` | `[u64; 2]` | `origin_session_uuid` | 128-bit originating session UUID. |
| `80 .. 88` | `u64` | `created_at_sec` | Snapshot creation timestamp in Unix epoch seconds. |
| `88 .. 92` | `u32` | `tile_count` | Number of Hyper-Tiles in container including Tile 15. |
| `92 .. 96` | `u32` | `crc32c_checksum` | CRC32C checksum of the manifest header (excluding bytes 92..96). |
| `96 .. 104` | `u64` | `total_records` | Total logical records across all tiles in the container. |
| `104 .. 360`| `[u64; 32]` | `_reserved1` | Reserved zero padding (256 bytes). |
| `360 .. 512`| `[u64; 19]` | `_reserved2` | Reserved zero padding (152 bytes). |

---

## 10. Sealed File Format & Reed-Solomon Parity

Sealed files (`.snapshot.s3a`, exports, cold archives, and immutable packages) append an audit directory, Reed-Solomon erasure parity, and an end cap at the tail of the file. Live database archives MUST NOT use the sealed footer, continuing to use the dual-slot A/B atomic commit protocol.

### 10.1 Sealed Layout

A sealed archive appends three consecutive structures immediately after the final committed Hyper-Tile:
```
+---------------------------------------------------------------+
| Hyper-Tile 0 .. N-1 (Data Tiles)                              |
+---------------------------------------------------------------+
| Sealed Directory (N * 40 Bytes)                               |
|   SealedDirectoryEntry 0 .. N-1                               |
+---------------------------------------------------------------+
| Reed-Solomon Parity Tiles (num_groups * M * 131,072 Bytes)    |
|   Parity Hyper-Tiles                                          |
+---------------------------------------------------------------+
| Sealed End Cap (64 Bytes, Fixed at EOF)                       |
+---------------------------------------------------------------+
```

### 10.2 SealedDirectoryEntry Layout (40 Bytes, 8-Byte Aligned)

For every committed Hyper-Tile $i \in [0, N-1]$, the directory contains an entry:

| Byte Offset | Type | Field Name | Description |
|:---:|:---:|:---|:---|
| `0 .. 4` | `u32` | `tile_index` | 0-based logical index of the tile in the archive sequence. |
| `4 .. 8` | `u32` | `tile_type` | Tile type identifier from the registry. |
| `8 .. 16` | `u64` | `generation` | Archive generation when tile was committed. |
| `16 .. 24` | `u64` | `offset` | Absolute byte offset of the Hyper-Tile from start of file. |
| `24 .. 32` | `u64` | `length` | Physical tile length in bytes (MUST be 131,072). |
| `32 .. 40` | `u64` | `xxh3_64` | 64-bit XXH3 integrity hash of the entire 128 KB tile. |

### 10.3 Reed-Solomon Erasure Parity

1. **Group Partitioning**: Data tiles are partitioned into groups of $K$ consecutive tiles (default $K=16$).
2. **Parity Generation**: For each group, $M$ parity tiles (default $M=2$) of 128 KB each are computed using Reed-Solomon erasure coding over GF($2^8$) with an invertible matrix.
3. **Repair Capability**: Any group containing up to $M$ damaged, corrupted, or missing tiles can be fully reconstructed.

### 10.4 SealedEndCap Layout (64 Bytes, Fixed at EOF)

The final 64 bytes of a sealed S3A archive comprise the `SealedEndCap`:

| Byte Offset | Type | Field Name | Description |
|:---:|:---:|:---|:---|
| `0 .. 8` | `[u8; 8]` | `magic` | Magic identifier. MUST be ASCII bytes `b"S3AEND\0\x01"` (`0x53, 0x33, 0x41, 0x45, 0x4E, 0x44, 0x00, 0x01`). |
| `8 .. 16` | `u64` | `footer_offset` | Absolute byte offset where the sealed footer directory begins. |
| `16 .. 24` | `u64` | `footer_length` | Total byte length of directory and parity tiles combined. |
| `24 .. 28` | `u32` | `tile_count` | Number of committed data tiles indexed by this sealed archive. |
| `28 .. 30` | `u16` | `k` | Reed-Solomon data shard parameter per group (default 16). |
| `30 .. 32` | `u16` | `m` | Reed-Solomon parity shard parameter per group (default 2). |
| `32 .. 48` | `[u64; 2]` | `file_uuid` | 128-bit file UUID matching the archive and tile headers. |
| `48 .. 52` | `u32` | `footer_crc32` | CRC32C checksum of the entire footer slice (directory + parity). |
| `52 .. 56` | `u32` | `endcap_crc32` | CRC32C checksum of the first 52 bytes of the end cap (`0..52`). |
| `56 .. 64` | `[u8; 8]` | `_reserved` | Reserved alignment padding (MUST be zeroed). |

### 10.5 Reader Conformance Rules for Sealed Files

A compliant reader opening a sealed S3A container MUST follow these rules:

1. **End Cap Inspection**: Check the last 64 bytes of the file. If the magic is missing or `endcap_crc32` does not match, the reader MUST report `endcap_corrupt` or `footer_missing` and fall back to rebuilding the directory by scanning self-addressed tiles from offset 512.
2. **Truncation Detection**: If physical file truncation cuts off mid-tile, the reader MUST fail with `SealedTruncated`.
3. **Out-of-Order Recovery**: If the directory or scanner discovers tiles out of order (`tile_index != physical_index`), the reader MUST reassemble tiles into logical sequence by `tile_index`.
4. **Parity Repair Execution**:
   - If a data tile fails its header or data CRC, and valid parity shards are present with corrupted tiles in the group $\le M$, the reader MUST reconstruct the damaged tile in memory and report the tile as `repaired`.
   - **Non-Destructive Guarantee**: In-memory verification MUST NOT silently modify the source file on disk. Repaired bytes are only written to disk if an explicit repair destination (`--repair <out>`) is instructed.
   - If corrupted tiles in any group exceed $M$, the reader MUST fail closed with `UnrepairableCorruption`.

---

## 11. Snapshot Anchoring & Trust Proofs (§Anchoring)

Sealed S3A archives (`.snapshot.s3a`, exports, cold archives) support cryptographic commitment into public decentralized calendars (OpenTimestamps) and RFC 3161 Timestamp Authorities (TSA).

### 11.1 Canonical Archive Fingerprint

The authoritative fingerprint of a sealed S3A archive is defined as the SHA-256 digest over the entire sealed directory structure:
$$\text{Fingerprint} = \text{SHA-256}(\text{file\_uuid} \parallel \text{tile\_count} \parallel \text{SealedDirectoryEntries}[0..N-1])$$

1. **Covered Scope**: The fingerprint commits to:
   - `file_uuid`: 16 bytes (little-endian $u64 \times 2$)
   - `tile_count`: 4 bytes (little-endian $u32$)
   - `directory`: The exact byte slice containing all $N$ consecutive 40-byte `SealedDirectoryEntry` structs. Because each entry covers the physical tile offset, generation, and 64-bit XXH3 integrity hash, the fingerprint cryptographically seals every byte of every Hyper-Tile in the container.
2. **Data Privacy**: Only the 32-byte SHA-256 fingerprint leaves the local machine during submission to public timestamping infrastructure. No raw data, titles, or author identities are ever exposed.

### 11.2 Anchoring Standards & Protocols

S3A mandates standard, interoperable cryptographic proof formats. Proprietary or hand-rolled proof structures MUST NOT be used. Implementations MUST support the following standards:

1. **OpenTimestamps (OTS) Standard Serialization**:
   - **Specification**: Adheres to the official OpenTimestamps file format specification (Peter Todd et al.).
   - **File Header & Magic**: Standalone proof files and proof byte streams embed the canonical 31-byte magic sequence:
     $$\texttt{\textbackslash x00OpenTimestamps\textbackslash x00\textbackslash x00Proof\textbackslash x00\textbackslash xbf\textbackslash x89\textbackslash xe2\textbackslash xe8\textbackslash x84\textbackslash xe8\textbackslash x92\textbackslash x94}$$
     followed by the 1-byte major version (`0x01`), the digest type tag (`0x08` for SHA-256), the 32-byte start digest (the archive fingerprint), and the execution graph encoded as varint-delimited operations.
   - **Standard Attestation Tags**: Attestations use `0x00`-prefixed 8-byte tags:
     - Pending Calendar Attestation: tag `[0x83, 0xdf, 0xe3, 0x0d, 0x2e, 0xf9, 0x0c, 0x8e]` (`83dfe30d2ef90c8e`).
     - Bitcoin Block Header Attestation: tag `[0x05, 0x88, 0x96, 0x0d, 0x73, 0xd7, 0x19, 0x01]` (`0588960d73d71901`).
   - **Calendar Interaction**:
     - *Submission*: Clients submit the 32-byte binary fingerprint via HTTP `POST /digest` to at least two independent public calendar pools (e.g., `https://a.pool.opentimestamps.org`, `https://b.pool.opentimestamps.org`). Calendar `/digest` responses return bare binary timestamps without the 31-byte file header. Clients deserialize these into valid execution branches. If a calendar returns an unparseable response or HTTP error, that calendar has failed; clients MUST NOT synthesize synthetic commitments. Each valid calendar timestamp is stored as an independent anchor in the footer.
     - *Upgrade*: When upgrading (`s3a-cli anchor upgrade`), clients query each calendar at `GET /timestamp/<commitment_hex>`, parsing the returned upgraded branch and splicing it into the pending attestation step to produce a confirmed Bitcoin block attestation.
   - **Independent Bitcoin Block Header Validation**:
     - Readers MUST NOT trust block hashes or Merkle roots embedded inside the proof stream.
     - Readers resolve the authoritative 80-byte raw Bitcoin block header at the attested height from a trusted or configurable source (default Blockstream Esplora REST API `/block-height/{height}` and `/block/{hash}/header`, local `bitcoind` RPC, or an offline header store).
     - The calculated Merkle root from executing the OTS proof operations MUST exactly match bytes 36..68 of the independent 80-byte block header with correct little-endian byte ordering.

2. **RFC 3161 Internet X.509 PKI Time-Stamp Protocol (TSP)**:
   - **Request Construction**: Clients construct an ASN.1 DER `TimeStampReq` specifying the SHA-256 algorithm OID `2.16.840.1.101.3.4.2.1` (`id-sha256`), a 64-bit cryptographic nonce, and `certReq = true`.
   - **Response Validation**: Responses are parsed as ASN.1 DER `TimeStampResp`. Clients MUST verify that `PKIStatus` equals `accepted(0)` or `grantedWithMods(1)`.
   - **Cryptographic Verification**: The encapsulated `ContentInfo` / `SignedData` token is verified by:
     - Parsing `TSTInfo` from the encapsulated content.
     - Asserting `messageImprint.hashAlgorithm` equals SHA-256 OID `2.16.840.1.101.3.4.2.1`.
     - Asserting `messageImprint.hashedMessage` strictly equals the 32-byte canonical archive fingerprint.
     - Validating the CMS signature and certificate chain against a configured X.509 trust store.

### 11.3 Sealed Footer ANCHORS Extension Layout

Anchor proofs are recorded in an optional `ANCHORS` extension slice appended after the Reed-Solomon parity tiles in the sealed footer:

```
+---------------------------------------------------------------+
| Sealed Directory (N * 40 Bytes)                               |
+---------------------------------------------------------------+
| Reed-Solomon Parity Tiles (num_groups * M * 131,072 Bytes)    |
+---------------------------------------------------------------+
| ANCHORS Extension (8-Byte Magic + 4-Byte Len + JSON Payload)  |
|   Magic: ASCII b"ANCHORS\0" (8 Bytes)                         |
|   Payload Length: u32 (4 Bytes, Little-Endian)                |
|   Payload: JSON serialized array of AnchorProof objects       |
|   Zero Padding to 8-byte alignment                            |
+---------------------------------------------------------------+
| Sealed End Cap (64 Bytes, Fixed at EOF)                       |
+---------------------------------------------------------------+
```

Because the `ANCHORS` extension is appended after the directory and parity tiles, attaching or upgrading an anchor updates only `footer_length`, `footer_crc32`, and `endcap_crc32`. **The Canonical Archive Fingerprint remains strictly invariant.**

### 11.4 Reader Verification Rules for Anchors

When verifying trust anchors via `s3a-cli verify --anchor <file>`:

1. The reader MUST recompute the canonical archive fingerprint from the sealed directory and compare it against the proof's recorded fingerprint.
2. **OpenTimestamps Verification**:
   - The reader MUST parse the proof bytes according to the standard OTS binary format.
   - The reader MUST trace the operations starting from the canonical fingerprint.
   - If `status == "pending"`, the reader verifies that the execution path reaches the expected calendar pending commitment.
   - If `status == "confirmed"`, the reader MUST independently look up the Bitcoin block header at the attested height, extract the Merkle root from bytes 36..68, and verify that the calculated OTS proof output matches the header's Merkle root byte-for-byte.
3. **RFC 3161 Verification**:
   - The reader MUST parse the DER `TimeStampResp` / `TimeStampToken`.
   - The reader MUST verify that `messageImprint.hashedMessage` strictly matches the canonical fingerprint.
   - The reader MUST verify the CMS signature and certificate validity.
4. **Tamper Detection**: If any tile, directory entry, block header, or proof byte is modified or forged, verification MUST fail with `match: no`.

---

## 12. Versioning & Compatibility Rules

1. **Wire Record Layout Versioning**:
   - Each tile declares its layout version in `HyperTileHeader.record_layout_version`.
   - **Version 0 Compatibility Alias**: Readers MUST treat `record_layout_version == 0` as a legacy compatibility alias for version `1`.
   - Writers MUST write `record_layout_version >= 1`.
2. **Fail-Closed Boundary Rules**:
   - If a tile's layout version is strictly less than the reader's `min_supported_layout_version`, the reader MUST reject the tile.
   - If a tile's layout version is strictly greater than the reader's `current_layout_version`, the reader MUST reject the tile.
3. **Non-Breaking Evolution**:
   - New optional fields MAY be added to fixed-size Pod records by utilizing trailing reserved zero-padding bytes without incrementing `record_layout_version`.
   - Any field added in reserved space MUST have default zero (`0`) semantics.
4. **Breaking Evolution**:
   - Changing field alignment, record size, or field offsets requires incrementing `record_layout_version`.

---

## 13. Known Deviations in Reference Implementation

To assist independent implementers, the following historic deviations in the reference implementation (`s3a-core` / `s3a-engine`) are documented:

1. **Descriptor Record Size Divergences**:
   - **Tile 1 (`embedding_128`)**: The wire layout of `EmbeddingRecord128` is 528 bytes (`id: u64` + `timestamp: u64` + `[f32; 128]`). In `registry.rs`, the descriptor's `record_size` registers 544 bytes due to historical padding.
   - **Tile 7 (`quantized_embedding_int8`)**: The wire layout of `QuantizedEmbeddingRecord256` is 288 bytes (`id: u64` + `timestamp: u64` + `scale: f32` + `offset: f32` + `flags: u64` + `[i8; 256]`). The registry descriptor registers `record_size = 40`.
   - **Tile 8 (`quantized_embedding_4bit`)**: The wire layout of `QuantizedEmbeddingRecord512` is 288 bytes. The registry descriptor registers `record_size = 24`.
2. **Tile ID 9 Dual-Purpose Legacy**:
   - Early prototypes mapped Tile 9 to an experimental `LearningActivityRecord`.
   - In the final specification, Tile ID 9 is strictly `RESERVED_BOUNDARY` (`TileTypeFlags::RESERVED`). Greenfield educational records MUST be written to Tile 10 (`HumanLrsRecord`) or Tile 28 (`elr_learning_event`).
3. **Tile Self-Addressing & Generation (Resolved in v1.1)**:
   - In specification v1.0, tile headers did not carry generation or file UUID. In specification v1.1-draft, `HyperTileHeader` embeds `generation: u64`, `tile_index: u32`, and `file_uuid: [u64; 2]`, establishing complete self-addressing across all tiles. Archive commits synchronize generation between `FileHeader` and all committed tiles.
