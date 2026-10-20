#!/usr/bin/env bash
# Builds fuzz corpora from the conformance vectors (spec-vectors/*.s3a are generated, not committed).
set -euo pipefail
cd "$(dirname "$0")"
tmp=$(mktemp -d)
(cd "$tmp" && cargo run -q --manifest-path "$OLDPWD/../Cargo.toml" -p s3a-spec-vectors) || echo "warning: generator exited non-zero; using vectors produced so far"
for t in tile_header sealed_footer vault_grant edge_reader; do
  mkdir -p corpus/$t
  cp "$tmp"/spec-vectors/*.s3a corpus/$t/ 2>/dev/null || true
done
echo "seeded $(ls corpus/tile_header | wc -l) files per target"
