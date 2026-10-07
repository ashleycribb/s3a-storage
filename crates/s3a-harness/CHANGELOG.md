# Changelog

All notable changes to `s3a-harness` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-10-06

### Added
- Native Model Context Protocol (MCP 2024-11-05) dispatcher using `serde_json` and JSON-RPC 2.0.
- Schema auto-generation (`ToolSchema`) deriving `inputSchema` directly from Rust parameter structs.
- Sandbox architecture (`Sandbox`) with configurable root directory (`--root`) and strict export containment inside `<root>/exports/`.
- Enforcement of `--read-only` flag across all MCP tool calls and store backends.
- Abstract `Store` trait backing LearnMCP tools with two production implementations:
  - `LocalStore`: Direct S3A Hyper-Tile access via `s3a-engine`.
  - `LrsStore`: HTTP xAPI 1.0.3 client supporting Basic authentication to `s3a-lrs-server`.
- LearnMCP tool implementations: `record_xapi_statement`, `get_session_context`, `list_academic_verbs`, and `get_researcher_timeline`.
- Preflight validation of statements against `research_traceability` profile and AI agent allowlist.
- Supervisory Human-in-the-Loop gate tools: `preflight_check`, `request_human_gate`, `record_human_decision`, and `record_ai_step`.
- Gated action tracking: actions requiring human clearance without an approved matching decision are rejected and logged to `GateRejectionLog`.
- Protocol test suite covering malformed JSON, notification silence, unknown methods, escaping, and parameter validation.
- End-to-end integration test spawning an ephemeral `s3a-lrs-server` and verifying statement submission and timeline recovery.
- Runnable example in `examples/research_agent_session.rs`.
- GitHub Actions CI workflow in `.github/workflows/harness.yml`.

### Changed
- Removed per-call `file` and `snapshot_file` arguments from MCP tool schemas in favor of `--root` sandbox confinement.
- Replaced hardcoded UUID constants with real session and project identifiers.
- Removed unverified marketing claims ("Hardware CRC32C Verified", "< 1ms") in favor of true measured execution latencies and live CRC32C computation.
