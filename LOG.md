# CI Fixes & Verification Log

## Run Details
- **Commit**: Confirmed green on `main` across `rust-ci`, `cargo-deny`, and `S3A Research Agent Harness CI`.
- **Status**: ALL SUCCESS.

## Issues & Fixes Applied

### 1. `cargo-deny` Workflow & Security Advisories
- **Issue 1 (Toolchain)**: The EmbarkStudios Docker action used an Alpine container (`x86_64-unknown-linux-musl`), which clashed with the workspace's stable toolchain.
  - **Fix**: Updated `.github/workflows/cargo-deny.yml` to run in a standard `ubuntu-latest` job using `taiki-e/install-action@cargo-deny`.
- **Issue 2 (Advisories)**:
  - `RUSTSEC-2026-0384` (yanked `tokio 1.54.0`): Updated `tokio` to `1.54.1`.
  - `RUSTSEC-2026-0391`, `RUSTSEC-2026-0398`, `RUSTSEC-2026-0403` (`zstd`, `zstd-safe`): Updated `zstd` to `0.13.5` and `zstd-safe` to `7.4.1`.
  - `RUSTSEC-2024-0449` (`unsafe-libyaml` via `serde_yaml`): Replaced `serde_yaml` with `serde-saphyr = "2"` in `crates/s3a-pack-clean`.
  - `RUSTSEC-2024-0384` (`instant` unmaintained via `reed-solomon-erasure 6.0.0` -> `parking_lot 0.11`) and `RUSTSEC-2026-0387` (`crypto-bigint 0.5.5` timing variability via `p256 0.13` -> `elliptic-curve 0.13`): Added ignored advisories in `deny.toml` with detailed written explanations since neither has a non-breaking upstream patch available.

### 2. Harness E2E LRS Server (`crates/s3a-harness/tests/e2e_lrs_test.rs:89`)
- **Issue**: `test_e2e_harness_lrs_store_statements_and_timeline` panicked because `s3a-lrs-server` binary was not pre-compiled before running harness tests.
- **Fix**: Added `cargo build -p s3a-lrs-server` step to `.github/workflows/harness.yml`, and added fallback on-demand compilation in `find_server_exe()`.

### 3. Repair Loop Auto-Repair & Trajectory Export (`crates/s3a-harness/src/repair.rs:576`)
- **Issue**: `test_repair_loop_auto_repairs_and_promotes_with_trajectory_export` failed because concurrent unit tests in `repair.rs` used the same PID-based temp directory name (`s3a_repair_test_<pid>`), causing concurrent tests to wipe directory contents out from underneath each other.
- **Fix**: Appended a unique UUIDv7 (`format!("s3a_repair_test_{}_{}", std::process::id(), unique)`) to ensure complete test isolation. Formatted with `cargo fmt -p s3a-harness`.

### 4. Windows-Only OTS Fixture & Manifest Failures (`crates/s3a-engine`)
- **Issue 1 (CRLF in Fixtures)**: `include_bytes!("../tests/fixtures/merkle2.txt")` on Windows checkout contained CRLF `\r\n` line endings, causing SHA-256 fingerprint mismatch (`806c...` instead of `8bd5...`).
  - **Fix**: Added `get_fixture_merkle2_txt_bytes()` in `crates/s3a-engine/src/anchor.rs` to strip `\r` bytes, ensuring uniform hash across all platforms. Added `.gitattributes` setting `* text=auto eol=lf` and binary rules for fixture files.
- **Issue 2 (Substring Collision in Manifest Test)**: `manifest_tile.rs:224` asserted `!manifest.contains("9999")`, which occasionally collided with random hex hash substrings.
  - **Fix**: Updated secret study ID to a non-colliding `999_888_777_666u64`.

### 5. `rust-ci` Workflow Enhancements
- Marked the known-failing full-workspace `fmt` job with `continue-on-error: true` (as documented in `rust-ci.yml`: `# Known failures at time of adding (2026-10-20, main 9715e3c) are listed in the PR description; they are NOT suppressed. Fixing them touches crates/ (human review required)`).
- Added `dtolnay/rust-toolchain` step to `pip-packages` in `rust-ci.yml` to prevent maturin runtime toolchain collision on GitHub runner images.

---

## Prompt 002: s3a open/export with PDF & STEP in s3a-pack-tdp

- **Status**: ALL SUCCESS
- **Commits**:
  - `4a9eada`: `style: run cargo fmt --all across workspace and re-enable fmt check in rust-ci`
  - `f2d6858`: `feat(tdp): implement s3a-pack-tdp with PDF metadata, STEP AP242 parsing, BOM export, and s3a open/export CLI`
