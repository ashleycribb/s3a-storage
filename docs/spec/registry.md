# S3A Tile Type Registry

This registry lists all assigned wire tile type identifiers (`HyperTileHeader.tile_type`), their owning subsystem or domain pack, capability requirements, and lifecycle status in accordance with the S3A Hyper-Tile Storage Specification.

## 1. Registry Architecture & Allocation Bands

Tile type identifiers are 32-bit unsigned integers (`u32`) partitioned into designated allocation bands:

| Range | Allocation Policy | Governance |
|:---|:---|:---|
| **0 – 9** | Core Storage Primitives & Reserved Boundary | Core Architecture Specification |
| **3 – 6** | First-Class Hardware & Domain Packs (Legacy frozen assignments inside core band)¹ | Standard Architecture Extensions |
| **10 – 18** | Scholar Explorer & Formal Ontology Packs | Research & Knowledge Provenance Extensions |
| **19** | Core Metadata Marks (Quarantine, Consent, Legal Hold, Retention) | Core Policy Specification |
| **20** | Research Notes & Verified Claims (Scholar Pack) | Research & Knowledge Provenance Extensions |
| **21 – 24** | Core xAPI 1.0.3 Learning Record Store (LRS) | Core LRS Specification |
| **25 – 31** | Core Enterprise Learner Record (ELR) Projections (IEEE P2997) | Core ELR Specification |
| **32** | Cryptographic Citation Fingerprints (Scholar Pack, spec amendment v1.2) | Research Provenance Extensions |
| **33** | Spatial Computing OGC GeoPose Basic-Quaternion (Spatial Pack, spec amendment) | Standard Domain Packs |
| **34 – 35** | Unallocated (standard domain packs, via spec amendment) | Standards Working Group |
| **36 – 40** | Vault identity/delegation/holding/transfer/event log (`s3a-vault`, in-tree; not yet listed in §2) | Vault Specification |
| **41 – 46** | Technical Order / OEM maintenance manual records (TO Pack, spec amendment) | Standard Domain Packs |
| **47 – 63** | Reserved for Standard Domain Packs (allocated via spec amendment) | Standards Working Group |
| **64 – $2^{32}-1$** | Open Third-Party & Vendor Extensions | Uncoordinated / Vendor Addons |

> ¹ **Note on IDs 3–6**: Tile IDs 3–6 (`wearable_micro`, `robotics_kinematic`, `gis_survey_mesh`, `data_availability`) are legacy pack assignments allocated inside the core band (0–9) and are now frozen. Greenfield standard domain packs MUST use IDs in the **32–63** band (via specification amendment), and third-party/application extensions MUST use **64+**.

---

## 2. Master Tile Type Assignments

