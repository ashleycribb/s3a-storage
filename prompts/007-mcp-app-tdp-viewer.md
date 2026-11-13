# 007: First MCP App: technical-data viewer

Goal: the DAF SBIR TDP viewer renders inside any MCP client (Claude, Grok, Aura agent).
1. New crate `s3a-mcp` (or extend the harness MCP server): read-only tools `open_assembly`, `get_part`, `get_bom`, `open_drawing`, `search_parts`, backed by `s3a-pack-tdp` and graph queries.
2. Implement the MCP Apps extension: a `ui://tdp-viewer` HTML resource (assembly tree, part properties, linked drawings and PDFs, BOM, plus SHA-256 fingerprint, approval status and record status). Check the current MCP Apps spec before coding and cite the version in the README.
3. Previews come from sandbox jobs (004); any PLM access goes through the credential proxy (005); every view is a signed record and an OTel span (006).
4. Read-only toward the customer PLM. Approvals are human actions only.
Tests: tool schemas, a resource fetch, and an end-to-end demo with a sample assembly. Clippy, tests, log, commit, push.
