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
