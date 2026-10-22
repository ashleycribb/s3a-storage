# S3A Hyper-Tile Storage Format Specification

**Document Version:** 1.5-draft  
**Status:** Implementable Standard  
**Date:** October 2026  
**License:** [Creative Commons Attribution 4.0 International (CC-BY-4.0)](LICENSE)  
**Patents:** [Royalty-Free Patent Non-Assertion Pledge](PATENTS.md)

### Specification Changelog

- **v1.5-draft (October 2026)**:
  - **Schema tile catalog (§14.5.4, B1, ADR-0008)**: payload layout corrected to match the reference code (it was mis-documented; no on-disk bytes change). Added the catalog reader contract: embedded catalog when present, registry fallback when absent, hard error on a bad Schema tile, unaligned-safe decoding, `field_count` sum MUST equal `total_fields`. Additive and optional; new vector `valid_schema_catalog`.
- **v1.4-draft (October 2026)**:
  - **Common Tile Header (§14.1, ADR-0003)**: bytes 12..16, 56..60, 60..64 documented as `data_crc32`, `header_crc32`, `generation` to match the reference code; they were wrongly listed as reserved. No on-disk bytes change.
- **v1.3-draft (October 2026)**:
  - **Offset correction (§3.2, ADR-0002)**: §3.2.2–§3.2.7 sub-structure offsets were shifted by 8 bytes and contradicted §3.1. §3.1 (and the reference code) was correct; §3.2 now matches. No on-disk bytes change.
  - **Byte order & reserved bytes (§3.3)**: all multi-byte fields are little-endian; readers MUST ignore reserved bytes, writers MUST zero them.
  - **Hashes computed by S3A (§3.4)** with conformance vectors in `spec-vectors/hashes/`.
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
| `6 .. 8` | `u16` | `flags` | Archive feature flags. Defined bits: `0x0001` PROFILE_EDGE, `0x0002` TILE_SIZE_4KB, `0x0004` TILE_SIZE_8KB, `0x0008` COMPACT_HEADER, `0x0010` SEAL_CHAIN_ACTIVE, `0x0020` ED25519_SIGNED (S3A-EDGE-PROFILE §2.2). Writers MUST NOT set any other bit. These bits change how the file is parsed, so readers MUST reject an archive whose active slot sets an undefined bit (§3.3, ADR-0013). |
| `8 .. 16` | `u64` | `generation` | Monotonically increasing commit counter. |
| `16 .. 20` | `u32` | `tile_count` | Number of committed Hyper-Tiles in the archive. |
| `20 .. 24` | `u32` | `_reserved1` (`tile_stride`) | Physical tile stride: `0` (= 131,072 bytes, standard profile), `4096` or `8192` (Edge profile, S3A-EDGE-PROFILE §2.2). Readers MUST reject any other value (§3.3, ADR-0013). The field name is historical; it is **not** a reserved byte. |
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
| `492 .. 496` | 4 B | `_reserved_addr`| `u32` | Reserved. Writers MUST zero; readers MUST ignore (§3.3). |
| `496 .. 512` | 16 B | `file_uuid` | `[u64; 2]` | 128-bit file UUID matching containing archive's `FileHeader.file_uuid`. |

### 3.2 Sub-Header Structures

#### 3.2.1 SimplexHull (208 Bytes, 8-Byte Aligned, Offset 48..256)
- `dim`: `u32` (Offset 48..52) - Active dimensionality ($0 \le \text{dim} \le 16$).
- `radius`: `f32` (Offset 52..56) - Bounding hypersphere radius.
- `min_bounds`: `[f32; 16]` (Offset 56..120) - Coordinate minimums along each axis.
- `max_bounds`: `[f32; 16]` (Offset 120..184) - Coordinate maximums along each axis.
- `centroid`: `[f32; 16]` (Offset 184..248) - Mean cluster centroid coordinates.
- `_reserved`: `[u32; 2]` (Offset 248..256) - Padding to 208 bytes.

#### 3.2.2 TileFilterMetadata (128 Bytes, 8-Byte Aligned, Offset 256..384)
- `filter_type`: `u16` (Offset 256..258) - Filter algorithm ($0 = \text{None}$, $1 = \text{BlockedBloom960}$).
- `hash_seed`: `u16` (Offset 258..260) - Seed for hash functions (default `0x533A`).
- `distinct_sensors`: `u32` (Offset 260..264) - Estimated or exact count of distinct keys.
- `bits`: `[u64; 15]` (Offset 264..384) - 960-bit SIMD bitmask for Bloom membership testing.

#### 3.2.3 TileProvenanceMetadata (32 Bytes, 8-Byte Aligned, Offset 384..416)
- `merkle_root`: `[u8; 32]` (Offset 384..416) - Payload provenance root. In the v1 reference writer this is `data_crc32 (u32 LE) || payload_len (u32 LE) || 24 zero bytes` (see §3.4); it is NOT yet a cryptographic digest.

#### 3.2.4 TileLifecycleMetadata (24 Bytes, 8-Byte Aligned, Offset 416..440)
- `tombstone_count`: `u32` (Offset 416..420) - Number of soft-deleted records in this tile.
- `stratum_tier`: `u8` (Offset 420..421) - Storage tier ($0 = \text{L0 memory}$, $1 = \text{L1 hot}$, $2 = \text{L2 warm}$, $3 = \text{L3 cold}$).
- `compaction_epoch`: `u8` (Offset 421..422) - Number of compaction passes this tile has undergone.
- `_reserved_tier`: `u16` (Offset 422..424) - Reserved padding.
- `min_expiry_timestamp`: `u64` (Offset 424..432) - Earliest TTL expiry timestamp.
- `max_expiry_timestamp`: `u64` (Offset 432..440) - Latest TTL expiry timestamp.

#### 3.2.5 TileSecurityMetadata (16 Bytes, 8-Byte Aligned, Offset 440..456)
- `tenant_id`: `u64` (Offset 440..448) - Organization / Workspace identifier.
- `security_tag_mask`: `u32` (Offset 448..452) - Security label bitmask.
- `min_clearance_level`: `u8` (Offset 452..453) - Required user clearance ($0..255$).
- `_reserved`: `[u8; 3]` (Offset 453..456) - Alignment padding.

#### 3.2.6 TileEncodingMetadata (16 Bytes, 8-Byte Aligned, Offset 456..472)
- `value_base`: `f64` (Offset 456..464) - Baseline offset for delta encoding.
- `value_scale`: `f32` (Offset 464..468) - Quantization scaling factor.
- `encoding_mode`: `u16` (Offset 468..470) - Encoding mode ($0 = \text{Raw}$, $1 = \text{Delta-FoR}$, $2 = \text{RunLength}$).
- `_reserved`: `u16` (Offset 470..472) - Alignment padding.

#### 3.2.7 TileLearnedIndex (8 Bytes, 4-Byte Aligned, Offset 472..480)
- `slope`: `f32` (Offset 472..476) - Monotonic record rate per time unit ($\Delta \text{records} / \Delta \text{time}$).
- `intercept`: `f32` (Offset 476..480) - Monotonic offset at `min_timestamp`.

### 3.3 Byte Order and Reserved Bytes

- All multi-byte integer and floating-point fields in the tile header and its sub-structures are encoded **little-endian** (IEEE-754 binary32/binary64 for floats). Array fields are stored element 0 first.
- Offsets in §3.1 and §3.2 are normative and are enforced by `crates/s3a-engine/tests/header_layout.rs`.
- Fields named `_reserved*` and padding bytes: writers MUST set them to zero; readers MUST ignore their contents and MUST NOT reject a tile because they are non-zero. (They remain covered by `header_crc32`.)
- This rule covers only bytes that carry no meaning in this version: tile-header `_reserved*` fields (e.g. `_reserved_addr`, bytes 492..496), sub-structure padding, and the file-header block padding (bytes 128..512, outside both 64-byte slots). It follows TCP's reserved bits: "Must be zero in generated segments and must be ignored in received segments" (RFC 9293 §3.1). A future version MAY give a reserved field meaning only with zero meaning "absent / old behaviour" (§ schema evolution), so old readers that ignore it stay correct.
- Fields that change how the file is **parsed** are not reserved, even where the historical struct name says `_reserved`: `FileHeader.flags` and `FileHeader._reserved1` (`tile_stride`). An unknown value there means the reader cannot interpret the layout, so it MUST reject the file instead of guessing. This is the "critical" case in PNG ("shall indicate to the user that the image contains information it cannot safely interpret", PNG 3rd ed. §5.4) and the "virtuous intolerance" of RFC 9413 §5.1. The check is applied to the arbitrated active slot. A reader MUST NOT fall back to an older slot because the newest slot sets an unknown flag.
- Validators (`s3a-cli verify`/`doctor`) MAY warn about non-zero reserved bytes, so writers that don't zero them get noticed (RFC 9413 §5.1), but MUST NOT fail on them.
- Conformance: `conformance/vectors-1.1` cases `reserved_*` (ignored) and `header_*` (rejected).

### 3.4 Hashes Computed by S3A

All values below are verified against the reference code and pinned by vectors in `spec-vectors/hashes/` (recomputed by `crates/s3a-vault/tests/hash_vectors.rs`).

| Hash | Algorithm | Exact input bytes | Output / encoding | Code | Vector |
|:---|:---|:---|:---|:---|:---|
| Payload checksum (`data_crc32`) | CRC32C (Castagnoli, reflected, init/xorout `0xFFFFFFFF`) | Active payload bytes `[0 .. payload_bytes)` | `u32`, stored LE at header 32..36 | `s3a-engine` writers (`crc32c::crc32c`) | `payload_crc32c.json` |
| Tile header checksum (`header_crc32`) | CRC32C | Header bytes `0..36` then `40..512` (the CRC field itself skipped) | `u32`, stored LE at 36..40 | `compute_tile_header_crc32` (`s3a-engine/src/lib.rs`) | `tile_header_crc32c.json` |
| Provenance root (`merkle_root`) | Not a hash in v1 | `data_crc32` (u32 LE) ‖ `payload_bytes` (u32 LE) ‖ 24 zero bytes | 32 raw bytes at 384..416 | `s3a-engine/src/lib.rs` tile writer | `provenance_root.json` |
| Seal hash (v2) | SHA-256 | ASCII `S3ASEAL2` ‖ `prev_seal_hash` (32) ‖ `seal_generation` u64 LE ‖ `tile_count` u32 LE ‖ `file_uuid[0]` u64 LE ‖ `file_uuid[1]` u64 LE ‖ raw `SealedTileIndexEntry` array bytes | 32 raw bytes; hex in tooling | `compute_seal_hash_v2` (`s3a-engine/src/sealed.rs`) | `seal_hash_v2.json` |
| Gate address | SHA-256 + CRC32C | 32-byte Ed25519 public key → `prefix = SHA-256(pk)[0..8]`; `checksum = CRC32C(prefix) & 0xFF` | `s3a-XXXX-XXXX-XXXX-XXXX-CC` (lowercase hex) ; URI `gate://…` | `GateAddress::from_public_key` (`s3a-vault/src/address.rs`) | `gate_address.json` |
| Sync inventory | SHA-256 | Each tile's active payload bytes (header excluded) | 32 raw bytes (set membership) | `s3a-vcs/src/sync.rs` | `sync_inventory.json` |

Note: the provenance root is currently derived from the CRC32C and length, not a cryptographic Merkle root; a domain-separated cryptographic root is a planned future change and will require a new layout version.


### 3.5 CRC32C Covered Ranges & Padding Rules

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

### 3.6 Tile Self-Addressing & Reader Verification Rules

In specification v1.1-draft, every Hyper-Tile is self-addressing:
1. **Protected Address Range**: The `file_uuid` (128-bit), `tile_index` (`u32`), and `generation` (`u64`) fields reside inside `HyperTileHeader` (offsets 480..512) and are strictly covered by `header_crc32`.
2. **Foreign Tile Rejection**: When opening an archive or verifying checksums, a compliant reader MUST compare each tile's `file_uuid` against the active `FileHeader.file_uuid`. If `header.file_uuid != active_header.file_uuid`, the reader MUST reject the tile with a `FileUuidMismatch` error.
3. **Legacy Unaddressed Tiles**: To preserve backwards compatibility with archives written under specification v1.0, readers MUST provide a legacy compatibility path:
   - A tile is evaluated as **Legacy Unaddressed** if and only if `record_layout_version <= 1` and `file_uuid == [0, 0]`.
   - Compliant readers MUST permit legacy unaddressed tiles to pass verification through the legacy path without failing closed.

### 3.7 Edge & Micro-Tile Profile Pointer

For resource-constrained microcontrollers (ESP32, nRF52, RP2040), embedded sensor nodes, and industrial telemetry loggers operating under strict memory limits ($\le 32\text{ KB}$ RAM, `no_std`, heapless execution), S3A defines the companion **Edge Profile Specification** in [`S3A-EDGE-PROFILE.md`](S3A-EDGE-PROFILE.md).

