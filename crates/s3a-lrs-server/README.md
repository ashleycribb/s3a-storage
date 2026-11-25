# s3a-lrs-server — xAPI 1.0.3 Learning Record Store

A fully conformant xAPI 1.0.3 LRS that stores statements directly in native
S3A binary archives — zero SQL, zero DuckDB, zero external database.

## Quickstart

Build it:

```sh
cargo build --release -p s3a-lrs-server
# binary: target/release/s3a-lrs-server
```

Start it with credentials (it refuses to start without them — fail-closed):

```sh
s3a-lrs-server --auth-user alice --auth-pass secret123 --port 8080
```

Defaults: binds `127.0.0.1:8080`, stores data in the current directory in
`lrs_data.s3a` (audit trail in `lrs_data.audit.s3a`). Override with
`--host`, `--port`, `--data-dir`, `--archive`.

Check it is up (no auth needed for this one):

```sh
curl http://127.0.0.1:8080/xapi/about
# {"version":["1.0.3"]}
```

Store a statement (Basic auth + the `X-Experience-API-Version` header are
both required):

```sh
curl -u alice:secret123 http://127.0.0.1:8080/xapi/statements \
  -H "Content-Type: application/json" \
  -H "X-Experience-API-Version: 1.0.3" \
  -d '{"actor":{"mbox":"mailto:alice@example.org"},
       "verb":{"id":"http://adlnet.gov/expapi/verbs/completed"},
       "object":{"id":"http://example.com/activity/2"}}'
# ["2b0a6274-c15e-45bd-a117-2cca4f2e145f"]   <- the stored statement id
```

Read statements back:

```sh
curl -u alice:secret123 "http://127.0.0.1:8080/xapi/statements" \
  -H "X-Experience-API-Version: 1.0.3"
# {"more":"","statements":[{...the statement above, with "stored" timestamp...}]}
```

## Auth behaviour (verified hands-on)

- No credentials at all → the server refuses to start and names the fix:
  `ERROR: Server refused to start: no credentials configured. Pass
  credentials via --auth-user / --auth-pass or --auth-config, or pass
  --insecure-dev.` (exit code 1)
- No credentials on a request → `401 {"error":"Unauthorized"}`.
- Wrong password → `401`.
- Missing `X-Experience-API-Version` header → `400`.
- `--insecure-dev` (local development only) seeds two demo accounts —
  `researcher` / `researcher123` (archive `scholar.s3a`, all scopes) and
  `explorer` / `explorer123` (archive `usage.s3a`, `statements/write`) —
  and permits unauthenticated reads. Never use it on a shared network.

## Credentials

- `--auth-user <user> --auth-pass <pass>` — one account, all scopes.
- `--auth-config <file>` — JSON array of
  `{"user","pass","scopes","archive"}` objects, or plain lines
  `user:pass:scopes:archive` (`scopes` comma-separated, `archive` optional).
- `--auth-cred user:pass:scopes:archive` — one account on the command line.
- A credential with an assigned `archive` always writes to that archive;
  without one it uses the default archive (`--archive`, default
  `lrs_data.s3a`). Archive names must be bare filenames ending in `.s3a`
  (no `/`, `\`, `..`, or absolute paths) and resolve inside `--data-dir`.

## Endpoints

| Path | Notes |
| --- | --- |
| `/xapi/about` | Unauthenticated; returns `{"version":["1.0.3"]}` |
| `/xapi/statements` | POST / GET / PUT; the core statement store |
| `/xapi/activities` | Activity lookup |
| `/xapi/activities/state` | State API |
| `/xapi/activities/profile` | Activity profile API |
| `/xapi/agents/profile` | Agent profile API |

Every statement is also written to the audit archive, and statements in
`usage.s3a` archives are validated against the Scholar Explorer usage
profile (`--strict-profiles` enforces profile validation on all archives).

## Where the data lives

Statements are S3A records in `<data-dir>/lrs_data.s3a` (plus
`lrs_data.audit.s3a` for the audit trail). Inspect them with the CLI:

```sh
s3a-cli status lrs_data.s3a
s3a-cli doctor lrs_data.s3a
```

Daily snapshot sealing/anchoring is available with `--daily-anchor`
(`--anchoring-interval <seconds>`, default 86400).
