# Roadmap: all green

Goal: a self-hosted version of the Claude Managed Agents topology (client apps, agent server, sandboxes pulling a work queue, state store, MCP tools, observability, model calls) with S3A as the signed state store. The DAF SBIR technical-data viewer is the demo that exercises every piece.

| # | Item | Was | Prompt |
|---|------|-----|--------|
| 3 | Graph as shared memory | yellow | 003 |
| 5 | One-command self-hosted stack | yellow | 008 |
| 6 | Sandboxes pulling a work queue | red | 004 |
| 7 | Credential proxy + host allowlist | red | 005 |
| 8 | MCP App (TDP viewer) | red | 007 |
| 9 | OpenTelemetry export | red | 006 |
| 10 | Reproducible research records | yellow | 009 |

002 (open/export PDF + STEP) is the prerequisite for the viewer.
Run in order. Each prompt must keep S3A core database-only (domain logic in packs), no money or crypto, and the rule: agents suggest, humans decide.
