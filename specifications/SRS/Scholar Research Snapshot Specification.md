# SRS — Scholar Research Snapshot Specification

**Specification Identifier:** SECS-SRS-1.0.0  
**Status:** Canonical Reference Specification  
**Version:** 1.0.0  
**Target File Format:** `*.snapshot.s3a` (S3A Binary Sector Container)  
**Target Audience:** Storage Architects, Data Engineers, Rust/WASM Developers, Academic Reproducibility Auditors  

---

## 1. Specification Goals

The **Scholar Research Snapshot (SRS)** defines the canonical, portable research object for the Scholar Explorer platform. It fulfills five strict scientific requirements:

1. **Portable**: Encapsulated into a single, self-contained binary file that moves frictionlessly between Web browsers, cloud servers, local workstations, and archival repositories.
2. **Versioned**: Every snapshot represents an immutable, point-in-time state checkpoint with a Git-like parent pointer for branchable research lineage.
3. **Deterministic**: Given identical research inputs and human decisions, the snapshot packing produces bit-for-bit identical binary sectors.
4. **Reproducible**: Third-party auditors can replay every agent run and human review decision from the snapshot without access to the original runtime environment.
5. **Privacy Preserving**: Strictly excludes environment variables, user credentials, API keys, and unreferenced local filesystem paths.

---

## 2. File Format Architecture: The S3A Binary Container

Snapshots use the **`.snapshot.s3a`** (or `.s3a`) file extension.

Rather than packing loose, slow JSON files inside a compressed ZIP archive (which introduces CPU decompression overhead and multi-second JSON parsing latency), **SRS is compiled directly into S3A 512-byte sector-aligned Hyper-Tiles**.

### Physical File Layout

```
OFFSET        BYTE SIZE   SECTION NAME             DESCRIPTION
─────────────────────────────────────────────────────────────────────────────
0x00000000    512 B       Sector 0: Superblock     Magic "S3ASTOR1", Manifest Header, Project & Snapshot UUIDs, CRC32C.
0x00000200    512 B       Tile 14: Workspace       Research question, collection filters, tags, token budget.
0x00000400    N * 64 B    Tile 12: Corpus Works    Packed 64B `AcademicPaperRecord` structs (DOIs, 3D Hilbert topic points).
[Sector-Align] M * 64 B    Tile 13: Evidence Graph  Packed 64B `ResearchGraphEdgeRecord` structs (Support/Contradict links).
[Sector-Align] P * 64 B    Tile 10: Human Actions   Packed 64B `HumanLrsRecord` structs (Researcher decisions & review scores).
[Sector-Align] Q * 64 B    Tile 11: Agent Runs      Packed 64B `AiTraceRecord` structs (Agent steps, tool executions, confidence).
[Sector-Align] Variable   Payload Appendix         Optional compact text chunk table & selective PDF byte streams.
```

---

## 3. Section 3: Manifest Header (Sector 0)

Sector 0 constitutes the 512-byte `SrsManifestHeader`:

```rust
#[repr(C, align(8))]
pub struct SrsManifestHeader {
    pub magic: [u8; 8],                // b"S3ASTOR1"
    pub schema_version: u32,           // SRS Specification version (e.g. 1)
    pub archive_flags: u32,            // Bit 0: IS_IMMUTABLE, Bit 1: IS_SNAPSHOT, Bit 2: WASM_EXPORTED
    pub project_uuid: [u64; 2],        // 128-bit persistent project identifier
    pub snapshot_uuid: [u64; 2],       // 128-bit unique checkpoint identifier
    pub parent_snapshot_uuid: [u64; 2],// 128-bit parent checkpoint (0 if root)
    pub origin_session_uuid: [u64; 2], // 128-bit session identifier
    pub created_at_sec: u64,           // Unix epoch timestamp (seconds)
    pub tile_count: u32,               // Number of Hyper-Tiles in container
    pub total_records: u64,            // Total record count across all tiles
    pub crc32c_checksum: u32,          // Hardware CRC32C of entire archive payload
    pub reserved: [u8; 436],           // Zero-padded to exact 512-byte sector boundary
}
```

