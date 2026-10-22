# S3A Pack Contract

This document defines the architectural policy for modular domain packs extending the S3A multi-dimensional storage engine.

## 1. Tile ID Allocation Policy

S3A Hyper-Tiles are 128 KB page-aligned binary storage units identified by an integer `tile_type: u32`. Tile IDs are partitioned into strict ownership bands:

| Band | Owner | Tile IDs | Description / Representative Structs |
|:---|:---|:---|:---|
| **Core** | `s3a-core` | `0, 1, 2, 7, 8` | Domain-agnostic storage primitives:<br>• `0`: Telemetry (`TelemetryRecord`, 64B)<br>• `1`: High-dim Embeddings (`EmbeddingRecord128`, 544B)<br>• `2`: Hybrid Indices<br>• `7`: Quantized Embeddings (INT8, `CompactEmbeddingRecord32`)<br>• `8`: Quantized Embeddings (4-bit) |
| **Wearable Pack** | `s3a-pack-wearable` | `3` | Wearable micro-telemetry & ultra-compact biometric sensor buffers:<br>• `3`: `CompactWearableRecord` (16B, PPG, IMU, heart-rate) |
| **Robotics Pack** | `s3a-pack-robotics` | `4` | High-frequency kinematic trajectories & manipulator state streams:<br>• `4`: `RoboticsKinematicRecord` (72B, 3D position, quaternion, velocity) |
| **GIS Pack** | `s3a-pack-gis` | `5` | Topographic point clouds, LiDAR, and subsurface strata meshes:<br>• `5`: `GisSurveyPointRecord` (32B, micro-degrees lat/lon, elevation mm) |
| **DA Pack** | `s3a-pack-da` | `6` | Decentralized data availability, rollups, and state root verification:<br>• `6`: `DaCommitmentRecord` (64B, KZG/state root, quorum bitmask) |
| **Reserved** | — | `9` | Preserved boundary slot. Do not claim without an RFC. |
| **Scholar Pack** | `s3a-pack-scholar` | `10..=15` | Academic literature, cognitive provenance, and learning analytics:<br>• `10`: `HumanLrsRecord` (64B) — Researcher decisions & gating ledger<br>• `11`: `AiTraceRecord` (64B) — Autonomous agent step traces & prompts<br>• `12`: `AcademicPaperRecord` (64B) — 3D Skilling Hilbert topic centroids<br>• `13`: `ResearchGraphEdgeRecord` (64B) — Citation & evidence graph<br>• `14`: `SrsWorkspaceRecord` (64B) — Research question & context<br>• `15`: `SrsManifestHeader` (512B) — Snapshot container manifest |
| **Ontology Pack** | `s3a-pack-ontology` | `16..=17` | Web ontologies (OWL), semantic grounding, and xAPI rule registry:<br>• `16`: `OntologyConceptRecord` (64B) — Normalized URI concepts, verbs & scaffolds<br>• `17`: `OntologyAxiomRecord` (64B) — Structural invariants, domain/range, and approved analogies |
| **Clean Pack** | `s3a-pack-clean` | none (uses core `64` RECORD, `68` BLOB, `72` PROVENANCE) | Raw-sealed intake, company rulebook cleaning, signed cleaned versions with provenance. No `addon_mask` bit needed. See [clean-pack.md](clean-pack.md). |
| **Reserved Gap** | — | `18..=63` | Reserved for future S3A standard extensions. |
| **Future Packs** | Third-party | `64..` | Net-new third-party domain packs (FinTech, Healthcare, Astronomy, etc.) must allocate IDs starting at 64 to prevent collisions with Core, System, Scholar, and Ontology packs. |

---

## 2. Capability Mask (`addon_mask`) Semantics

The S3A 64-byte `FileHeader` contains an immutable-per-slot `addon_mask: u32` bitfield protected by hardware CRC32C checksums.

