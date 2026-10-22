# Clean Pack (`s3a-pack-clean`)

The Clean Pack cleans incoming data **before** it reaches the S3A store, without ever touching the original. It is a pack under [pack-contract.md](pack-contract.md). It makes no `s3a-core` changes and claims no new tile IDs: it uses the existing generic core tiles `BLOB` (68), `PROVENANCE` (72) and `RECORD` (64), so it needs no `addon_mask` bit. AI suggestions are out of scope for v1. Every change comes from a deterministic rule in a human-maintained rulebook.

## Flow

1. **Raw seal first.** `raw::intake` reads each source file untouched and writes it into a new **sealed raw archive**: the bytes go in `BLOB` tiles, keyed by their SHA-256, and a raw manifest goes in a `PROVENANCE` tile. A sealed archive is never overwritten: the writer refuses an existing path.
2. **Source citation.** Each raw item records `item_sha256` (the SHA-256 of the exact source bytes), `size`, `source_path`, `file_name`, `connector`, and `org` / `user` / `device` / `location` when known, plus `captured_at` (unix seconds). The `snapshot_hash` is the SHA-256 of the canonical item list. `raw::load` rechecks every item's bytes against its fingerprint.
3. **Cleaning.** `clean::clean` reads only from the sealed raw archive and applies the **company rulebook**. It produces a separate **cleaned version** containing:
   - `records`: each record cites `raw_item_sha256`, `raw_snapshot_hash`, `source_path` and `row`.
   - `changes`: each change lists the field, the value before and after, `rule_id` + `rule_version`, `made_by` (`rule-engine`) and `approved_by` (`rulebook:<version>@sha256:<hash>`).
   - `review_queue`: see below.
   - `cleaned_hash` = SHA-256 of the canonical body, Ed25519-signed by the operator key. The hash contains no timestamps or signer, so **the same raw snapshot and the same rulebook always give the same cleaned hash**.
4. **Verify.** `clean::verify` recomputes the hash, checks the signature and checks that every record, change and queue entry cites an item in the raw snapshot. `s3a-cli clean verify --rulebook` also replays the cleaning and compares hashes. Both archives also pass the core `s3a-cli verify`.

## Company rulebook (YAML)

See [`crates/s3a-pack-clean/examples/rulebook.yaml`](../crates/s3a-pack-clean/examples/rulebook.yaml).

- `rulebook_version` is required. `approvers` lists the hex Ed25519 keys of **humans** who may approve updates. `agents` lists AI agent keys, which may never approve one.
- `categories` describe the company's files:
  - `match`: the file extensions, plus an optional `name_prefix` naming convention.
  - `fields`: name, `type` (`string`, `integer`, `decimal`, `date` = YYYY-MM-DD, or `bool`), `required`, and `allowed` values.
  - `rules`: each rule has an `id`, a `version`, an `op` and an optional `field`. Ops are `trim`, `uppercase`, `lowercase`, `map` and `date_iso` (with `formats`).
- Deterministic rules are applied automatically.

### Review queue (nothing is guessed)

| Situation | Tag | Mark |
|---|---|---|
| No category matches the file | `UNCLASSIFIED` | `MARK_NEEDS_REVIEW` |
| Several categories match, an unknown column, an unsupported format, or a malformed row | `NEEDS_RULE` | `MARK_QUARANTINE` |
| A value fails its type or allowed list, a required field is empty, or a date is ambiguous (e.g. `03/04/2026` when both MM/DD and DD/MM are allowed) | `NEEDS_RULE` (per row) | `MARK_QUARANTINE` |

Quarantined rows are left out of the cleaned records. Their raw bytes stay in the raw archive. A human resolves queue entries by updating the rulebook.

### Rulebook updates are human-signed

`s3a-cli clean rulebook-sign old.yaml new.yaml --key human.hex --reason ... --out update.json` produces a `RulebookUpdate` that binds `from_sha256`, `to_sha256`, `to_version` and the reason. `rulebook-verify` accepts it only if:
- the signer is in the **old** rulebook's `approvers`;
- the signer is not in `agents`;
- the version changed;
- the hashes match.

## CLI

```
s3a-cli clean ingest <file>... --rulebook rb.yaml --raw-out raw.s3a --clean-out clean.s3a --key key.hex \
    [--connector file] [--org ..] [--user ..] [--device ..] [--location ..] [--captured-at <unix>]
s3a-cli clean verify raw.s3a clean.s3a [--rulebook rb.yaml]    # exit 0 ok, 2 failed
s3a-cli clean queue clean.s3a
s3a-cli clean rulebook-sign | rulebook-verify ...
```

`key.hex` is a 64-hex-character Ed25519 seed. Keep it out of git.

## Limits (v1)

- Only CSV content is parsed. Other formats that match a category are queued as `NEEDS_RULE`.
- The signature proves which key signed. The pack cannot prove that a human held an approver key; that is enforced by key custody.
- Records are stored as JSON chunked across `RECORD` tiles, not as fixed-size binary records.
- No connector interface beyond local files yet, and no AI suggestions.
