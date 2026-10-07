# S3A Research Agent Harness (`s3a-harness`)

An open-source, publishable research-agent harness and high-performance **Model Context Protocol (MCP)** server for autonomous AI agents, literature synthesis, and cognitive dual-actor provenance.

Built on the **S3A Hyper-Tile** storage architecture and **IEEE 9274.1.1 (xAPI 1.0.3)** profile standards, `s3a-harness` connects AI agent workflows directly to structured scientific memory, empirical claim graphs, and supervisory human-in-the-loop gates.

---

## Features

- **Strict MCP 2024-11-05 & JSON-RPC 2.0**: Protocol version negotiation, zero notification replies, standard error codes (`-32700`, `-32600`, `-32601`, `-32602`), and schema generation derived from Rust structs.
- **Strict Path Sandbox**: All snapshot writes are confined to `<root>/exports/`. Rejects directory traversal (`..`), absolute path escapes, and includes an explicit `--read-only` enforcement mode.
- **Supervisory Human-In-The-Loop Gates**: Built-in `preflight_check`, `request_human_gate`, `record_human_decision`, and `record_ai_step`. Unvetted or low-confidence actions without matching human approval are automatically rejected and logged.
- **LearnMCP xAPI Suite**: Full support for `record_xapi_statement`, `get_session_context`, `list_academic_verbs`, and `get_researcher_timeline`.
- **Pluggable Storage Backends (`Store`)**:
  - **Local**: Zero-copy S3A Hyper-Tiles via `s3a-engine`.
  - **LRS**: Remote HTTP client communicating with `s3a-lrs-server` over xAPI 1.0.3 with Basic authentication.
- **Real Measured Metrics**: Zero hardcoded constants or simulated latencies; all metrics and CRC32C checksums are verified and measured in wall-clock time.

---

## 5-Minute Quickstart

### 1. Build and Install

```bash
# Clone the repository
git clone https://github.com/your-org/s3a-storage.git
cd s3a-storage

# Build the release binary
cargo build --release -p s3a-harness
```

The compiled binary will be located at `target/release/s3a-harness` (or `target/release/s3a-harness.exe` on Windows).

---

### 2. Configure with Claude Desktop

Add `s3a-harness` to your Claude Desktop configuration file:

- **macOS**: `~/Library/Application Support/Claude/claude_desktop_config.json`
- **Windows**: `%APPDATA%\Claude\claude_desktop_config.json`
- **Linux**: `~/.config/Claude/claude_desktop_config.json`

```json
{
  "mcpServers": {
    "s3a-academic-harness": {
      "command": "/absolute/path/to/s3a-storage/target/release/s3a-harness",
      "args": [
        "mcp",
        "--root", "/path/to/my/research/workspace",
        "--db", "scholar_lab.s3a"
      ]
    }
  }
}
```

Restart Claude Desktop. You will see the S3A research tools available in your chat sessions.

---

### 3. Configure with Cursor

Create or edit `.cursor/mcp.json` in your project root:

```json
{
  "mcpServers": {
    "s3a-harness": {
      "command": "/absolute/path/to/s3a-storage/target/release/s3a-harness",
      "args": [
        "mcp",
        "--root", "./research_data",
        "--read-only"
      ]
    }
  }
}
```

---

## Command Line Interface

```
Usage:
  s3a-harness mcp [OPTIONS]             Start Model Context Protocol (MCP) server over stdin/stdout
    --root <dir>                        Sandbox root directory (default: current directory)
    --read-only                         Enforce read-only mode (rejects writes)
    --db <filename>                     Default S3A database filename inside root (default: scholar_lab.s3a)
    --lrs-endpoint <url>                Optional remote LRS HTTP endpoint (e.g. http://127.0.0.1:8080)
    --lrs-user <username>               LRS Basic auth username
    --lrs-pass <password>               LRS Basic auth password
    --session-uuid <hex_uuid>           Current session UUID (32-char hex)
    --project-uuid <hex_uuid>           Current project UUID (32-char hex)
    --agent-id <u64>                    Task agent identifier (default: 42)

  s3a-harness package [OPTIONS] <live_db> <out_snapshot.s3a>
    Package live research database into portable SRS snapshot

  s3a-harness intake <snapshot.s3a>
    Mount and verify portable SRS snapshot

  s3a-harness loop-demo
    Run autonomous inquiry demo with confidence tiers
```

---

## Available MCP Tools

| Tool Name | Description |
|---|---|
| `s3a_search_library` | Spatial 3D Hilbert topic/methodology search across literature corpus. |
| `s3a_get_synthesis_matrix` | Multi-dimensional evidence matrix with live CRC32C verification. |
| `s3a_get_vault_audit_trail` | Dual-actor provenance ledger matching human decisions to AI traces. |
| `s3a_export_srs` | Export Scholar Research Snapshot container to `<root>/exports/`. |
| `s3a_intake_srs` | Zero-copy mmap intake of an SRS snapshot container. |
| `record_xapi_statement` | Validate and write xAPI statement with agent allowlist checks. |
| `get_session_context` | Query session participants, statement counts, and active activities. |
| `list_academic_verbs` | List registered academic and Teacher Assistant verbs. |
| `get_researcher_timeline` | Chronologically ordered timeline of human and AI research actions. |
| `preflight_check` | Detect Type 1-4 hallucination risks before emitting text. |
| `request_human_gate` | Request supervisor clearance (returns pending; agent waits). |
| `record_human_decision` | Record human supervisor approval or rejection of a gate. |
| `record_ai_step` | Record AI step; unapproved gated actions are rejected and logged. |

---

## Security Model

1. **Path Sandbox**: Snapshot outputs are strictly quarantined to `<root>/exports/`. Path components containing `..`, `/`, `\`, or absolute drive roots are unconditionally rejected.
2. **Read-Only Mode**: Running with `--read-only` disables all mutating operations while allowing full search and synthesis querying.
3. **Agent Gate Enforcement**: AI agents configured with `AGENT_FLAG_REQUIRES_HUMAN_GATE` or producing outputs with confidence below `0.70` are blocked from committing steps until an explicit human decision is recorded.

---

## License

Dual-licensed under either:

- Apache License, Version 2.0 ([LICENSE-APACHE](../../LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](../../LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.