| Tile ID | Canonical Name | Owner / Crate | Required Addon | Record Size | Payload Type | Status |
|:---:|:---|:---|:---:|:---:|:---:|:---:|
| **0** | `telemetry` | Core Engine (`s3a-core`) | None | 32 B | Fixed Pod | Active |
| **1** | `embedding_128` | Core Engine (`s3a-core`) | None | 528 B¹ | Fixed Pod | Active |
| **2** | `hybrid_index` | Core Engine (`s3a-core`) | None | Variable | Inverted Index | Active |
| **3** | `wearable_micro` | Wearable Pack (`s3a-pack-wearable`) | `0x0008` (`ADDON_WEARABLE`) | 16 B | Micro-Tile Pod | Active |
| **4** | `robotics_kinematic` | Robotics Pack (`s3a-pack-robotics`) | `0x0004` (`ADDON_ROBOTICS`) | 72 B | Fixed Pod | Active |
| **5** | `gis_survey_mesh` | GIS Pack (`s3a-pack-gis`) | `0x0010` (`ADDON_GIS`) | 32 B | Fixed Pod | Active |
| **6** | `data_availability` | Data Availability Pack (`s3a-pack-da`) | `0x0020` (`ADDON_DA`) | 64 B | Fixed Pod | Active |
| **7** | `quantized_embedding_int8` | Core Engine (`s3a-core`) | None | 288 B² | Fixed Pod | Active |
| **8** | `quantized_embedding_4bit` | Core Engine (`s3a-core`) | None | 288 B³ | Fixed Pod | Active |
| **9** | `reserved_boundary` | Core Engine (`s3a-core`) | None | 0 B | Boundary Guard | **Reserved**⁴ |
| **10** | `human_lrs` | Scholar Pack (`s3a-pack-scholar`) | `0x0001` (`ADDON_EDU`) | 64 B | Fixed Pod | Active |
| **11** | `ai_trace_log` | Scholar Pack (`s3a-pack-scholar`) | `0x0001` (`ADDON_EDU`) | 64 B | Fixed Pod | Active |
| **12** | `academic_papers` | Scholar Pack (`s3a-pack-scholar`) | `0x0001` (`ADDON_EDU`) | 64 B | Fixed Pod | Active |
| **13** | `research_graph` | Scholar Pack (`s3a-pack-scholar`) | `0x0001` (`ADDON_EDU`) | 64 B | Fixed Pod | Active |
| **14** | `srs_workspace` | Scholar Pack (`s3a-pack-scholar`) | `0x0001` (`ADDON_EDU`) | 64 B | Fixed Pod | Active |
| **15** | `srs_manifest` | Scholar Pack (`s3a-pack-scholar`) | `0x0001` (`ADDON_EDU`) | 512 B | Manifest Header | Active |
| **16** | `ontology_concept` | Ontology Pack (`s3a-pack-ontology`) | `0x0040` (`ADDON_ONTOLOGY`) | 64 B | Fixed Pod | Active |
| **17** | `ontology_axiom` | Ontology Pack (`s3a-pack-ontology`) | `0x0040` (`ADDON_ONTOLOGY`) | 64 B | Fixed Pod | Active |
| **18** | `agent_registry` | Scholar Pack (`s3a-pack-scholar`) | `0x0001` (`ADDON_EDU`) | 64 B | Fixed Pod | Active |
| **19** | `marks` | Core Policy (`s3a-core`) | None | 64 B | Fixed Pod | Active |
| **20** | `note_claims` | Scholar Pack (`s3a-pack-scholar`) | `0x0001` (`ADDON_EDU`) | 64 B | Fixed Pod | Active |
| **21** | `statement_body` | Core LRS (`s3a-core`) | None | Variable | 64B Hdr + JSON Blob | Active |
| **22** | `statement_index` | Core LRS (`s3a-core`) | None | 64 B | Fixed Pod | Active |
| **23** | `document_store` | Core LRS (`s3a-core`) | None | Variable | 64B Hdr + Doc Blob | Active |
| **24** | `attachment_blob` | Core LRS (`s3a-core`) | None | Variable | 64B Hdr + Raw Blob | Active |
| **25** | `elr_person` | Core ELR (`s3a-core`) | None | Variable | 96B Hdr + JSON | Active |
| **26** | `elr_organization` | Core ELR (`s3a-core`) | None | Variable | 96B Hdr + JSON | Active |
| **27** | `elr_learning_resource` | Core ELR (`s3a-core`) | None | Variable | 96B Hdr + JSON | Active |
| **28** | `elr_learning_event` | Core ELR (`s3a-core`) | None | Variable | 96B Hdr + JSON | Active |
| **29** | `elr_competency_assertion` | Core ELR (`s3a-core`) | None | Variable | 96B Hdr + JSON | Active |
| **30** | `elr_credential_assertion` | Core ELR (`s3a-core`) | None | Variable | 96B Hdr + JSON | Active |
| **31** | `elr_goal` | Core ELR (`s3a-core`) | None | Variable | 96B Hdr + JSON | Active |
| **32** | `citation_fingerprint` | Scholar Pack (`s3a-pack-scholar`) | `0x0001` (`ADDON_EDU`) | 96 B | Fixed Pod | Active |
| **33** | `geopose_basic_quat` | Spatial Pack (`s3a-spatial`) | `0x0080` (`ADDON_SPATIAL`) | 128 B | Fixed Pod | Active |
| **41** | `to_section` | Technical Order Pack (`s3a-pack-to`) | `0x0100` (`ADDON_TO`) | 128 B | Fixed Pod | Active |
| **42** | `to_wcn` | Technical Order Pack (`s3a-pack-to`) | `0x0100` (`ADDON_TO`) | 128 B | Fixed Pod | Active |
| **43** | `to_fault_node` | Technical Order Pack (`s3a-pack-to`) | `0x0100` (`ADDON_TO`) | 128 B | Fixed Pod | Active |
| **44** | `to_fault_edge` | Technical Order Pack (`s3a-pack-to`) | `0x0100` (`ADDON_TO`) | 128 B | Fixed Pod | Active |
| **45** | `to_spec_cell` | Technical Order Pack (`s3a-pack-to`) | `0x0100` (`ADDON_TO`) | 160 B | Fixed Pod | Active |
| **46** | `to_figure_anchor` | Technical Order Pack (`s3a-pack-to`) | `0x0100` (`ADDON_TO`) | 160 B | Fixed Pod | Active |
| **47** | `can_dtc` | CAN Bridge (`s3a-can-bridge`) | `0x0200` (`ADDON_CAN`) | 128 B | Fixed Pod | Active |
| **48** | `can_trigger` | CAN Bridge (`s3a-can-bridge`) | `0x0200` (`ADDON_CAN`) | 128 B | Fixed Pod | Active |
| **49** | `to_procedure_step` | Technical Order Pack (`s3a-pack-to`) | `0x0100` (`ADDON_TO`) | 160 B | Fixed Pod | Active |
| **50** | `to_reference` | Technical Order Pack (`s3a-pack-to`) | `0x0100` (`ADDON_TO`) | 128 B | Fixed Pod | Active |
| **51** | `to_locator` | Technical Order Pack (`s3a-pack-to`) | `0x0100` (`ADDON_TO`) | 160 B | Fixed Pod | Active |
| **52** | `to_scan_agreement` | Technical Order Pack (`s3a-pack-to`) | `0x0100` (`ADDON_TO`) | 384 B | Fixed Pod | Active |
| **64** | `record` | Core Engine (`s3a-core`) | None | 64 B | Fixed Pod | Active |
| **68** | `blob` | Core Engine (`s3a-core`) | None | Variable | 40B Prefix + Raw Chunk | Active |
| **72** | `provenance` | Core Engine (`s3a-core`) | None | Variable | 40B Prefix + Provenance Chunk | Active |
| **77** | `schema` | Core Engine (`s3a-core` codec, `s3a-engine` reader) | None | Variable | Compact Binary Catalog (§14.5.4) | Active⁶ |
| **78** | `manifest` | Core Engine (`s3a-core`) | None | Variable | Plain UTF-8 Text | Active |
| **79** | `specification` | Core Engine (`s3a-core`) | None | Variable | Compressed Markdown (Deflate) | Active |
| **83** | `record_root` | Core Engine (`s3a-core`, `s3a-engine::record_roots`) | None | 64 B | Fixed Pod | Active⁷ |

---

## 3. Reference Implementation Notes

1. **Tile 1 Size**: `EmbeddingRecord128` wire layout is exactly 528 bytes (`u64` id + `u64` timestamp + `[f32; 128]` vector). In `registry.rs`, descriptor field `record_size` registers 544 bytes due to historic padding allocation.
2. **Tile 7 Size**: `QuantizedEmbeddingRecord256` wire layout is 288 bytes (`u64` id + `u64` timestamp + `f32` scale + `f32` offset + `u64` flags + `[i8; 256]` vector). In `registry.rs`, descriptor field `record_size` registers 40 bytes.
3. **Tile 8 Size**: `QuantizedEmbeddingRecord512` wire layout is 288 bytes (`u64` id + `u64` timestamp + `f32` scale + `f32` offset + `u64` flags + `[u8; 256]` packed nibbles). In `registry.rs`, descriptor field `record_size` registers 24 bytes.
4. **Tile 9 Boundary**: Tile ID 9 is strictly `RESERVED_BOUNDARY`. Writers MUST NOT write to Tile ID 9 (`RegistryError::ReservedTile(9)`). The legacy prototype `learning_record_store` assignment is deprecated; greenfield educational activity records MUST be written to Tile ID 10 (`HumanLrsRecord`) or Tile ID 28 (`elr_learning_event`).
5. **Tile 19 Policy Marks**:
   - `MARK_QUARANTINE` (1): Integrity quarantine; readers hide by default.
   - `MARK_NEEDS_REVIEW` (2): Supervisory review flag.
   - `MARK_CONSENT_WITHDRAWN` (3): Privacy consent withdrawn; used for actor delete requests; readers hide by default.
   - `MARK_LEGAL_HOLD` (4): Regulatory preservation; cannot be purged.
   - `MARK_ACCESS_HEAT` (5): Ephemeral caching metric.
   - `MARK_RETENTION_EXPIRED` (6): Retention window elapsed. Used exclusively by retention purge routines before compaction (never conflated with consent withdrawal). The purge receipt records the expiration reason. Readers hide expired records by default (`is_record_hidden`).