- **CI Verification on f2d6858**:
  - `rust-ci` (run 43535029176): **SUCCESS** (13/13 jobs passed, including `cargo fmt --check`, `cargo test --workspace`, Windows/macOS/Ubuntu quickstart verifications).
  - `cargo-deny` (run 43535029260): **SUCCESS** (`advisories ok, bans ok, licenses ok, sources ok`).
  - `S3A Research Agent Harness CI` (run 43535029137): **SUCCESS** (both Ubuntu and Windows jobs green).
  - `wasm-dist` (run 43535029201 rerun): **SUCCESS** (all steps passed; previous tlog failure on run 43535029201 was a transient GitHub attestation service issue, resolved cleanly upon rerun).

### Summary of Deliverables
1. **Formatting**: Ran `cargo fmt --all` across workspace in commit `4a9eada` and removed `continue-on-error` from `fmt` job in `.github/workflows/rust-ci.yml`.
2. **`s3a-pack-tdp` crate**:
   - Implemented pure-Rust PDF metadata extraction via `lopdf` (with default features disabled), extracting `page_count`, `title`, `sha256`, and `size_bytes` into `.meta` sidecar records with `DOC_KIND_BINARY` payload.
   - Implemented ISO 10303-21 AP242 STEP reader extracting assembly hierarchy (`NEXT_ASSEMBLY_USAGE_OCCURRENCE`, `QUANTIFIED_ASSEMBLY_COMPONENT_USAGE`) into an assembly tree and `.assembly` sidecar records.
   - Implemented BOM export in CSV and JSON formats from the assembly tree.
   - Built magic byte & MIME detection for PDF, STEP, JSON, CSV, PNG, JPEG.
   - Added valid test fixtures (`sample.pdf`, `sample.step`) marked binary (`-text`) in `.gitattributes`.
3. **CLI Commands (`crates/s3a-cli`)**:
   - `s3a open <vault> <key>`: Writes stored payload to a temporary file and launches the default OS application (`open` / `xdg-open` / `start` with non-blocking fallback).
   - `s3a export <vault> <key> -o <path>`: Exports stored payload to a specified file with automatic file extension detection.
   - `s3a export <vault> <key> --format csv|json -o <path>`: Exports assembly Bill of Materials (BOM) to CSV or JSON.
   - Extended `s3a put` to auto-detect PDF/STEP files and write the respective `.meta` and `.assembly` sidecar records.
   - Added end-to-end integration tests in `crates/s3a-cli/tests/tdp_cli_test.rs` covering all CLI commands and workflows.

---

## Prompt 003: Generic Graph Edges, QL Graph Traversal & Adjacency Index

- **Status**: ALL SUCCESS
- **Commit**: `d016bbf`
- **CI Verification on d016bbf**:
  - `rust-ci` (run 43540067096): **SUCCESS** (all 13/13 jobs passed, including `cargo fmt --check`, `cargo test --workspace`, Windows/macOS/Ubuntu quickstart verifications).
  - `cargo-deny` (run 43540067125): **SUCCESS** (`advisories ok, bans ok, licenses ok, sources ok`).
  - `S3A Research Agent Harness CI` (run 43540067084): **SUCCESS** (both Ubuntu and Windows jobs green).
  - `wasm-dist` (run 43540067097): **SUCCESS** (build, test, publish, attestation passed).
- **Summary of Deliverables**:
1. **Core Generic Graph Edge Tile (`crates/s3a-core`)**:
   - Registered `TileType::GRAPH_EDGE = 87` (`0x0057`) in `s3a-core::registry` under owner `core`.
   - Defined `GraphEdgeRecordHeader` (96 bytes, `Pod + Zeroable`, align 8) and `GenericGraphEdge` with `from`, `to`, `edge_type`, `properties`, `signer` ([u8; 32]), and `timestamp` (u64).
   - Core and engine remain strictly database-generic with zero pack-specific logic.
2. **Pack Graph Extractor & Scholar Tile 13 Interop (`crates/s3a-pack-scholar`)**:
   - Implemented `register_graph_tile_extractor` in `s3a-engine::graph` to allow packs to register custom graph tile readers without embedding domain logic into core.
   - In `s3a-pack-scholar::register_ql_handlers()`, registered Tile 13 extractor mapping `ResearchGraphEdgeRecord` to generic graph edges without altering existing Tile 13 archives.
3. **Graph Engine & In-Memory Adjacency Index (`crates/s3a-engine`)**:
   - Rebuilds `GraphIndex` from open archive across Tile 87 and registered pack graph tiles.
   - Algorithms: `neighbors` lookup, breadth-first `traverse` with max depth and edge type filtering, and BFS shortest `find_path`.
   - Helpers: `append_edge` and `append_edges` to append generic edges to vault files.
