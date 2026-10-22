# S3A Hyper-Tile conformance suite (`vectors-1.0`, `vectors-1.1`)

**`vectors-1.1`** (62 cases) is a superset of `vectors-1.0`. It adds ADR-0013:
- **Reserved bytes:** `reserved_tile_header_nonzero` and `reserved_file_header_padding_nonzero` must be **ignored** (outcome `ok`).
- **Layout-affecting header fields:** `header_unknown_flag_bit`, `header_invalid_tile_stride` and `header_unknown_flag_newest_slot_only` must be **rejected**, with no fallback to the older slot.
- **Unknown tile rewrite:** the writer-level `kind: "rewrite"` case `unknown_tile_rewrite`. Your implementation gets `<impl> rewrite <in> <out>` (your compaction or rewrite path). Expectations such as `tile_payload_sha256.9999` are checked on the **output**; dotted keys address nested report objects. Add `tile_payload_sha256: {"<type>": [sha256 hex of each payload]}` to your report.

Run 1.1 with `--level writer` to include the rewrite case.

A set of test vectors that **any** implementation can run, in any language. Each case is a file in `vectors-1.0/files/` (or pure hash data for Merkle cases) plus an `expect` object in `vectors-1.0/manifest.json`. The Rust reference reader is checked against the same suite in CI (`cargo test -p s3a-spec-vectors`, test `conformance_suite`).

The suite is versioned. `vectors-1.0` is frozen: cases may be added only in a new version directory (`vectors-1.1`, …), and an existing case's file or `expect` never changes inside a version.

## What's in it (56 cases)

| Group | Cases | Covers |
|---|---|---|
| Core valid / invalid | 13 | empty, single and multi tile, compaction, dual header slots, snapshot container, consent-withdrawn mark hidden, torn-write fallback to the older slot, bad magic, both slots corrupt, tile CRC mismatch, truncated tile, unsupported `record_layout_version` |
| Unknown tile | 1 | tile type 9999 must open and pass CRC checks (`valid_unknown_tile_type`) |
| Schema catalog | 1 | Schema tile 77 present (`valid_schema_catalog`) |
| Sealed (v1.1) | 9 | sealed ok, footer missing, end cap corrupt, truncated mid-tile, 1 tile repaired by Reed-Solomon, 3 tiles unrepairable, out-of-order tiles, foreign `file_uuid`, legacy unaddressed tiles |
| Legacy | 2 | unmodified v1.0 file (must be **rejected**, checksum) and the same file after `s3a-cli migrate` (must be ok, same tile types) |
| Record-root tile 83 | 3 | roots ok, roots absent, and a CRC-consistent record tamper (`record_roots: "mismatch"`, outcome `error`) |
| Edge / vault profiles | 7 | 4 KB edge tiles, truncated edge file, signed seal chain, vault files (reader level only checks they open and verify) |
| Merkle proofs (RFC 6962) | 20 | valid audit paths for tree sizes 1–8, tampered leaf, wrong index, index ≥ size, Bitcoin CVE-2012-2459 duplicate-leaf |

## The reader report

For an archive case, your implementation reads the file and prints **one JSON object**:

| Key | Type | Meaning |
|---|---|---|
| `outcome` | string | `ok`, `ok_with_warning` (sealed, verified, but the directory had to be rebuilt / tiles out of order), `repaired` (sealed, Reed-Solomon fixed tiles in memory), or `error` (must not be trusted) |
| `tile_count` | int | tiles in the active header |
| `generation` | int | active header generation |
| `active_slot` | int | 0 = slot A, 1 = slot B, after arbitration/fallback |
| `is_sealed` | bool | a valid sealed footer was found |
| `repaired_tiles` | int | tiles repaired (sealed only) |
| `tile_types` | object | `{"<tile_type decimal>": <number of tiles>}` |
| `record_roots` | string | `ok`, `mismatch` or `absent` (tile 83) |
| `error_class` | string | optional; e.g. `UnsupportedLayoutVersion` |

For a `merkle-proof` case (fields `leaf_hex`, `index`, `tree_size`, `proof_hex`, `root_hex`), print `{"valid": true|false}` per RFC 6962 §2.1.1 (leaf hash = SHA-256(0x00‖leaf), node = SHA-256(0x01‖l‖r)).

A case **passes** when every key in `expect` equals the same key in your report. Extra keys are ignored, so a reader can start with just `outcome` and add keys over time. Archives the reader cannot open still print a report, with `"outcome":"error"`.

## Conformance levels

- **Reader:** passes every case with `"level": "reader"` (all 56 in 1.0).
- **Writer:** Reader level, plus every file your writer produces (unsealed and sealed) gives `outcome: ok` under the reference reader (`s3a-conformance read <file>`). It must also never modify an input file.

## Running it against your implementation

Give your implementation a command line with two subcommands:

```
<impl> read <file.s3a>      # prints the reader report JSON
<impl> merkle               # reads one case JSON on stdin, prints {"valid": bool}
```

Then run the stdlib-only runner (Python ≥ 3.8):

```
python3 conformance/run.py --impl "<impl>" conformance/vectors-1.0
# vectors-1.0: 56 passed, 0 failed     (exit 0; exit 1 lists FAIL lines)
```

The runner checks each vector file's `sha256` from the manifest first, so a corrupted checkout shows up as a failure rather than a false pass.

Reference implementation:

```
cargo build -p s3a-spec-vectors --bin s3a-conformance
python3 conformance/run.py --impl target/debug/s3a-conformance conformance/vectors-1.0
target/debug/s3a-conformance run conformance/vectors-1.0     # same check, in Rust
```

## Regenerating (maintainers only)

`s3a-conformance gen conformance/vectors-1.0` rebuilds the files and the manifest. Every `expect` is taken from the spec-vectors `expected.json` (or written explicitly in the generator), and generation fails if the reference reader disagrees. Files contain random UUIDs, so their bytes and sha256 change on every regeneration. CI checks that regenerated *ids and expectations* still match the committed manifest. Only regenerate when starting a new suite version.

## Notes for implementers

- `sealed_footer_missing` and `sealed_endcap_corrupt` are, byte for byte, ordinary unsealed archives. They expect `outcome: ok, is_sealed: false`, because a reader can't tell "never sealed" from "seal lost".
- `invalid_torn_write_fallback`: the newest header slot is corrupt. Expect `outcome: ok` with `active_slot`/`generation` from the older slot.
- `legacy_v1_0_telemetry`: v1.0 tile header CRCs cover bytes 0..36 only. Current readers reject it; migration is a separate, non-destructive step.