---

## 4. Addon Capability Mask Allocation

The `FileHeader.addon_mask` bitmask registers authorized domain extensions enabled on an archive:

| Bit Position | Bitmask Value | Addon Constant | Owner Pack | Scope |
|:---:|:---:|:---|:---|:---|
| **0** | `0x0001` | `ADDON_EDU` | `s3a-pack-scholar` | Scholar Explorer, SRS snapshots, dual-trace provenance |
| **1** | `0x0002` | `ADDON_FINTECH` | Reserved | High-frequency financial tick streaming (reserved) |
| **2** | `0x0004` | `ADDON_ROBOTICS` | `s3a-pack-robotics` | Kinematic 3D trajectory streaming |
| **3** | `0x0008` | `ADDON_WEARABLE` | `s3a-pack-wearable` | Wearable micro-telemetry & 4 KB Flash pages |
| **4** | `0x0010` | `ADDON_GIS` | `s3a-pack-gis` | Topographic 3D mesh & geospatial indexing |
| **5** | `0x0020` | `ADDON_DA` | `s3a-pack-da` | Data availability & state commitments |
| **6** | `0x0040` | `ADDON_ONTOLOGY` | `s3a-pack-ontology` | Formal concept ontology & axiom registry |
| **7** | `0x0080` | `ADDON_SPATIAL` | `s3a-spatial` | OGC GeoPose Basic-Quaternion records |
| **8** | `0x0100` | `ADDON_TO` | `s3a-pack-to` | Technical Order and OEM maintenance manual records |
| **9** | `0x0200` | `ADDON_CAN` | `s3a-can-bridge` | J1939/OBD ingest and fail-closed fault-isolation triggers |
| **10 – 31** | `0x0400`.. | -- | Unallocated | Reserved for future standardization |

---

## 5. Adding New Tile Types

When defining a new domain pack or third-party tile type:
1. **Packs MUST Claim IDs in Allocated Bands**:
   - Standard domain packs claim IDs within 32–63 through specification amendment.
   - Third-party, experimental, or private extensions MUST use tile IDs $\ge 64$.
2. **Readers MUST Handle Unknown Tile Types Gracefully**:
   - Compliant readers MUST skip unknown tile types during targeted queries.
   - Compliant compactors and tools MUST preserve unknown tile types intact ("copy-through"), never discarding them or failing.
6. **Tile 77 Schema (B1, ADR-0008)**: In-file catalog of tile types. Assigned since the §14 catalog; listed here by B1. It is not yet a descriptor in `crates/s3a-core/src/registry.rs` (writes go through the unregistered-type path). `MmapReader::catalog()` returns it when present and falls back to the registry when absent. Note: the edge profile's separate `u16` namespace uses 77 for `TILE_TYPE_CONFIG`; the two namespaces never mix.
7. **Tile 83 Record Root (ADR-0012)**: Optional, derived. One 64-byte entry per tile whose payload splits into `record_count` equal records: `tile_index u32 | tile_type u32 | tile_id u64 | record_count u64 | seal_generation u64 | root [u8;32]` (little-endian), root = RFC 6962 Merkle Tree Hash of the records. Written last, just before sealing, so the unchanged v2 seal hash covers it. Not `COPY_THROUGH_COMPACT`: compactor/fusion drop it and every seal drops and (if `--record-roots`) regenerates it. Readers that don't know it skip it (§7.2).

## 6. Algorithm IDs