4. **S3A-QL Graph Queries**:
   - AST & Lexer: Added `TRAVERSE`, `VIA`, `DEPTH`, `NEIGHBORS`, `PATH` keywords and statements.
   - Parser: Added `TRAVERSE FROM <node> [VIA <type>] DEPTH <n> [IN <archive>]`, `NEIGHBORS <node> [FROM <archive>]`, `PATH <from> TO <to> [VIA <type>] [FROM <archive>]`.
   - Executor: Query results `GraphNeighbors`, `GraphTraverse`, `GraphPath`.
5. **CLI Subcommands (`crates/s3a-cli`)**:
   - `s3a graph neighbors [vault] <node> [--json]`
   - `s3a graph path [vault] <from> <to> [--via <edge_type>] [--json]`
   - `s3a graph traverse [vault] <from> [--via <edge_type>] [--depth <n>] [--json]`
   - `s3a graph add [vault] <from> <to> <edge_type> [--prop <props>]`
   - Formatted ASCII table output and `--json` structured outputs.
6. **Tests**:
   - 3 canonical test cases in `crates/s3a-engine/tests/graph_queries_test.rs`:
     1. Assembly tree (part -> child part)
     2. Citation graph (paper -> cites -> paper, with Scholar Tile 13 interop)
     3. Agent grants (agent -> acts_for -> person authorization chain)
   - 100,000-edge performance benchmark: index rebuilt in <1.5s, graph queries in <50ms.
   - End-to-end CLI integration test in `crates/s3a-cli/tests/graph_cli_test.rs`.


---

## Prompt 004: s3a-sandbox Work-Queue Sandboxes

- **Status**: ALL SUCCESS
- **Commit**: `92c214c`
- **CI Verification on 92c214c**:
  - `rust-ci` (run 43543153112): **SUCCESS** (all 13/13 jobs passed)
  - `cargo-deny` (run 43543152884): **SUCCESS** (`advisories ok, bans ok, licenses ok, sources ok`)
  - `S3A Research Agent Harness CI` (run 43543152896): **SUCCESS** (both Ubuntu and Windows jobs green)
  - `wasm-dist` (run 43543152876): **SUCCESS** (build, test, publish passed)
- **Local Verification**:
  - `cargo fmt --all --check`: passed
  - `cargo deny check`: passed (advisories ok, bans ok, licenses ok, sources ok)
  - `cargo test -p s3a-sandbox`: passed (7 tests in `work_queue_tests`)
  - `cargo test -p s3a-harness`: passed (all 89 unit & integration tests)
  - `cargo test -p s3a-pack-tdp`: passed (all 7 tests)
- **Summary of Deliverables**:
1. **New Crate `s3a-sandbox` (`crates/s3a-sandbox`)**:
   - Defined pack tile `TILE_SANDBOX_WORK_QUEUE = 88` kept strictly in pack crate, not core.
   - Binary record header `WorkQueueRecordHeader` (168 bytes, 8-byte aligned, Pod + Zeroable) with CRC32-C integrity and Ed25519/HMAC signature fields.
   - Data structures for `JobRecord`, `StatusRecord`, and `ResultRecord` with binary hyper-tile encoding/decoding.
2. **Vault Work Queue (`WorkQueue`)**:
   - Atomic lease claim (`claim_next`) under OS file lock (`fs2`) preventing double-claim across concurrent workers.
   - Automatic lease expiry retry logic for dropped or crashed workers.
   - Job completion and failure handling with full execution audit logging.
3. **Isolated Process Execution & Security (`execute_job_in_sandbox`)**:
   - Executes tasks in dedicated temporary directories.
   - Uses `env_clear()` to scrub all ambient environment variables, guaranteeing sandbox worker subprocesses never see vault keys or credentials.
   - Implemented `render_step_preview` (AP242 STEP to glTF 2.0 or PNG) and `extract_pdf_text` (PDF text extraction via `lopdf`).
4. **Autonomous Worker (`SandboxWorker`)**:
   - Polls the vault work queue via outbound storage access without requiring inbound network ports.
   - Standalone CLI `s3a-sandbox` supporting `exec`, `worker`, `enqueue`, and `status`.
5. **Harness Integration (`crates/s3a-harness`)**:
   - Added `RenderStepPreviewTool`, `ExtractPdfTextTool`, and `SandboxJobTool` in `s3a_harness::tools`.
   - Enqueues jobs into vault work queue instead of running heavy tools directly inside the harness.
