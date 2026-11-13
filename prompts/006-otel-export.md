# 006: OpenTelemetry export

Goal: every agent step shows up in standard monitoring tools.
1. Add an optional OTLP exporter (feature `otel`) to `s3a-harness` and `s3a-sandbox`: each tool call, job, grant check and proxy request becomes a span with agent identity, job id, and the S3A record hash so a span can be verified against the signed log.
2. `s3a trace export --otlp <endpoint>` replays an existing vault log.
3. Signed S3A records stay the source of truth; OTel is a copy.
Tests with an in-memory exporter. Clippy, tests, log, commit, push.
