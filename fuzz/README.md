# S3A fuzzing (cargo-fuzz)

Lives outside `crates/` and outside the main workspace. Fuzzes untrusted-input parsers:
`tile_header` (file/header/tile reader), `sealed_footer` (v2 sealed index), `vault_grant`
(DelegationGrantRecord verify), `edge_reader` (edge flash recovery).

    rustup toolchain install nightly && cargo install cargo-fuzz --locked
    ./fuzz/seed.sh
    cd fuzz && cargo +nightly fuzz run tile_header corpus/tile_header -- -max_total_time=60

Crashes land in `fuzz/artifacts/<target>/`. Fixes to core parsers need human review.