---

## 4. Section 4: Workspace Metadata (Tile 14)

Defines the active scholarly inquiry context:
- **Research Question**: The natural language inquiry guiding synthesis (supports evolutionary versions).
- **Active Collections**: Bitmask and naming of selected collections (e.g. `gno_vault`, `openalex_recent`).
- **Topic Centroid**: 3D Skilling Hilbert coordinate cluster centroid representing the query embedding space.
- **Search Constraints**: Publication year ranges, peer-review filters, and open-access mandates.

---

## 5. Section 5: Corpus Works (Tile 12)

Every selected academic work is stored as a 64-byte `AcademicPaperRecord`:
- `topic_x`, `topic_y`, `topic_z`: 3D coordinates on the Skilling compact Hilbert space.
- `doi_prefix_hash`, `doi_suffix_hash`: 64-bit cryptographic hashes for $O(1)$ DOI lookups.
- `citation_count`: Published citations.
- `flags`: Open-Access flag, peer-review certification bit, retraction status.

---

## 6. Section 6: Evidence & Claims Graph (Tile 13)

Evidence relationships connect scholarly claims to supporting or refuting corpus works:
- Stored as 64-byte `ResearchGraphEdgeRecord` structs.
- **Predicate IDs**:
  - `1`: `SUPPORTS` (Empirical evidence verifies the claim)
  - `2`: `CONTRADICTS` (Empirical evidence refutes or limits the claim)
  - `3`: `CONTEXTUAL` (Passage provides background or boundary condition)
  - `4`: `INCONCLUSIVE` (Under-specified or conflicting evidence)
- **Weight**: 32-bit float representing extraction confidence and sample-size weight.

---

## 7. Section 7: Provenance Ledger (Tiles 10 & 11)

Maintains the complete, tamper-proof dual-actor interaction audit trail:
- **`HumanLrsRecord` (Tile 10)**:
  - 64-byte Pod record capturing every human researcher decision (`questioned`, `delegated`, `decided`, `annotated`).
  - Contains actor hash, verb ID, decision score, and direct `S3ACoordinate` pointer to the corresponding AI trace.
- **`AiTraceRecord` (Tile 11)**:
  - 64-byte Pod record capturing every autonomous agent action (`searched`, `synthesized`, `verified`, `flagged`).
  - Contains agent ID, confidence rating, tool execution hash, and direct pointer back to the triggering human decision.
- **Cross-Tracing Guarantee**: Bidirectional dereferencing between human and AI records resolves in **sub-50 nanoseconds**.

---

## 8. Section 8: Import Protocol & Validation

When an S3A engine intakes a `.snapshot.s3a` file:
1. **Magic & Boundary Check**: Validates that byte length is an exact multiple of 512 bytes and begins with `b"S3ASTOR1"`.
2. **Hardware CRC32C Validation**: Verifies payload checksum via hardware AVX-512 / SSE4.2 instructions in microseconds.
3. **Compatibility Negotiation**: Validates `schema_version <= CURRENT_SRS_VERSION`. If older, runs zero-copy sector migration.
4. **Instant Ingestion**: Directly memory-maps the snapshot via `memmap2` with zero deserialization overhead.

---

## 9. Section 9: Local Lab Bootstrap

Once the snapshot is mounted locally:
1. **Immediate In-Memory Materialization ($O(1)$)**: Workspace filters, 3D paper clusters, and claims are active within 1 millisecond.
2. **Lazy PDF Retrieval**: Full-text PDFs are not required to be bundled inside the snapshot; they are retrieved asynchronously on-demand using the DOI/OA-URL references in Tile 12.
3. **Local Agent Wakeup**: The local autonomous research agent resumes execution from the exact point of the web session, honoring all human gating verdicts stored in Tile 10.
