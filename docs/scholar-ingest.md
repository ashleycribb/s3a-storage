# Scholar Pack Ingestion & Snapshot Specification (`scholar_ingest_v1`)

This document defines the production-credible ingestion pipeline and portable snapshot container architecture for the **Scholar Explorer Domain Pack (`s3a-pack-scholar`)**.

---

## 1. Physical Tile Mapping Table

All Scholar Pack domain data is mapped into dedicated Hyper-Tiles (10–15) behind the `ADDON_EDU` (0x0001) capability bitmask:

| Source (DuckDB / Logical) | Tile ID | Pack Record Struct | Alignment | Description |
| :--- | :---: | :--- | :---: | :--- |
| `xapi_statement_log`, `human_rework_decisions` | **10** | `HumanLrsRecord` | 64B | Researcher feedback, xAPI actions, approval/rework decisions |
| `reasoning_paths`, `hallucination_events` | **11** | `AiTraceRecord` | 64B | AI agent steps, hallucination flags, bilateral cross-trace pointers |
| `source_package_papers`, literature DOIs | **12** | `AcademicPaperRecord` | 64B | Literature corpus, 3D semantic topic coordinates, citation counts |
| `scholar_advisor_edges`, `scholar_brain_edges` | **13** | `ResearchGraphEdgeRecord` | 64B | Subject-predicate-object knowledge graph relations & weights |
| `workspace.json`, project context | **14** | `SrsWorkspaceRecord` | 64B | Active research question, topic centroid, collection bitmasks |
| Container / Batch Metadata | **15** | `SrsManifestHeader` | 512B | Authoritative SRS snapshot manifest, Project/Snapshot UUIDs, CRC32C |

---

## 2. Layout & Wire Versioning (`v1`)

- The active layout version for S3A Scholar Pack is defined as:
  ```rust
  pub const SCHOLAR_LAYOUT_VERSION_V1: u32 = 1;
  ```
- **Authoritative Tile 15**: Every exported Scholar Research Snapshot (`.snapshot.s3a` or `*.srs.s3a`) persists an `SrsManifestHeader` in Tile 15 with `schema_version = 1` and a hardware CRC32C checksum.
- **Pre-v1 Lab Files**: Legacy database files lacking an authoritative Tile 15 manifest or featuring unversioned schemas (`schema_version != 1`) are treated as unsupported; inspection tools will issue a warning (`"pre-v1 lab file"`), and snapshot intake will fail closed to prevent silent corruption.

---

## 3. Two-Stage Ingestion Pipeline

To decouple python analytics from low-level binary storage layouts, migration uses a two-stage architecture:

```
[DuckDB Source Databases]
  (academic.duckdb, advisor.duckdb, brain.duckdb)
               │
               ▼  Stage 1: Python Extraction (scripts/migrate_and_benchmark_duckdb.py)
[Intermediate NDJSON Files] (schema: scholar_ingest_v1)
  ├── academic_papers.jsonl
  ├── graph_edges.jsonl
  ├── human_lrs.jsonl
  ├── ai_traces.jsonl
  └── workspace.json
               │
               ▼  Stage 2: Native Rust Ingestion (s3a-cli scholar ingest)
[S3A Database Archive (*.s3a)]
  ├── Header Slot 0/1 (addon_mask: 0x0001 [ADDON_EDU])
  └── Hyper-Tiles 10..=15
```

### Stage 1: Extraction
```bash
python scripts/migrate_and_benchmark_duckdb.py \
    --academic path/to/academic.duckdb \
    --advisor path/to/scholar_advisor.duckdb \
    --brain path/to/scholar_brain.duckdb \
    --export-dir scratch/scholar_ingest_v1 \
    --out scholar_migrated.s3a \
    --fresh
```

### Stage 2: Ingestion
```bash
s3a-cli scholar ingest scratch/scholar_ingest_v1 scholar_migrated.s3a --fresh
```

---

## 4. Snapshot Container Invariants

A portable Scholar Research Snapshot (`.snapshot.s3a` / `*.srs.s3a`) adheres to the following rules:

1. **Authoritative Tile 15**:
   - `export_srs_snapshot` writes all payload tiles (10–14) first, then appends Tile 15 with the complete record count and final tile count.
   - `intake_srs_snapshot` reads Tile 15 directly. If Tile 15 is missing, has an incorrect magic (`b"S3ASTOR1"`), an unsupported layout version (`!= 1`), or an invalid CRC32C checksum, intake fails immediately with an `InvalidData` error.
2. **Capability Verification**:
   - The container must have `(addon_mask & ADDON_EDU) != 0`.
3. **Hardware CRC32C Validation**:
   - `reader.verify_checksums()` verifies all dual header slots and individual tile payloads before any record is exposed.
4. **Live Mounting**:
   - `s3a-cli snapshot intake <snapshot.s3a> --into <live.s3a>` validates the container, activates `ADDON_EDU` on the destination live database, and appends all records into the live database.

---

## 5. CLI Reference

### Snapshot Commands
```bash
# Export a snapshot from a live S3A database
s3a-cli snapshot export <live.s3a> <out.snapshot.s3a> [project_uuid_hex]

# Zero-copy memory-map intake and verification
s3a-cli snapshot intake <in.snapshot.s3a>

# Intake snapshot and mount records into a live database
s3a-cli snapshot intake <in.snapshot.s3a> --into <live.s3a>

# Inspect snapshot manifest, hardware CRC32, capability mask, and tile histogram
s3a-cli snapshot inspect <snapshot.s3a>
```

### Ingestion Commands
```bash
# Ingest intermediate NDJSON directory into an S3A archive (idempotent with --fresh)
s3a-cli scholar ingest <intermediate_dir> <out.s3a> [--fresh]
```
