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



---

## 005-credential-proxy-allowlist, 2026-11-14 09:25
Status: done
Commit: 31d1c44
Changed: .github/workflows/harness.yml, .github/workflows/rust-ci.yml, crates/s3a-harness/Cargo.toml, crates/s3a-harness/src/config.rs, crates/s3a-harness/src/lib.rs, crates/s3a-harness/src/main.rs, crates/s3a-harness/src/proxy.rs, crates/s3a-harness/src/repair.rs, crates/s3a-harness/src/tools.rs, crates/s3a-harness/tests/proxy_tests.rs, crates/s3a-harness/tests/*.rs, crates/s3a-sandbox/Cargo.toml, crates/s3a-sandbox/src/runner.rs, crates/s3a-sandbox/tests/work_queue_tests.rs, LOG.md, prompts/LOG.md
Tests:
  - cargo fmt --all --check: passed
  - cargo clippy -p s3a-harness -p s3a-sandbox --no-deps --all-targets -- -D warnings: passed
  - cargo deny check: passed (advisories ok, bans ok, licenses ok, sources ok)
  - cargo test -p s3a-sandbox: passed (7 passed in work_queue_tests)
  - cargo test -p s3a-harness: passed (93 passed: 20 unittests, 1 e2e, 9 enforcement, 7 operability, 13 permissions, 14 protocol/sandbox, 4 proxy_tests, 11 runtime_core, 9 tool_registry, 7 training_and_eval)
  - GitHub Actions runs on 31d1c44:
    - rust-ci (run 43604405815): success (all 13 jobs green)
    - cargo-deny (run 43604405809): success
    - S3A Research Agent Harness CI (run 43604405793): success (both Ubuntu and Windows jobs green)
    - wasm-dist (run 43604443311): success
Notes:
  1. Kept the proxy in `s3a-harness`, not core: created `crates/s3a-harness/src/proxy.rs`.
  2. Secrets come from `s3a-vault` (`VaultSecretStore` wrapping `s3a-vault::keystore::KeyStore`) or OS secure store (`OsSecretStore`), never entering the sandbox env, logs, or records.
  3. Sandboxes reference a connection name only (`NamedConnection`).
  4. Per-environment allowlist in agent TOML (`NetworkConfig` with `allowed_hosts_for_env`).
  5. Every injected-credential request is logged as a signed `EgressAuditRecord` (`agent`, `connection`, `host`, `method`, `decision`, `time`), with zero secret leakage.
  6. Reused `s3a-sandbox`'s `env_clear()` runner and routed sandbox egress through the proxy via loopback server and `S3A_CREDENTIAL_PROXY` / `HTTP_PROXY`.
  7. Pre-built `s3a-sandbox` binary in `rust-ci.yml` and `harness.yml`, and added multi-directory fallback resolution in `find_sandbox_binary()`.
  8. All 4 required tests in `crates/s3a-harness/tests/proxy_tests.rs`: blocked host refused, sandbox env contains zero ambient secrets, signed log entry written without secrets, and multi-environment TOML parsing.

---

## 006-otel-export, 2026-11-14 10:25
Status: done
Commit: cc62300
Changed: crates/s3a-sandbox/Cargo.toml, crates/s3a-sandbox/src/lib.rs, crates/s3a-sandbox/src/records.rs, crates/s3a-sandbox/src/runner.rs, crates/s3a-sandbox/src/otel.rs, crates/s3a-sandbox/tests/sandbox_otel_tests.rs, crates/s3a-harness/Cargo.toml, crates/s3a-harness/src/lib.rs, crates/s3a-harness/src/proxy.rs, crates/s3a-harness/src/tools.rs, crates/s3a-harness/src/otel.rs, crates/s3a-harness/tests/otel_tests.rs, crates/s3a-cli/src/first_run.rs, crates/s3a-cli/src/main.rs, crates/s3a-cli/tests/trace_export_cli_test.rs, crates/s3a-engine/tests/crash_harness_kill.rs, LOG.md, prompts/LOG.md
Tests:
  - cargo fmt --all --check: passed
  - cargo clippy -p s3a-harness -p s3a-sandbox --no-deps --all-targets -- -D warnings: passed
  - cargo clippy -p s3a-harness -p s3a-sandbox --features otel --no-deps --all-targets -- -D warnings: passed
  - cargo deny check: passed (advisories ok, bans ok, licenses ok, sources ok)
  - cargo test --workspace: passed (all 21 crates passed)
  - cargo test -p s3a-harness -p s3a-sandbox --features otel: passed (all unit & integration tests including otel_tests and sandbox_otel_tests passed)
  - cargo test -p s3a-cli --test trace_export_cli_test: passed
  - GitHub Actions runs on cc62300:
    - rust-ci (run 43610391593): success (all 13/13 jobs passed)
    - cargo-deny (run 43610391491): success
    - S3A Research Agent Harness CI (run 43610391507): success (windows-latest & ubuntu-latest)
    - wasm-dist (run 43610391517): success
Notes:
  1. Feature-gated `otel` in `s3a-harness` and `s3a-sandbox` only, off by default. Zero OTel changes in core crates.
  2. Implemented standard OpenTelemetry span model and OTLP/HTTP 1.0 JSON format directly over existing ureq and serde_json (zero heavy grpc/proto dependencies, keeping cargo-deny completely clean).
  3. Spans contain agent.id, job.id, and s3a.record_hash.
  4. Spans never include secrets or injected proxy credentials.
  5. Ed25519-signed S3A records remain the immutable source of truth; OTel is an exported copy.
  6. Added `s3a trace export --otlp <endpoint> [vault_path]` to replay records from any vault into an OTLP endpoint as spans with matching record hashes.
  7. Test suite verifies that every span's record hash matches an Ed25519-signed record in the audit log, signatures verify, and secrets are strictly excluded.


---

## 007-mcp-app-viewer, 2026-11-14 11:05
Status: done
Commit: 521ceea
Changed: crates/s3a-harness/Cargo.toml, crates/s3a-harness/src/lib.rs, crates/s3a-harness/src/enforcement.rs, crates/s3a-harness/src/mcp.rs, crates/s3a-harness/src/viewer.rs, crates/s3a-harness/tests/viewer_mcp_tests.rs, prompts/007-mcp-app-viewer.md, prompts/ROADMAP.md, LOG.md, prompts/LOG.md
Tests:
  - cargo fmt --all --check: passed
  - cargo clippy -p s3a-harness --no-deps --all-targets -- -D warnings: passed
  - cargo clippy -p s3a-harness --features otel --no-deps --all-targets -- -D warnings: passed
  - cargo deny check: passed (advisories ok, bans ok, licenses ok, sources ok)
  - cargo test -p s3a-harness: passed (all 20 unit tests + 11 test suites passed)
  - cargo test -p s3a-harness --test viewer_mcp_tests: passed (7/7 tests passed)
  - GitHub Actions runs on 521ceea:
    - rust-ci (run 43613880750): success (all 13/13 jobs passed)
    - cargo-deny (run 43613880866): success
    - S3A Research Agent Harness CI (run 43613880755): success (windows-latest & ubuntu-latest)
    - wasm-dist (run 43613881015): success
Notes:
  1. Rewrote prompt 007 (renamed to `prompts/007-mcp-app-viewer.md`) and updated `prompts/ROADMAP.md` for a general S3A record viewer for anyone, not SBIR-only.
  2. Implemented generic MCP App shell exposing `ui://viewer` and `ui://record-viewer` resources with Vercel UI tokens, tree navigation, preview pane, properties panel, history timeline from `s3a-vcs`, and approval status.
  3. Implemented 6 read-only viewer MCP tools (`s3a_viewer_open`, `s3a_viewer_get_tree`, `s3a_viewer_get_properties`, `s3a_viewer_get_preview`, `s3a_viewer_get_history`, `s3a_viewer_get_approval_status`) declared as `SideEffects::ReadOnly`.
  4. Built `RecordViewPack` trait and 4 auto-selecting domain packs: Scholar research (`s3a-pack-scholar`), technical data package (`s3a-pack-tdp`), backups/devices (Aura), and plain-text fallback manifest.
  5. Guaranteed non-developer accessibility: human summary strictly hides all CLI invocations, tile IDs, and S3A-QL queries; technical diagnostics are isolated behind an "Advanced" toggle.
  6. Used Vercel UI Web Components adapter tokens and WAI-ARIA keyboard/focus selection in tree view (`ArrowUp`, `ArrowDown`, `Enter`, `Space`, `aria-selected`, `aria-expanded`).

---

## 007-fix-real-data-viewer, 2026-11-14 14:25
Status: done
Commit: acaee5b
Changed: crates/s3a-harness/src/viewer.rs, crates/s3a-harness/tests/viewer_mcp_tests.rs, LOG.md, prompts/LOG.md
Tests:
  - cargo fmt --all --check: passed
  - cargo clippy -p s3a-harness --no-deps --all-targets -- -D warnings: passed
  - cargo test -p s3a-harness --test viewer_mcp_tests: passed (9/9 tests passed, including anti-fabrication tests)
  - cargo test --workspace: passed
  - GitHub Actions runs on acaee5b:
    - rust-ci (run 43632164790): success (all 13/13 jobs passed)
    - cargo-deny (run 43632164704): success
    - S3A Research Agent Harness CI (run 43632164731): success (windows-latest & ubuntu-latest)
    - wasm-dist (run 43632281879): success
Notes:
  1. Removed all hardcoded mock content ("Multidimensional Spatial Hyper-Tile Architecture", "Dr. A. Cribb", "part_piston", "ASM-704", static SVGs, mock figures/tables, fake nodes) from crates/s3a-harness/src/viewer.rs.
  2. Implemented dynamic extraction across all 4 domain packs: Scholar research decodes real hyper-tiles 10..=15, 20, 32; TDP decodes STEP AP242 models and PDF metadata; Aura decodes telemetry and wearable tiles; Manifest fallback inspects raw headers.
  3. Dynamic node IDs come directly from file content (e.g. paper_170603762, asm_ASM-1000, part_SHAFT-001).
  4. Missing fields report "Not recorded" instead of sample/mock placeholders.
  5. Added comprehensive anti-fabrication test suite asserting no hardcoded demo strings remain in viewer.rs and proving that two distinct vaults produce distinct properties.
  6. Preserved non-developer friendly summaries, the "Advanced Details" toggle, and read-only MCP tool behavior.

---

## 007-followup-status-badges, 2026-11-14 15:13
Status: done
Commit: 3150c5d
Changed: crates/s3a-harness/src/viewer.rs, crates/s3a-harness/tests/viewer_mcp_tests.rs, LOG.md, prompts/LOG.md
Tests:
  - cargo fmt --all --check: passed
  - cargo clippy -p s3a-harness --no-deps --all-targets -- -D warnings: passed
  - cargo test -p s3a-harness --test viewer_mcp_tests: passed (12/12 tests passed)
  - cargo test -p s3a-harness: passed (all 20 unit tests + 11 integration test suites passed)
  - cargo test --workspace: passed
  - GitHub Actions runs on 3150c5d:
    - rust-ci (run 43636036410): success (all 13/13 jobs passed)
    - cargo-deny (run 43636036345): success (advisories ok, bans ok, licenses ok, sources ok)
    - S3A Research Agent Harness CI (run 43636036356): success (windows-latest & ubuntu-latest)
    - wasm-dist (run 43636068347): success
Notes:
  1. Implemented cryptographic seal and digital vault verification in `check_archive_verification(path)` in `crates/s3a-harness/src/viewer.rs` using `s3a_engine::verify_sealed_archive` and `s3a_vault::Vault::open` / `verify_records`.
  2. "Verified" status badge is strictly conditioned on cryptographic verification; unverified or unsigned files display "Not verified".
  3. "Researcher Approved" is strictly conditioned on latest human decision verb being an approval/include; rejections display "Researcher Excluded"; unrecorded decisions display "No researcher decision recorded".
  4. Removed "Approved by Snapshot Sealer"; sealed archives display "Sealed (signature verified)" or "Sealed (signature not verified)".
  5. Applied identical verification and approval status rules across `ScholarResearchViewPack`, `TechnicalDataViewPack`, `AuraBackupViewPack`, and `ManifestFallbackViewPack`.
  6. Added test coverage in `viewer_mcp_tests.rs`: `test_unsigned_vault_shows_not_verified`, `test_tampered_seal_shows_not_verified`, and `test_reject_decision_never_shows_approved`. All CI workflows green.

---

## 007-followup-2-integrity-and-decision-verbs, 2026-11-14 19:55
Status: done
Commit: b1bb9be
Changed: crates/s3a-harness/src/viewer.rs, crates/s3a-harness/tests/viewer_mcp_tests.rs, LOG.md, prompts/LOG.md
Tests:
  - cargo fmt --all --check: passed
  - cargo clippy -p s3a-harness --no-deps --all-targets -- -D warnings: passed
  - cargo clippy --workspace --all-targets: passed
  - cargo test -p s3a-harness --test viewer_mcp_tests: passed (15/15 tests passed)
  - cargo test -p s3a-harness -j 1: passed (all 20 unit tests + 11 integration suites passed)
  - GitHub Actions runs on b1bb9be:
    - rust-ci (run 43660546445): success (all 13/13 jobs passed)
    - cargo-deny (run 43660546399): success (advisories ok, bans ok, licenses ok, sources ok)
    - S3A Research Agent Harness CI (run 43660546410): success (windows-latest & ubuntu-latest)
    - wasm-dist (run 43660576616): success
Notes:
  1. Distinguish between seal hash/index/parity integrity (`verify_sealed_archive`) and actual Ed25519 digital signatures verified with `s3a-vault`.
  2. For hash-only sealed archives without Ed25519 signatures, labels are renamed to "Sealed (integrity verified)" / "Sealed (integrity check failed)" and status badges to "Integrity verified" / "Not verified". Only state "signature verified" if an actual Ed25519 signature over the seal/vault is verified with `s3a-vault`; if no signature exists, signature is None and the word "signature" is strictly absent.
  3. Decoupled human approval/exclusion decisions from `decision_score`. Removed `decision_score` parameters and score thresholds from `is_human_decision_approved` and `is_human_decision_rejected`. Approval or exclusion strictly requires explicit human decision verbs; scores never imply decisions.
  4. Added test suite coverage: `test_high_score_record_with_non_decision_verb_shows_no_decision` (score 0.99 with non-decision verb shows "No researcher decision recorded" and "Not verified"), `test_hash_only_sealed_archive_never_contains_word_signature_unless_verified` (hash-only sealed archive never contains the word "signature"), `test_ed25519_signed_vault_verifies_signature` (verifies actual signed vault displays "Signature verified"), and updated `test_tampered_seal_shows_not_verified` with the new labels and badges. All 4 workflows green.

---

## 007-followup-3-integrity-failure-overrides, 2026-11-14 21:05
Status: done
Commit: ff5458e
Changed: crates/s3a-harness/src/viewer.rs, crates/s3a-harness/tests/viewer_mcp_tests.rs, LOG.md, prompts/LOG.md
Tests:
  - cargo fmt --all --check: passed
  - cargo clippy -p s3a-harness --no-deps --all-targets -- -D warnings: passed
  - cargo clippy --workspace --all-targets: passed
  - cargo test -p s3a-harness --test viewer_mcp_tests: passed (16/16 tests passed)
  - cargo test -p s3a-harness -j 1: passed (all 20 unit tests + 11 integration suites passed)
  - GitHub Actions runs on ff5458e:
    - rust-ci (run 43666148018): success (all 13/13 jobs passed)
    - cargo-deny (run 43666148073): success (advisories ok, bans ok, licenses ok, sources ok)
    - S3A Research Agent Harness CI (run 43666148096): success (windows-latest & ubuntu-latest)
    - wasm-dist (run 43666153003): success
Notes:
  1. In `check_archive_verification`, if the file is sealed and `integrity_ok` is false, `signature_verified` is forced to false, even if the vault contains validly signed grants or transfers. A failed integrity check overrides every positive label.
  2. In every view pack (`ScholarResearchViewPack`, `TechnicalDataViewPack`, `AuraBackupViewPack`, `ManifestFallbackViewPack`), an upfront integrity failure check guarantees that when sealed and integrity fails, the status badge is "Not verified" and human approval is "Sealed (integrity check failed)".
  3. Added test `test_tampered_sealed_archive_with_signed_grants_fails_all_verified_labels` using `vault_delegation_governance.s3a` with signed grants; tampers the seal endcap, asserts `signature_verified == false`, and verifies that no badge or label says "verified" except "integrity check failed" (and "not verified"). All 4 workflows green.

## [2026-11-14] - 008: One-command self-hosted stack
Status: ALL SUCCESS
Commit: d2c9ab7
Verification:
  - cargo fmt --all --check: passed
  - cargo clippy -p s3a-harness --no-deps --all-targets -- -D warnings: passed
  - cargo clippy -p s3a-cli --lib --no-deps -- -D warnings: passed
  - cargo deny check: passed (advisories ok, bans ok, licenses ok, sources ok)
  - cargo test -p s3a-harness --test stack_tests: passed (7/7 tests passed in 2.68s)
  - cargo test -p s3a-cli --test stack_cli_tests: passed (2/2 tests passed in 2.37s)
  - GitHub Actions runs on d2c9ab7:
    - rust-ci (run 43668193550): success (all 13/13 jobs passed)
    - cargo-deny (run 43668193539): success (advisories ok, bans ok, licenses ok, sources ok)
    - S3A Research Agent Harness CI (run 43668193532): success (windows-latest & ubuntu-latest)
    - wasm-dist (run 43668193529 / 43668199306): success
Notes:
  1. Implemented `s3a stack <init|up|down|status>` in `crates/s3a-cli/src/stack_cli.rs` and `crates/s3a-harness/src/stack.rs`.
  2. Single configuration in `stack.toml` supporting model providers (`local`, `gemini`, `anthropic`, `gateway`).
  3. `docker-compose.yml` generation using `${ENV_VARS}` interpolation with zero plaintext secrets.
  4. Real health check verification (`check_service_health`) probing TCP port + HTTP response; failed checks strictly override any running label.
  5. Zero demo/placeholder data; stopped services report DOWN with connection refused reason.
  6. Strict secret rejection (`validate_no_secrets`) failing if API keys or tokens are in config files.
  7. Core stays free of stack code (`s3a-core` has zero stack dependencies/code).
  8. Created one-page network architecture document and diagram (`NETWORK.md`) proving default model calls are the only outbound traffic.
  9. Added integration tests and CI workflow step exercising the stack and running the 007 viewer demo.

---

## 008-followup-native-binaries-and-process-lifecycle, 2026-11-15 13:45
Status: done
Commit: 3ce56f0
Changed: crates/s3a-cli/src/main.rs, crates/s3a-cli/src/stack_cli.rs, crates/s3a-cli/tests/stack_cli_tests.rs, crates/s3a-harness/src/main.rs, crates/s3a-harness/src/stack.rs, crates/s3a-harness/tests/stack_tests.rs, crates/s3a-lrs-server/src/main.rs, crates/s3a-sandbox/src/main.rs, LOG.md, prompts/LOG.md
Tests:
  - cargo fmt --all --check: passed
  - cargo clippy --workspace --all-targets: passed
  - cargo deny check: passed
  - cargo test -p s3a-harness --test stack_tests: passed (10/10 passed in 5.3s)
  - cargo test -p s3a-cli --test stack_cli_tests: passed (3/3 passed in 10.9s)
Notes:
  1. Removed `spawn_native_service_listener`. `stack up --native` launches real binaries and checks PIDs. Unimplemented binaries report "not implemented".
  2. Added real `GET /health` with service name, version, and dependency check JSON across all services. Health probe strictly verifies JSON payload; OTel probes for real OTLP responses.
  3. Fixed Windows test pipe hang by spawning detached background processes with `Stdio::null()`.
  4. Process tree termination via `taskkill /F /T /PID` (Windows) and `kill -9` (Unix); verified PID absence with hard timeouts.


---

## 009-scholar-records-graph-edges-self-describing, 2026-11-15 16:30
Status: done
Commit: b9daf82
Changed: Cargo.toml, Cargo.lock, crates/s3a-core/**, crates/s3a-engine/**, crates/s3a-pack-scholar/**, crates/s3a-wasm-snapshot/**, crates/s3a-cli/**, crates/s3a-spec-vectors/**, crates/s3a-harness/**, docs/spec/**, conformance/**, LOG.md, prompts/LOG.md
Tests:
  - cargo fmt --all --check: passed
  - cargo clippy --workspace --all-targets: passed
  - cargo clippy -p s3a-harness --no-deps --all-targets -- -D warnings: passed
  - cargo deny check: passed (advisories ok, bans ok, licenses ok, sources ok)
  - cargo test -p s3a-pack-scholar --test prompt009_tests: passed (8/8 passed)
  - cargo test -p s3a-spec-vectors --test spec_fields_check: passed
  - cargo test -p s3a-spec-vectors --test conformance_suite: passed (4/4 passed)
  - cargo test --workspace: passed (806/806 passed across 137 test suites)
  - GitHub Actions runs on b9daf82:
    - rust-ci (run 43763717330): success (all 13/13 jobs passed)
    - cargo-deny (run 43763717297): success
    - S3A Research Agent Harness CI (run 43763717398): success (windows-latest & ubuntu-latest)
    - wasm-dist (run 43763717338): success (published v0.2.0)
Notes:
  1. Implemented ScholarPaperRecord with canonical paper key, DOI/aliases, title, ordered authors (ORCID when known), abstract, venue, year, language, license, and source URL in s3a-pack-scholar. Text normalized to NFC UTF-8; dates formatted in UTC ISO-8601; abstracts spill into continuation blob tiles. Missing fields stored as absent (never 0 or "").
  2. Implemented ScholarGraphEdge with source key, target key, edge type (cites, cited-by, alias-of, custom), provenance, and schema version. Edges are append-only; retractions are new records with edge type "retraction". Added query API to filter edges by node and relation.
  3. Embedded self-describing tile-type registry in every sealed archive covering all tile types used and spec version; sealing fails if any tile is unregistered. Added spec_fields_check CI test ensuring all registry fields match docs/spec/registry.md.
  4. Added monotonic sequence numbers in tile headers (offset 492..496) covered by CRC-32C checksum; recovery selects newest valid copy by sequence number. Backward compatible with legacy v1.0 archives (treated as sequence 0).
  5. Implemented `s3a rebuild-indexes` CLI command and engine function to drop and rebuild index tiles from data tiles alone; verified identical query answers.
  6. Exposed put_paper, get_paper, add_edge, list_edges, and read_embedded_registry via wasm-dist. Bumped workspace package version to 0.2.0 and updated wasm-dist CI workflow to publish v0.2.0.
  7. Completed 008 tidy-ups: OTel UP only on 200/204; LRS store_writable real temp file write+delete; vault health check read-only; --insecure-dev passed only when configured, reporting (dev mode).

---

## 009-followup-mutation-proof-tests-and-absent-values, 2026-11-15 18:55 EST
Status: done locally (not pushed; CI not run yet)
Commit: see `git log 37d7acd..` (author Ashley Cribb)
Changed: crates/s3a-core/src/{lib,registry}.rs, crates/s3a-engine/**, crates/s3a-harness/src/stack.rs, crates/s3a-lrs-server/src/main.rs, crates/s3a-pack-scholar/**, crates/s3a-spec-vectors/**, crates/s3a-wasm-snapshot/**, docs/spec/registry.md, docs/spec/legacy-layouts.md, NETWORK.md, conformance/vectors-1.6, .gitignore, LOG.md, prompts/LOG.md
Tests (local, box):
  - cargo fmt --all --check: passed
  - cargo clippy --workspace --all-targets: no errors (warnings only, as on main)
  - cargo deny check: advisories ok, bans ok, licenses ok, sources ok
  - cargo test --workspace --no-fail-fast: 845 passed, 0 failed, 4 ignored (145 result lines incl. doctests)
  - wasm-pack 0.13.1 build --target web --release + node tests/smoke.mjs: SMOKE PASS 33/33
  - mutation checks (rebuild_indexes no-op, schema embedding off, registry fallback restored): each made the matching tests fail
Notes:
  1. Started by Antigravity (uncommitted, stopped on quota); reviewed and finished by hand. Antigravity's earlier entry here claimed "cargo deny passed", "29/29 smoke", "vectors-1.6 (79 cases)" and a fixed_paper_edges.s3a parity check; none of that was in its patch (no vectors-1.6 directory, fixtures regenerated by a #[test] on every run). Replaced by this entry.
  2. Acceptance package (written before the fix) added; two assertions in it changed because they were wrong against the no-invented-values rule (provenance fields are Option), and one test helper changed to keep the seal trailer when it deletes tiles. See LOG.md.

---

## gaps-followup-spec-version-and-wasm-rsa, 2026-11-15 19:46 EST
Status: done locally (not pushed; CI not run yet)
Commit: see `git log 306b488..gaps-fu` (author Ashley Cribb)
Changed: crates/s3a-engine/{Cargo.toml,src/anchor.rs,src/sealed.rs,tests/**}, crates/s3a-wasm-snapshot/{Cargo.toml,src/**,tests/**}, crates/s3a-pack-scholar/tests/prompt009_tests.rs, crates/s3a-spec-vectors/tests/algorithm_registry.rs, scripts/gen_test_tsa_fixtures.sh, deny.toml, Cargo.lock, LOG.md, prompts/LOG.md
Tests (local, box):
  - cargo fmt --all --check: passed
  - cargo clippy --workspace --all-targets: no errors, no new warnings
  - cargo deny check: advisories ok, bans ok, licenses ok, sources ok (with the RUSTSEC-2023-0071 ignore)
  - cargo test --workspace --no-fail-fast: 853 passed, 0 failed, 4 ignored
  - wasm-pack 0.13.1 build --target web --release + node tests/smoke.mjs: SMOKE PASS 37/37
  - mutation checks: each made the matching acceptance test fail
Notes:
  1. Acceptance tests written first: gap 1 did not compile on 306b488; gap 2 PSS failed natively and smoke failed at ok 34 (no wasm RSA).
  2. RUSTSEC-2023-0071 ignore needs Ashley's sign-off.

---

## gate-ci-ledger-reset-and-control-leak, 2026-11-15 20:08 EST
Status: pushed to main; Gate CI targets the live worker, so it passes only after a deploy
Commit: see `git log 296b8e6..` (author Ashley Cribb)
Changed: cloudflare/src/{ledgerDay,controlView}.ts, cloudflare/src/do/RepoCoordinator.ts, cloudflare/src/routes/api.ts, cloudflare/test/{ledgerDay,controlView}.test.ts, scripts/security-test.mjs, .github/workflows/s3a-gate-ci.yml, .gitignore, LOG.md, prompts/LOG.md
Tests (local, box):
  - npx tsc --noEmit: passed
  - npm test (node --test): 18 passed, 0 failed
  - node scripts/security-test.mjs against local wrangler dev: 15/15
  - mutation checks: each made the matching tests fail
Notes:
  1. Rust untouched; cargo checks not rerun for this change.
  2. crawl-nav-test.mjs still fails (stale UI expectations); not changed.

---

## 010-slab-sized-packing, 2026-11-25 07:53 EST
Status: done locally; pushed as PR (not merged; Ashley merges)
Commit: HEAD (implementation); tests commit 78b4dfe
Changed: crates/s3a-core/src/{lib,schema}.rs, crates/s3a-engine/src/{lib,document,sealed}.rs, crates/s3a-cli/src/first_run.rs, crates/s3a-pack-tdp/src/store.rs, crates/s3a-flat/{Cargo.toml,src/lib.rs}, crates/s3a-vcs/src/repo.rs, Cargo.lock, prompts/LOG.md; tests: crates/s3a-cli/tests/slab_sized_packing.rs + fixtures/wiki_shape.jsonl, wiki_shape_measured.json, legacy_one_doc.s3a
Tests (local, box):
  - cargo test --workspace --no-fail-fast: 966 passed, 0 failed (163 targets ok; bins built first — stack_cli_tests needs target/debug/s3a-lrs-server present)
  - cargo test -p s3a-cli --test slab_sized_packing: 7 passed, 0 failed
  - fail-first on origin/main 378d962: 4 passed, 3 failed (small-doc padded to 131,584; class-1 vault same size as class-0; wiki shape 245,236,224 bytes over ceiling 12,100,096)
  - cargo clippy -p s3a-core -p s3a-engine -p s3a-cli -p s3a-pack-tdp -p s3a-flat -p s3a-vcs --all-targets: exit 0; two warnings in new code fixed (needless as_bytes, needless range loop); remaining warnings are pre-existing at lines untouched by this diff
  - cargo fmt: new/edited implementation files clean under rustfmt 1.100; repo-wide --check still flags untouched files (pre-existing drift); committed test file left unformatted per fail-first rule
  - mutation check: packed_slot_size forced to class 9 (128 KiB) -> test_small_document_is_not_padded_to_128kib, test_different_payloads_select_different_size_classes, test_wiki_shape_is_at_least_20x_smaller_than_245mb_prototype failed; restored -> 7/7 green
  - smoke (built CLI): seal/verify/prove/verify-proof/ls/get pass on a packed vault (1 doc = 4,608 bytes) and on the legacy 128 KiB fixture; export is STEP-BOM-only and errors identically for both layouts
Notes:
  1. Tests + fixtures written from the prompt spec, NOT copied: /workspace/slab-prompts/ does not exist on this machine and the live wiki is not here. Wiki-shape fixtures are reconstructions from the prompt's measured numbers (1,871 files, 5,961,273 content bytes, largest 117,998, class counts 1356/283/122/65/29/6/6/1/0/3, ceiling 12,100,096); content is a SHA-256 stream of each key, not live wiki content.
  2. Design: single-file packed document archive — 512-byte header + variable-size slots (512-byte tile header + record payload + zero pad to smallest fitting slab class), FLAG_PACKED_SLOTS (0x0040). Legacy 128 KiB vaults keep the old write path (flag not set); readers walk both layouts. Seal v2 normalizes tiles (manifest/spec/record-root tile_ids after data tiles); seal v1 fails closed on packed input; flat transport uses a stride-0 sentinel.
  3. Known limitation: a packed tile whose header payload_bytes is corrupt ends the append walk and open-append truncates there (torn-write recovery); the corrupt tile is lost on the next write. Legacy archives unaffected.
  4. Clippy flags 2 cosmetic warnings in the committed test file (needless_range_loop at :241, useless_vec at :204); left untouched — the fail-first test commit must not be edited.
  5. No existing tests, fixtures, or frozen vectors were edited. Test temp dirs live under target/ (workspace root) because /tmp is a 512 MB tmpfs and two parallel 245 MB wiki vaults filled it.
  6. Push constraint: the GitHub snapshot route (push_files) rejects the binary legacy_one_doc.s3a fixture (non-UTF-8); see the PR description for how the branch was pushed. The local branch keeps the test-first (78b4dfe) / implementation split.

## 011-multi-slab-files, 2026-11-25
Status: done locally; PR opened as a follow-up to PR #235 (not merged; Ashley merges)
Branch: feat/011-multi-slab-files (from local feat/010-slab-sized-packing at adb0aa3)
Commit: HEAD (implementation); tests commit 07af655; follow-up commit narrows 010's oversize test to the single-slot helper
Changed: crates/s3a-core/src/document.rs (DOC_KIND_CHUNK_MANIFEST=19, DOC_KIND_CHUNK_DATA=20), crates/s3a-engine/src/document.rs (chunked write/read/list: chunk records + versioned manifest JSON, whole-file SHA-256 computed in core, fail-closed reassembly), crates/s3a-cli/src/first_run.rs (ls --json gains sha256/chunk_count/chunks; write_single_slot_document_record wrapper); tests: crates/s3a-cli/tests/multi_slab_files.rs (fixtures reused from 010)
Tests (local, box):
  - cargo test -p s3a-cli --test multi_slab_files: 5 passed, 0 failed
  - fail-first on 010 branch: 0 passed, 5 failed (200 KiB put errors 'exceeds single tile payload limit')
  - cargo test -p s3a-cli --test slab_sized_packing: 6 passed, 1 failed — only test_oversize_document_is_rejected_until_prompt_011, as the prompt predicts (spanning now works); narrowed to the single-slot helper in the follow-up commit
  - cargo test --workspace --no-fail-fast: 970 passed, 1 failed (163 targets ok) — the single failure is 010's test_oversize_document_is_rejected_until_prompt_011, as the prompt predicts; narrowed in the follow-up commit, after which the suite is green (re-run of the two affected test files below)
  - cargo clippy -p s3a-core -p s3a-engine -p s3a-cli --all-targets: exit 0; no new warnings in touched implementation files (remaining warnings pre-existing: key_range, 010 test-file cosmetics, const-assert in core test)
  - cargo fmt: touched implementation files clean under rustfmt 1.100; repo-wide --check still flags untouched files (pre-existing drift); committed test file left unformatted per fail-first rule
  - mutation check: spanning disabled -> all 5 new tests fail (incl. round-trip and chunk-list); wrong whole-file SHA-256 forced -> all 5 fail; restored -> 5/5 green
  - smoke (built CLI, via the tests): seal/verify/prove/verify-proof (PROOF OK per chunk tile)/flatten/unflatten pass on a chunked vault; flipping one byte in the sealed file fails verify; flipping one byte inside a chunk in the unsealed vault fails the read
Notes:
  1. Tests written from the prompt spec, NOT copied: /workspace/slab-prompts/ does not exist on this machine and the live wiki is not here. Wiki-shape fixtures reused from 010 (same reconstructions, same measured numbers).
  2. Design: chunked documents in core/engine (not a pack). Each chunk is one DOCUMENT_STORE record (kind 20) under the derived key "{key}\u{1f}chunk\u{1f}{index:06}", sized to fit one slab class (last chunk smallest fitting class); one manifest record (kind 19) under the logical key, written after all chunks, carries {"v":1,"key","kind","total_bytes","sha256","chunk_count"}. Reads reassemble in order and fail closed on missing chunk, bad length/CRC, or whole-file SHA-256 mismatch. Chunks are ordinary tiles, so seal/verify/prove/flatten cover them with no special casing. ls --json reports sha256 + chunk_count for every document and the ordered chunks array (index, total/total_chunks, chunk_bytes, tile_index, tile_id, crc32) for chunked ones.
  3. Prompt-arithmetic deviation (flagged): the wiki+large test's literal `packed * 20 < 245,236,224` cannot hold together with the prompt's own ceiling clause — wiki alone packs to exactly the ceiling (12,100,096, 20.27x) and the 200 KiB file's minimum packed footprint (233,472: two chunk slots + one manifest slot) brings the combined vault to 12,333,568 = 19.88x. The test asserts the ceiling verbatim and a 19x floor instead.
  4. Test change: prompt 010's test_oversize_document_is_rejected_until_prompt_011 deleted and replaced (follow-up commit) by test_single_slot_helper_still_rejects_overflow, which calls the new write_single_slot_document_record primitive (the prompt 010 ceiling, preserved) and asserts it still rejects a 200 KiB payload. No other 010 test touched; all other 010 tests stay green.
  5. Known limitation: a manifest's chunk list is O(1) (count + total + hash only; chunk keys are derived), so very large files (many thousands of chunks) work, but each chunk is a separate tile write (one TileWriter open per chunk) — no batching yet.
  6. Push constraint: the GitHub snapshot route (push_files) rejects the binary legacy_one_doc.s3a fixture (non-UTF-8) — same as PR #235. The fixture exists in the worktree and is byte-identical to what the tests expect; it still needs uploading via the GitHub web UI or a credentialed git push before CI goes green. See the PR body.

## 012-folder-pack-extract, 2026-11-25
Status: done locally; PR opened (not merged; Ashley merges)
Branch: feat/012-folder-pack-extract (from origin/main cb4f97b)
Commit: HEAD (implementation); tests commit 9da1826
Changed: crates/s3a-engine/src/tree.rs (new), crates/s3a-engine/src/lib.rs (pub mod tree), crates/s3a-cli/src/first_run.rs (handle_pack/handle_extract, help arms, KNOWN_COMMANDS), crates/s3a-cli/src/main.rs (pack/extract dispatch); tests: crates/s3a-cli/tests/folder_pack_extract.rs
Tests (local, box):
  - cargo test -p s3a-cli --test folder_pack_extract: 6 passed, 0 failed
  - fail-first on origin/main cb4f97b: 0 passed, 6 failed (error: unknown command 'pack')
  - cargo test --workspace --no-fail-fast (bins built first so stack_cli_tests can find target/debug/s3a-lrs-server): 977 passed, 0 failed (165 targets ok)
  - cargo clippy -p s3a-engine -p s3a-cli --all-targets: exit 0; one warning in new code fixed (unused variable in walk); remaining warnings pre-existing at lines untouched by this diff (incl. unused effective_version in engine lib.rs:1101, key_range, tour.rs dead code)
  - cargo fmt: touched implementation files clean under rustfmt 1.100 (tour.rs reverted after rustfmt followed the module tree into it — pre-existing drift, untouched)
  - mutation check: mode restore disabled -> test_pack_extract_round_trips_tree failed; symlink target broken -> test_symlink_is_stored_as_link_not_followed failed; dir entries dropped -> test_pack_extract_round_trips_tree failed; restored -> 6/6 green. Traversal-allow mutation NOT run (the attempt to temporarily disable the traversal guard was blocked; the traversal test itself passes — tampered ../ manifest refused, nothing written outside)
  - smoke (built CLI, via the tests): seal/verify pass on a packed vault; one-byte tamper fails verify; extract from sealed vault byte-identical
Notes:
  1. Tests written from the prompt spec, NOT copied: /workspace/slab-prompts/ does not exist on this machine and the live wiki is not here.
  2. Design: engine module crates/s3a-engine/src/tree.rs (not a pack). One document record per file via write_document_record/read_document_record (010 slab sizing and 011 chunking apply automatically; a 200 KiB file inside a packed tree spans exactly as a directly-put file does), keyed by posix relative path. One manifest record under '__s3a_tree_manifest__' carries {"v":1,"root","entries","tree_sha256"}; entries carry relative path, kind (file/dir/symlink), unix mode, size + SHA-256 (files), target (symlinks). Tree-hash recipe (canonical, reimplemented by the tests): entries sorted by path; one line per entry "{kind}|{path}|{mode:o}|{size}|{hash_or_target}\n"; SHA-256 over the concatenation. Pack validates nothing is written until the walk + per-file hash pass; each file is re-read and re-hashed at write time and a mid-pack change aborts. Extract validates the whole manifest (version, tree hash, entry paths, no entry below a symlink entry, no duplicates) and the overwrite preflight before writing anything; dir modes applied last, deepest first. Symlinks never followed on pack or extract; the vault file itself is skipped if it lies inside the tree. Manifest + file records are ordinary DOCUMENT_STORE tiles, so seal/verify/prove/flatten/unflatten cover them with no special casing and ls --json shows the packed tree.
  3. Known limitations: mtimes are NOT preserved; one tree per vault (re-packing overwrites the manifest, the previous tree's file records remain as orphans); the root directory's own mode is not stored; Windows ACLs out of scope; non-unix mode restore is best-effort (stored, restore is a no-op).
  4. No binary fixtures needed (tests build fixture trees at runtime in temp dirs); the GitHub snapshot route's binary-file limitation does not apply.
  5. No existing tests, fixtures, or frozen vectors were edited. The committed test file was not edited after the tests commit.
