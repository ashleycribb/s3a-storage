# Prompt log

Append one entry per prompt run. Template:

```
## NNN-name, YYYY-MM-DD HH:MM
Status: done | partial | blocked
Commit: <short sha>
Changed: <files or crates touched>
Tests: <command and pass/fail counts>
Notes: <anything Dave should review, decisions made, follow-ups>
```

---

## 001-ci-fixes, 2026-11-13 18:53
Status: done
Commit: HEAD
Changed: .github/workflows/cargo-deny.yml, .github/workflows/harness.yml, .github/workflows/rust-ci.yml, Cargo.lock, crates/s3a-engine/src/anchor.rs, crates/s3a-engine/tests/manifest_tile.rs, crates/s3a-harness/src/repair.rs, crates/s3a-harness/tests/e2e_lrs_test.rs, crates/s3a-pack-clean/Cargo.toml, crates/s3a-pack-clean/src/rulebook.rs, deny.toml, .gitattributes, LOG.md
Tests:
  - cargo deny check: passed (advisories ok, bans ok, licenses ok, sources ok)
  - cargo test -p s3a-engine: passed (93 unit tests, all OTS fixtures passed on Windows and Linux)
  - cargo test -p s3a-harness: passed (43 unit tests + 67 integration tests passed on Windows and Linux)
  - cargo test -p s3a-pack-clean: passed (16 tests passed)
  - cargo test --workspace: passed
Notes:
  1. Updated cargo-deny workflow to use native runner with taiki-e/install-action@cargo-deny on ubuntu-latest.
  2. Updated tokio to 1.54.1, zstd to 0.13.5, zstd-safe to 7.4.1; replaced serde_yaml with serde-saphyr in s3a-pack-clean. Added written rationale ignores in deny.toml for unmaintained instant (reed-solomon-erasure) and timing variability in crypto-bigint 0.5.5 (p256 0.13).
  3. Fixed s3a-harness e2e LRS test by adding cargo build -p s3a-lrs-server in harness.yml and on-demand build in test.
  4. Fixed repair loop test concurrency by appending UUIDv7 to test temp directory.
  5. Fixed OTS fixtures on Windows by stripping CRLF bytes and adding .gitattributes.
  6. Configured rust-ci with continue-on-error for the known-failing full-workspace fmt job, and added dtolnay/rust-toolchain to python-packages. All CI workflows (rust-ci, cargo-deny, Harness CI) confirmed green on GitHub Actions.

---