### Bit Allocations
- `ADDON_NONE = 0x0000` (Pure general database archive)
- `ADDON_EDU = 1 << 0` (0x0001: Scholar Pack capability active)
- `ADDON_FINTECH = 1 << 1` (0x0002: Reserved for financial tick pack)
- `ADDON_ROBOTICS = 1 << 2` (0x0004: Robotics kinematics pack active)
- `ADDON_WEARABLE = 1 << 3` (0x0008: Wearable micro-telemetry pack active)
- `ADDON_GIS = 1 << 4` (0x0010: GIS survey mesh pack active)
- `ADDON_DA = 1 << 5` (0x0020: Data availability / state commitments pack active)
- `ADDON_ONTOLOGY = 1 << 6` (0x0040: Formal Ontology & Axiom Registry Pack active)

### Enforcement Rules
1. **Write Gate (Fail Closed)**:
   Attempting to append or write domain pack tiles without the corresponding capability bit is strictly prohibited:
   - Tile 3 requires `ADDON_WEARABLE` (0x0008)
   - Tile 4 requires `ADDON_ROBOTICS` (0x0004)
   - Tile 5 requires `ADDON_GIS` (0x0010)
   - Tile 6 requires `ADDON_DA` (0x0020)
   - Tiles 10..=15 require `ADDON_EDU` (0x0001)
   - Tiles 16..=17 require `ADDON_ONTOLOGY` (0x0040)
   The engine returns `io::ErrorKind::PermissionDenied`. Capabilities must be explicitly authorized using `enable_addon(mask)` or CLI `s3a-cli addon enable <file> <name>`.
2. **Read Gate (Data Safety)**:
   Reading existing pack tiles in an archive always succeeds even if the mask is zero or cleared, preventing data loss. Legacy archives with tiles 3–6 remain fully readable. Inspection tools issue an advisory warning if unmasked pack tiles are detected.
3. **Compaction Inheritance**:
   When merging archives using `Compactor::compact` or `TileFusionEngine`, the output archive inherits the bitwise OR of all input archives' `addon_mask` values. Non-specialized pack tiles are copied through intact.
4. **Unified Commit Path**:
   Enabling a pack updates the alternate header slot and bumps the generation counter using the identical dual-slot commit logic as record appends.

---

## 3. Ingest & Snapshot Specification

For complete mapping specifications between external data sources (e.g. DuckDB), intermediate NDJSON schemas (`scholar_ingest_v1`), and Scholar Pack tiles (10–15), see [Scholar Ingestion & Snapshot Specification](scholar-ingest.md).

---

## 4. Durability & Crash Recovery

For the complete technical specification of the dual-slot A/B commit architecture, append ordering barriers, compaction replacement protocol, and the crash recovery outcome matrix, see [S3A Durability Contract](durability.md).

---

## 5. Schema Evolution & Tile Registry

For the complete specification of the tile type registry (`TileTypeRegistry`), hyper-tile header layout versioning (`record_layout_version`), additive evolution rules, compactor version preservation, and Tile 9 collision resolution, see [S3A Schema Evolution Contract](schema-evolution.md).

---

## 6. Query Interface (S3A-QL v1)

For the frozen versioned query language grammar, statement reference (`FETCH`, `SIFT`, `INSERT`, `DELETE`, `COMPACT`, `FUSE`), pack domain handoff, and machine-readable error models, see [S3A-QL v1 Specification](s3a-ql-v1.md).

---

## 7. Production Operations & DBA Toolbox

For the administrator manual covering archive inspection, verification, health diagnosis (`doctor`), crash recovery (`recover`), status reporting, and machine-readable `--json` flags, see [S3A Production Operations Manual](ops.md).

---

## 8. Non-Core Additive Web & AI Extensions (`s3a-wasm-snapshot`)

To maintain architectural purity, browser and WebAssembly integrations are strictly segregated from `s3a-core` and `s3a-engine`:
- **Crate**: `s3a-wasm-snapshot` (additive workspace package).
- **Target**: `wasm32-unknown-unknown` and browser environments.
- **Role**: Allows the Scholar Explorer web application and client-side AI agents to intake, verify, evaluate, and export `.snapshot.s3a` archives directly from in-memory byte buffers (`Uint8Array` / `&[u8]`) without POSIX disk dependencies.
- **Pre-Flight TA Check**: Client-side AI agents query in-memory snapshot ontology and approved analogy records to ensure zero hallucinations before emitting responses.




