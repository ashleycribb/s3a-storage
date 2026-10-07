# S3A-QL v1 Language Specification & Query Contract

This specification defines the frozen versioned query and operations grammar for **S3A-QL v1** in the S3A Database storage engine.

---

## 1. Design Principles

- **Storage-Direct & Analytical**: Direct zero-copy projection into SIMD-friendly Morton and Hilbert-arranged 128 KB Hyper-Tiles.
- **Fail-Closed Domain Handoff**: Domain-specific query domains (`KINEMATICS`, `GIS_MESH`, `DA_COMMITMENTS`, `HUMAN_LRS`, `AI_TRACES`, `ACADEMIC_PAPERS`) either execute through their registered domain pack handler or fail with an explicit, actionable error.
- **Machine-Readable Errors**: Typed `QlError` with stable error codes (`parse_error`, `pack_required`, `addon_required`, `unsupported_layout`, `not_found`, `io_error`, `internal_error`) and standard JSON serialization.
- **Optional Version Preamble**: Explicit `VERSION 1;` or `PRAGMA VERSION 1;` preambles allow callers to assert version compatibility upfront.

---

## 2. Formal Grammar (EBNF)

```ebnf
Statement           ::= [ VersionPreamble ] ( FetchStmt | SiftStmt | InsertStmt | DeleteStmt | CompactStmt | FuseStmt ) [ ";" ]

VersionPreamble     ::= ( "VERSION" | "PRAGMA" "VERSION" ) Integer ";"

FetchStmt           ::= "FETCH" "RECORD" "AT" Coordinate "FROM" FilePath
Coordinate          ::= "L" Integer ":T" Integer ":R" Integer

SiftStmt            ::= "SIFT" SiftDomain "FROM" FilePath [ WhereClause ] [ LimitClause ]
SiftDomain          ::= CoreDomain | PackDomain
CoreDomain          ::= "TELEMETRY" | "EMBEDDINGS"
PackDomain          ::= "KINEMATICS" | "GIS_MESH" | "DA_COMMITMENTS"
                      | "HUMAN_LRS" | "AI_TRACES" | "ACADEMIC_PAPERS"
                      | "LEARNING_ACTIVITIES"

WhereClause         ::= "WHERE" Condition { "AND" Condition } [ ComposeClause ]
LimitClause         ::= "LIMIT" Integer

Condition           ::= TimeRangeCond
                      | EqualityCond
                      | SpatialSimplexCond
                      | GisRangeCond
                      | BlockRangeCond
                      | SimilarityCond

TimeRangeCond       ::= "TIME" "BETWEEN" Integer "AND" Integer
EqualityCond        ::= ( "SENSOR_ID" | "METRIC_ID" | "ACTOR_ID" | "VERB_ID" ) "=" Integer
SpatialSimplexCond  ::= "SPATIAL_BOX" "IN" "SIMPLEX" "(" "MIN" Vector3D "," "MAX" Vector3D ")"
GisRangeCond        ::= ( "LATITUDE" | "LONGITUDE" | "ELEVATION" ) "BETWEEN" Float "AND" Float
BlockRangeCond      ::= "BLOCK_HEIGHT" "BETWEEN" Integer "AND" Integer

SimilarityCond      ::= "SIMILARITY" "TO" FloatVector "USING" ( "COSINE" | "DOT_PRODUCT" ) "THRESHOLD" Float
ComposeClause       ::= "COMPOSE" "SIMILARITY" "TO" FloatVector "WEIGHT" Float
                        [ "AND" "SPATIAL_PROXIMITY" Vector3D "WEIGHT" Float ]
                        [ "AND" "DECAY" "HALFLIFE" Float "WEIGHT" Float ]

InsertStmt          ::= "INSERT" "TELEMETRY" "(" Integer "," Integer "," Integer "," Float ")" "INTO" FilePath
DeleteStmt          ::= "DELETE" "TELEMETRY" "WHERE" DeleteConditions "FROM" FilePath
DeleteConditions    ::= DeleteCond { "AND" DeleteCond }
DeleteCond          ::= ( "SENSOR_ID" | "METRIC_ID" | "TIME" ) "=" Integer

CompactStmt         ::= "COMPACT" "ARCHIVE" FilePath
FuseStmt            ::= "FUSE" "TILES" "FROM" "(" FilePathList ")" "INTO" FilePath [ WithOptions ]
FilePathList        ::= FilePath { "," FilePath }
WithOptions         ::= "WITH" "(" OptionAssign { "," OptionAssign } ")"
OptionAssign        ::= Identifier "=" ( "TRUE" | "FALSE" )

Vector3D            ::= "[" Float "," Float "," Float "]"
FloatVector         ::= "[" Float { "," Float } "]"
FilePath            ::= StringLiteral | Identifier
```

---

## 3. Statement Reference

### 3.1 `FETCH RECORD AT`
Direct $O(1)$ coordinate lookup within an archive.
```sql
VERSION 1;
FETCH RECORD AT L0:T0:R12 FROM "telemetry.s3a";
```
Returns: Single record at stratum level `L0`, tile index `T0`, and record payload offset `R12`. Fails with `not_found` if coordinate exceeds committed tile or record bounds.

### 3.2 `SIFT TELEMETRY`
Range sieve over Core Telemetry Hyper-Tiles (Tile 0). Pruned instantly via SIMD min/max timestamp headers.
```sql
SIFT TELEMETRY FROM "fleet.s3a"
WHERE TIME BETWEEN 1700000000 AND 1700003600
  AND SENSOR_ID = 42
LIMIT 100;
```