Archives may declare the algorithms they use in the about entry (TILE-SPEC §14.5.4, keys `seal_hash_alg` and `sig_alg`). This table is the only list of valid IDs. It follows RFC 7696 (BCP 201) §2.1: "Once an algorithm identifier is added to the registry, it should not be changed or removed. However, it is desirable to mark a registry entry as deprecated when implementation is no longer advisable." ([RFC 7696](https://www.rfc-editor.org/rfc/rfc7696))

| ID | Key | Status | Definition | Notes |
|---|---|---|---|---|
| `sha256` | `seal_hash_alg` | recommended | SHA-256, [FIPS 180-4](https://csrc.nist.gov/pubs/fips/180-4/upd1/final) | Implied when the key is absent. Every S3A file to date uses it. |
| `none` | `sig_alg` | allowed | No signature | Unsigned archive. |
| `ed25519` | `sig_alg` | recommended | Ed25519 (PureEdDSA), [RFC 8032 §5.1](https://www.rfc-editor.org/rfc/rfc8032) | 64-byte signature over the 32-byte seal hash (S3A-EDGE-PROFILE §6.2). RFC 8032 §8.5: "if a high 128-bit security level is enough, use of Ed25519 is RECOMMENDED". |
| `ml-dsa-44` | `sig_alg` | reserved | ML-DSA-44, [FIPS 204](https://csrc.nist.gov/pubs/fips/204/final) | S3A encoding not yet specified. |
| `ml-dsa-65` | `sig_alg` | reserved | ML-DSA-65, FIPS 204 | As above. |
| `ml-dsa-87` | `sig_alg` | reserved | ML-DSA-87, FIPS 204 | As above. |

Why ML-DSA is reserved: RFC 8032 §1 says "A sufficiently large quantum computer would be able to break both" Ed25519 and Ed448. FIPS 204 specifies ML-DSA, which "is believed to be secure, even against adversaries in possession of a large-scale quantum computer" (FIPS 204 abstract). Reserving the IDs now means a future file can name them without a format change.

### 6.1 Statuses

- **recommended**: writers SHOULD use it, and every reader MUST implement it.
- **allowed**: writers MAY use it, and every reader MUST implement it.
- **deprecated**: writers MUST NOT create new data with it, but readers MUST still verify it on existing files. Archives are stored data: RFC 7696 §4 says "To recover the protected stored data, protocol implementations need to support older algorithms, even when they no longer use the older algorithms for the protection of new stored data."
- **reserved**: the ID is assigned, but its S3A encoding is not specified yet. Readers MUST reject files that declare it (`UnsupportedAlgorithm`), and writers MUST NOT write it.
- An ID that is not in this table for that key is **unknown**. Readers MUST reject it (`UnknownAlgorithm`) and name the ID in the error.

### 6.2 Deprecation policy

1. An entry is never removed and an ID is never reused, so old files keep their meaning.
2. Status moves only forward: reserved → allowed or recommended → deprecated. Each change is a spec changelog entry plus a conformance suite version.
3. Give notice before deprecating (RFC 7696 §2.2.3: "authors SHOULD provide notice to implementers about expected algorithm transitions"). The notice goes in the changelog one spec version before the status changes.
4. Deprecating an ID never makes an existing file unreadable. To move an archive to a new algorithm, re-seal it into a new file. The old file stays verifiable.
5. Promoting a reserved ID needs three things: an S3A encoding in the spec, a reference implementation, and conformance vectors.

### 6.3 Adding an algorithm

Open a spec PR that adds a row with a fetched link to the algorithm's defining standard, the reference code, and a new conformance suite version. Keep the list short. RFC 7696 §3.2: once deployed, "it is quite difficult to remove algorithms"; §4: "Support for too many algorithms can lead to implementation vulnerabilities."

Reference code: `s3a_core::algorithms` (Rust) and `ALGORITHMS` in `readers/python/s3a_reader/reader.py`. A test keeps them in sync.

---

## 7. Tile Type Record Field Specifications

Every tile type registered in the schema registry defines its canonical fields, wire type codes, byte offsets, and byte widths. Conforming implementations embed these descriptors in sealed archives so readers with no pack code can inspect and decode fields.

### 7.1 Core Storage Tiles

#### Tile 0: `telemetry` (Record Size: 32 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `timestamp` | U64 (4) | 0 | 8 | false | Timestamp in epoch microseconds |
| 2 | `sensor_id` | U32 (3) | 8 | 4 | false | Sensor identifier |
| 3 | `metric_id` | U32 (3) | 12 | 4 | false | Metric type identifier |
| 4 | `value` | F64 (10) | 16 | 8 | false | 64-bit IEEE 754 floating point reading |
| 5 | `flags` | U64 (4) | 24 | 8 | false | Provenance and quality flags |

#### Tile 1: `embedding_128` (Record Size: 528 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `id` | U64 (4) | 0 | 8 | false | 64-bit embedding identifier |
| 2 | `timestamp` | U64 (4) | 8 | 8 | false | Generation timestamp |
| 3 | `vector` | BYTES (13) | 16 | 512 | false | 128-dimensional f32 dense vector |

#### Tile 19 / 20: `marks` (Record Size: 64 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `mark_id` | U64 (4) | 0 | 8 | false | Unique identifier of mark assertion |
| 2 | `target_tile_type` | U32 (3) | 8 | 4 | false | Tile type of target record |
| 3 | `mark_kind` | U32 (3) | 12 | 4 | false | Kind of policy mark (quarantine, hold, etc.) |
| 4 | `target_tile_id` | U64 (4) | 16 | 8 | false | Target hyper-tile ID |
| 5 | `record_index` | U32 (3) | 24 | 4 | false | Index of record within target tile |
| 6 | `set_or_clear` | U8 (1) | 28 | 1 | false | 1 = set, 0 = clear |
| 7 | `timestamp` | U64 (4) | 32 | 8 | false | Action timestamp in epoch seconds |
| 8 | `actor_id` | U64 (4) | 40 | 8 | false | Identifier of actor asserting mark |

#### Tile 21: `statement_body` (Header Size: 64 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `statement_id` | BYTES (13) | 0 | 16 | false | 128-bit statement UUID |
| 2 | `stored_timestamp_ms` | U64 (4) | 16 | 8 | false | Store insertion time in epoch ms |
| 3 | `timestamp_ms` | U64 (4) | 24 | 8 | false | Client assertion time in epoch ms |
| 4 | `actor_hash` | U64 (4) | 32 | 8 | false | 64-bit hash of statement actor |
| 5 | `verb_hash` | U32 (3) | 40 | 4 | false | 32-bit hash of statement verb |
| 6 | `activity_hash` | U32 (3) | 44 | 4 | false | 32-bit hash of primary object activity |
| 7 | `registration_hash` | U32 (3) | 48 | 4 | false | 32-bit hash of registration context |
| 8 | `json_len` | U32 (3) | 52 | 4 | false | Byte length of statement JSON payload |
| 9 | `flags` | U32 (3) | 56 | 4 | false | Voiding and attachment status flags |
| 10 | `header_crc32` | U32 (3) | 60 | 4 | false | CRC32C of statement body header |

#### Tile 22: `statement_index` (Record Size: 64 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `statement_id` | BYTES (13) | 0 | 16 | false | 128-bit statement UUID |
| 2 | `stored_timestamp_ms` | U64 (4) | 16 | 8 | false | Store insertion time in epoch ms |
| 3 | `timestamp_ms` | U64 (4) | 24 | 8 | false | Client assertion time in epoch ms |
| 4 | `actor_hash` | U64 (4) | 32 | 8 | false | 64-bit hash of statement actor |
| 5 | `verb_hash` | U32 (3) | 40 | 4 | false | 32-bit hash of statement verb |
| 6 | `activity_hash` | U32 (3) | 44 | 4 | false | 32-bit hash of primary object activity |
| 7 | `registration_hash` | U32 (3) | 48 | 4 | false | 32-bit hash of registration context |
| 8 | `body_tile_id` | U32 (3) | 52 | 4 | false | Target tile ID containing statement body |
| 9 | `body_offset` | U32 (3) | 56 | 4 | false | Byte offset of body within target tile |
| 10 | `flags` | U32 (3) | 60 | 4 | false | Status flags |

#### Tile 23: `document_store` (Header Size: 64 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `doc_key_hash` | U64 (4) | 0 | 8 | false | 64-bit hash of document key |
| 2 | `created_at` | U64 (4) | 8 | 8 | false | Creation timestamp |
| 3 | `updated_at` | U64 (4) | 16 | 8 | false | Last update timestamp |
| 4 | `content_len` | U32 (3) | 24 | 4 | false | Byte length of document payload |
| 5 | `content_type` | U32 (3) | 28 | 4 | false | Document media type code |
| 6 | `flags` | U32 (3) | 32 | 4 | false | Status flags |
| 7 | `header_crc32` | U32 (3) | 36 | 4 | false | CRC32C of document header |

#### Tile 24: `attachment_blob` (Header Size: 64 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `attachment_sha256` | BYTES (13) | 0 | 32 | false | SHA-256 digest of attachment data |
| 2 | `byte_length` | U64 (4) | 32 | 8 | false | Total binary length in bytes |
| 3 | `content_type_hash` | U32 (3) | 40 | 4 | false | Hash of MIME content type |
| 4 | `flags` | U32 (3) | 44 | 4 | false | Status flags |
| 5 | `header_crc32` | U32 (3) | 48 | 4 | false | CRC32C of attachment header |

#### Tile 65: `columnar` (Header Size: Variable)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `column_count` | U16 (2) | 4 | 2 | false | Number of vertical columns |
| 2 | `record_count` | U32 (3) | 6 | 4 | false | Number of rows encoded |
| 3 | `compression_codec` | U16 (2) | 10 | 2 | false | Compression codec ID |
| 4 | `source_tile_type` | U32 (3) | 16 | 4 | false | Source row tile type |

#### Tile 66: `array` (Header Size: Variable)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `ndim` | U16 (2) | 6 | 2 | false | Number of tensor dimensions |
| 2 | `dtype` | U16 (2) | 8 | 2 | false | Numeric data type code |
| 3 | `compression_codec` | U16 (2) | 10 | 2 | false | Compression codec ID |
| 4 | `shape` | BYTES (13) | 16 | 64 | false | Tensor dimensions |
| 5 | `chunk_shape` | BYTES (13) | 80 | 64 | false | Chunk dimensions |
| 6 | `chunk_coords` | BYTES (13) | 144 | 64 | false | Chunk spatial coordinates |
| 7 | `fill_value` | BYTES (13) | 208 | 8 | false | Default background fill value |
| 8 | `chunk_hash` | BYTES (13) | 216 | 32 | false | Hash of chunk data |
| 9 | `axis_names` | F32 (9) | 248 | 64 | false | Named tensor axis labels |

#### Tile 71: `consent` (Record Size: 136 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `subject_id` | UUID (12) | 0 | 16 | false | 128-bit participant UUID |
| 2 | `study_id` | U64 (4) | 16 | 8 | false | Research study identifier |
| 3 | `purpose` | U32 (3) | 24 | 4 | false | Permitted purpose bitmask |
| 4 | `scope` | U32 (3) | 28 | 4 | false | Data sharing scope mask |
| 5 | `granted_at` | U64 (4) | 32 | 8 | false | Grant timestamp in epoch seconds |
| 6 | `expires_at` | U64 (4) | 40 | 8 | false | Expiration timestamp |
| 7 | `withdrawn_at` | U64 (4) | 48 | 8 | false | Withdrawal timestamp (0 if active) |
| 8 | `legal_hold` | U8 (1) | 56 | 1 | false | Legal hold flag (blocks erasure) |
| 9 | `granted_by` | U64 (4) | 64 | 8 | false | Authorizing actor identifier |
| 10 | `prev_consent_hash` | BYTES (13) | 72 | 32 | false | Preceding consent grant hash |
| 11 | `consent_hash` | BYTES (13) | 104 | 32 | false | Cryptographic hash of consent grant |

#### Tile 73: `erasure` (Record Size: 144 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `erased_record_hash` | BYTES (13) | 0 | 32 | false | Hash of erased payload record |
| 2 | `target_tile_type` | U32 (3) | 32 | 4 | false | Tile type of target record |
| 3 | `record_index` | U32 (3) | 36 | 4 | false | Index of record within tile |
| 4 | `target_tile_id` | U64 (4) | 40 | 8 | false | Target hyper-tile ID |
| 5 | `reason` | U32 (3) | 48 | 4 | false | Regulatory reason code |
| 6 | `_reserved_u32` | U32 (3) | 52 | 4 | false | Alignment padding |
| 7 | `authority` | U64 (4) | 56 | 8 | false | Authorizing governance identifier |
| 8 | `requested_at` | U64 (4) | 64 | 8 | false | Timestamp request was logged |
| 9 | `executed_at` | U64 (4) | 72 | 8 | false | Timestamp erasure was committed |
| 10 | `prev_erasure_hash` | BYTES (13) | 80 | 32 | false | Hash of prior erasure entry |
| 11 | `erasure_hash` | BYTES (13) | 112 | 32 | false | Cryptographic hash of erasure entry |

#### Tile 77: `schema` (Header Size: 32 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `schema_id` | U32 (3) | 0 | 4 | false | Schema catalog identifier |
| 2 | `schema_format` | U16 (2) | 4 | 2 | false | Format version code |
| 3 | `type_count` | U16 (2) | 6 | 2 | false | Number of type records encoded |
| 4 | `total_fields` | U32 (3) | 8 | 4 | false | Total field records in catalog |
| 5 | `schema_crc32` | U32 (3) | 12 | 4 | false | CRC32C over catalog records |

#### Tile 83: `record_root` (Record Size: 64 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `tile_index` | U32 (3) | 0 | 4 | false | 0-based index of tile in file |
| 2 | `tile_type` | U32 (3) | 4 | 4 | false | Wire tile type identifier |
| 3 | `tile_id` | U64 (4) | 8 | 8 | false | Logical hyper-tile identifier |
| 4 | `record_count` | U64 (4) | 16 | 8 | false | Number of records in tile |
| 5 | `seal_generation` | U64 (4) | 24 | 8 | false | Seal generation number |
| 6 | `root` | BYTES (13) | 32 | 32 | false | RFC 6962 Merkle tree root of records |

#### Tile 87: `graph_edge` (Record Size: 56 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `edge_id` | BYTES (13) | 0 | 16 | false | 128-bit edge UUID |
| 2 | `source_hash` | U64 (4) | 16 | 8 | false | Hash of source node key |
| 3 | `target_hash` | U64 (4) | 24 | 8 | false | Hash of target node key |
| 4 | `edge_type_hash` | U32 (3) | 32 | 4 | false | Hash of edge relation type string |
| 5 | `weight` | F32 (9) | 36 | 4 | false | Relation weight or confidence |
| 6 | `timestamp_sec` | U64 (4) | 40 | 8 | false | Creation timestamp in epoch seconds |
| 7 | `flags` | U64 (4) | 48 | 8 | false | Edge status flags |

---

### 7.2 Scholar Pack Tiles

#### Tile 10: `human_lrs` (Record Size: 64 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `session_uuid` | BYTES (13) | 0 | 16 | false | 128-bit shared session UUID |
| 2 | `timestamp_sec` | U64 (4) | 16 | 8 | false | Recorded timestamp |
| 3 | `actor_hash` | U64 (4) | 24 | 8 | false | Hash of human researcher |
| 4 | `verb_id` | U32 (3) | 32 | 4 | false | xAPI verb ID |
| 5 | `decision_score` | F32 (9) | 36 | 4 | false | Decision feedback score |
| 6 | `ai_trace_coord` | BYTES (13) | 40 | 16 | false | O(1) pointer to AI agent trace step |
| 7 | `object_hash` | U32 (3) | 56 | 4 | false | Hash of target paper or claim |
| 8 | `flags` | U32 (3) | 60 | 4 | false | Status flags |

#### Tile 11: `ai_trace_log` (Record Size: 64 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `session_uuid` | BYTES (13) | 0 | 16 | false | 128-bit shared session UUID |
| 2 | `timestamp_sec` | U64 (4) | 16 | 8 | false | Step execution timestamp |
| 3 | `agent_id` | U64 (4) | 24 | 8 | false | AI agent identifier hash |
| 4 | `step_type` | U32 (3) | 32 | 4 | false | Step category code |
| 5 | `confidence` | F32 (9) | 36 | 4 | false | Model confidence score |
| 6 | `human_coord` | BYTES (13) | 40 | 16 | false | Pointer to human decision |
| 7 | `hilbert_index` | U32 (3) | 56 | 4 | false | Spatial coordinate |
| 8 | `flags` | U32 (3) | 60 | 4 | false | Status flags |

#### Tile 12: `academic_papers` (Record Size: 64 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `key_hash` | U64 (4) | 0 | 8 | false | 64-bit hash of canonical paper key |
| 2 | `sha256_hash` | BYTES (13) | 8 | 32 | false | SHA-256 hash of paper record JSON |
| 3 | `total_bytes` | U32 (3) | 40 | 4 | false | Total bytes of UTF-8 JSON payload across all chunks |
| 4 | `chunk_index` | U16 (2) | 44 | 2 | false | 0-based chunk index |
| 5 | `total_chunks` | U16 (2) | 46 | 2 | false | Total chunks for this paper |
| 6 | `chunk_bytes` | U32 (3) | 48 | 4 | false | Bytes in this chunk payload |
| 7 | `chunk_crc32` | U32 (3) | 52 | 4 | false | CRC32C of this chunk's payload |
| 8 | `schema_version` | U32 (3) | 56 | 4 | false | Schema layout version (2 for v2) |
| 9 | `reserved` | U32 (3) | 60 | 4 | false | Alignment padding |
| 10 | `chunk_data` | STRING (11) | 64 | 0 | false | UTF-8 JSON chunk of `chunk_bytes` bytes; byte length 0 = variable |

Layout v2 (`record_layout_version = 2`): one 64-byte blob header followed by `chunk_data`. A paper
spans `total_chunks` tiles that share `sha256_hash`; concatenating `chunk_data` in `chunk_index`
order gives `total_bytes` bytes of UTF-8 JSON whose SHA-256 is `sha256_hash`. Layout v1 (fixed
64-byte records, read-only) is documented in [legacy-layouts.md](legacy-layouts.md).

#### Tile 13: `research_graph` (Record Size: 64 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `source_hash` | U64 (4) | 0 | 8 | false | CRC32C of the NFC source key, zero-extended |
| 2 | `target_hash` | U64 (4) | 8 | 8 | false | CRC32C of the NFC target key, zero-extended |
| 3 | `total_bytes` | U32 (3) | 16 | 4 | false | Bytes of `edge_json` |
| 4 | `schema_version` | U32 (3) | 20 | 4 | false | Blob encoding version (2) |
| 5 | `payload_crc32` | U32 (3) | 24 | 4 | false | CRC32C of `edge_json` |
| 6 | `is_retraction` | U32 (3) | 28 | 4 | false | 1 = retraction record |
| 7 | `reserved` | BYTES (13) | 32 | 32 | false | Zero |
| 8 | `edge_json` | STRING (11) | 64 | 0 | false | UTF-8 JSON edge; byte length 0 = variable |

Layout v2 (`record_layout_version = 2`): one edge per tile, a 64-byte blob header followed by
`edge_json`. A retraction carries `retracted_edge_id`, the lowercase hex SHA-256 of the canonical
JSON of the one edge it hides. Fields absent from a record are omitted, never filled with
defaults. Layout v1 (fixed 64-byte records, read-only) is documented in
[legacy-layouts.md](legacy-layouts.md).

#### Tile 14: `srs_workspace` (Record Size: 64 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `project_uuid` | BYTES (13) | 0 | 16 | false | 128-bit project UUID |
| 2 | `question_hash` | U64 (4) | 16 | 8 | false | Hash of research question |
| 3 | `collection_mask` | U64 (4) | 24 | 8 | false | Corpus inclusion mask |
| 4 | `topic_centroid_xyz` | BYTES (13) | 32 | 12 | false | Centroid topic coordinates |
| 5 | `status` | U32 (3) | 44 | 4 | false | Review status code |
| 6 | `pinned_works_count` | U32 (3) | 48 | 4 | false | Pinned works count |
| 7 | `claims_count` | U32 (3) | 52 | 4 | false | Extracted claims count |
| 8 | `flags` | U64 (4) | 56 | 8 | false | Status flags |

#### Tile 15: `srs_manifest` (Record Size: 64 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `magic` | STRING (11) | 0 | 16 | false | Manifest magic identifier |
| 2 | `schema_version` | U32 (3) | 16 | 4 | false | Snapshot schema version |
| 3 | `archive_flags` | U32 (3) | 20 | 4 | false | Snapshot archive flags |
| 4 | `project_uuid` | BYTES (13) | 24 | 16 | false | Project UUID |
| 5 | `snapshot_uuid` | BYTES (13) | 40 | 16 | false | Snapshot UUID |
| 6 | `parent_snapshot_uuid` | BYTES (13) | 56 | 16 | false | Parent snapshot UUID |
| 7 | `origin_session_uuid` | BYTES (13) | 72 | 16 | false | Origin session UUID |
| 8 | `created_at_sec` | U64 (4) | 88 | 8 | false | Creation timestamp |
| 9 | `tile_count` | U32 (3) | 96 | 4 | false | Total tiles in snapshot |
| 10 | `crc32c_checksum` | U32 (3) | 100 | 4 | false | Snapshot checksum |
| 11 | `total_records` | U64 (4) | 104 | 8 | false | Total records across tiles |

#### Tile 16: `ontology_concept` (Record Size: 64 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `concept_id` | U64 (4) | 0 | 8 | false | 64-bit hash of canonical URI |
| 2 | `parent_id` | U64 (4) | 8 | 8 | false | Subsumption parent concept hash (0 if root) |
| 3 | `ontology_id` | U32 (3) | 16 | 4 | false | Enclosing ontology ID |
| 4 | `concept_kind` | U16 (2) | 20 | 2 | false | 1=Class, 2=Verb, 3=ActivityType, etc. |
| 5 | `xapi_profile_id` | U16 (2) | 22 | 2 | false | Associated xAPI profile ID |
| 6 | `hilbert_coord` | U32 (3) | 24 | 4 | false | Topological interval index for SIMD subtree sifting |
| 7 | `flags` | U32 (3) | 28 | 4 | false | Active, Tombstone, Provisional |
| 8 | `reserved` | BYTES (13) | 32 | 32 | false | Padding to exact 64 bytes |

#### Tile 17: `ontology_axiom` (Record Size: 64 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `subject_concept_id` | U64 (4) | 0 | 8 | false | Subject concept hash |
| 2 | `object_concept_id` | U64 (4) | 8 | 8 | false | Object concept hash |
| 3 | `predicate_id` | U32 (3) | 16 | 4 | false | Predicate type |
| 4 | `axiom_type` | U16 (2) | 20 | 2 | false | 1=Hierarchical, 2=PedagogicalScaffold, 3=xAPIRule |
| 5 | `weight_fp16` | U16 (2) | 22 | 2 | false | Relevance / confidence weight |
| 6 | `target_tile_type` | U16 (2) | 24 | 2 | false | Target S3A tile type e.g. 10 (LRS), 12 (Paper) |
| 7 | `required_addon_mask` | U16 (2) | 26 | 2 | false | Required addon bitmask |
| 8 | `flags` | U32 (3) | 28 | 4 | false | Active, Tombstone, Provisional |
| 9 | `reserved` | BYTES (13) | 32 | 32 | false | Padding to exact 64 bytes |

#### Tile 18: `agent_registry` (Record Size: 64 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `agent_id` | U64 (4) | 0 | 8 | false | Registered agent identifier |
| 2 | `created_at_sec` | U64 (4) | 8 | 8 | false | Registration timestamp |
| 3 | `public_key_hash` | U64 (4) | 16 | 8 | false | Agent public key hash |
| 4 | `name` | STRING (11) | 24 | 32 | false | Canonical agent name |
| 5 | `capabilities` | U32 (3) | 56 | 4 | false | Capability mask |
| 6 | `flags` | U32 (3) | 60 | 4 | false | Status flags |

#### Tile 20: `note_claims` (Record Size: 160 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `claim_id` | U64 (4) | 0 | 8 | false | Claim identifier hash |
| 2 | `timestamp_sec` | U64 (4) | 8 | 8 | false | Assertion timestamp in epoch seconds |
| 3 | `session_uuid` | BYTES (13) | 16 | 16 | false | 128-bit research session UUID |
| 4 | `author_hash` | U64 (4) | 32 | 8 | false | Author researcher or agent hash |
| 5 | `confidence` | F32 (9) | 40 | 4 | false | Claim validation confidence |
| 6 | `kind` | U16 (2) | 44 | 2 | false | Claim category code |
| 7 | `flags` | U16 (2) | 46 | 2 | false | Status flags |
| 8 | `keyword_bloom` | U32 (3) | 48 | 4 | false | 32-bit token bloom filter |
| 9 | `text_hash` | U32 (3) | 52 | 4 | false | CRC32C hash of text content |
| 10 | `embedding_tile_id` | U32 (3) | 56 | 4 | false | Pointer to embedding tile |
| 11 | `embedding_offset` | U32 (3) | 60 | 4 | false | Record offset in embedding tile |
| 12 | `citation_fingerprint_id` | U64 (4) | 64 | 8 | false | 64-bit ID of CitationFingerprintRecord / paper_id (Phase C) |
| 13 | `passage_page` | U32 (3) | 72 | 4 | false | Page number in source document (Phase C) |
| 14 | `passage_char_offset` | U32 (3) | 76 | 4 | false | Character start offset within page (Phase C) |
| 15 | `passage_char_length` | U32 (3) | 80 | 4 | false | Character length of passage (Phase C) |
| 16 | `reserved_passage` | U32 (3) | 84 | 4 | false | Alignment padding (Phase C) |
| 17 | `quote_sha256` | BYTES (13) | 88 | 32 | false | SHA-256 hash of the exact quoted passage text (Phase C) |
| 18 | `producer_id` | U64 (4) | 120 | 8 | false | Human researcher or AI agent ID (Phase C) |
| 19 | `verification_statement_uuid` | BYTES (13) | 128 | 16 | false | 128-bit statement UUID, verified-claim / flagged-hallucination (Phase C) |
| 20 | `reserved` | BYTES (13) | 144 | 16 | false | Reserved zero padding to 160 bytes (Phase C) |

#### Tile 32: `citation_fingerprint` (Record Size: 96 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `paper_id` | U64 (4) | 0 | 8 | false | 64-bit ID or hash of paper / DOI |
| 2 | `doi_hash` | U64 (4) | 8 | 8 | false | 64-bit hash of normalized DOI string |
| 3 | `content_sha256` | BYTES (13) | 16 | 32 | false | SHA-256 digest of fetched content bytes |
| 4 | `content_kind` | U32 (3) | 48 | 4 | false | Content kind: PDF (1), HTML (2), ABSTRACT (3) |
| 5 | `crossref_version` | U32 (3) | 52 | 4 | false | Crossref work update sequence / version number |
| 6 | `fetched_at` | U64 (4) | 56 | 8 | false | Timestamp when content was fetched, epoch seconds |
| 7 | `source_url_hash` | U64 (4) | 64 | 8 | false | 64-bit hash of source URL |
| 8 | `flags` | U64 (4) | 72 | 8 | false | Status flags |
| 9 | `reserved` | BYTES (13) | 80 | 16 | false | Reserved zero padding to 96 bytes |

#### Tile 96: `committee_decision` (Record Size: 160 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `submission_hash` | BYTES (13) | 0 | 16 | false | Hash of submission ID |
| 2 | `reviewer_hash` | BYTES (13) | 16 | 16 | false | Hash of reviewer ID |
| 3 | `comment_hash` | BYTES (13) | 32 | 16 | false | Hash of review comments |
| 4 | `prev_hash` | BYTES (13) | 48 | 32 | false | SHA-256 of previous decision record |
| 5 | `timestamp_sec` | U64 (4) | 80 | 8 | false | Decision timestamp in epoch seconds |
| 6 | `verdict` | U32 (3) | 88 | 4 | false | Numeric verdict |
| 7 | `layout_version` | U32 (3) | 92 | 4 | false | Schema layout version |
| 8 | `signature` | BYTES (13) | 96 | 64 | false | Ed25519 signature over signing message |

---

### 7.3 Other Standard Domain Packs & Extensions

#### Tile 3: `wearable_micro` (Record Size: 16 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `timestamp` | U64 (4) | 0 | 8 | false | Timestamp in epoch microseconds |
| 2 | `heart_rate` | U16 (2) | 8 | 2 | false | Heart rate reading (bpm) |
| 3 | `spo2` | U16 (2) | 10 | 2 | false | Blood oxygen level percentage |
| 4 | `motion_flags` | U32 (3) | 12 | 4 | false | Accelerometer motion mask |

#### Tile 4: `robotics_kinematic` (Record Size: 72 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `timestamp_sec` | U64 (4) | 0 | 8 | false | Trajectory timestamp in epoch seconds |
| 2 | `joint_id` | U32 (3) | 8 | 4 | false | Robotic actuator joint ID |
| 3 | `trajectory_id` | U32 (3) | 12 | 4 | false | Path trajectory segment ID |
| 4 | `position_xyz` | BYTES (13) | 16 | 24 | false | 3D joint coordinate vectors (3 x f64) |
| 5 | `velocity_xyz` | BYTES (13) | 40 | 24 | false | 3D velocity vectors (3 x f64) |
| 6 | `flags` | U64 (4) | 64 | 8 | false | Status flags |

#### Tile 5: `gis_survey_mesh` (Record Size: 32 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `point_id` | U64 (4) | 0 | 8 | false | Survey mesh point ID |
| 2 | `latitude` | F64 (10) | 8 | 8 | false | WGS84 latitude coordinate |
| 3 | `longitude` | F64 (10) | 16 | 8 | false | WGS84 longitude coordinate |
| 4 | `elevation` | F32 (9) | 24 | 4 | false | Elevation above sea level (meters) |
| 5 | `flags` | U32 (3) | 28 | 4 | false | Precision flags |

#### Tile 6: `data_availability` (Record Size: 64 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `commitment_id` | U64 (4) | 0 | 8 | false | Data availability commitment ID |
| 2 | `epoch_number` | U64 (4) | 8 | 8 | false | Blockchain epoch number |
| 3 | `kzg_commitment` | BYTES (13) | 16 | 40 | false | KZG cryptographic polynomial commitment |
| 4 | `status` | U64 (4) | 56 | 8 | false | Verification status bitmask |

#### Tile 80: `vcs_commit` (Record Size: 96 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `commit_id` | BYTES (13) | 0 | 32 | false | SHA-256 commit identifier |
| 2 | `parent_commit_id` | BYTES (13) | 32 | 32 | false | SHA-256 parent commit identifier |
| 3 | `author_hash` | U64 (4) | 64 | 8 | false | Commit author hash |
| 4 | `timestamp` | U64 (4) | 72 | 8 | false | Commit timestamp in epoch seconds |
| 5 | `message_len` | U32 (3) | 80 | 4 | false | Byte length of commit message |
| 6 | `flags` | U32 (3) | 84 | 4 | false | Commit status flags |

#### Tile 81: `vcs_ref` (Record Size: 80 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `ref_name` | STRING (11) | 0 | 32 | false | Branch or tag reference name |
| 2 | `commit_id` | BYTES (13) | 32 | 32 | false | SHA-256 target commit identifier |
| 3 | `updated_at` | U64 (4) | 64 | 8 | false | Update timestamp in epoch seconds |
| 4 | `flags` | U64 (4) | 72 | 8 | false | Reference flags |

#### Tile 82: `vcs_resolution` (Record Size: 80 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `conflict_id` | BYTES (13) | 0 | 32 | false | Conflict identifier |
| 2 | `resolved_commit_id` | BYTES (13) | 32 | 32 | false | Resolved target commit identifier |
| 3 | `resolved_at` | U64 (4) | 64 | 8 | false | Resolution timestamp in epoch seconds |
| 4 | `resolver_hash` | U64 (4) | 72 | 8 | false | Hash of resolver identity |

#### Tile 85: `gate_directory` (Record Size: 64 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `node_uuid` | BYTES (13) | 0 | 16 | false | 128-bit gate network node UUID |
| 2 | `peer_endpoint` | STRING (11) | 16 | 32 | false | Relay socket endpoint address |
| 3 | `registered_at` | U64 (4) | 48 | 8 | false | Registration timestamp |
| 4 | `flags` | U64 (4) | 56 | 8 | false | Node status flags |

#### Tile 86: `gate_mailbox` (Record Size: 64 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `message_id` | BYTES (13) | 0 | 16 | false | 128-bit message UUID |
| 2 | `recipient_uuid` | BYTES (13) | 16 | 16 | false | 128-bit recipient node UUID |
| 3 | `sender_uuid` | BYTES (13) | 32 | 16 | false | 128-bit sender node UUID |
| 4 | `queued_at` | U64 (4) | 48 | 8 | false | Queued timestamp in epoch seconds |
| 5 | `payload_len` | U32 (3) | 56 | 4 | false | Payload byte length |
| 6 | `flags` | U32 (3) | 60 | 4 | false | Delivery status flags |

#### Tile 64: `record` (Record Size: 64 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `timestamp` | U64 (4) | 0 | 8 | false | Timestamp in epoch seconds |
| 2 | `record_type` | U32 (3) | 8 | 4 | false | Event or record type identifier |
| 3 | `flags` | U64 (4) | 12 | 8 | false | Status and attribute flags |
| 4 | `payload` | BYTES (13) | 20 | 40 | false | Opaque payload bytes |
| 5 | `record_crc32` | U32 (3) | 60 | 4 | false | CRC32C checksum of record |

#### Tile 68: `blob` (Record Size: 40 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `blob_key` | BYTES (13) | 0 | 32 | false | 256-bit blob key / content hash |
| 2 | `chunk_index` | U32 (3) | 32 | 4 | false | Zero-based chunk index |
| 3 | `total_chunks` | U32 (3) | 36 | 4 | false | Total chunk count |

#### Tile 72: `provenance` (Record Size: 40 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `provenance_key` | BYTES (13) | 0 | 32 | false | 256-bit provenance key / manifest hash |
| 2 | `chunk_index` | U32 (3) | 32 | 4 | false | Zero-based chunk index |
| 3 | `total_chunks` | U32 (3) | 36 | 4 | false | Total chunk count |




