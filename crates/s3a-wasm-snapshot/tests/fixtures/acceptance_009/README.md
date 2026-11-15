# acceptance_009 WASM parity fixtures

- `base.s3a`: deterministic empty archive (file header only; fixed UUID `[0x0009_0009_0009_0009, 0x5EED_5EED_5EED_5EED]`, fixed timestamps, ADDON_EDU). Committed with the acceptance tests.
- `paper.json`, `edge.json`: inputs. Committed with the acceptance tests.
- `expected_put_paper.s3a` = native `ScholarPaperRecord::put_to_bytes(base, paper.json)`.
- `expected_add_edge.s3a` = native `ScholarGraphEdge::add_to_bytes(expected_put_paper, edge.json)`.

The two `expected_*` files are generated ONCE from native Rust, after the paper and edge
feature groups (validation, no invented values, edge ids) are final, in their own commit:

    cargo run -p s3a-pack-scholar --example acceptance_009_gen_wasm_fixtures

The generator writes only missing files and refuses to overwrite. After that commit the
fixtures are frozen: never regenerate or edit them to make a test pass. A real format change
needs a new fixture directory and Ashley's approval.

Native check: `cargo test -p s3a-pack-scholar --test acceptance_009_wasm_parity`.
WASM check: the `acceptance_009` block appended to `crates/s3a-wasm-snapshot/tests/smoke.mjs`
compares the built WASM output with the same files, byte for byte.