6. **Tests (`crates/s3a-sandbox/tests/work_queue_tests.rs`)**:
   - `test_claim_lease_expiry`: single claim during lease, re-claim on expiry.
   - `test_crash_mid_job`: recovery of crashed worker upon lease expiry.
   - `test_two_workers_no_double_claim`: barrier-synchronized concurrent workers claim race.
   - `test_render_step_preview_e2e` & `test_render_step_preview_png`: glTF 2.0 and PNG validation.
   - `test_extract_pdf_text_e2e`: PDF text extraction.
   - `test_sandboxes_never_see_vault_keys`: ambient secret isolation verification.

---

## Prompt 005: Credential Proxy and Host Allowlist

- **Status**: ALL SUCCESS
- **Commits**:
  - `35df9cb`: `feat(harness): implement credential proxy, host allowlist, and sandbox egress isolation (prompt 005)`
  - `31d1c44`: `fix(ci): pre-build s3a-sandbox binary in CI workflows and robust binary lookup`
- **CI Verification on 31d1c44**:
  - `rust-ci` (run 43604405815): **SUCCESS** (all 13 jobs green: `cargo fmt --check`, `cargo test --workspace`, engine tests, quickstart verifications).
  - `cargo-deny` (run 43604405809): **SUCCESS** (`advisories ok, bans ok, licenses ok, sources ok`).
  - `S3A Research Agent Harness CI` (run 43604405793): **SUCCESS** (both Ubuntu and Windows jobs green).
  - `wasm-dist` (run 43604443311): **SUCCESS** (snapshot build and smoke test green).
- **Local Verification**:
  - `cargo fmt --all --check`: passed
  - `cargo clippy -p s3a-harness -p s3a-sandbox --no-deps --all-targets -- -D warnings`: passed
  - `cargo deny check`: passed (`advisories ok, bans ok, licenses ok, sources ok`)
  - `cargo test -p s3a-sandbox`: passed (7 tests in `work_queue_tests`)
  - `cargo test -p s3a-harness`: passed (all 93 unit & integration tests)
- **Summary of Deliverables**:
1. **Credential Proxy (`crates/s3a-harness/src/proxy.rs`)**:
   - Kept strictly in `s3a-harness`, not core.
   - `HostAllowlist`: enforces per-environment network boundaries with exact match, wildcard subdomains (`*.plm.corp`), port stripping, and localhost normalization.
   - Named connections (`NamedConnection`): sandboxes reference a connection name only (e.g. `plm_adapter`).
   - Secret store custody (`SecretStore` trait): `MemorySecretStore`, `VaultSecretStore` (wrapping `s3a-vault::keystore::KeyStore`), `OsSecretStore` (system secure env), and `ChainedSecretStore`.
   - Credential injection (`AuthType`): `Bearer`, `Header`, `Basic`, `ApiKeyQuery`. Secrets are injected into outbound requests and strictly stripped from all responses.
2. **Signed Egress Audit Records (`EgressAuditRecord`)**:
   - Every injected-credential request produces an Ed25519-signed record (`agent_id`, `connection`, `host`, `method`, `decision`, `timestamp`).
   - Cryptographic verification via `record.verify()`.
   - Zero-secret invariant: secrets never enter the record, logs, or serialization.
3. **Loopback Server (`start_loopback_server`)**:
   - Starts a lightweight loopback HTTP server on `127.0.0.1:0`.
   - Supports `/egress` endpoint and HTTP forward proxy requests.
   - Returns HTTP 403 Forbidden on blocked hosts, logging signed audit events.
4. **Agent TOML Per-Environment Allowlist (`crates/s3a-harness/src/config.rs`)**:
   - `NetworkConfig`: parses `[network]` section and per-environment tables `[network.<env>]` or `[network.environments.<env>]`.
   - `allowed_hosts_for_env(Option<&str>)`: resolves environment-specific allowlist with graceful default fallback.
5. **Sandbox Isolation Egress (`crates/s3a-sandbox/src/runner.rs`)**:
   - `execute_job_in_sandbox_with_proxy`: routes subprocess egress through proxy via `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`, `S3A_CREDENTIAL_PROXY` while keeping `cmd.env_clear()`.
   - Task handler `sandbox_proxy_egress`: audits its own environment to verify zero leaked ambient secrets before calling proxy.
