# 002: s3a-cli open / export with PDF and STEP

Goal: tiles show real documents and tables instead of hex.
1. `s3a open <vault> <key>` writes the stored payload to a temp file and opens it with the OS default app; `s3a export <vault> <key> -o <path>` writes it out. Detect type from stored MIME or magic bytes.
2. PDF: store as BINARY document records with a sidecar metadata record (page count, title, SHA-256). Use a pure-Rust reader (lopdf) for metadata; pdfium optional behind a feature flag.
3. STEP (AP242): parse the header and product/assembly structure with an open reader into an assembly tree record (part number, name, parent, quantity). No reverse engineering of closed CAD; native Creo arrives as STEP exports.
4. BOM: `s3a export --format csv|json` for an assembly tree.
5. Put PDF/STEP handling in a new pack crate `s3a-pack-tdp`, not core.
Tests: round-trip a sample PDF and a small STEP fixture (mark fixtures `-text` in .gitattributes). Run clippy -D warnings and cargo test --workspace. Log, commit, push.
