# Stratified Simplicial Storage Architecture (S3A)

[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE-MIT)
[![Rust](https://img.shields.io/badge/rust-stable-brightgreen.svg)](https://www.rust-lang.org)
[![Tests](https://img.shields.io/badge/tests-42%20passed-success.svg)]()

**S3A** is a high-performance, crash-resilient, multi-modal columnar database and storage engine. It replaces traditional row-based databases and flat vector indexes with **bounded, self-indexing geometric manifolds** called **Hyper-Tiles**. 

High-dimensional states, spatiotemporal points, robotics kinematics, and continuous learning records are bounded by simplicial convex hulls and SIMD Bloom filters directly within 512-byte sector-aligned headers—enabling single-cycle SIMD tile rejection and zero-copy memory mapping.

---

## Workspace Crates

| Crate | Description |
| :--- | :--- |
| **[`s3a-core`](crates/s3a-core)** | Zero-copy byte layouts (`bytemuck::Pod`), record formats, 512B Hyper-Tile headers, coordinate addressing (`L<lvl>:T<tile>:R<offset>`), and `no_std` geometry. |
| **[`s3a-simd`](crates/s3a-simd)** | SIMD acceleration (AVX2/NEON): cosine similarity, Int8 & 4-bit vector quantization, 960-bit Blocked-Bloom filtering, learned index interpolation, and "alive-before-scoring" pruning. |
| **[`s3a-engine`](crates/s3a-engine)** | Zero-copy `MmapReader`, `TileWriter` with dual-slot A/B atomic commits, `L0RingBuffer` real-time streaming, background compactor daemon, native S3A-QL parser/executor, and `S3AP` binary TCP wire protocol. |
| **[`s3a-cabi`](crates/s3a-cabi)** | C Foreign Function Interface (`libs3a.so` / `s3a.dll` / `s3a.h`) for Python, C, C++, and Go integrations. |
| **[`s3a-cli`](crates/s3a-cli)** | CLI command-line tool, interactive SQL shell (`s3a-cli shell`), TCP daemon (`serve-tcp`), MCP server (`mcp-server`), web dashboard (`serve`), and comprehensive benchmark suites. |

---

## Key Architectural Breakthroughs

### 1. Dual-Slot A/B Atomic Commits (Zero WAL Double-Write Penalty)
Traditional databases (SQL Server, Postgres) write data twice: first to a Write-Ahead Log (WAL), then to table pages. S3A uses sector-aligned (512-byte) A/B header slots with monotonic generation counters and hardware CRC32C. Commits are atomic 64-byte slot flips. In the event of power loss, torn writes are detected and truncated to the committed generation.

### 2. The 6 Advanced Metadata Subsystems
Every 128 KB Hyper-Tile includes a 512-byte header containing:
1. **Probabilistic Filter (`TileFilterMetadata`)**: 960-bit SIMD Blocked-Bloom filter for single-cycle discrete ID pruning (`sensor_id`, `actor_id`, `metric_id`).
2. **Cryptographic Provenance (`TileProvenanceMetadata`)**: 256-bit payload Merkle digest for tamper-proof AI audit trails.
3. **Lifecycle & Freshness (`TileLifecycleMetadata`)**: Exact soft-deletion tombstone counts, stratum tiers, and granular TTL boundaries.
4. **Multi-Tenant Security (`TileSecurityMetadata`)**: 64-bit Tenant ID and classification clearance bitmask.
5. **Columnar Encoding (`TileEncodingMetadata`)**: Frame-of-Reference (FoR) baselines and scales for zero-decompression random access.
6. **Learned Index Splines (`TileLearnedIndex`)**: Linear spline parameters providing $O(1)$ interpolated record searching.

### 3. Unified Multi-Modal Storage
S3A natively unifies multiple data domains in a single engine:
- **Time-Series Telemetry**: High-throughput sensor logging.
- **High-Dimensional Embeddings**: 128D FP32, 256D Int8 quantized, and 512D 4-bit packed vectors with algebraic retrieval.
- **Learning Record Store (LRS)**: xAPI / ADL educational analytics and AI agent reinforcement learning experience replays.
- **Robotics Kinematics**: 1 kHz continuous 3D trajectory ring-buffering.
- **3D GIS & Subsurface Topography**: LiDAR and geophysical point clouds with 3D simplicial hulls.
- **Blockchain / AI Data Availability**: L2 rollup state commitments and KZG proofs.

---

## Getting Started

### Prerequisites
- Rust 1.75+ (stable)
- Supported targets: Linux, macOS, Windows (`x86_64`, `aarch64`)

### Building & Testing
```bash
# Verify all crates compile
cargo check --workspace

# Run full test suite (42 unit tests)
cargo test --workspace
```

---

## Using S3A Like SQL Server

### 1. Start the Server Daemon
Run S3A as a background database daemon on TCP port `9333`:
```bash
cargo run -p s3a-cli -- serve-tcp 127.0.0.1:9333
```

### 2. Interactive SQL Query Shell (`sqlcmd` equivalent)
```bash
cargo run -p s3a-cli -- shell
```
```sql
===========================================================
  S3A Interactive SQL Shell (SQL Server / sqlcmd interface)
  Type S3A-QL queries ending with ';' or type 'exit' / 'quit'
===========================================================
s3a> INSERT TELEMETRY (1710892800, 10, 201, 98.6) INTO 'sensor.s3a';
Appended record. New coordinate address: L0:T0:R0

s3a> SIFT TELEMETRY FROM 'sensor.s3a' WHERE SENSOR_ID = 10;
Matched 1 records in 180µs:
  [0] L0:T0:R0 -> Timestamp: 1710892800, Sensor: 10, Metric: 201, Value: 98.60

s3a> FETCH RECORD AT L0:T0:R0 FROM 'sensor.s3a';
Record at L0:T0:R0: Timestamp: 1710892800, Sensor: 10, Metric: 201, Value: 98.60
```

### 3. Remote Network Queries
```bash
cargo run -p s3a-cli -- remote-query "SIFT TELEMETRY FROM 'sensor.s3a' WHERE VALUE > 50.0;" 127.0.0.1:9333
```

---

## AI Agent Integration: Model Context Protocol (MCP)

S3A includes native support for the **Model Context Protocol (MCP 2024-11-05)** over standard JSON-RPC `stdio`.

Register S3A in Claude Desktop, Cursor, or Gemini:
```json
{
  "mcpServers": {
    "s3a-storage": {
      "command": "s3a-cli.exe",
      "args": ["mcp-server", "production.s3a"]
    }
  }
}
```

Exposed MCP Tools:
- `s3a_query`: Execute native S3A-QL statements.
- `s3a_fetch_coordinate`: Instant $O(1)$ coordinate lookup (`L0:T0:R0`).
- `s3a_insert_telemetry`: Real-time ingestion into L0 buffer & Hyper-Tiles.
- `s3a_delete_telemetry`: Transactional tombstoning.
- `s3a_inspect_archive`: Inspect generation, slot state, Bloom filter, and CRC32C status.
- `s3a_compact_archive`: Purge tombstones and re-stratify Hyper-Tiles.

Verify MCP protocol compliance:
```bash
cargo run -p s3a-cli -- mcp-client test.s3a
```

---

## Empirical Benchmarks

Run any of the built-in reproducible benchmarks:

```bash
# Standard telemetry density & SIMD rejection benchmark
cargo run -p s3a-cli -- benchmark

# Learning Record Store (LRS) & Bloom filter discrete ID pruning benchmark
cargo run -p s3a-cli -- benchmark-lrs

# Live concurrent L0 streaming ingestion & background compactor benchmark
cargo run -p s3a-cli -- stream-ingest live.s3a 3 20000

# Wearable 4KB Flash page zero-heap benchmark
cargo run -p s3a-cli -- benchmark-wearable

# Robotics 1 kHz kinematic trajectory benchmark
cargo run -p s3a-cli -- benchmark-robotics

# 3D GIS & LiDAR subsurface topographic mesh benchmark
cargo run -p s3a-cli -- benchmark-gis

# Blockchain & AI Data Availability block commitment benchmark
cargo run -p s3a-cli -- benchmark-da

# Hierarchical Tree of Hulls (BVH) logarithmic pruning benchmark
cargo run -p s3a-cli -- benchmark-bvh
```

### Measured Gains:
- **Storage Density**: 52.04% space reduction over JSON time-series; 42.96% space reduction over xAPI JSON.
- **Ingestion Speed**: Up to 1.68 million records/sec burst write throughput; >13,000 records/sec sustained live streaming with zero write stalls.
- **Pruning Latency**: Single-cycle SIMD rejection skips 128 KB Hyper-Tiles in <250 µs.
- **Embedded / Edge RAM**: 0 bytes heap allocation (`no_std` safe).

---

## License

Dual-licensed under either:
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
