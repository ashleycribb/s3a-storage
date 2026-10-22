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
| **33 – 63** | Reserved for Standard Domain Packs (allocated via spec amendment) | Standards Working Group |
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
| **7 – 31** | `0x0080`.. | -- | Unallocated | Reserved for future standardization |

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