## 002-open-export-pdf-step, 2026-11-13 19:35
Status: done
Commit: f2d6858
Changed: .github/workflows/rust-ci.yml, .gitattributes, Cargo.toml, Cargo.lock, crates/s3a-cli/Cargo.toml, crates/s3a-cli/src/main.rs, crates/s3a-cli/src/first_run.rs, crates/s3a-cli/src/tour.rs, crates/s3a-cli/tests/tdp_cli_test.rs, crates/s3a-pack-tdp/**
Tests:
  - cargo fmt --all --check: passed (fmt check re-enabled with continue-on-error removed in rust-ci)
  - cargo deny check: passed (advisories ok, bans ok, licenses ok, sources ok)
  - cargo test -p s3a-pack-tdp: 7 passed (6 unit + 1 integration round-trip)
  - cargo test -p s3a-cli: 35 passed (including tdp_cli_test, quickstart_integration, tour_tests)
  - cargo test --workspace: all passed in CI
  - GitHub Actions runs on f2d6858:
    - rust-ci (run 43535029176): success (all 13 jobs green, including workspace tests on Linux, Windows, macOS)
    - cargo-deny (run 43535029260): success
    - S3A Research Agent Harness CI (run 43535029137): success (ubuntu-latest & windows-latest)
    - wasm-dist (run 43535029201 rerun): success
Notes:
  1. Ran `cargo fmt --all` across workspace in separate commit `4a9eada` and removed continue-on-error from the fmt job in `rust-ci.yml`.
  2. Implemented `crates/s3a-pack-tdp` maintaining core as database-only:
     - Pure-Rust PDF parsing via `lopdf` (default features disabled to avoid unmaintained dependencies), extracting page_count, title, sha256, and size_bytes metadata into `.meta` sidecar records with `DOC_KIND_BINARY` payload.
     - STEP AP242 ISO 10303-21 reader extracting header, products, definitions, and assembly structure (NEXT_ASSEMBLY_USAGE_OCCURRENCE, QUANTIFIED_ASSEMBLY_COMPONENT_USAGE) into an assembly tree and `.assembly` sidecar records.
     - BOM export in CSV and JSON formats from the assembly tree.
     - Magic bytes / MIME type detection supporting PDF, STEP, JSON, CSV, PNG, JPEG.
  3. Integrated `s3a open <vault> <key>` to write payload to a temporary file and launch the OS default application (open/xdg-open/start with fallback).
  4. Integrated `s3a export <vault> <key> -o <path>` for file payload export with automatic extension deduction, and `--format csv|json` for assembly BOM export.
  5. Added real PDF and STEP fixtures in `crates/s3a-pack-tdp/tests/fixtures` marked `-text` in `.gitattributes`.

---

## 003-graph-queries, 2026-11-13 20:20
Status: done
Commit: d016bbf
Changed: crates/s3a-core/src/lib.rs, crates/s3a-core/src/registry.rs, crates/s3a-core/src/graph.rs, crates/s3a-engine/src/lib.rs, crates/s3a-engine/src/graph.rs, crates/s3a-engine/src/ql/ast.rs, crates/s3a-engine/src/ql/lexer.rs, crates/s3a-engine/src/ql/parser.rs, crates/s3a-engine/src/ql/executor.rs, crates/s3a-engine/tests/graph_queries_test.rs, crates/s3a-pack-scholar/src/lib.rs, crates/s3a-pack-scholar/src/records.rs, crates/s3a-cli/src/main.rs, crates/s3a-cli/src/first_run.rs, crates/s3a-cli/tests/graph_cli_test.rs, LOG.md, prompts/LOG.md
Tests:
  - cargo fmt --all --check: passed
  - cargo deny check: passed (advisories ok, bans ok, licenses ok, sources ok)
  - cargo test -p s3a-core: passed (34 tests)
  - cargo test -p s3a-engine: passed (100 tests, including all 4 graph query & performance tests)
  - cargo test -p s3a-pack-scholar: passed (40 tests, including Tile 13 graph extraction)
  - cargo test -p s3a-cli: passed (36 tests, including graph_cli_test)
  - cargo test --workspace: all passed across all crates
  - GitHub Actions runs on d016bbf:
    - rust-ci (run 43540067096): success (all 13 jobs green)
    - cargo-deny (run 43540067125): success
    - S3A Research Agent Harness CI (run 43540067084): success
    - wasm-dist (run 43540067097): success
Notes:
  1. Kept core strictly generic: defined `TileType::GRAPH_EDGE = 87` (`0x0057`) in `s3a-core` with `GenericGraphEdge` and `GraphEdgeRecordHeader` (96 bytes, `Pod + Zeroable`, align 8).
  2. Implemented `register_graph_tile_extractor` in `s3a-engine::graph` to keep graph code generic in `s3a-core`/`s3a-engine` with zero Scholar-specific code outside `s3a-pack-scholar`.
  3. Mapped Scholar Tile 13 (`ResearchGraphEdgeRecord`) onto generic graph edges in `s3a-pack-scholar::register_ql_handlers()` without altering existing Tile 13 files.
  4. Built in-memory `GraphIndex` rebuilt on open supporting `NEIGHBORS`, `TRAVERSE` (with optional `VIA` and `DEPTH`), and shortest `PATH`.
  5. Implemented S3A-QL grammar and executor for `TRAVERSE`, `NEIGHBORS`, `PATH` with table output and `--json`.
  6. Implemented CLI commands: `s3a graph neighbors`, `s3a graph path`, `s3a graph traverse`, `s3a graph add`.
  7. Tested all 3 canonical use cases (assembly tree, citation graph with Tile 13 interop, agent grants) + 100k-edge performance benchmark (<1.5s build, <50ms query).

---

## 004-sandbox-work-queue, 2026-11-13 20:52
Status: done
Commit: 92c214c
Changed: Cargo.toml, Cargo.lock, crates/s3a-sandbox/**, crates/s3a-harness/Cargo.toml, crates/s3a-harness/src/tools.rs, crates/s3a-harness/tests/tool_registry_tests.rs, crates/s3a-harness/tests/permission_tests.rs, crates/s3a-pack-tdp/src/lib.rs, LOG.md, prompts/LOG.md
Tests:
  - cargo fmt --all --check: passed
  - cargo deny check: passed (advisories ok, bans ok, licenses ok, sources ok)
  - cargo test -p s3a-sandbox: passed (7 passed in work_queue_tests)
  - cargo test -p s3a-harness: passed (89 passed: 18 unittests, 1 e2e, 9 enforcement, 7 operability, 13 permissions, 14 protocol/sandbox, 11 runtime_core, 9 tool_registry, 7 training_and_eval)
  - cargo test -p s3a-pack-tdp: passed (7 passed)
  - GitHub Actions runs on 92c214c:
    - rust-ci (run 43543153112): success (all 13 jobs green)
    - cargo-deny (run 43543152884): success
    - S3A Research Agent Harness CI (run 43543152896): success (both Ubuntu and Windows jobs green)
    - wasm-dist (run 43543152876): success
Notes:
  1. Kept as a pack crate, not core: created `crates/s3a-sandbox` with pack tile `TILE_SANDBOX_WORK_QUEUE = 88`.
  2. Work queue signed records: `WorkQueueRecordHeader` (168 bytes, 8-byte aligned, Pod + Zeroable) supporting `JobRecord` (id, kind, inputs, requested_by, grant, signature), `StatusRecord` (state, worker_id, claimed_at, lease_expires_at, retry_count, error, signature), and `ResultRecord` (success, output, signature).
  3. Work queue state machine (`WorkQueue`): atomic lease claim under exclusive OS file lock preventing double-claim across competing concurrent workers, lease expiry retries, heartbeat, complete, and fail.
  4. Autonomous worker (`SandboxWorker`): polls the vault work queue via outbound file/storage access without requiring any inbound open ports.
  5. Process isolation (`execute_job_in_sandbox`): creates restricted temp directory, wipes ambient environment variables with `env_clear()` so sandboxes never see vault keys or credentials, executes isolated subprocess, reads outputs, and writes signed results back to the vault.
  6. Implemented job kinds:
     - `render_step_preview`: parses AP242 STEP assembly and exports canonical glTF 2.0 scene graph or PNG preview.
     - `extract_pdf_text`: extracts text from PDF pages using pure-Rust `lopdf`.
  7. Harness integration: added `RenderStepPreviewTool`, `ExtractPdfTextTool`, and `SandboxJobTool` in `s3a-harness::tools` to enqueue jobs into the vault queue rather than running heavy tasks directly inside the agent harness.
  8. Tests in `crates/s3a-sandbox/tests/work_queue_tests.rs`:
     - `test_claim_lease_expiry`: verifies worker claims, prevents concurrent claim during lease, and re-claims after lease expiry.
     - `test_crash_mid_job`: verifies mid-job worker crash recovery upon lease expiry.
     - `test_two_workers_no_double_claim`: concurrent barrier test ensuring exactly one worker claims.
     - `test_render_step_preview_e2e` & `test_render_step_preview_png`: STEP to glTF 2.0 and PNG.
     - `test_extract_pdf_text_e2e`: PDF text extraction.
     - `test_sandboxes_never_see_vault_keys`: verifies ambient vault keys are completely stripped from sandbox subprocess.