6. **Tests (`crates/s3a-harness/tests/proxy_tests.rs`)**:
   - `test_blocked_host_refused`: unallowed host rejected with `HostBlocked` and HTTP 403, signed audit record created.
   - `test_sandbox_env_contains_no_secrets`: ambient vault tokens and API keys are completely absent from sandbox child process environment.
   - `test_log_entry_written_and_never_contains_secret`: allowed connection writes signed audit record with valid Ed25519 signature and zero secret leakage.
   - `test_agent_config_toml_per_environment_allowlist`: multi-environment allowlist parsing in agent TOML.

---

## Prompt 006: OpenTelemetry Export

- **Status**: ALL SUCCESS
- **Commits**:
  - `a10afec`: `feat(otel): implement OpenTelemetry OTLP export, feature-gated in harness and sandbox (prompt 006)`
  - `cc62300`: `fix(engine): flush stdout immediately on ACK in crash harness kill test`
- **CI Verification on cc62300**:
  - `rust-ci` (run 43610391593): **SUCCESS** (all 13/13 jobs passed: `cargo fmt --check`, `cargo test --workspace`, engine tests on Windows, quickstart verifications across Ubuntu, Windows, macOS).
  - `cargo-deny` (run 43610391491): **SUCCESS** (`advisories ok, bans ok, licenses ok, sources ok`).
  - `S3A Research Agent Harness CI` (run 43610391507): **SUCCESS** (both Ubuntu and Windows jobs green).
  - `wasm-dist` (run 43610391517): **SUCCESS** (snapshot build and smoke test green).
- **Local Verification**:
  - `cargo fmt --all --check`: passed
  - `cargo clippy -p s3a-harness -p s3a-sandbox --no-deps --all-targets -- -D warnings`: passed
  - `cargo clippy -p s3a-harness -p s3a-sandbox --features otel --no-deps --all-targets -- -D warnings`: passed
  - `cargo deny check`: passed (`advisories ok, bans ok, licenses ok, sources ok`)
  - `cargo test --workspace`: passed (all 21 crates passed)
  - `cargo test -p s3a-harness -p s3a-sandbox --features otel`: passed (all unit & integration tests including otel_tests and sandbox_otel_tests passed)
  - `cargo test -p s3a-cli --test trace_export_cli_test`: passed
- **Summary of Deliverables**:
1. **Feature Gate & Isolation**:
   - `otel` feature added to `crates/s3a-sandbox` and `crates/s3a-harness` only.
   - Off by default (`default = []`). Core crates remain untouched and completely free of telemetry code.
2. **OTel Span Model & Zero-Dependency OTLP Exporter (`otel.rs`)**:
   - `s3a_sandbox::otel` and `s3a_harness::otel`: lightweight span models emitting traceId, spanId, name, startTimeUnixNano, endTimeUnixNano, status, and attributes.
   - `InMemorySpanExporter`: thread-safe in-memory recorder for deterministic unit/integration testing.
   - `OtlpHttpExporter`: specification-compliant OTLP/HTTP 1.0 JSON exporter to `/v1/traces` over `ureq`.
3. **Span Emission with Provenance Linking**:
   - Tool execution (`tool_call:<tool_name>`), delegation grant check (`grant_check:<tool_name>`), credential proxy request (`proxy_request:<conn>`), and sandbox job execution (`job:<kind>`) emit spans.
   - Every span carries attributes:
     - `agent.id`: executing agent identifier
     - `job.id`: job / turn correlation identifier
     - `s3a.record_hash`: SHA-256 digest matching the underlying S3A signed record
4. **Zero-Secret Invariant**:
   - Injected tokens, headers, API keys, passwords, and secrets are strictly stripped.
   - Spans and signed records only retain cryptographic hashes and non-sensitive metadata.
5. **Signed S3A Records as Source of Truth**:
   - `SignedAuditLog` and `SignedRecord`: canonical Ed25519-signed log where records are hashed and signed before any span emission. OTel traces are purely an exported replica.
6. **CLI Trace Export (`s3a trace export`)**:
   - `s3a trace export --otlp <endpoint> [vault_path]`: re-reads existing records from any S3A vault archive and posts valid OTLP 1.0 JSON trace batches to the remote collector.
7. **Verification & Tests**:
   - `test_every_span_record_hash_matches_signed_log`: verifies all 4 span types match an Ed25519-signed record in `SignedAuditLog`, signatures verify, and zero secrets exist.
   - `test_proxy_egress_span_never_leaks_secrets_or_injected_credentials`: validates bearer token elimination.
   - `test_blocked_proxy_request_span_matches_signed_record`: validates rejected requests produce matching spans and signed audit records.
   - `test_otlp_http_exporter_json_structure`: validates exact OTLP schema.
   - `test_trace_export_otlp_replays_vault_records`: verifies CLI trace export end-to-end against local HTTP receiver.
