# 001: Fix remaining CI on main

No behavior changes beyond item 1. Don't retag; we'll cut rc2 after Dave reviews.

1. Add `--version` / `-V` to the `s3a` binary, printing `s3a <CARGO_PKG_VERSION>`, with a test.
2. Harness CI: `crates/s3a-harness/tests/e2e_lrs_test.rs:89` expects `target/debug/s3a-lrs-server`, which the workflow never builds. Fix by making the test use `env!("CARGO_BIN_EXE_s3a-lrs-server")` (add the server crate as a dev-dependency if needed) or by building that binary in the workflow before tests.
3. Windows-only failures in `crates/s3a-engine/src/anchor.rs` (~line 1915; `test_ots_forged_bitcoin_attestation_fails_verification`, `test_offline_confirmed_ots_fixture_against_bitcoin_header`, `test_offline_pending_ots_fixture`). Likely CRLF conversion of fixture files. Add `.gitattributes` entries marking the fixtures binary / `-text`, or load them with `include_bytes!`.
4. cargo-deny advisories: `cargo update -p tokio` to a patched release (Interval race on 1.54.0); update or replace the crates behind the constant-time and "abandoned stream leaks into next stream" advisories; for unmaintained `instant` and `unsafe-libyaml`, update parent crates, or add a `deny.toml` ignore with a written reason if no fix exists.

Verify:
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`

Then append your entry to `prompts/LOG.md`, commit, and push `main`.
