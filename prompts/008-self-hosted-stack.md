# 008: One-command self-hosted stack

Goal: install the whole lab or Aura node in one step.
1. `s3a stack up` (and a docker-compose.yml) starting: vault, harness, sandbox worker(s), LRS server, MCP server, optional OTel collector. Config in one `stack.toml` (model provider: local, Gemini, Anthropic, or a gateway).
2. `s3a stack status` and `s3a stack down`. Works on Linux and Windows.
3. Default model calls are the only outbound traffic; document it in a one-page network diagram.
Tests: a CI job that brings the stack up and runs the 007 demo. Clippy, tests, log, commit, push.
