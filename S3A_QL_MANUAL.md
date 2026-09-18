# S3A-QL / STQL Reference Manual
## *Simplicial Tile Query Language for the Stratified Simplicial Storage Architecture*

---

## Table of Contents
1. [Introduction & Architectural Design Goals](#1-introduction--architectural-design-goals)
2. [S3A-QL vs SQL Physical Paradigm Comparison](#2-s3a-ql-vs-sql-physical-paradigm-comparison)
3. [Language Grammar & EBNF Specification](#3-language-grammar--ebnf-specification)
4. [Direct Coordinate Addressing (`FETCH AT`)](#4-direct-coordinate-addressing-fetch-at)
5. [SIMD Sifting Statements (`SIFT`)](#5-simd-sifting-statements-sift)
   - [5.1 Telemetry Sifting](#51-telemetry-sifting)
   - [5.2 High-Frequency 1 kHz Robotics Trajectory Sifting](#52-high-frequency-1-khz-robotics-trajectory-sifting)
   - [5.3 3D GIS Survey & Subsurface Topography Mesh Sifting](#53-3d-gis-survey--subsurface-topography-mesh-sifting)
   - [5.4 Multi-Dimensional Vector Embedding Similarity Sifting](#54-multi-dimensional-vector-embedding-similarity-sifting)
   - [5.5 L2 Rollup & Decentralized AI Data Availability Sampling](#55-l2-rollup--decentralized-ai-data-availability-sampling)
6. [Data Modification & Lifecycle Operations (`INSERT`, `DELETE`, `COMPACT`)](#6-data-modification--lifecycle-operations-insert-delete-compact)
7. [Multi-Tile Fusion Operations (`FUSE TILES`)](#7-multi-tile-fusion-operations-fuse-tiles)
8. [Language Bindings & Integration](#8-language-bindings--integration)
   - [8.1 CLI Integration (`s3a-cli`)](#81-cli-integration-s3a-cli)
   - [8.2 C-ABI Reference (`s3a-cabi`)](#82-c-abi-reference-s3a-cabi)
   - [8.3 REST API & AI Agent Function Calling Console](#83-rest-api--ai-agent-function-calling-console)
9. [Appendix: Error Codes & Status Indicators](#9-appendix-error-codes--status-indicators)

---

## 1. Introduction & Architectural Design Goals

The **Stratified Simplicial Storage Architecture (S3A)** is a domain-specialized, zero-copy, hardware-accelerated storage engine built for high-throughput sensor telemetry, spatial trajectories, vector embeddings, and zero-copy blockchain Data Availability (DA).

Unlike relational engines built for row-oriented or columnar B-Trees, S3A structures all data into fixed-size **Hyper-Tiles (128 KB)** and **Micro-Tiles (4 KB)** with embedded $n$-dimensional **Simplicial Convex Hulls** (`SimplexHull`).

**S3A-QL** (or **STQL** — *Simplicial Tile Query Language*) is the native, zero-compilation query language for S3A archives. It maps directly to physical memory mapping (`mmap`), x86-64 AVX2 / ARM Neon SIMD registers, and $O(1)$ coordinate offsets without intermediate abstract syntax tree (AST) overhead or query execution planners.

### Core Design Goals:
* **Zero Allocation & Zero Copy:** S3A-QL queries operate on memory-mapped byte slices (`bytemuck::cast_slice`) with zero heap allocation during scanning.
* **Deterministic SIMD Sifting:** Queries leverage single-cycle SIMD scalar comparisons on tile header bounding boxes (`can_reject_tile_time`, `can_reject_tile_range`) to skip entire 128 KB tiles in $O(1)$ time.
* **Direct Coordinate Addressability:** S3A-QL supports Excel-style cell addressing (`L<level>:T<tile_id>:R<record_offset>`) for $O(1)$ record lookups.
* **Native AI Agent Interoperability:** All S3A-QL queries map 1:1 to JSON schemas for LLM tool function calling (`/api/agent/execute`).

---

## 2. S3A-QL vs SQL Physical Paradigm Comparison

| Feature | Relational SQL | S3A-QL / STQL |
| :--- | :--- | :--- |
| **Data Organization** | Tables, Rows, Columns | Page-Aligned 128 KB Hyper-Tiles & 4 KB Micro-Tiles |
| **Point Lookup** | B-Tree Index Traversal ($O(\log N)$) | Coordinate Address (`L0:T12:R4`) Direct Calculation ($O(1)$) |
| **Spatial Querying** | R-Tree / PostGIS Spatial Index | Single-Cycle SIMD Simplex Convex Hull Rejection |
| **Time Filtering** | Partition Scans / Index Scans | SIMD Single-Cycle Min/Max Header Timestamp Filter |
| **Vector Search** | External Vector Index Plugin (HNSW, IVFFlat) | Native Hardware-Accelerated SIMD Cosine/Dot-Product Sifting |
| **Garbage Collection** | Vacuum / Background Compaction | Append-Only Tombstoning & Stratified Hyper-Tile Compactor |
| **Multi-Tile Merge** | Distributed Join Engine | `FUSE TILES` Primitive with Hull Re-Indexing & Deduplication |

---

## 3. Language Grammar & EBNF Specification

```ebnf
QueryStatement     ::= FetchStatement | SiftStatement | InsertStatement | DeleteStatement | CompactStatement | FuseStatement ;

CoordinateAddress  ::= "L" Digit+ ":T" Digit+ ":R" Digit+ ;

FetchStatement     ::= "FETCH" "RECORD" "AT" CoordinateAddress "FROM" FilePath ;

SiftStatement      ::= "SIFT" SiftDomain "FROM" FilePath [ "WHERE" FilterClause ] [ "LIMIT" Digit+ ] ;

SiftDomain         ::= "TELEMETRY" | "KINEMATICS" | "GIS_MESH" | "EMBEDDINGS" | "DA_COMMITMENTS" ;

FilterClause       ::= Condition ( "AND" Condition )* ;

Condition          ::= TimeCondition | SpatialCondition | SimilarityCondition | FieldCondition ;

TimeCondition      ::= "TIME" "BETWEEN" Number "AND" Number ;

SpatialCondition   ::= "SPATIAL_BOX" "IN" "SIMPLEX" "(" "MIN" Vector3D "," "MAX" Vector3D ")" ;

SimilarityCondition ::= "SIMILARITY" "TO" Vector "USING" ("COSINE" | "DOT_PRODUCT") "THRESHOLD" Float ;

InsertStatement    ::= "INSERT" "TELEMETRY" "(" Timestamp "," SensorID "," MetricID "," Value ")" "INTO" FilePath ;

DeleteStatement    ::= "DELETE" "TELEMETRY" "WHERE" "SENSOR_ID" "=" SensorID "AND" "METRIC_ID" "=" MetricID "AND" "TIME" "=" Timestamp "FROM" FilePath ;

CompactStatement   ::= "COMPACT" "ARCHIVE" FilePath ;

FuseStatement      ::= "FUSE" "TILES" "FROM" "(" FilePathList ")" "INTO" FilePath [ "WITH" "(" OptionList ")" ] ;
```

---

## 4. Direct Coordinate Addressing (`FETCH AT`)

S3A provides an Excel-style coordinate system (`S3ACoordinate`) where every record has a unique physical address formatted as `L<level>:T<tile_id>:R<record_offset>`.

### Syntax
```sql
FETCH RECORD AT L<level>:T<tile_id>:R<record_offset> FROM "file.s3a";
```

### Example
```sql
-- Direct O(1) fetch of record at Level 0, Tile 4, Record Offset 12
FETCH RECORD AT L0:T4:R12 FROM "telemetry.s3a";
```

### Physical Mechanics
1. S3A-QL parses the coordinate string into `S3ACoordinate { level: 0, tile_id: 4, record_offset: 12 }`.
2. The engine computes the absolute byte offset:
   $$\text{Offset} = \text{FILE\_HEADER\_SIZE} + (\text{tile\_id} \times \text{HYPER\_TILE\_SIZE}) + \text{HYPER\_TILE\_HEADER\_SIZE} + (\text{record\_offset} \times \text{RECORD\_SIZE})$$
3. Returns the zero-copy dereferenced record slice in $O(1)$ time.

---

## 5. SIMD Sifting Statements (`SIFT`)

The `SIFT` statement performs single-cycle hardware-accelerated sifting over memory-mapped S3A Hyper-Tiles.

### 5.1 Telemetry Sifting
Sifts time-series telemetry records filtering by sensor, metric, and timestamp bounds.

```sql
SIFT TELEMETRY
FROM "sensor_data.s3a"
WHERE TIME BETWEEN 1000 AND 5000
  AND SENSOR_ID = 1
  AND METRIC_ID = 101;
```

### 5.2 High-Frequency 1 kHz Robotics Trajectory Sifting
Sifts high-frequency kinematics records (6 DoF pose, orientation quaternions, linear/angular velocity) using 3D Simplex Convex Hull SIMD rejection.

```sql
SIFT KINEMATICS
FROM "robot_arm_1.s3a"
WHERE TIME BETWEEN 0_us AND 10_000_000_us
  AND SPATIAL_BOX IN SIMPLEX (
      MIN [0.0, 0.0, 0.0],
      MAX [5.0, 2.0, 2.0]
  );
```

### 5.3 3D GIS Survey & Subsurface Topography Mesh Sifting
Sifts 3D point cloud LiDAR and subsurface geophysical strata measurements using latitude, longitude, and elevation/depth bounds.

```sql
SIFT GIS_MESH
FROM "subsurface_geophysics.s3a"
WHERE LATITUDE  BETWEEN 37.770000 AND 37.780000
  AND LONGITUDE BETWEEN -122.420000 AND -122.410000
  AND ELEVATION BETWEEN -200.0 AND 50.0;
```

### 5.4 Multi-Dimensional Vector Embedding Similarity Sifting
Sifts multi-dimensional vector embeddings using hardware-accelerated AVX2 / ARM Neon SIMD kernels (`cosine_similarity` or `dot_product`).

```sql
SIFT EMBEDDINGS
FROM "vector_store.s3a"
WHERE TIME BETWEEN 1600000000 AND 1700000000
  AND SIMILARITY TO [0.012, -0.451, 0.882, ..., 0.104]
      USING COSINE THRESHOLD 0.85
LIMIT 10;
```

### 5.5 L2 Rollup & Decentralized AI Data Availability Sampling
Samples L2 Rollup state commitments and KZG roots for zero-copy state availability verification.

```sql
SIFT DA_COMMITMENTS
FROM "da_commitments.s3a"
WHERE BLOCK_HEIGHT BETWEEN 1_000_000 AND 1_050_000;
```

---

## 6. Data Modification & Lifecycle Operations (`INSERT`, `DELETE`, `COMPACT`)

S3A is an append-only, stratified storage architecture. Updates and deletions write new versions or tombstones, which are garbage-collected during compaction.

### 6.1 Insert Telemetry (CREATE)
```sql
INSERT TELEMETRY (1000, 1, 101, 42.50) INTO "sensor_data.s3a";
```

### 6.2 Delete Telemetry (DELETE via Tombstoning)
```sql
DELETE TELEMETRY
WHERE SENSOR_ID = 1 AND METRIC_ID = 101 AND TIME = 1000
FROM "sensor_data.s3a";
```

### 6.3 Compact Archive (Garbage Collection)
Compacts sparse tiles, purges tombstoned records, resolves versioned updates, and aligns remaining data into full 128 KB Hyper-Tiles.

```sql
COMPACT ARCHIVE "sensor_data.s3a";
```

---

## 7. Multi-Tile Fusion Operations (`FUSE TILES`)

Consolidates sparse or distributed Hyper-Tile archives into a unified, densified S3A archive while recalculating bounding hulls and resolving duplicate versioned updates.

```sql
FUSE TILES
FROM ("edge_node_1.s3a", "edge_node_2.s3a", "edge_node_3.s3a")
INTO "fused_cluster.s3a"
WITH (
    RESOLVE_VERSIONED_UPDATES = TRUE,
    PURGE_TOMBSTONES = TRUE,
    RECALCULATE_SIMPLEX_HULLS = TRUE
);
```

---

## 8. Language Bindings & Integration

### 8.1 CLI Integration (`s3a-cli`)
```bash
# Execute direct coordinate address lookup
s3a-cli lookup telemetry.s3a L0:T2:R10

# Sift records matching sensor criteria
s3a-cli get telemetry.s3a 1 101 1000 5000

# Fuse multi-tile archives
s3a-cli fuse consolidated.s3a node1.s3a node2.s3a
```

### 8.2 C-ABI Reference (`s3a-cabi`)
```c
// Direct O(1) coordinate address lookup in C
TelemetryRecord record;
int status = s3a_lookup_coordinate("telemetry.s3a", 0, 2, 10, &record);

// Multi-tile fusion in C
const char* inputs[] = {"node1.s3a", "node2.s3a"};
int fuse_status = s3a_fuse_telemetry(inputs, 2, "fused.s3a");
```

### 8.3 REST API & AI Agent Function Calling Console
The S3A web engine (`s3a-cli serve 8080`) provides a JSON REST API for autonomous AI agents executing S3A-QL queries:

```json
POST /api/agent/execute
{
  "tool_name": "s3a_lookup_coordinate",
  "file": "test.s3a",
  "arguments": {
    "coordinate": "L0:T0:R0"
  }
}
```

#### Sample Response:
```json
{
  "status": "success",
  "tool": "s3a_lookup_coordinate",
  "coordinate": "L0:T0:R0",
  "record": {
    "timestamp": 1000,
    "sensor_id": 1,
    "metric_id": 101,
    "value": 42.5
  }
}
```

---

## 9. Appendix: Error Codes & Status Indicators

| Error Code | Error Variant | Description |
| :--- | :--- | :--- |
| `0` | `SUCCESS` | Operation completed successfully. |
| `-1` | `S3A_ERR_GENERIC` | Invalid argument, null pointer, or file access error. |
| `E001` | `CorruptedHeader` | Magic signature mismatch or header checksum failure. |
| `E002` | `ChecksumMismatch` | Payload data CRC32C validation failed. |
| `E003` | `OutOfBounds` | S3ACoordinate address refers to non-existent tile or record offset. |
| `E004` | `PayloadOverflow` | Record exceeds 128 KB Hyper-Tile or 4 KB Micro-Tile capacity. |
