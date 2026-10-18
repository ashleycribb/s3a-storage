# Stratified Simplicial Storage Architecture (S3A)

[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE-MIT)
[![Rust](https://img.shields.io/badge/rust-stable-brightgreen.svg)](https://www.rust-lang.org)
[![Tests](https://img.shields.io/badge/tests-417%20passed-success.svg)]()

**S3A** is a high-performance, crash-resilient, multi-modal columnar database and storage engine. It replaces traditional row-based databases and flat vector indexes with **bounded, self-indexing geometric manifolds** called **Hyper-Tiles**. 

High-dimensional states, spatiotemporal points, robotics kinematics, and continuous learning records are bounded by simplicial convex hulls and SIMD Bloom filters directly within 512-byte sector-aligned headers—enabling single-cycle SIMD tile rejection and zero-copy memory mapping.

---

## Workspace Crates & Architecture

### Core Storage & Execution Engine
| Crate | Description |
| :--- | :--- |
| **[`s3a-core`](crates/s3a-core)** | Zero-copy byte layouts (`bytemuck::Pod`), record formats, 512B Hyper-Tile headers, coordinate addressing (`L<lvl>:T<tile>:R<offset>`), and `no_std` geometry. |
| **[`s3a-simd`](crates/s3a-simd)** | SIMD acceleration (AVX2/NEON): cosine similarity, Int8 & 4-bit vector quantization, 960-bit Blocked-Bloom filtering, learned index interpolation, and "alive-before-scoring" pruning. |
| **[`s3a-engine`](crates/s3a-engine)** | Zero-copy `MmapReader`, `TileWriter` with dual-slot A/B atomic commits, `L0RingBuffer` real-time streaming, background compactor daemon, native S3A-QL parser/executor, and `S3AP` binary TCP wire protocol. |
| **[`s3a-edge`](crates/s3a-edge)** | Ultra-low-latency edge ingestion and streaming query engine for distributed sensor and edge nodes. |
| **[`s3a-cabi`](crates/s3a-cabi)** | C Foreign Function Interface (`libs3a.so` / `s3a.dll` / `s3a.h`) for Python, C, C++, and Go integrations. |
| **[`s3a-cli`](crates/s3a-cli)** | CLI command-line tool, interactive SQL shell (`s3a-cli shell`), TCP daemon (`serve-tcp`), MCP server (`mcp-server`), web dashboard (`serve`), and comprehensive benchmark suites. |

### Autonomous Agent Swarms & Edge Coordination
| Crate / Subsystem | Description |
| :--- | :--- |
| **[`s3a-gate` (Cloudflare)](cloudflare/)** | Edge routing worker + `RepoCoordinator` Durable Object per project. Provides GitHub-style UI shell, WebCrypto Ed25519 commit and supervisor authorization, automated in-gate PM/builder agent (`gate-agent`), R2 screenshot evidence verification barrier, and rehearsal simulation dashboard. |
| **[`s3a-swarm`](crates/s3a-swarm)** | High-concurrency agent swarm coordination CLI (`s3a-swarm run --remote`), deterministic LWW merge rules, signature tie-breaking, shadow-mode evaluation, and epistemic conflict detection. |
| **[`s3a-harness`](crates/s3a-harness)** | Open-source Model Context Protocol (MCP 2024-11-05) research-agent harness, path-confinement sandbox, and supervisor clearance gates (`ask_human`). |
| **[`s3a-ledger`](crates/s3a-ledger)** | Granular micro-billing, budget caps, ledger records, and tamper-evident token expenditure accounting for agent swarms. |
| **[`s3a-trace`](crates/s3a-trace)** | Agent trajectory capture, replay harness, and fine-tuning export in ShareGPT and ChatML formats. |
| **[`s3a-vcs`](crates/s3a-vcs)** | Cryptographic version control for S3A archives (branching, commits, Merkle seals, deterministic union merges, and read-only Git repo export). |
| **[`s3a-vault`](crates/s3a-vault)** | Decentralized Ed25519 identity, `gate://` addressing, scoped capability delegations, and key rotations. |
| **[`s3a-lrs-server`](crates/s3a-lrs-server)** | Standards-compliant IEEE P2997 ELR projection, deterministic rules engine, and batch synchronization server (`POST /xapi/s3a/batch`). |
| **[`s3a-wasm-snapshot`](crates/s3a-wasm-snapshot)** | Additive in-memory WebAssembly snapshot engine for browser-side research, vector search, and agent executions with zero panics across the WASM boundary. |

### Domain Packs & Conformance
| Crate | Description |
| :--- | :--- |
| **[`s3a-pack-scholar`](crates/s3a-pack-scholar)** | Literature graph, academic papers, claims, evidence, dual-trace provenance, and citation recheck. |
| **[`s3a-pack-ontology`](crates/s3a-pack-ontology)** | Formal ontology concepts, axioms, analogy bridges, and pedagogical rules. |
| **[`s3a-pack-da`](crates/s3a-pack-da)** | Blockchain and AI Data Availability block commitment pack with KZG polynomial verification. |
| **[`s3a-pack-gis`](crates/s3a-pack-gis)** | 3D GIS, LiDAR, and subsurface topographic mesh geometry pack. |
| **[`s3a-pack-robotics`](crates/s3a-pack-robotics)** | 1 kHz continuous 3D kinematic trajectory ring-buffering pack. |
| **[`s3a-pack-wearable`](crates/s3a-pack-wearable)** | Zero-heap 4KB Flash page wearable biosensor pack. |
| **[`s3a-spec-vectors`](crates/s3a-spec-vectors)** | Conformance test vectors and validation suites. |

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

## Documentation & Specifications

- **[Autonomous Platform Engineering Pattern (Rehearsal Loop)](docs/PLATFORM-PATTERN.md)**: Generic rehearsal-driven simulation loop, declarative tournament formats, game packs, auto-filing of missing pages/logic, and real R2 visual evidence verification barriers.
- **[S3A Batched Synchronization Contract](docs/sync.md)**: Client-to-server batched synchronization contract between Scholar Explorer WebAssembly client, Google AI Studio agents, and the S3A LRS server.
- **[Formal Research Traceability Profile & Ontology](docs/vocab/scholar-xapi-profile.jsonld)**: W3C PROV-O aligned xAPI Profile (JSON-LD) and OWL Ontology (Turtle: `docs/vocab/scholar.ttl`) establishing human epistemic judgment invariants.
- **[S3A-QL v1 Language Specification & Query Contract](docs/s3a-ql-v1.md)**: Frozen grammar, statements (`FETCH`, `SIFT`, `INSERT`, `DELETE`, `COMPACT`, `FUSE`), pack domain handoff, and machine-readable error models.
- **[Production Operations Manual & DBA Runbook](docs/ops.md)**: Administrator operations (`inspect`, `verify`, `status`, `doctor`, `recover`, `compact`, `fuse`, `addon`, `snapshot`), machine-readable `--json` output, and recovery runbooks.
- **[S3A Pack Contract](docs/pack-contract.md)**: Modular domain pack architecture, wire tile ID allocation, and capability mask (`addon_mask`) fail-closed write gates.
- **[Durability & Crash Recovery Contract](docs/durability.md)**: Dual-slot A/B commit mechanics, append write barriers, and crash-safe compaction replacement.
- **[Schema Evolution Contract](docs/schema-evolution.md)**: Hyper-tile record layout versioning (`record_layout_version`), global registry, and additive evolution rules.
- **[Scholar Ingestion & Snapshot Specification](docs/scholar-ingest.md)**: Ingestion of external literature corpora into Scholar Hyper-Tiles and portable SRS snapshot containers.
- **[Commercial Vision & Business Architecture](docs/VISION-BUSINESS.md)**: Autonomous agent data cleaning, problem scoping, human-in-the-loop governance, and commercial engagement flow.
- **[Pull Request Triage & Human Governance Checklist](docs/PR-TRIAGE.md)**: Swarm duplicate PR closure, canonical PR identification, Playwright QA evaluation, and sensitive subsystem review policies.
- **[End-to-End Walkthrough & Architecture Log](walkthrough.md)**: Comprehensive architectural walkthrough covering all 32 development phases, IEEE P2997 ELR projection, VCS engine, Cloudflare Workers gate, and full 417-test suite verification.

---

## License

Dual-licensed under either:
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.

