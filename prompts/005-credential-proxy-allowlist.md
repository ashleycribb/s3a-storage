# 005: Credential proxy and host allowlist

Goal: credentials never enter the sandbox; network is limited per environment.
1. In `s3a-harness`, add an egress proxy that sandbox jobs must use. It injects credentials for named connections (e.g. a read-only PLM adapter, an MCP server) from the vault or an OS secrets store. Sandboxes reference a connection name only.
2. Per-environment allowlist of hosts in the agent TOML; all other hosts are refused and logged.
3. Every injected-credential request is logged as a signed record (agent, connection, host, time), never the secret.
Tests: blocked host refused; sandbox env contains no secrets; log entry written. Clippy, tests, log, commit, push.
