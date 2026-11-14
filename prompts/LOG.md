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
