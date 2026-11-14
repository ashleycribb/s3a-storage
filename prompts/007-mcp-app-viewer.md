# 007: General S3A MCP App Record Viewer

Goal: A general S3A record viewer MCP App (`ui://` resource and read-only tools) that renders inside any MCP client (Claude, Grok, Aura agent) for any user, not SBIR-only.

1. **Generic Shell (`ui://viewer`)**:
   - MCP Apps extension resource `ui://viewer` serving an interactive HTML panel.
   - Core layout components:
     - Record tree / hierarchical list.
     - Properties panel (metadata, record properties, attributes).
     - Document preview (PDF, images/SVG/renders, formatted text).
     - Fingerprint / signature verification (SHA-256 digest, Ed25519 signature) and human approval status.
     - Version control history from `s3a-vcs` (commits, branches, author, timeline).
   - Read-only tools backing the UI and agent queries: `s3a_viewer_open`, `s3a_viewer_get_tree`, `s3a_viewer_get_properties`, `s3a_viewer_get_preview`, `s3a_viewer_get_history`, `s3a_viewer_get_status`.

2. **Domain View Packs**:
   - Views register with the shell and are automatically activated by the tile types detected in the archive:
     - **Research View (`s3a-pack-scholar`)**: paper metadata, sections, figures, tables, references, citation counts, open-access status, provenance, and completeness.
     - **Technical Data View (`s3a-pack-tdp`)**: AP242 STEP assembly tree, part properties, linked 2D/3D drawings, Bill of Materials (BOM).
     - **Backups & Devices View (Aura)**: snapshots, filesystem trees, device telemetry events, status logs.
     - **Fallback View**: unknown or unmapped tiles fall back safely to a plain-text manifest summary.

3. **Non-Developer Usability**:
   - Opening an `.s3a` file displays a clean, human-readable summary by default.
   - Zero CLI commands, zero tile IDs (e.g. `TILE_TYPE=23`), and zero S3A-QL syntax visible to standard users.
   - All developer details (hardware CRC32C checksums, raw coordinate addresses, Merkle roots, raw hex hashes, tile numbers) are tucked behind an "Advanced" toggle.

4. **UI SDK & Keyboard Interaction**:
   - Uses `vercel-labs/ui` (UI SDK, Web Components adapter, pinned beta version) for accessible keyboard navigation (`ArrowUp`/`ArrowDown`/`ArrowLeft`/`ArrowRight`/`Enter`/`Space`), focus management, and selection states in trees and lists.
   - Strictly read-only; no mutating actions or write operations.

5. **Testing & Verification**:
   - Unit and integration tests per view (Research, Technical Data, Backups & Devices, Fallback manifest) with sample files.
   - Protocol tests for `resources/list`, `resources/read`, and viewer tools.
   - Non-developer test ensuring no developer/CLI leakage unless the Advanced toggle is active.
   - CI verification: `cargo fmt --all --check`, `clippy -D warnings`, `cargo test --workspace`, pushed and confirmed green on GitHub Actions (`rust-ci`, `cargo-deny`, `Harness CI`, `wasm-dist`).
