# Security Policy

## Reporting a vulnerability

Please **do not** open a public issue for security problems.

Report privately via GitHub Security Advisories:
https://github.com/ashleycribb/s3a-storage/security/advisories/new

Include affected crate(s)/files, version or commit, reproduction steps, and impact.
You should receive an acknowledgement within 7 days. Please allow time for a fix
before public disclosure.

## Scope

- Rust crates under `crates/` (file format, engine, vault, edge profile)
- Spec and conformance vectors (`docs/spec/`, `spec-vectors/`)
- CI workflows under `.github/`

## Supported versions

Only the latest `main` is supported during the 0.x series.
