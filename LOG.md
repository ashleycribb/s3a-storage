# CI Fixes & Verification Log

## Run Details
- **Commit**: `706cfc0` (`fix(ci): resolve cargo-deny advisories, fix harness e2e and repair tests, fix windows OTS fixtures`)
- **Status**: Verifying GitHub Actions runs (`rust-ci`, `cargo-deny`, `S3A Research Agent Harness CI`)

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
- **Fix**: Added `cargo build -p s3a-lrs-server` step to `.github/workflows/harness.yml`, and added fallback compilation in `find_server_exe()`.

### 3. Repair Loop Auto-Repair & Trajectory Export (`crates/s3a-harness/src/repair.rs:576`)
- **Issue**: `test_repair_loop_auto_repairs_and_promotes_with_trajectory_export` failed because concurrent unit tests in `repair.rs` used the same PID-based temp directory name (`s3a_repair_test_<pid>`), causing concurrent tests to wipe directory contents out from underneath each other.
- **Fix**: Appended a unique UUIDv7 (`format!("s3a_repair_test_{}_{}", std::process::id(), unique)`) to ensure complete test isolation. Also ran `cargo fmt -p s3a-harness`.

### 4. Windows-Only OTS Fixture & Manifest Failures (`crates/s3a-engine`)
- **Issue 1 (CRLF in Fixtures)**: `include_bytes!("../tests/fixtures/merkle2.txt")` on Windows checkout contained CRLF `\r\n` line endings, causing SHA-256 fingerprint mismatch (`806c...` instead of `8bd5...`).
  - **Fix**: Added `get_fixture_merkle2_txt_bytes()` in `crates/s3a-engine/src/anchor.rs` to strip `\r` bytes, ensuring uniform hash across all platforms. Added `.gitattributes` setting `* text=auto eol=lf` and binary rule for fixture files.
- **Issue 2 (Substring Collision in Manifest Test)**: `manifest_tile.rs:224` asserted `!manifest.contains("9999")`, which occasionally collided with random hex hash substrings.
  - **Fix**: Updated secret study ID to a non-colliding `999_888_777_666u64`.