The Edge Profile specifies 4 KB and 8 KB flash-sector aligned micro-tiles, power-loss crash safety rules, on-device Ed25519 hash-chain sealing, and transport-agnostic sharing protocols while guaranteeing that all edge files remain valid S3A files directly readable by the full engine.

### 3.8 Digital Vault Profile Pointer

For cryptographic identity custody, verifiable credentials (W3C VC / Open Badges 3.0), autonomous agent delegation grants, physical thing management, gate network addressing, approval checkpoints, and digital holdings stored natively within valid S3A archives (`.vault.s3a` / `.s3a`), S3A defines the companion **Digital Vault Specification** in [`S3A-VAULT.md`](S3A-VAULT.md).

The Vault Profile establishes Ed25519 key storage boundaries, anti-privilege-escalation agent governance, physical asset binding, vault gate network addressing (`gate://`), approval checkpoint contracts, ownership transfer chains, and disaster recovery mechanisms while strictly excluding financial currencies and tokens.

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
2. **Compaction and rewrite (copy-through by default)**: Any tool that rewrites an archive (compaction, tile fusion, migration, repair) and encounters a tile type it does not recognize MUST copy that tile through intact (`write_raw_hyper_tile`), whether or not the type carries `TileTypeFlags::COPY_THROUGH_COMPACT`. The payload bytes MUST be preserved verbatim; only the relocation fields the writer owns (`tile_id`, `tile_index`, `generation`, `file_uuid` and the header CRCs) may change. A tool MAY drop unknown tiles only when the user explicitly requests it (for example a `--drop-unknown` option); silent dropping is non-compliant. The one exception is erasure: record bytes targeted by an Erasure tile (type 73) record MUST still be zeroed. `COPY_THROUGH_COMPACT` remains a hint for *known* types that a compactor may copy without domain re-encoding; it no longer controls whether unknown tiles survive (ADR-0011). Rationale (ADR-0013): every S3A tile carries its own CRC and its payload does not depend on other tiles' bytes, so an unknown tile has PNG's "ancillary, safe-to-copy" properties. PNG lets a decoder "safely ignore" an unknown ancillary chunk, and lets an editor copy a safe-to-copy chunk "whether or not the PNG editor recognizes the chunk type, and regardless of the extent of the datastream modifications" (PNG 3rd ed. §5.4, §14.2). A tile type whose content is **derived from** other tiles (PNG's "unsafe-to-copy", e.g. Record Root tile 83) MUST be registered as a known type, so rewriters can drop and regenerate it. Unregistered types are always treated as safe to copy. Conformance: `conformance/vectors-1.1` case `unknown_tile_rewrite`.
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

## 10. Sealed File Format & Reed-Solomon Parity (v2 Specification)

Sealed files (`.snapshot.s3a`, exports, cold archives, and immutable packages) append an audit directory index, optional Reed-Solomon erasure parity, and a 64-byte end cap at the tail of the file. Live database archives MUST NOT use the sealed footer, continuing to use the dual-slot A/B atomic commit protocol.

### 10.1 Sealed Layout v2

In v2, the tile directory index is placed immediately prior to the 64-byte End Cap at the EOF. Readers can locate and access any tile in the file by reading ONLY the tail bytes (End Cap + index directory), enabling sub-millisecond HTTP range read access on cloud object storage:

```
+---------------------------------------------------------------+
| Hyper-Tile 0 .. N-1 (Committed Data Tiles)                    |
+---------------------------------------------------------------+
| Optional Reed-Solomon Parity Shards (num_groups * M * 128 KB) |
+---------------------------------------------------------------+
| Optional ANCHORS Extension Slice (Variable Length, 8-byte align)|
+---------------------------------------------------------------+
| Sealed Tile Directory Index (128 + N * 80 Bytes)              |
|   SealedIndexHeaderV2 (128 Bytes, 8-byte aligned)             |
|   SealedTileIndexEntry 0 .. N-1 (N * 80 Bytes, 8-byte aligned)|
+---------------------------------------------------------------+
| Sealed End Cap v2 (64 Bytes, Fixed at EOF)                    |
+---------------------------------------------------------------+
```

### 10.2 SealedTileIndexEntry Layout (80 Bytes, 8-Byte Aligned)

For every committed Hyper-Tile $i \in [0, N-1]$, the directory contains an 80-byte self-describing entry:

| Byte Offset | Type | Field Name | Description |
|:---:|:---:|:---|:---|
| `0 .. 8` | `u64` | `offset` | Absolute byte offset of the Hyper-Tile from start of file. |
| `8 .. 16` | `u64` | `length` | Physical tile length in bytes (MUST be 131,072 for Hyper-Tiles). |
| `16 .. 20` | `u32` | `type_id` | Tile type identifier from the registry (§14.6). |
| `20 .. 22` | `u16` | `type_version` | Record layout version of the tile. |
| `22 .. 24` | `[u8; 2]` | `_pad` | Reserved. Writers MUST zero; readers MUST ignore (§3.3). |
| `24 .. 28` | `u32` | `record_count` | Number of committed active records in this tile. |
| `28 .. 32` | `u32` | `tile_index` | 0-based logical sequence index of the tile. |
| `32 .. 40` | `u64` | `min_key` | Minimum indexing key in tile (e.g. earliest timestamp). |
| `40 .. 48` | `u64` | `max_key` | Maximum indexing key in tile (e.g. latest timestamp). |
| `48 .. 80` | `[u8; 32]` | `payload_hash` | SHA-256 cryptographic digest of the tile payload bytes. |

### 10.3 SealedIndexHeaderV2 Layout (128 Bytes, 8-Byte Aligned)

Preceding the tile index entries in the directory, a 128-byte header defines the container epoch:

| Byte Offset | Type | Field Name | Description |
|:---:|:---:|:---|:---|
| `0 .. 8` | `[u8; 8]` | `magic` | Directory magic: ASCII `b"S3AIDX02"`. |
| `8 .. 12` | `u32` | `tile_count` | Number of committed data tiles indexed in this archive. |
| `12 .. 14` | `u16` | `flags` | Bit 0 (`0x0001`): `SEALED_FLAG_PARITY_ENABLED`. |
| `14 .. 16` | `u16` | `k` | Reed-Solomon data shard parameter per group (default 16). |
| `16 .. 18` | `u16` | `m` | Reed-Solomon parity shard parameter per group (default 2). |
| `18 .. 24` | `[u8; 6]` | `_padding` | Alignment padding (MUST be zeroed). |
| `24 .. 40` | `[u64; 2]` | `file_uuid` | 128-bit file UUID matching the archive and tile headers. |
| `40 .. 72` | `[u8; 32]` | `prev_seal_hash` | SHA-256 digest of predecessor seal (all zero for genesis). |
| `72 .. 80` | `u64` | `seal_generation` | Monotonically increasing seal sequence generation number. |
| `80 .. 88` | `u64` | `sealed_timestamp`| Unix epoch seconds when the archive seal was stamped. |
| `88 .. 120` | `[u8; 32]` | `seal_hash` | Authoritative 32-byte SHA-256 seal chain fingerprint. |
| `120 .. 128` | `[u8; 8]` | `_reserved` | Reserved. Writers MUST zero; readers MUST ignore (§3.3). |

### 10.4 SealedEndCapV2 Layout (64 Bytes, Fixed at EOF)

The final 64 bytes of a sealed S3A archive comprise the `SealedEndCapV2`:

| Byte Offset | Type | Field Name | Description |
|:---:|:---:|:---|:---|
| `0 .. 8` | `[u8; 8]` | `magic` | Magic identifier. MUST be ASCII bytes `b"S3AEND\0\x02"`. |
| `8 .. 16` | `u64` | `index_offset` | Absolute byte offset where the tile directory index begins. |
| `16 .. 24` | `u64` | `index_len` | Total byte length of index directory (`128 + tile_count * 80`). |
| `24 .. 56` | `[u8; 32]` | `index_hash` | SHA-256 cryptographic digest over the entire index directory. |
| `56 .. 58` | `u16` | `footer_version` | Footer format version (MUST be `2` for v2 archives). |
| `58 .. 60` | `u16` | `flags` | Bit 0: Parity enabled flag (`SEALED_FLAG_PARITY_ENABLED`). |
| `60 .. 64` | `u32` | `endcap_crc32` | CRC32C checksum of the first 60 bytes of the end cap (`0..60`). |

### 10.5 Seal Hash Chain & Tamper Evidence

Each seal records `prev_seal_hash` and `seal_generation`:
$$\text{seal\_hash} = \text{SHA-256}(b"\text{S3ASEAL2}" \parallel \text{prev\_seal\_hash} \parallel \text{seal\_generation} \parallel \text{tile\_count} \parallel \text{file\_uuid} \parallel \text{entries\_bytes})$$

1. **Genesis Rule**: The root seal (`seal_generation == 1`) MUST have `prev_seal_hash` equal to 32 all-zero bytes (`[0u8; 32]`).
2. **Chain Monotonicity**: Each subsequent seal generation MUST increment by exactly 1 (`gen_{i} = gen_{i-1} + 1`), and its `prev_seal_hash` MUST match the predecessor's `seal_hash`.
3. **Verification**: Readers verify the chain and fail closed on any break (`SealedChainBreak`) or swapped/reordered seal (`SealedSwappedGeneration`).
4. **Anchor Invariance**: Because decentralized anchors (OpenTimestamps and RFC 3161 TSAs) commit to `seal_hash`, anchoring a v2 archive mathematically attests to the entire cumulative history of the dataset.

### 10.6 Optional Reed-Solomon Erasure Parity

1. **Off By Default**: In v2 archives, Reed-Solomon parity shards are optional and disabled by default (`flags = 0`). Sealing with `s3a-cli seal <in> <out>` creates a compact, zero-overhead archive. Parity is enabled with the `--parity` flag.
2. **Galois Field $K+M$ Coding**: When enabled (`SEALED_FLAG_PARITY_ENABLED`), data tiles are grouped into blocks of $K$ (default 16) and encoded using the maintained `reed-solomon-erasure` crate into $M$ (default 2) 128 KB parity shards over GF($2^8$).
3. **Automatic Repair**: The CLI command `s3a-cli repair <in_file> <out_file>` inspects all data tiles, reconstructs up to $M$ damaged or corrupted tiles per group using surviving data and parity shards, re-verifies hashes, and writes an intact archive to `<out_file>`. If corruptions exceed $M$, it fails cleanly with `UnrepairableCorruption`.

### 10.7 Legacy v1 Compatibility (Back-Compat)

1. **v1 Archive Support**: Containers sealed with footer version 1 (`b"S3AEND\0\x01"`, 40-byte directory entries, and endcap format with `footer_offset`/`footer_length`) remain 100% readable and verifiable.
2. **Automatic Detection**: Compliant v2 readers inspect the last 64 bytes. If `b"S3AEND\0\x01"` is detected, the reader automatically falls back to the v1 verification and directory parsing path without requiring user intervention.

### 10.8 Reader Conformance Rules for Sealed Files

A compliant reader opening a sealed S3A container MUST follow these rules:

1. **Tail Inspection & Range Reads**: Inspect the final 64 bytes. Validate `magic` and `endcap_crc32`.
   - In v2: Read the index directory at `index_offset..index_offset + index_len`. Verify `index_hash`. Parse `SealedIndexHeaderV2` and `SealedTileIndexEntry` records. Range reads for individual tiles use `entry.offset` and `entry.length`.
   - In v1: Parse `SealedEndCap` at EOF and directory entries at `footer_offset`.
2. **Corruption & Scanning Fallback**: If the end cap is missing or corrupt, report `endcap_corrupt` or `footer_missing` and fall back to scanning self-addressed tiles from offset 512.
3. **Truncation Detection**: If physical file truncation cuts off mid-tile or before EOF, fail with `SealedTruncated`.
4. **Out-of-Order Recovery**: If tiles are out of logical order, sort by `tile_index`.
5. **Parity Repair Execution**: If tiles are damaged and valid parity shards are present, reconstruct damaged tiles in memory. If written to disk, write only to an explicit repair destination (`--repair <out>` or `s3a-cli repair`).

### 10.9 Byte-Level Rules for Independent Readers

These rules were implicit in the reference implementation and were made explicit while writing the independent Python reader (`readers/python`). They do not change the format.

1. **Seal hash encoding** (§10.5): every field is encoded at its `SealedIndexHeaderV2` width, little-endian: `seal_generation` as `u64`, `tile_count` as `u32`, `file_uuid` as two `u64`. `entries_bytes` is the raw `N * 80` entry bytes.
2. **`payload_hash`** (§10.2) is SHA-256 over the tile's active payload, bytes `512 .. 512 + payload_bytes` of the tile (not the whole 128 KB tile).
3. **Parity groups**: data tiles are split in index order into groups of `k`; group `g` holds tiles `g*k .. min((g+1)*k, N)`. The last group may be short (`k_g < k`); it is coded as a `k_g + m` code, with no zero-padding shards. Each shard is one whole 131,072-byte tile, header included.
4. **Parity placement**: the `m` parity shards of group `g` start at byte `512 + N*131072 + g*m*131072`, immediately after the data tiles and before any ANCHORS slice and the index.
5. **Code construction** (identical to `reed-solomon-erasure` 6.x `galois_8`): GF(2^8) with reducing polynomial `0x11D` and generator `2`. Build the `(k_g+m) x k_g` Vandermonde matrix `V[r][c] = r^c` (with `0^0 = 1`), then the encoding matrix `E = V * inverse(V[0..k_g])`. Its top `k_g` rows are the identity (systematic code), and parity shard `p` is `sum_c E[k_g+p][c] * tile_c`, computed byte by byte. To reconstruct, take any `k_g` surviving shards, invert the matching rows of `E` and multiply.
6. **What counts as a damaged tile** (rule 5 of §10.8): `header_crc32` is non-zero and wrong, or `data_crc32` is wrong, or `payload_hash` differs from the index entry. A repaired tile MUST pass all three checks again. More than `m` damaged tiles in one group is `UnrepairableCorruption`. With parity disabled, any damaged tile is an error.
7. **Outcomes**: a repaired archive reports `repaired`. Tiles found out of order (entry `i` points to a tile whose `tile_index != i`) are sorted by `tile_index` and the archive reports `ok_with_warning`. An end cap that is absent, has a bad `endcap_crc32` or points past EOF is treated as "not sealed": the file is read as a live archive (§2.2), and bytes after the last committed tile are ignored. A *valid* end cap with a wrong `index_hash` or `seal_hash` is an error, not a fallback.
8. **Record roots on a failed archive**: a verifier computes the record-root check (Appendix) even when a tile already failed its seal or CRC check, so the report can say `record_roots: "mismatch"`. The archive is still an error.

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

### 11.4 Read-Only Tour Architecture (`s3a-cli tour`) & Sealed Inspection

The `s3a-cli tour <file>` command provides a high-level, interactive, read-only walkthrough of S3A container contents ("floors"), schemas, cryptographic trust chains, array tensors, and participant consent governance.

#### 11.4.1 Fast Path vs Fallback Mechanics
1. **Sealed Footer v2 Fast Path**:
   - The reader inspects the terminal 64-byte `SealedEndCapV2` at `file_len - 64`.
   - If `magic == SEALED_ENDCAP_MAGIC_V2` and `endcap_crc32` matches, the reader jumps directly to `index_offset`, reading ONLY `index_len` bytes.
   - Directory entries (`SealedTileIndexEntry`) supply floor types, record counts, offsets, and min/max key ranges without reading tile payloads.
   - If an embedded Schema Tile (`type_id = 77`) is indexed, the reader reads only that single tile to resolve human-readable type names and field definitions.
2. **v1 & Unsealed Fallbacks**:
   - If `SealedEndCap` (v1) magic is present, directory entries are resolved from the v1 directory table.
   - If unsealed (live database), the reader falls back to reading dual-slot `FileHeader` metadata and scanning tile headers.

#### 11.4.2 Core Interfaces & Navigation
- **Lobby**: Emits file version, footer version, tile ("floor") count, seal status (hash chain and anchor status verified ✓/✗), consent summary, and registered schema names per type.
- **Floor List**: Emits one summary line per floor: index, canonical type name, layout version, record count, size in bytes, min/max keys, and short payload hash. For Array tiles (Tile 66): shape dimensions, axis names, physical units, chunk offset coordinates, and min/max value bounds.
- **`--floor N`**: Details floor $N$, presenting schema field definitions (ID, name, type, byte offset, byte length, nullability) and first $K$ records (or array statistical moments: count, min, max, mean, sum).
- **`--slice axis=range,...`**: Queries multi-dimensional array slices across chunks. Bounding boxes are evaluated against chunk coordinates; **only overlapping chunks are read into memory**.
- **`--cross key=<value>`**: Scans records across all floors in strictly monotonic index order matching a query key (e.g. `subject=<uuid>`, `sensor_id=<id>`, `actor=<id>`).
- **`--history`**: Displays the historical seal generation chain in chronological order (generation, sealed timestamp, previous seal hash, seal hash, anchor attestation status).

#### 11.4.3 Mathematical Read Bounds & Consent Policy
1. **Strict Bytes-Read Bound**: On v2 archives, running lobby / floor listing reads strictly $\ge \text{index\_offset}$ bytes. Zero tile payload bytes are accessed. When querying floor $N$, only the footer index and the single $[\text{offset}_N .. \text{offset}_N + \text{len}_N]$ range are read.
2. **Participant Research Consent Enforcement**: Under GDPR/HIPAA governance, all records associated with participants or devices lacking active consent for `purpose=research` (or explicitly masked by Tile 19 `MARK_CONSENT_WITHDRAWN`) are withheld from output and counted in `withheld_count`.

#### 11.4.4 Verification Citations
- **Implementation**: `crates/s3a-cli/src/tour.rs`
- **CLI Dispatch**: `crates/s3a-cli/src/main.rs:355`
- **Automated Tests**: `crates/s3a-cli/tests/tour_tests.rs`:
  - `test_tour_lobby_v1_and_v2`
  - `test_tour_floor_detail`
  - `test_tour_array_slice_reads_only_covered_chunks`
  - `test_tour_cross_section_ordering`
  - `test_tour_consent_withheld_counts`
  - `test_tour_bytes_read_bound`
  - `test_tour_cli_binary_and_json`

---

## 12. Versioning & Compatibility Rules

1. **Wire Record Layout Versioning**:
   - Each tile declares its layout version in `HyperTileHeader.record_layout_version`.
   - **Version 0 Compatibility Alias**: Readers MUST treat `record_layout_version == 0` as a legacy compatibility alias for version `1`.
   - Writers MUST write `record_layout_version >= 1`.
2. **Fail-Closed Boundary Rules** (in this specification version every tile type has `current_layout_version = 1` and `min_supported_layout_version = 1`, so after the version-0 alias any `record_layout_version > 1` MUST be rejected with `UnsupportedLayoutVersion`):
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

---

## 14. Tile Type Catalog

### 14.1 Overview & Common Tile Header

The **Tile Type Catalog** establishes a standardized, extensible vocabulary of tile abstractions across the S3A Hyper-Tile ecosystem. While `HyperTileHeader` (§3.1) provides physical framing, self-addressing, and low-level hardware checksums across every 128 KB block, the Tile Type Catalog defines the logical semantic contracts governing how payloads are structured, parsed, indexed, and audited.

All cataloged tile types share a canonical 64-byte logical header prefix embedded within the tile metadata framing:

#### Common Tile Header Layout (64 Bytes, 8-Byte Aligned, Little-Endian)

The header has 62 bytes of fields and 2 bytes of reserved padding (`_reserved1`), yielding a fixed **64-byte**, 8-byte-aligned structure. All integer fields are encoded in **Little-Endian (LE)** byte order.

| Offset (Bytes) | Size | Field Name | Type | Endianness | Description |
|:---:|:---:|:---|:---:|:---:|:---|
| `0 .. 2` | 2 B | `type_id` | `u16` | Little-Endian | Canonical tile type identifier from the allocation bands (e.g. 64–127 for Core). |
| `2 .. 4` | 2 B | `type_version` | `u16` | Little-Endian | Per-type schema/layout specification version (1-based, starts at 1). |
| `4 .. 6` | 2 B | `flags` | `u16` | Little-Endian | Type-specific bitmask flags (e.g., compression, encryption, ordering). |
| `6 .. 8` | 2 B | `_reserved1` | `u16` | Little-Endian | Explicit alignment padding (MUST be written as `0`, ignored by reader). |
| `8 .. 12` | 4 B | `record_count` | `u32` | Little-Endian | Number of logical entries, elements, or records in the payload. |
| `12 .. 16` | 4 B | `data_crc32` | `u32` | Little-Endian | CRC32C (Castagnoli) over the active payload bytes. |
| `16 .. 24` | 8 B | `payload_len` | `u64` | Little-Endian | Byte length of active payload data ($0 \le \text{payload\_len} \le 130,560$). |
| `24 .. 56` | 32 B | `payload_hash` | `[u8; 32]` | Raw Bytes | SHA-256 cryptographic digest calculated over the active payload bytes. |
| `56 .. 60` | 4 B | `header_crc32` | `u32` | Little-Endian | CRC32C over header bytes `0..56` ‖ `60..64`. |
| `60 .. 64` | 4 B | `generation` | `u32` | Little-Endian | Commit generation when the tile was sealed (low 32 bits). |

> **Header Size Summary**: fields $2+2+2+4+4+8+32+4+4 = 62$ bytes, plus `_reserved1` (2 B) = 64 bytes.

#### Integrity Fields
When `flags` has `TILE_FLAG_CRC_CHAIN` (value `2`) set, readers SHOULD verify `header_crc32` and `data_crc32` and treat mismatches as corruption. `_reserved1` MUST be written as `0` and MUST be ignored by readers. Byte-level details and vectors: `S3A-EDGE-PROFILE.md` §2.4.1, `spec-vectors/hashes/edge_*_crc32c.json`, ADR-0003.

#### Reader Skip Enforcement Rule
A compliant S3A reader MUST inspect `type_id` upon reading each tile. If the reader encounters an unrecognized, unsupported, or private `type_id`, the reader **MUST skip the unknown tile using `payload_len` and MUST NOT fail closed or abort execution**. Readers MUST continue processing subsequent known tiles in the container uninterrupted.

---

### 14.2 Allocation Bands & ID Mapping Scheme

To prevent identifier collisions with historical wire formats, tile type identifiers (`type_id`) are partitioned into strict numerical bands:

| Range (`u16`) | Allocation Category | Governance & Description |
|:---|:---|:---|
| **0 – 32** | **Legacy Frozen Wire Assignments** | Historical v1.0–v1.2 allocations (including boundary guard 9). Permanently frozen for backward compatibility. |
| **33 – 63** | **Reserved for Standard Packs** | Reserved for future foundational domain pack specifications. |
| **64 – 127** | **Core Catalog Primitives** | Standardized in this specification (Types 64–77). Guaranteed disjoint from all legacy IDs. |
| **128 – 255** | **Reserved Core Extensions** | Reserved for future core architectural primitives. |
| **256 – 4095** | **Registered Domain Packs** | Governed through specification amendments and domain pack RFCs (e.g. Scholar, Ontology, Robotics, GIS, Wearable). |
| **4096 – 65535** | **Private & Experimental** | Uncoordinated, vendor-specific, experimental, or internal organization tile types. No registry coordination required. |

#### Existing Tile ID Mapping & Renumbering Analysis

In the initial reference implementation (spec v1.0 through v1.2), tile types were assigned sequentially in `u32` space (IDs 0 through 32), mixing core engine primitives with early domain pack assignments (e.g. Wearable on 3, Robotics on 4, GIS on 5, DA on 6, and Scholar on 10–15, 18, 20, 32).

To ensure zero collisions with legacy implementations while providing a clean architectural taxonomy, all new Core catalog types are assigned exclusively in the **64–127 band**, leaving legacy IDs 0–32 untouched and frozen.

The table below maps the historical wire assignments into the Catalog ID scheme and documents the planned renumbering:

| Historical Wire ID | Canonical Type Name | Owning Layer | Historical Role | Modern Catalog Type ID | Code Renumbering Status | Notes / Conflicts |
|:---:|:---|:---|:---|:---:|:---:|:---|
| **0** | `telemetry` | Core Engine | Core (`time-series`) | `75` (`0x004B`) | None (Legacy 0 Frozen) | Wire ID 0 maps to Core `time-series` |
| **1** | `embedding_128` | Core Engine | Core (`vector`) | `67` (`0x0043`) | None (Legacy 1 Frozen) | Historical dense float32 vector format |
| **2** | `hybrid_index` | Core Engine | Core (`index`) | `69` (`0x0045`) | None (Legacy 2 Frozen) | Historical inverted keyword/sparse index |
| **3** | `wearable_micro` | Wearable Pack | Registered Pack | `320–335` | **Do Not Renumber in Code** | Legacy pack assigned in legacy band; frozen |
| **4** | `robotics_kinematic`| Robotics Pack | Registered Pack | `288–303` | **Do Not Renumber in Code** | Legacy pack assigned in legacy band; frozen |
| **5** | `gis_survey_mesh` | GIS Pack | Registered Pack | `304–319` | **Do Not Renumber in Code** | Legacy pack assigned in legacy band; frozen |
| **6** | `data_availability`| DA Pack | Registered Pack | `336–351` | **Do Not Renumber in Code** | Legacy pack assigned in legacy band; frozen |
| **7** | `quantized_embedding_int8` | Core Engine | Core (`vector`) | `67` (`0x0043`) | None (Legacy 7 Frozen) | Int8 quantized vector format |
| **8** | `quantized_embedding_4bit` | Core Engine | Core (`vector`) | `67` (`0x0043`) | None (Legacy 8 Frozen) | 4-bit nibble quantized vector format |
| **9** | `reserved_boundary`| Core Engine | Core Boundary | *N/A* (Boundary) | None (Legacy 9 Frozen) | Boundary guard; writes rejected |
| **10** | `human_lrs` | Scholar Pack | Registered Pack | `256–271` | **Do Not Renumber in Code** | Scholar xAPI activity trace |
| **11** | `ai_trace_log` | Scholar Pack | Registered Pack / `provenance` | `72` (`0x0048`) / `256` | **Do Not Renumber in Code** | AI agent dual-trace provenance |
| **12** | `academic_papers` | Scholar Pack | Registered Pack / `record` | `64` (`0x0040`) / `256` | **Do Not Renumber in Code** | Scholar paper metadata record |
| **13** | `research_graph` | Scholar Pack | Registered Pack / `graph` | `74` (`0x004A`) / `256` | **Do Not Renumber in Code** | Citation / co-authorship graph |
| **14** | `srs_workspace` | Scholar Pack | Registered Pack | `256–271` | **Do Not Renumber in Code** | Spaced repetition workspace |
| **15** | `srs_manifest` | Scholar Pack | Registered Pack | `256–271` | **Do Not Renumber in Code** | Snapshot manifest header |
| **16** | `ontology_concept`| Ontology Pack | Registered Pack | `272–287` | **Do Not Renumber in Code** | Formal concept registry |
| **17** | `ontology_axiom` | Ontology Pack | Registered Pack | `272–287` | **Do Not Renumber in Code** | Subsumption & relationship axioms |
| **18** | `agent_registry` | Scholar Pack | Registered Pack / `provenance` | `72` (`0x0048`) / `256` | **Do Not Renumber in Code** | Authorized AI agent catalog |
| **19** | `marks` | Core Policy | Core (`consent` / `erasure`) | `71` (`0x0047`) / `73` | None (Legacy 19 Frozen) | Policy flags (consent, legal hold, expiry) |
| **20** | `note_claims` | Scholar Pack | Registered Pack | `256–271` | **Do Not Renumber in Code** | Research claim notes & Bloom filter |
| **21** | `statement_body` | Core LRS | Core (`blob` / `record`) | `68` (`0x0044`) / `64` | None (Legacy 21 Frozen) | xAPI statement JSON blob |
| **22** | `statement_index` | Core LRS | Core (`index`) | `69` (`0x0045`) | None (Legacy 22 Frozen) | Secondary statement index |
| **23** | `document_store` | Core LRS | Core (`blob`) | `68` (`0x0044`) | None (Legacy 23 Frozen) | Document profile attachments |
| **24** | `attachment_blob` | Core LRS | Core (`blob`) | `68` (`0x0044`) | None (Legacy 24 Frozen) | Raw binary attachments |
| **25–31**| `elr_*` | Core ELR | Core (`record`) | `64` (`0x0040`) | None (Legacy 25–31 Frozen)| IEEE P2997 enterprise learner records |
| **32** | `citation_fingerprint` | Scholar Pack | Registered Pack / `provenance` | `72` (`0x0048`) / `256` | **Do Not Renumber in Code** | Cryptographic DOI citation fingerprint |

> **Specification Authority Note on Renumbering**: Existing compiled crates and production archives MUST NOT renumber wire IDs 0–32 in code or disk images. The legacy mapping is frozen for backward compatibility. New domain packs MUST allocate exclusively within `256–4095`. Core catalog types operate on assigned IDs `64–127`.

---

### 14.3 Core Tile Specifications

#### 14.3.1 Record Tile (`record`) — Type ID: `64` (`0x0040`) (Core)
- **Status**: **IMPLEMENTED** (Writer: `TileWriter::write_hyper_tile` in `crates/s3a-engine/src/lib.rs:621`; Reader: `MmapReader::get_tile` in `crates/s3a-engine/src/lib.rs:798`; Test: `test_tile_writer_and_mmap_reader_roundtrip` in `crates/s3a-engine/src/lib.rs:3348`).
- **Purpose**: High-throughput storage of fixed-size, plain-old-data (Pod) tabular structs aligned to 8-byte boundaries, enabling zero-copy memory mapping and constant-time $O(1)$ random access by record index.
- **Header Extension Fields**:
  - `record_size`: `u32` — Exact byte size of each record struct (MUST be a multiple of 8).
  - `record_layout_version`: `u16` — Schema evolution version of the record struct.
  - `schema_id`: `u32` — Target schema identifier defined in a `schema` tile, or 0 if self-evident.
- **Payload Layout**:
  - Array of contiguous byte slices: `[RecordStruct; record_count]`. Total active payload length equals `record_count * record_size`.
- **Required vs Optional Fields**:
  - Required: `record_size` (must be $> 0$ and $\le 130,560$), `record_layout_version`, `record_count`.
  - Optional: `schema_id` (defaults to 0).
- **Integrity Rules**:
  - `payload_len` MUST strictly equal `record_count * record_size`.
  - Every individual record MUST be 8-byte aligned relative to payload start (offset 512).
  - `payload_hash` MUST equal the SHA-256 digest of `payload_bytes`.
- **Academic Research Example**: Longitudinal clinical trial survey responses recording patient ID, treatment arm, visit week, and 12 standardized biomarker ratings across 50,000 patient-weeks.
- **Why It Matters**: Fixed-size record tiles eliminate serialization and deserialization overhead by allowing operating systems to map binary files directly into compiled computational structs in CPU memory. This enables research pipelines to stream hundreds of millions of experimental observations across cluster nodes without runtime parsing penalties.

---

#### 14.3.2 Columnar Tile (`columnar`) — Type ID: `65` (`0x0041`) (Core)
- **Status**: **IMPLEMENTED** (Writer: `encode_columnar_telemetry` in `crates/s3a-engine/src/lib.rs:56`, `ColumnarTileBuilder::build` in `crates/s3a-core/src/columnar.rs:172`, `Compactor::compact_with_columnar_opt_in` in `crates/s3a-engine/src/lib.rs:2630`; Reader: `decode_columnar_telemetry` in `crates/s3a-engine/src/lib.rs:108`, `ColumnarTileView::parse` in `crates/s3a-core/src/columnar.rs:207`, `QuerySieve::query_telemetry_with_coords` in `crates/s3a-engine/src/lib.rs:1700`; Test: `test_roundtrip_equality_vs_record_tile` in `crates/s3a-engine/tests/columnar_tile.rs:39`, `test_compaction_opt_in_columnar_roundtrip` in `crates/s3a-engine/tests/columnar_tile.rs:198`).
- **Purpose**: Vertically partitioned column-oriented storage designed for vectorized analytical scans (SIMD), high-ratio dictionary/Run-Length compression, and selective attribute projections.
- **Header Extension Fields**:
  - `column_count`: `u16` — Number of attribute columns stored in this tile.
  - `compression_codec`: `u16` — Compression algorithm applied to columns ($0 = \text{Raw}$, $1 = \text{LZ4}$, $2 = \text{ZSTD}$, $3 = \text{Snappy}$).
  - `null_bitmap_offset`: `u32` — Byte offset within payload to the packed validity/null bitmaps.
  - `column_descriptors_offset`: `u32` — Byte offset within payload to the column metadata array.
- **Payload Layout**:
  1. `ColumnDescriptors`: Array of `[ColumnDescriptor; column_count]`, each declaring data type (`u16`), offset (`u32`), uncompressed length (`u32`), and compressed length (`u32`).
  2. `NullBitmaps`: Packed bitmasks ($\lceil \text{record\_count} / 8 \rceil$ bytes per column) indicating value presence.
  3. `ColumnData`: Contiguous blocks of homogeneous, column-wise values.
- **Required vs Optional Fields**:
  - Required: `column_count`, `null_bitmap_offset`, `column_descriptors_offset`.
  - Optional: `compression_codec` (defaults to 0 for uncompressed raw vectors).
- **Integrity Rules**:
  - Every column vector MUST contain exactly `record_count` logical values (inclusive of nulls).
  - Decompressed column buffers MUST match the declared primitive byte width $\times \text{record\_count}$.
  - `payload_hash` covers the raw on-disk columnar byte stream before decompression.
- **Academic Research Example**: Genome-Wide Association Study (GWAS) summary statistics storing Chromosome, Base-Pair Position, Reference Allele, Effect Allele, Beta Coefficient, and $P$-value across 15 million genetic variants.
- **Why It Matters**: Columnar tiles allow analytical queries to read only the specific attributes needed for a hypothesis test, avoiding costly disk I/O across unreferenced fields. Vectorized CPU instructions can evaluate filter conditions across millions of data points simultaneously at memory bus bandwidth.

---

#### 14.3.3 N-Dimensional Array Tile (`array`) — Type ID: `66` (`0x0042`) (Core)
- **Status**: **IMPLEMENTED** (Writer: `S3ACrudEngine::write_array_chunk` in `crates/s3a-engine/src/lib.rs:2448`, `ArrayTileBuilder::build` in `crates/s3a-core/src/array.rs:192`; Reader: `S3ACrudEngine::read_array_slice` in `crates/s3a-engine/src/lib.rs:2476`, `ArrayTileView::parse` in `crates/s3a-core/src/array.rs:206`; Test: `test_array_roundtrip_2d_3d_4d` in `crates/s3a-engine/tests/array_tile.rs:43`, `test_array_slice_across_chunk_boundaries` in `crates/s3a-engine/tests/array_tile.rs:98`).
- **Purpose**: Native storage of dense multi-dimensional tensors, volumetric imaging cubes, and numerical simulation grids with explicit spatial dimensionality, units, and chunk coordinates.
- **Header Extension Fields**:
  - `ndim`: `u16` — Dimensionality of the array ($1 \le \text{ndim} \le 8$).
  - `dtype`: `u16` — Numerical primitive type ($1 = \text{i8}$, $2 = \text{u8}$, $3 = \text{i16}$, $4 = \text{u16}$, $5 = \text{i32}$, $6 = \text{u32}$, $7 = \text{f32}$, $8 = \text{f64}$, $9 = \text{c64}$).
  - `shape`: `[u64; 8]` — Full tensor dimensions $[D_0, D_1, \dots, D_{\text{ndim}-1}]$. Unused dimensions MUST be `0`.
  - `chunk_coords`: `[u64; 8]` — Logical chunk coordinate offset $[C_0, C_1, \dots, C_{\text{ndim}-1}]$ within a partitioned dataset.
  - `axis_names`: `[u8; 64]` — Null-terminated comma-separated list of axis names (e.g., `b"time,depth,lat,lon\0"`).
  - `units`: `[u8; 64]` — Null-terminated physical measurement units (e.g., `b"micromoles_per_liter\0"`).
- **Payload Layout**:
  - Contiguous row-major (C-order) primitive elements representing the active tensor block. Byte length equals $\prod_{i=0}^{\text{ndim}-1} \text{shape}[i] \times \text{sizeof(dtype)}$.
- **Required vs Optional Fields**:
  - Required: `ndim`, `dtype`, `shape[0..ndim]`, `chunk_coords[0..ndim]`.
  - Optional: `axis_names` (defaults to empty string), `units` (defaults to arbitrary units).
- **Integrity Rules**:
  - `payload_len` MUST strictly equal $\prod \text{shape} \times \text{sizeof(dtype)}$.
  - `ndim` MUST NOT exceed 8.
  - Every active dimension $\text{shape}[i]$ MUST be $> 0$.
- **Academic Research Example**: 4D Functional Magnetic Resonance Imaging (fMRI) volumetric brain scans ($64 \times 64 \times 32 \times 1200$ voxels over time) capturing Blood-Oxygen-Level-Dependent (BOLD) signals during language acquisition tasks.
- **Why It Matters**: Multidimensional array tiles provide native, self-describing storage for tensor datasets without requiring external scientific formats like HDF5 or NetCDF. Researchers can directly map multidimensional observations into NumPy or PyTorch arrays for accelerated machine learning and scientific modeling.

---

#### 14.3.4 Vector Embedding Tile (`vector`) — Type ID: `67` (`0x0043`) (Core)
- **Status**: **IMPLEMENTED** (Writer: `TileWriter::write_hyper_tile` in `crates/s3a-engine/src/lib.rs:621`; Reader: `QuerySieve::query_embeddings_spatial` in `crates/s3a-engine/src/lib.rs:1877`; Test: `test_composable_s3a_ql_algebraic_scoring` in `crates/s3a-engine/src/lib.rs:3954`; SIMD distance kernels: `cosine_similarity` in `crates/s3a-simd/src/lib.rs:136`).
- **Purpose**: High-density dense embedding storage supporting approximate nearest neighbor (ANN) search, semantic similarity retrieval, and SIMD-accelerated distance metrics (Cosine, Dot Product, Euclidean L2).
- **Header Extension Fields**:
  - `dimensions`: `u32` — Dimensionality of the embedded vector space (e.g. 128, 256, 384, 768, 1536).
  - `metric_kind`: `u16` — Mathematical distance metric ($0 = \text{Cosine}$, $1 = \text{DotProduct}$, $2 = \text{EuclideanL2}$, $3 = \text{Manhattan}$).
  - `quantization`: `u16` — Numerical representation ($0 = \text{Float32}$, $1 = \text{Int8}$, $2 = \text{Packed4Bit}$).
  - `model_name`: `[u8; 48]` — UTF-8 name of the foundation/embedding model that produced the vectors.
  - `model_version`: `[u8; 16]` — Canonical version or checkpoint hash of the embedding model.
- **Payload Layout**:
  - For `Float32`: Contiguous sequence of `[RecordId (u64), Timestamp (u64), [f32; dimensions]]`.
  - For `Int8`: Contiguous sequence of `[RecordId (u64), Timestamp (u64), Scale (f32), Offset (f32), Flags (u64), [i8; dimensions]]`.
  - For `Packed4Bit`: Contiguous sequence of `[RecordId (u64), Timestamp (u64), Scale (f32), Offset (f32), Flags (u64), [u8; dimensions / 2]]`.
- **Required vs Optional Fields**:
  - Required: `dimensions` ($> 0$), `metric_kind`, `quantization`.
  - Optional: `model_name` and `model_version` (RECOMMENDED for scientific reproducibility).
- **Integrity Rules**:
  - Vector element counts MUST match `dimensions`.
  - When `metric_kind == 0` (Cosine), vectors stored in `Float32` SHOULD be normalized ($|v| = 1.0 \pm 10^{-4}$).
  - Tile `SimplexHull` (§3.2.1) MUST enclose all vector endpoints to support bounding-sphere branch pruning during search.
- **Academic Research Example**: 768-dimensional SPECTER2 document embeddings representing semantic concept vectors for 12,000 biomedical review papers in an Alzheimer's disease meta-analysis.
- **Why It Matters**: Dedicated vector tiles embed neural representations alongside model provenance, ensuring semantic embeddings remain reproducible across evolving foundation model checkpoints. Native AVX-512 and NEON SIMD kernels can directly scan embeddings to power sub-millisecond semantic search without separate vector databases.

---

#### 14.3.5 Chunked Blob Tile (`blob`) — Type ID: `68` (`0x0044`) (Core)
- **Status**: **IMPLEMENTED** (Writer: `LrsStore::store_statement` in `crates/s3a-engine/src/lrs_store.rs:1779`; Reader: `LrsStore::get_attachment` in `crates/s3a-engine/src/lrs_store.rs:2545`; Test: `test_attachments_store_and_retrieve` in `crates/s3a-engine/src/lrs_store.rs:3512`).
- **Purpose**: Storage of arbitrary unstructured binary objects (PDF manuscripts, high-resolution microscopy TIFFs, audio captures, raw spectrometer dumps) partitioned into deterministic 128 KB chunk streams.
- **Header Extension Fields**:
  - `blob_id`: `[u8; 16]` — 128-bit UUID uniquely identifying the overall binary object.
  - `total_blob_size`: `u64` — Total unfragmented byte length of the complete binary object across all chunks.
  - `chunk_index`: `u32` — 0-based sequential chunk index within the object ($0 \le \text{chunk\_index} < \text{total\_chunks}$).
  - `total_chunks`: `u32` — Total number of Hyper-Tile chunks comprising this binary object.
  - `mime_type`: `[u8; 32]` — Null-terminated ASCII media type (e.g., `b"application/pdf\0"`, `b"image/tiff\0"`).
  - `chunk_hash`: `[u8; 32]` — SHA-256 cryptographic digest calculated exclusively over this tile's payload chunk.
- **Payload Layout**:
  - Raw binary byte stream containing up to 130,560 bytes of the active slice: `blob_bytes[chunk_index * 130560 .. (chunk_index + 1) * 130560]`.
- **Required vs Optional Fields**:
  - Required: `blob_id`, `total_blob_size`, `chunk_index`, `total_chunks`, `chunk_hash`.
  - Optional: `mime_type` (defaults to `b"application/octet-stream\0"`).
- **Integrity Rules**:
  - `chunk_hash` MUST match the SHA-256 digest of active payload bytes.
  - If `chunk_index == total_chunks - 1`, `payload_len` MUST equal `total_blob_size - (chunk_index * 130560)`.
  - Readers assembling the blob MUST verify that all chunks $0 \le i < \text{total\_chunks}$ are contiguous and share the same `blob_id`.
- **Academic Research Example**: High-resolution 80 MB cryo-electron microscopy (Cryo-EM) protein structure density map `.mrc` files attached directly alongside their atomic model coordinates.
- **Why It Matters**: Chunked blob tiles allow massive non-tabular research assets to reside in the exact same container as their analytical metadata, eliminating broken external URL links. Per-chunk cryptographic digests guarantee that raw primary source data has never suffered silent corruption or bit rot.

---

#### 14.3.6 Secondary Index Tile (`index`) — Type ID: `69` (`0x0045`) (Core)
- **Status**: **IMPLEMENTED** (Writer: `TileWriter::write_hyper_tile_ext` in `crates/s3a-engine/src/lib.rs:461` and `bloom_filter_insert` in `crates/s3a-simd/src/lib.rs:271`; Reader: `can_reject_tile_bloom` in `crates/s3a-simd/src/lib.rs:301`; Test: `test_simd_bloom_filter_rejection` in `crates/s3a-simd/src/lib.rs:459`).
- **Purpose**: Secondary associative indexing structures providing sub-millisecond point lookups, inverted key sieving, and range scans over primary record tiles without full-archive table scans.
- **Header Extension Fields**:
  - `target_tile_type`: `u16` — Tile `type_id` indexed by this structure (e.g. `64` / `0x0040` for records).
  - `index_kind`: `u16` — Mathematical indexing structure ($1 = \text{BTree}$, $2 = \text{HashTable}$, $3 = \text{InvertedList}$, $4 = \text{LearnedSplineCDF}$).
  - `key_type`: `u16` — Data type of indexed keys ($1 = \text{u64}$, $2 = \text{UUID128}$, $3 = \text{Hash256}$, $4 = \text{StringPrefix32}$).
  - `fanout_or_buckets`: `u32` — B-Tree branch fanout factor or Hash bucket slot count.
- **Payload Layout**:
  - For B-Trees: Page-aligned node structures containing sorted `[Key, TargetTileIndex (u32), RecordOffset (u32)]` tuples.
  - For Hash Tables: Power-of-two linear probing slot arrays containing `[KeyHash (u64), TargetTileIndex (u32), RecordOffset (u32)]`.
  - For Inverted Lists: Key identifiers mapped to variable-length, delta-compressed post lists of matching tile indices.
- **Required vs Optional Fields**:
  - Required: `target_tile_type`, `index_kind`, `key_type`.
  - Optional: `fanout_or_buckets` (inferred from node capacity if 0).
- **Integrity Rules**:
  - Keys within B-Tree leaf and interior nodes MUST be strictly monotonically ascending.
  - Target tile references (`TargetTileIndex`) MUST refer to valid, existing tiles within the same archive file.
  - `payload_hash` covers the entire binary index serialization.
- **Academic Research Example**: Inverted author and DOI index mapping 1,500,000 digital object identifiers directly to their publication record tiles across a decade of physics preprints.
- **Why It Matters**: Secondary index tiles turn massive multi-gigabyte research archives into instantly queryable databases without requiring external search engines. Researchers can locate specific records among billions of observations in a single disk seek.

---

### 14.4 Trust & Integrity Tile Specifications

#### 14.4.1 Cryptographic Seal Tile (`seal`) — Type ID: `70` (`0x0046`) (Core Trust)
- **Status**: **IMPLEMENTED** (Writer: `seal_archive` in `crates/s3a-engine/src/sealed.rs:646` / `seal_archive_v2` in `crates/s3a-engine/src/sealed.rs:613`; Reader: `verify_sealed_archive` in `crates/s3a-engine/src/sealed.rs:669` / `read_sealed_index_v2` in `crates/s3a-engine/src/sealed.rs:163`; Test: `test_seal_and_verify_roundtrip` in `crates/s3a-engine/src/sealed.rs:1347` / `test_index_lookup_reads_only_footer_bytes` in `crates/s3a-engine/tests/sealed_footer_v2.rs:66` / `test_cli_seal_anchor_upgrade_and_verify` in `crates/s3a-cli/tests/cli_ops.rs:114`).
- **Purpose**: Establishes verifiable container immutability, Merkle epoch chain links, and cryptographic anchor proofs anchored to public decentralized timestamping calendars.
- **Header Extension Fields**:
  - `snapshot_hash`: `[u8; 32]` — Authoritative canonical archive fingerprint (SHA-256 over directory entries §11.1).
  - `prev_seal_hash`: `[u8; 32]` — SHA-256 fingerprint of the previous sealed epoch in an evolving dataset chain (all zeros for genesis).
  - `epoch_sequence`: `u64` — Strictly monotonic epoch revision number ($0, 1, 2, \dots$).
  - `sealed_timestamp`: `u64` — Unix epoch timestamp when the sealing operation was finalized.
  - `anchor_count`: `u32` — Number of cryptographic attestation proofs embedded in this tile's payload.
- **Payload Layout**:
  - Sequence of serialized `AnchorProof` structures (§11.3):
    1. `proof_type`: `u16` ($1 = \text{OpenTimestamps}$, $2 = \text{RFC3161}$).
    2. `proof_len`: `u32` — Byte length of proof payload.
    3. `proof_bytes`: Binary OTS execution graph or ASN.1 DER `TimeStampResp` token.
- **Required vs Optional Fields**:
  - Required: `snapshot_hash`, `prev_seal_hash`, `epoch_sequence`, `sealed_timestamp`.
  - Optional: `anchor_count` (0 if sealed locally prior to public calendar submission).
- **Integrity Rules**:
  - `snapshot_hash` MUST reconcile byte-for-byte with the archive's canonical directory fingerprint.
  - Embedded OpenTimestamps proofs MUST trace a valid Merkle path matching the independent Bitcoin block header at the attested height.
  - Embedded RFC 3161 proofs MUST validate against trusted X.509 root certificates and match `messageImprint`.
- **Academic Research Example**: Tamper-evident freezing of a completed phase-III oncology clinical trial dataset prior to submission to regulatory authorities (FDA/EMA).
- **Why It Matters**: Seal tiles provide mathematical certainty that scientific datasets have remained untouched since their recorded moment of publication or submission. By chaining cryptographic proofs to public timestamping infrastructure, researchers establish undeniable chronological priority for scientific breakthroughs.

---

#### 14.4.2 Subject Consent & Governance Tile (`consent`) — Type ID: `71` (`0x0047`) (Core Trust)
- **Status**: **IMPLEMENTED** (Writer: `S3ACrudEngine::grant_consent` in `crates/s3a-engine/src/lib.rs:2106`, `TileWriter::write_hyper_tile` in `crates/s3a-engine/src/lib.rs:621`; Reader: `QuerySieve::query_consent_records` in `crates/s3a-engine/src/lib.rs:1426`; Test: `test_consent_grant_query_visible` in `crates/s3a-engine/tests/consent_and_schema_tiles.rs:85`).
- **Purpose**: Enforces declarative research participant consent boundaries, institutional review board (IRB) scope constraints, and GDPR/HIPAA compliance directly within the storage layer.
- **Header Extension Fields**:
  - `record_count`: `u32` — Number of `ConsentRecord` structures embedded in this tile.
  - `payload_bytes`: `u32` — Byte length of active consent record stream.
- **Payload Layout**:
  - Contiguous array of `ConsentRecord` structures (exactly 136 bytes each, 8-byte aligned, `Pod`/`Zeroable`):
    - `subject_id`: `[u8; 16]` — 128-bit pseudonymized UUID participant identifier (offset 0..16).
    - `study_id`: `u64` — Institutional Review Board protocol / study ID (offset 16..24).
    - `purpose`: `u32` — Declared processing purpose (`1 = Research`, `2 = Analytics`, `3 = ModelTraining`, `4 = Sharing`) (offset 24..28).
    - `scope`: `u32` — Tile type / record kind mask covered (0 = full wildcard) (offset 28..32).
    - `granted_at`: `u64` — Unix epoch timestamp when consent was granted (offset 32..40).
    - `expires_at`: `u64` — Unix epoch expiration timestamp (0 = no expiration) (offset 40..48).
    - `withdrawn_at`: `u64` — Unix epoch withdrawal timestamp (0 = active) (offset 48..56).
    - `legal_hold`: `u8` — Legal hold indicator (`1 = ActiveLegalHold`, `0 = Standard`) (offset 56..57).
    - `_reserved`: `[u8; 7]` — 8-byte alignment padding (offset 57..64).
    - `granted_by`: `u64` — Actor ID of authorized human/admin actor (offset 64..72).
    - `prev_consent_hash`: `[u8; 32]` — Cryptographic chain digest over prior subject consent record (offset 72..104).
    - `consent_hash`: `[u8; 32]` — SHA-256 cryptographic digest over preceding 104 bytes (offset 104..136).
- **Required vs Optional Fields**:
  - Required: `subject_id`, `study_id`, `purpose`, `scope`, `granted_at`, `granted_by`, `consent_hash`.
  - Optional: `expires_at` (0 = indefinite), `withdrawn_at` (0 = not withdrawn), `legal_hold` (0 = no hold).
- **Integrity Rules**:
  - **Append-Only Governance**: Consent revocation is strictly append-only; existing records are NEVER edited in place. The latest record per `(subject_id, study_id, purpose)` determines active consent status.
  - **Engine Read/Export Enforcement**: `QuerySieve` evaluates subject consent for the declared purpose before returning records. Queries and exports with a declared purpose skip records whose participant lacks active consent for that purpose (default export purpose = `Research`; `ModelTraining` strictly requires explicit consent).
  - **Legal Hold Invariant**: If `legal_hold != 0` on an active consent record or subject, compliant purge (`purge_telemetry_record`) and compaction routines MUST refuse deletion and MUST preserve all records.
  - **Tile 19 Compatibility**: Tile 19 administrative marks (`MARK_CONSENT_WITHDRAWN`, `MARK_LEGAL_HOLD`) remain fully honored; Tile 71 is additive without requiring data migration.
  - **Human Authorization Gate**: Consent records can ONLY be authored by authorized human/administrative actors (`is_admin_or_human == true`); autonomous AI agents are forbidden from authoring consent records.
  - **Hash Chain Verification**: Per-subject cryptographic hashes (`prev_consent_hash` $\to$ `consent_hash`) form an immutable audit chain verifiable via `QuerySieve::verify_consent_chain`.
- **Academic Research Example**: Longitudinal psychiatric cohort consent tracking permitting academic depression biomarker discovery while strictly forbidding commercial re-identification.
- **Why It Matters**: Consent tiles embed human research participant rights directly into the physical storage substrate rather than relying on external policy software. Compliant analytical engines can automatically mask or refuse access to participant data when consent expires or purposes do not match.

---

#### 14.4.3 Multi-Agent Provenance Tile (`provenance`) — Type ID: `72` (`0x0048`) (Core Trust)
- **Status**: **IMPLEMENTED** (Writer: `ScholarCrudExt::create_ai_traces` in `crates/s3a-pack-scholar/src/ext.rs:66`; Reader: `ScholarQueryExt::query_ai_traces` in `crates/s3a-pack-scholar/src/ext.rs:378`; Test: `test_dual_trace_human_and_ai_cross_reference` in `crates/s3a-pack-scholar/src/lib.rs:215`; Agent registry tracking in `crates/s3a-pack-scholar/src/ext.rs:159`).
- **Purpose**: Audits the end-to-end lineage of research findings, capturing whether artifacts were produced by a human researcher, an autonomous AI agent, or a foundation model, including model versions, parameters, and input record digests.
- **Header Extension Fields**:
  - `actor_kind`: `u16` — Nature of the producing entity ($1 = \text{HumanResearcher}$, $2 = \text{AutonomousAiAgent}$, $3 = \text{FoundationModel}$, $4 = \text{LaboratorySensor}$).
  - `model_version`: `[u8; 32]` — Canonical checkpoint hash or release string of the producing model.
  - `tool_call_id`: `u64` — Deterministic identifier linking to the execution tool step or laboratory protocol.
  - `input_hash_count`: `u16` — Number of upstream input artifact hashes committed in the payload.
- **Payload Layout**:
  - Header parameters followed by an array of 32-byte cryptographic hashes `[InputRecordHash; input_hash_count]` referencing the exact prior records from which this finding was derived.
- **Required vs Optional Fields**:
  - Required: `actor_kind`, `input_hash_count`.
  - Optional: `model_version` (REQUIRED if `actor_kind` is AI or Model), `tool_call_id`.
- **Integrity Rules**:
  - If `actor_kind == 2` (AI Agent), the agent ID MUST be registered in the container's active `agent_registry` tile.
  - All declared upstream `InputRecordHash` digests SHOULD resolve to committed records within the current or parent archives.
- **Academic Research Example**: Automated systematic literature review provenance documenting that a meta-analysis summary was generated by an AI research agent analyzing 42 specific paper hashes.
- **Why It Matters**: Provenance tiles solve the reproducibility crisis by tracking whether scientific claims originated from human experimentation, automated instruments, or generative AI models. Reviewers can verify the exact computational lineage and input artifacts that produced every scientific figure and finding.

---

#### 14.4.4 Right-to-be-Forgotten Erasure Tile (`erasure`) — Type ID: `73` (`0x0049`) (Core Trust)
- **Status**: **IMPLEMENTED** (Writer: `S3ACrudEngine::record_erasure` in `crates/s3a-engine/src/lib.rs:2208`, `S3ACrudEngine::create_erasure_record` in `crates/s3a-engine/src/lib.rs:2176`; Reader: `QuerySieve::query_erasure_records` in `crates/s3a-engine/src/lib.rs:1542`, `QuerySieve::verify_erasure_chain` in `crates/s3a-engine/src/lib.rs:1563`; Test: `test_erase_record_gone_after_compaction_and_receipt_remains_and_verifies` in `crates/s3a-engine/tests/erasure_tile.rs:69`, `test_legal_hold_blocks_erasure` in `crates/s3a-engine/tests/erasure_tile.rs:136`).
- **Purpose**: Provides verifiable cryptographic audit receipts proving permanent data destruction under GDPR Article 17, HIPAA, or IRB mandates while preserving archive Merkle chain continuity.
- **Header Extension Fields**:
  - `record_count`: `u32` — Number of `ErasureRecord` audit receipts embedded in this tile.
  - `payload_bytes`: `u32` — Byte length of active erasure record stream.
- **Payload Layout**:
  - **Zero Bytes of Erased Data Retained**: The payload NEVER retains any erased target data. The payload contains contiguous 144-byte `ErasureRecord` audit receipts (`[ErasureRecord; record_count]`, 8-byte aligned, `Pod`/`Zeroable`):
    - `erased_record_hash`: `[u8; 32]` — SHA-256 digest of the destroyed record (offset 0..32).
    - `target_tile_type`: `u16` — Tile type of target record (offset 32..34).
    - `target_record_index`: `u32` — Record index within target tile (offset 34..38).
    - `reason`: `u16` — Regulatory justification (`1 = Withdrawal`, `2 = RetentionExpiry`, `3 = LegalRequest`, `4 = Correction`) (offset 38..40).
    - `authority_id`: `u64` — Authorized human/admin compliance actor ID (offset 40..48).
    - `requested_at`: `u64` — Unix epoch timestamp when destruction was requested (offset 48..56).
    - `executed_at`: `u64` — Unix epoch timestamp when purge was finalized (offset 56..64).
    - `prev_erasure_hash`: `[u8; 32]` — Cryptographic chain digest over prior erasure receipt (offset 64..96).
    - `erasure_hash`: `[u8; 32]` — SHA-256 cryptographic digest over preceding 96 bytes (offset 96..128).
    - `_reserved`: `[u8; 16]` — Alignment padding (offset 128..144).
- **Required vs Optional Fields**:
  - Required: `erased_record_hash`, `target_tile_type`, `target_record_index`, `reason`, `authority_id`, `requested_at`, `executed_at`, `erasure_hash`.
  - Optional: `prev_erasure_hash` (zeros for genesis erasure record).
- **Integrity Rules**:
  - **Zero Payload Retained**: Erased data bytes are never retained in the tile payload.
  - **Purge Execution**: Compaction overwrites the target record bytes with zero bytes (`0x00`) via `TileFusionEngine`.
  - **Legal Hold Enforcement**: An active legal hold (via `ConsentRecord::legal_hold`) strictly blocks erasure purge and returns a clear error.
  - **Idempotency & Auditing**: Writing multiple erasure records for the same record hash is idempotent.
  - **Cryptographic Receipt Chain**: Erasure receipts form a per-record cryptographic chain (`prev_erasure_hash` $\to$ `erasure_hash`), verified via `QuerySieve::verify_erasure_chain`.
  - **Human Authorization Gate**: Erasure can only be requested by an authorized human/admin actor (`is_admin_or_human == true`); AI agents are forbidden from authorizing erasure.
- **Academic Research Example**: Formal GDPR Article 17 erasure certificate proving that a withdrawn participant's genomic sequencing reads have been purged from a university biobank repository.
- **Why It Matters**: Erasure tiles provide verifiable mathematical proof of compliance with privacy mandates without corrupting the cryptographic integrity of historical archive chains. Institutions can audit that sensitive data was irreversibly destroyed under valid legal authority while maintaining immutable audit trails.

---

### 14.5 Structural & Semantic Tile Specifications

#### 14.5.1 Attributed Graph Tile (`graph`) — Type ID: `74` (`0x004A`) (Core Structure)
- **Status**: **IMPLEMENTED** (Writer: `ScholarCrudExt::create_research_graph_edges` in `crates/s3a-pack-scholar/src/ext.rs:118`; Reader: `ScholarQueryExt::query_research_graph` in `crates/s3a-pack-scholar/src/ext.rs:478`; Test: `test_srs_snapshot_export_and_intake_roundtrip` in `crates/s3a-pack-scholar/src/lib.rs:270`).
- **Purpose**: High-performance storage of attributed networks, citation graphs, taxonomic hierarchies, and knowledge graphs in cache-aligned binary arrays.
- **Header Extension Fields**:
  - `node_count`: `u32` — Total count of discrete vertices/nodes represented in this graph segment.
  - `edge_count`: `u32` — Total count of directed or undirected connections in this segment.
  - `graph_kind`: `u16` — Graph topology ($1 = \text{Directed}$, $2 = \text{Undirected}$, $3 = \text{Bipartite}$, $4 = \text{Hypergraph}$).
  - `node_type`: `u16` — Entity type identifier for nodes.
  - `edge_type`: `u16` — Relationship type identifier for edges (e.g. `cites`, `refutes`, `collaborates_with`).
- **Payload Layout**:
  - Compact Compressed Sparse Row (CSR) representation:
    1. `NodeOffsets`: Array of `u32` offsets into the edge array (`node_count + 1` entries).
    2. `EdgeTargets`: Contiguous array of destination node IDs (`edge_count` entries of `u32` or `u64`).
    3. `EdgeWeights`: Optional contiguous array of `f32` relationship weights.
- **Required vs Optional Fields**:
  - Required: `node_count`, `edge_count`, `graph_kind`, `node_type`, `edge_type`.
  - Optional: `EdgeWeights` (defaults to unweighted edges if omitted).
- **Integrity Rules**:
  - All destination node indices in `EdgeTargets` MUST be $< \text{node\_count}$ (or refer to valid external node keys).
  - CSR node offsets MUST be strictly monotonic: $\text{NodeOffsets}[i] \le \text{NodeOffsets}[i+1]$.
- **Academic Research Example**: Global scientific citation graph connecting 450,000 papers with 6,200,000 citation edges categorized as supportive, contradictory, or neutral.
- **Why It Matters**: Graph tiles store complex relational topologies in cache-aligned binary arrays optimized for multi-hop graph traversal algorithms. Researchers can run PageRank, community detection, or claim verification algorithms directly on memory-mapped files without spinning up external graph databases.

---

#### 14.5.2 Monotonic Time-Series Tile (`time-series`) — Type ID: `75` (`0x004B`) (Core Structure)
- **Status**: **IMPLEMENTED** (Writer: `TileWriter::write_hyper_tile` in `crates/s3a-engine/src/lib.rs:621`; Reader: `MmapReader::get_tile` in `crates/s3a-engine/src/lib.rs:798`; Test: `test_tile_writer_and_mmap_reader_roundtrip` in `crates/s3a-engine/src/lib.rs:3348`).
- **Purpose**: Compact streaming storage of monotonically ordered temporal metrics supporting SIMD delta-of-delta compression, downsampling, and learned monotonic index lookups.
- **Header Extension Fields**:
  - `time_unit`: `u16` — Temporal unit of timestamps ($0 = \text{Seconds}$, $1 = \text{Milliseconds}$, $2 = \text{Microseconds}$, $3 = \text{Nanoseconds}$).
  - `encoding_mode`: `u16` — Encoding algorithm ($0 = \text{Raw}$, $1 = \text{DeltaFrameOfReference}$, $2 = \text{DoubleDelta}$, $3 = \text{GorillaFloat}$).
  - `base_timestamp`: `u64` — Minimum baseline timestamp offset for the entire tile.
  - `sample_interval`: `u32` — Regular sampling period in `time_unit` increments, or 0 for irregular streams.
- **Payload Layout**:
  - Packed bitstream:
    1. Bit-packed timestamp deltas relative to `base_timestamp`.
    2. Bit-packed or quantized value observations.
- **Required vs Optional Fields**:
  - Required: `time_unit`, `encoding_mode`, `base_timestamp`.
  - Optional: `sample_interval` (0 indicates variable sampling frequency).
- **Integrity Rules**:
  - Timestamps MUST be strictly monotonically non-decreasing ($\Delta t \ge 0$).
  - Decoded timestamps MUST fall within the range $[\text{min\_timestamp}, \text{max\_timestamp}]$ declared in the tile header.
- **Academic Research Example**: 250 Hz continuous biometric wearable telemetry recording photoplethysmography (PPG) pulse intervals and galvanic skin conductance during cognitive fatigue trials.
- **Why It Matters**: Time-series tiles compress dense temporal sensor telemetry by up to 90% using SIMD-optimized integer and floating-point delta algorithms. Analytical engines can binary-search billions of timestamped data points in microseconds using embedded monotonic learned index models.

---

#### 14.5.3 Geospatial & Mesh Tile (`spatial`) — Type ID: `76` (`0x004C`) (Core Structure)
- **Status**: **IMPLEMENTED** (Writer: `GisCrudExt::create_gis_points` in `crates/s3a-pack-gis/src/ext.rs:14`; Reader: `GisMmapExt::query_gis_mesh_3d` in `crates/s3a-pack-gis/src/ext.rs:70`; Test: `test_gis_write_and_query_mesh_3d` in `crates/s3a-pack-gis/src/lib.rs:91`).
- **Purpose**: Storage of 2D/3D vector geometries, topographic coordinates, and spatial meshes supporting spatial R-Tree filtering and bounding-hull branch pruning.
- **Header Extension Fields**:
  - `srid`: `u32` — Spatial Reference System Identifier (e.g. `4326` for WGS84, `3857` for Web Mercator).
  - `geometry_type`: `u16` — Primitive geometry ($1 = \text{Point}$, $2 = \text{LineString}$, $3 = \text{Polygon}$, $4 = \text{MultiPolygon}$, $5 = \text{Mesh3D}$).
  - `index_kind`: `u16` — Spatial clustering algorithm ($1 = \text{MortonZOrder}$, $2 = \text{HilbertCurve}$, $3 = \text{RTreeLeaf}$).
  - `bbox`: `[f64; 6]` — Bounding box coordinates $[X_{\min}, Y_{\min}, Z_{\min}, X_{\max}, Y_{\max}, Z_{\max}]$.
- **Payload Layout**:
  - FlatGeobuf or Well-Known Binary (WKB) coordinate buffers followed by polygon rings or vertex/triangle mesh indices.
- **Required vs Optional Fields**:
  - Required: `srid`, `geometry_type`, `bbox`.
  - Optional: `index_kind` (defaults to unindexed coordinate list if 0).
- **Integrity Rules**:
  - All vertices and geometry points in the payload MUST fall strictly within the declared `bbox` volume.
  - Polygons MUST be topologically valid (closed exterior rings with counter-clockwise winding).
- **Academic Research Example**: High-resolution LIDAR canopy elevation mesh and forest ecology biodiversity survey plots across 1,000 square kilometers of conservation parkland.
- **Why It Matters**: Spatial tiles integrate geographic and geometric indexing directly into tile headers to prune non-matching geographic regions before reading payload data. Spatial data scientists can run fast bounding-box filters and spatial joins on terabyte-scale datasets directly from disk.

---

#### 14.5.4 Embedded Schema Registry Tile (`schema`) — Type ID: `77` (`0x004D`) (Core Structure)
- **Status**: **IMPLEMENTED** (Writer: `append_schema_tile_if_missing` in `crates/s3a-engine/src/lib.rs:1292`, `Compactor::compact_pass` in `crates/s3a-engine/src/lib.rs:2703`; Reader: `MmapReader::get_schema_definitions` in `crates/s3a-engine/src/lib.rs:908`, `MmapReader::resolve_tile_type_name` in `crates/s3a-engine/src/lib.rs:924`; Test: `test_schema_tile_roundtrip` in `crates/s3a-engine/tests/consent_and_schema_tiles.rs:395`, `test_inspect_shows_schema_names` in `crates/s3a-engine/tests/consent_and_schema_tiles.rs:512`).
- **Purpose**: Embeds canonical field dictionaries, data types, and layout contracts directly into the archive file, enabling self-describing, zero-dependency data interchange and decades-long archival readability.
- **Header Extension Fields**:
  - `record_count`: `u32` — Must be 0 for variable-length serialized schema payload.
  - `record_layout_version`: `u16` — Layout version for the schema binary encoding (1).
  - `payload_bytes`: `u32` — Total byte length of encoded schema payload.
- **Payload Layout**:
  - Binary encoding: one `CompactSchemaHeader`, then `type_count` `CompactSchemaTypeRecord`s, then all `total_fields` `CompactSchemaFieldRecord`s (fields are grouped in type order, not interleaved). All little-endian. Layout as implemented in `crates/s3a-core/src/schema.rs` (corrected in v1.5; earlier text listed a `magic` field and a different field order that the code never wrote):
    1. `CompactSchemaHeader` (32 bytes): `schema_id: u32` (1), `schema_format: u16` (1 = compact binary), `type_count: u16`, `total_fields: u32`, `schema_crc32: u32` (CRC32C over all type and field record bytes), `_reserved: [u8; 16]`.
    2. `CompactSchemaTypeRecord` (80 bytes): `type_id: u32`, `type_version: u32`, `name: [u8; 32]`, `owning_pack: [u8; 32]`, `record_size: u32` (0 = variable), `field_count: u16`, `_reserved: u16`.
    3. `CompactSchemaFieldRecord` (48 bytes): `field_id: u16`, `field_type: u16`, `byte_offset: u32`, `byte_len: u32`, `null_allowed: u8`, `_reserved: [u8; 3]`, `name: [u8; 32]`. Names are NUL-padded UTF-8.
- **Catalog reader contract (B1, v1.5)**:
  - Readers MUST decode records without assuming alignment (`pod_read_unaligned`); a payload at any offset MUST NOT cause a panic.
  - The sum of `field_count` over all type records MUST equal `total_fields`; otherwise the tile is invalid.
  - `MmapReader::catalog()` uses the first Schema tile. Its tile `header_crc32` and `data_crc32` (when non-zero) and its `schema_crc32` MUST all validate; any failure is an error (`CatalogError`), never a silent fallback.
  - If the file has no Schema tile, readers MUST fall back to the compiled-in tile-type registry. Files without a Schema tile remain fully valid in v1.x.
  - A catalog entry for a registered type SHOULD agree with the registry on `record_size` and fall within its supported layout-version window (`schema::catalog_mismatches`). Disagreement is reported, not auto-corrected.
  - Edge-profile archives use a separate `u16` type namespace (77 = `TILE_TYPE_CONFIG` there) and never carry this tile.
- **Required vs Optional Fields**:
  - Required: `magic`, `version`, `type_count`, `total_fields`, `schema_crc32`.
  - Optional: Field definitions can be empty for opaque binary tiles.
- **Integrity Rules**:
  - **Seal and Compaction Emission**: On compaction (`Compactor::compact_pass`) and seal (`seal_file`), the engine automatically emits a Schema tile enumerating every tile type present in the container.
  - **Graceful Reader Fallback**: Readers encountering unknown tile types query embedded schema definitions to resolve human-readable type names and field layouts, safely skipping payloads they cannot natively decode.
  - **CLI `inspect` Integration**: `s3a-cli inspect` resolves tile type names from the embedded Schema tile, displaying the canonical names instead of `"UNKNOWN"`.
  - `schema_crc32` MUST validate against decoded type and field descriptor bytes.
- **Academic Research Example**: Embedded data dictionary in a public 20-year astronomical sky survey archive defining telescope sensor calibration parameters and spectral band classifications.
- **Why It Matters**: Embedded schema tiles guarantee that archive files remain completely decipherable decades into the future without relying on external compiler tools or source code repositories. Scientific archivists can inspect, validate, and convert historical datasets using self-contained structural metadata stored right inside the container.

---

#### 14.5.5 Manifest Tile (`manifest` / `78` / `0x004E`)

- **Purpose**: Provides a human-readable, plain-text UTF-8 manifest placed at the beginning of sealed S3A archives (Tile 0, byte offset 512, payload starting at byte 1024) to ensure immediate decipherability in any standard text editor (`head`, `less`, VS Code, Notepad) without requiring specialized tooling.
- **Placement & Layout**:
  - Located at Tile 0 (byte offset 512) directly following the 512-byte Dual File Header.
  - Payload starts at byte offset 1024 (`FILE_HEADER_SIZE + HYPER_TILE_HEADER_SIZE`).
  - Standard `HyperTileHeader`: `tile_type = 78`, `record_count = 0`, `record_layout_version = 1`, `payload_bytes = len(manifest_utf8)`.
  - CRC32C covers the UTF-8 payload bytes in `data_crc32`.
- **Contents**:
  - `"THIS IS AN S3A FILE"` banner with official specification URL.
  - Archive metadata: File title/purpose, creator (vault gate address/name if present), file UUID, created and sealed timestamps.
  - Tile directory table listing floor index, type ID, type name, record count, and tile size.
  - Consent governance summary: aggregate participant consent counts (total, active research, withdrawn), privacy status notice, with zero payload or personal identifier leaks.
  - Seal integrity status, generation, previous seal hash, and parity configuration.
  - Plain-text instruction banner: `"how to read: s3a-cli tour <file> or the S3A reader app"`.
- **Integrity & Reader Rules**:
  - Compliant readers handle the manifest via the unknown-type skip rule (§7.2); existing v1/v2 readers open archives containing Tile 78 without modification.
  - `s3a-cli tour <file>` renders the manifest in both text lobby output and floor detail inspection (`--floor 0`), as well as JSON output (`lobby.manifest`).

---

#### 14.5.6 Embedded Specification Tile (`specification` / `79` / `0x004F`)

- **Purpose**: Self-documenting storage container holding the complete S3A Storage Format Specification (`S3A-TILE-SPEC.md`) directly within the sealed file when the `--embed-spec` flag is provided during sealing.
- **Placement & Compression**:
  - Located at Tile 1 (offset 73,728 / 72 KiB) immediately following the Tile 0 Manifest tile.
  - Compressed via standard Deflate (zlib-compatible, RFC 1950/1951) into the hyper-tile payload buffer.
  - `HyperTileHeader`: `tile_type = 79`, `record_count = 0`, `record_layout_version = 1`, `payload_bytes = len(compressed_bytes)`.
  - CRC32C covers the compressed payload bytes in `data_crc32`.
- **Integrity & Reader Rules**:
  - Transparently decompressed on demand via `MmapReader::get_embedded_spec()`.
  - Archived files remain 100% self-describing and self-documenting across decades of technological shifts.

---

### 14.6 Adoption Examples Matrix

The table below illustrates how diverse academic disciplines and research workflows compose multiple tile types within a single unified S3A archive container:

| Research Use Case / Dataset | Composed Tile Types | Role of Each Tile Type in the Dataset | Realized Architectural Advantage |
|:---|:---|:---|:---|
| **Scholar Study**<br>*(Longitudinal Multimodal Learning Trial)* | • `record` (`64` / `0x0040`)<br>• `time-series` (`75` / `0x004B`)<br>• `vector` (`67` / `0x0043`)<br>• `provenance` (`72` / `0x0048`)<br>• `consent` (`71` / `0x0047`) | • `record`: Learner assessment scores, demographics, and task completion logs.<br>• `time-series`: Micro-second keystroke and pupil dilation telemetry streams.<br>• `vector`: 384-D concept mastery embeddings.<br>• `provenance`: Distinguishes student submissions from AI tutoring interventions.<br>• `consent`: IRB ethics approvals and age-of-majority participant permissions. | Unifies heterogeneous biometric streams, assessment scores, and participant privacy rules into a single portable container without relational database sprawl. |
| **Citation Graph**<br>*(Bibliometric Open Access Corpus)* | • `graph` (`74` / `0x004A`)<br>• `record` (`64` / `0x0040`)<br>• `blob` (`68` / `0x0044`)<br>• `index` (`69` / `0x0045`)<br>• `seal` (`70` / `0x0046`) | • `graph`: 10-million-edge citation and co-authorship topology.<br>• `record`: Paper publication dates, journal venue metadata, and author IDs.<br>• `blob`: Complete Open Access PDF manuscripts and LaTeX source tarballs.<br>• `index`: Sub-millisecond inverted search index on paper DOIs and ISBNs.<br>• `seal`: Daily OpenTimestamps Bitcoin anchoring proving corpus state at snapshot time. | Enables multi-hop citation traversals and full-text document retrieval in a single memory-mapped container with cryptographic proof of document authenticity. |
| **Semantic Paper Search**<br>*(Neural Literature Discovery Engine)* | • `vector` (`67` / `0x0043`)<br>• `index` (`69` / `0x0045`)<br>• `record` (`64` / `0x0040`)<br>• `schema` (`77` / `0x004D`) | • `vector`: 768-D SPECTER2 abstract embeddings.<br>• `index`: Quantized inverted vector cluster indices and learned CDF search models.<br>• `record`: Paper titles, abstract text strings, and venue classifications.<br>• `schema`: Embedded field dictionary defining biomedical keyword taxonomies. | Delivers sub-millisecond nearest-neighbor search directly against raw file storage, eliminating the need to deploy and synchronize an external vector database cluster. |
| **IRB Consent Audit**<br>*(Longitudinal Clinical Research Biobank)* | • `consent` (`71` / `0x0047`)<br>• `erasure` (`73` / `0x0049`)<br>• `seal` (`70` / `0x0046`)<br>• `provenance` (`72` / `0x0048`) | • `consent`: Subject-specific consent permissions, study scopes, and expiration dates.<br>• `erasure`: Verifiable zero-payload destruction receipts for withdrawn participants.<br>• `seal`: Chained epoch seals proving audit logs were not retrospectively altered.<br>• `provenance`: Institutional compliance officer authorizations and tool logs. | Guarantees compliance with GDPR Article 17 and HIPAA audits by providing cryptographic, tamper-evident proof that sensitive medical data was destroyed under lawful order. |

---

### 14.7 Implementation Status Summary

The table below catalogs all 16 tile abstractions defined in this section and summarizes their implementation status within the current reference codebase (`crates/s3a-core`, `crates/s3a-engine`, `crates/s3a-simd`, `crates/s3a-pack-*`, `crates/s3a-cli`).

Per specification audit policy, a tile type is classified as **IMPLEMENTED** only if an on-disk writer AND reader exist in `crates/` alongside a dedicated automated test verifying round-trip operation. Any type lacking an on-disk writer, reader, or test in `crates/` is classified as **SPECIFIED-ONLY**.

| Canonical Type Name | Type ID (Dec) | Type ID (Hex) | Category | Status | Verified Codebase Citations (Writer, Reader, Test) |
|:---|:---:|:---:|:---:|:---:|:---|
| `record` | 64 | `0x0040` | Core | **IMPLEMENTED** | **Writer**: `TileWriter::write_hyper_tile` (`crates/s3a-engine/src/lib.rs:621`)<br>**Reader**: `MmapReader::get_tile` (`crates/s3a-engine/src/lib.rs:798`)<br>**Test**: `test_tile_writer_and_mmap_reader_roundtrip` (`crates/s3a-engine/src/lib.rs:3348`) |
| `columnar` | 65 | `0x0041` | Core | **IMPLEMENTED** | **Writer**: `encode_columnar_telemetry` (`crates/s3a-engine/src/lib.rs:56`) / `Compactor::compact_with_columnar_opt_in` (`crates/s3a-engine/src/lib.rs:2630`)<br>**Reader**: `decode_columnar_telemetry` (`crates/s3a-engine/src/lib.rs:108`) / `QuerySieve::query_telemetry_with_coords` (`crates/s3a-engine/src/lib.rs:1700`)<br>**Test**: `test_roundtrip_equality_vs_record_tile` (`crates/s3a-engine/tests/columnar_tile.rs:39`) / `test_compaction_opt_in_columnar_roundtrip` (`crates/s3a-engine/tests/columnar_tile.rs:198`) |
| `array` | 66 | `0x0042` | Core | **IMPLEMENTED** | **Writer**: `S3ACrudEngine::write_array_chunk` (`crates/s3a-engine/src/lib.rs:2448`) / `ArrayTileBuilder::build` (`crates/s3a-core/src/array.rs:192`)<br>**Reader**: `S3ACrudEngine::read_array_slice` (`crates/s3a-engine/src/lib.rs:2476`) / `ArrayTileView::parse` (`crates/s3a-core/src/array.rs:206`)<br>**Test**: `test_array_roundtrip_2d_3d_4d` (`crates/s3a-engine/tests/array_tile.rs:43`) / `test_array_slice_across_chunk_boundaries` (`crates/s3a-engine/tests/array_tile.rs:98`) |
| `vector` | 67 | `0x0043` | Core | **IMPLEMENTED** | **Writer**: `TileWriter::write_hyper_tile` (`crates/s3a-engine/src/lib.rs:621`)<br>**Reader**: `QuerySieve::query_embeddings_spatial` (`crates/s3a-engine/src/lib.rs:1877`)<br>**Test**: `test_composable_s3a_ql_algebraic_scoring` (`crates/s3a-engine/src/lib.rs:3954`) |
| `blob` | 68 | `0x0044` | Core | **IMPLEMENTED** | **Writer**: `LrsStore::store_statement` attachments (`crates/s3a-engine/src/lrs_store.rs:1779`)<br>**Reader**: `LrsStore::get_attachment` (`crates/s3a-engine/src/lrs_store.rs:2545`)<br>**Test**: `test_attachments_store_and_retrieve` (`crates/s3a-engine/src/lrs_store.rs:3512`) |
| `index` | 69 | `0x0045` | Core | **IMPLEMENTED** | **Writer**: `TileWriter::write_hyper_tile_ext` (`crates/s3a-engine/src/lib.rs:461`) / `bloom_filter_insert` (`crates/s3a-simd/src/lib.rs:271`)<br>**Reader**: `can_reject_tile_bloom` (`crates/s3a-simd/src/lib.rs:301`)<br>**Test**: `test_simd_bloom_filter_rejection` (`crates/s3a-simd/src/lib.rs:459`) |
| `seal` | 70 | `0x0046` | Trust | **IMPLEMENTED** | **Writer**: `seal_archive` (`crates/s3a-engine/src/sealed.rs:646`) / `seal_archive_v2` (`crates/s3a-engine/src/sealed.rs:613`)<br>**Reader**: `verify_sealed_archive` (`crates/s3a-engine/src/sealed.rs:669`) / `read_sealed_index_v2` (`crates/s3a-engine/src/sealed.rs:163`)<br>**Test**: `test_seal_and_verify_roundtrip` (`crates/s3a-engine/src/sealed.rs:1347`) / `test_index_lookup_reads_only_footer_bytes` (`crates/s3a-engine/tests/sealed_footer_v2.rs:66`) |
| `consent` | 71 | `0x0047` | Trust | **IMPLEMENTED** | **Writer**: `S3ACrudEngine::grant_consent` (`crates/s3a-engine/src/lib.rs:2106`) / `TileWriter::write_hyper_tile` (`crates/s3a-engine/src/lib.rs:621`)<br>**Reader**: `QuerySieve::query_consent_records` (`crates/s3a-engine/src/lib.rs:1426`)<br>**Test**: `test_consent_grant_query_visible` (`crates/s3a-engine/tests/consent_and_schema_tiles.rs:85`) |
| `provenance` | 72 | `0x0048` | Trust | **IMPLEMENTED** | **Writer**: `ScholarCrudExt::create_ai_traces` (`crates/s3a-pack-scholar/src/ext.rs:66`)<br>**Reader**: `ScholarQueryExt::query_ai_traces` (`crates/s3a-pack-scholar/src/ext.rs:378`)<br>**Test**: `test_dual_trace_human_and_ai_cross_reference` (`crates/s3a-pack-scholar/src/lib.rs:215`) |
| `erasure` | 73 | `0x0049` | Trust | **IMPLEMENTED** | **Writer**: `S3ACrudEngine::record_erasure` (`crates/s3a-engine/src/lib.rs:2208`) / `S3ACrudEngine::create_erasure_record` (`crates/s3a-engine/src/lib.rs:2176`)<br>**Reader**: `QuerySieve::query_erasure_records` (`crates/s3a-engine/src/lib.rs:1542`) / `QuerySieve::verify_erasure_chain` (`crates/s3a-engine/src/lib.rs:1563`)<br>**Test**: `test_erase_record_gone_after_compaction_and_receipt_remains_and_verifies` (`crates/s3a-engine/tests/erasure_tile.rs:69`) / `test_legal_hold_blocks_erasure` (`crates/s3a-engine/tests/erasure_tile.rs:136`) |
| `graph` | 74 | `0x004A` | Structure | **IMPLEMENTED** | **Writer**: `ScholarCrudExt::create_research_graph_edges` (`crates/s3a-pack-scholar/src/ext.rs:118`)<br>**Reader**: `ScholarQueryExt::query_research_graph` (`crates/s3a-pack-scholar/src/ext.rs:478`)<br>**Test**: `test_srs_snapshot_export_and_intake_roundtrip` (`crates/s3a-pack-scholar/src/lib.rs:270`) |
| `time-series` | 75 | `0x004B` | Structure | **IMPLEMENTED** | **Writer**: `TileWriter::write_hyper_tile` (`crates/s3a-engine/src/lib.rs:621`)<br>**Reader**: `MmapReader::get_tile` (`crates/s3a-engine/src/lib.rs:798`)<br>**Test**: `test_tile_writer_and_mmap_reader_roundtrip` (`crates/s3a-engine/src/lib.rs:3348`) |
| `spatial` | 76 | `0x004C` | Structure | **IMPLEMENTED** | **Writer**: `GisCrudExt::create_gis_points` (`crates/s3a-pack-gis/src/ext.rs:14`)<br>**Reader**: `GisMmapExt::query_gis_mesh_3d` (`crates/s3a-pack-gis/src/ext.rs:70`)<br>**Test**: `test_gis_write_and_query_mesh_3d` (`crates/s3a-pack-gis/src/lib.rs:91`) |
| `schema` | 77 | `0x004D` | Structure | **IMPLEMENTED** | **Writer**: `append_schema_tile_if_missing` (`crates/s3a-engine/src/lib.rs:1292`) / `Compactor::compact_pass` (`crates/s3a-engine/src/lib.rs:2703`)<br>**Reader**: `MmapReader::get_schema_definitions` (`crates/s3a-engine/src/lib.rs:908`) / `MmapReader::resolve_tile_type_name` (`crates/s3a-engine/src/lib.rs:924`)<br>**Test**: `test_schema_tile_roundtrip` (`crates/s3a-engine/tests/consent_and_schema_tiles.rs:395`) / `test_inspect_shows_schema_names` (`crates/s3a-engine/tests/consent_and_schema_tiles.rs:512`) |
| `manifest` | 78 | `0x004E` | Structure | **IMPLEMENTED** | **Writer**: `seal_buffer_v2` / `generate_plain_text_manifest` (`crates/s3a-engine/src/sealed.rs:154`)<br>**Reader**: `MmapReader::get_manifest` (`crates/s3a-engine/src/lib.rs:945`) / `s3a-cli tour` (`crates/s3a-cli/src/tour.rs:315`)<br>**Test**: `test_manifest_present_and_plain_text_readable_after_seal` (`crates/s3a-engine/tests/manifest_tile.rs:59`) / `test_tour_displays_manifest_in_lobby_and_floor` (`crates/s3a-cli/tests/tour_tests.rs:449`) |
| `specification` | 79 | `0x004F` | Structure | **IMPLEMENTED** | **Writer**: `seal_buffer_v2` (`crates/s3a-engine/src/sealed.rs:743`)<br>**Reader**: `MmapReader::get_embedded_spec` (`crates/s3a-engine/src/lib.rs:962`)<br>**Test**: `test_embedded_spec_tile_roundtrip` (`crates/s3a-engine/tests/manifest_tile.rs:245`) |




## Appendix: Record-Root Tile (Type ID `83` / `0x0053`, ADR-0012)

Optional tile listing per-tile RFC 6962 record roots. Payload: `record_count` entries of 64 bytes,
little-endian: `tile_index u32`, `tile_type u32`, `tile_id u64`, `record_count u64`,
`seal_generation u64`, `root [u8; 32]`. A tile is covered iff `record_count > 0` and
`payload_bytes` divides evenly into `record_count` records; `root` is the RFC 6962 Merkle Tree Hash
of those records (leaf `SHA-256(0x00||r)`, node `SHA-256(0x01||l||r)`).

- Writers append it as the last tile(s) immediately before sealing; the v2 seal hash
  (`compute_seal_hash_v2`) is unchanged and covers it like any tile.
- It is derived data. Compactors and fusion MUST drop it; sealers MUST drop any prior record-root
  tile and MAY regenerate it. It never carries forward a stale root.
- Verifiers that know it recompute every root and report any missing, extra or differing entry,
  or entries with mixed `seal_generation`. Readers that don't know it skip it (§7.2).
- Files without it are unaffected.