### 3.3 `SIFT EMBEDDINGS`
Vector similarity and multi-modal algebraic scoring over 128-dimensional embedding Hyper-Tiles (Tile 1).
```sql
SIFT EMBEDDINGS FROM "knowledge.s3a"
WHERE TIME BETWEEN 1700000000 AND 1700090000
COMPOSE SIMILARITY TO [0.12, 0.45, -0.23, ...] WEIGHT 0.70
    AND SPATIAL_PROXIMITY [37.77, -122.41, 100.0] WEIGHT 0.15
    AND DECAY HALFLIFE 86400.0 WEIGHT 0.15
LIMIT 20;
```

### 3.4 Domain Pack SIFT Handshake
When querying pack domains, S3A-QL delegates execution to the registered pack handler. If the pack is missing or unlinked, it fails closed with `pack_required`. If the archive does not have the capability bit enabled in its `addon_mask`, it fails closed with `addon_required`.

| Domain | Tile ID | Pack Dependency | Required Addon Bit |
|:---|:---|:---|:---|
| `KINEMATICS` | 4 | `s3a-pack-robotics` | `ADDON_ROBOTICS` (`0x0004`) |
| `GIS_MESH` | 5 | `s3a-pack-gis` | `ADDON_GIS` (`0x0010`) |
| `DA_COMMITMENTS` | 6 | `s3a-pack-da` | `ADDON_DA` (`0x0020`) |
| `HUMAN_LRS` | 10 | `s3a-pack-scholar` | `ADDON_EDU` (`0x0001`) |
| `AI_TRACES` | 11 | `s3a-pack-scholar` | `ADDON_EDU` (`0x0001`) |
| `ACADEMIC_PAPERS` | 12 | `s3a-pack-scholar` | `ADDON_EDU` (`0x0001`) |
| `LEARNING_ACTIVITIES` | 10 | `s3a-pack-scholar` | `ADDON_EDU` (`0x0001`) |

*Note: Greenfield LRS queries target Scholar Pack Tile 10 (`HumanLrsRecord`). Legacy core Tile 9 writes are permanently denied.*

#### Example: Robotics Trajectory Sieve
```sql
SIFT KINEMATICS FROM "drone_flight.s3a"
WHERE TIME BETWEEN 1000000 AND 5000000
  AND SPATIAL_BOX IN SIMPLEX(MIN [-10.0, -10.0, 0.0], MAX [10.0, 10.0, 50.0])
LIMIT 50;
```

#### Example: Scholar Literature Sieve
```sql
SIFT ACADEMIC_PAPERS FROM "scholar_corpus.s3a"
LIMIT 10;
```

### 3.5 Lifecycle Statements (`INSERT`, `DELETE`, `COMPACT`, `FUSE`)
```sql
-- Append a new telemetry sample
INSERT TELEMETRY (1700001234, 1, 101, 98.6) INTO "live.s3a";

-- Soft-delete matching records by emitting tombstone
DELETE TELEMETRY WHERE SENSOR_ID = 1 AND METRIC_ID = 101 AND TIME = 1700001234 FROM "live.s3a";

-- Consolidate and purge tombstones
COMPACT ARCHIVE "live.s3a";

-- Fuse multiple archives into consolidated target
FUSE TILES FROM ("day1.s3a", "day2.s3a") INTO "week.s3a"
WITH (PURGE_TOMBSTONES = TRUE, RESOLVE_VERSIONED_UPDATES = TRUE);
```

---

## 4. Error Model & Machine-Readable JSON

All errors emitted by the query parser or engine return typed `QlError` instances:

| Error Code | Meaning | Example Message |
|:---|:---|:---|
| `parse_error` | Syntax, token, or grammar error | `"Expected token Semicolon, found Identifier"` |
| `pack_required` | Required domain pack is not registered | `"Domain query requires pack 's3a-pack-robotics' for feature 'kinematics'"` |
| `addon_required` | Archive `addon_mask` lacks required capability | `"Archive does not have capability 0x0001 ('scholar / edu') enabled"` |
| `unsupported_layout` | Tile layout version newer than reader capabilities | `"Unsupported record layout version 2 for tile type 10"` |
| `not_found` | Coordinate or address out of bounds | `"Record not found at coordinate L0:T999:R0"` |
| `io_error` | Filesystem or memory map error | `"The system cannot find the file specified"` |
| `internal_error` | Unexpected engine or compactor failure | `"Compaction lock contention"` |

### JSON Output Schemas

#### Successful Result (`s3a-cli query --json`)
```json
{
  "ok": true,
  "type": "sift_telemetry",
  "elapsed_us": 142,
  "count": 1,
  "records": [
    {
      "coord": "L0:T0:R0",
      "timestamp": 1000,
      "sensor_id": 42,
      "metric_id": 99,
      "value": 123.45
    }
  ]
}
```

#### Pack Data Result
```json
{
  "ok": true,
  "type": "pack_data",
  "elapsed_us": 85,
  "domain": "human_lrs",
  "count": 50,
  "summary": "Found 50 human decision/LRS records in 'scholar.s3a'"
}
```

#### Error Result
```json
{
  "ok": false,
  "code": "pack_required",
  "message": "Domain query requires pack 's3a-pack-robotics' for feature 'kinematics'",
  "pack": "s3a-pack-robotics",
  "feature": "kinematics"
}
```
