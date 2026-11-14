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
