# s3a_reader: independent Python reader

A second S3A reader in pure Python 3 stdlib (no Rust, no third-party packages). It was written from the spec text only (`docs/spec/S3A-TILE-SPEC.md`, `docs/spec/S3A-EDGE-PROFILE.md`, `conformance/README.md`), without porting the Rust code. Its job is to show that the spec is enough to build a reader, and to find the places where it wasn't.

```
PYTHONPATH=readers/python python3 -m s3a_reader read file.s3a     # reader report JSON
PYTHONPATH=readers/python python3 -m s3a_reader merkle < case.json
PYTHONPATH=readers/python python3 conformance/run.py --impl "python3 -m s3a_reader" --level reader conformance/vectors-1.1
```

Status: vectors-1.0 56/56 and vectors-1.1 61/61 reader-level cases pass, including Reed-Solomon repair. CI runs both (job `python-reader`).

Scope: it reads and verifies, and never writes. It does not cover sealed footer v1 (§10.7), the 32-byte ultra-compact edge header (EDGE §2.4.2), anchors, or the vault/edge signature chains. The conformance suite only requires vault and edge files to open and pass their CRCs.

## Spec gaps found and fixed

| # | Gap | Fix |
|---|---|---|
| 1 | `FileHeader.flags`: the known-bits list (ADR-0013, TILE-SPEC §2.1) omitted `0x0020` ED25519_SIGNED, which EDGE §2.2.2 defines. A signed edge archive would have been rejected. | Added `0x0020` to the TILE-SPEC row and ADR-0013. The reference `KNOWN_HEADER_FLAGS` now includes `FLAG_ED25519_SIGNED`. |
| 2 | Parity coding was defined only as "uses the `reed-solomon-erasure` crate": no field, matrix, shard size, short last group or parity offset. | Added TILE-SPEC §10.9 rules 3–5. |
| 3 | The seal-hash formula gave no field widths. | §10.9 rule 1. |
| 4 | `payload_hash` scope (active payload vs whole tile) was not stated. | §10.9 rule 2. |
| 5 | Damage criteria, and the outcomes for repair, out-of-order tiles, and bad end cap vs bad index hash, were not stated. | §10.9 rules 6–7. |
| 6 | §12 said to reject versions above the reader's `current_layout_version`, but no value was given. | §12.2: the current version is 1 for every type. |
| 7 | Compact header extent "P (from tile_stride or 256)" was ambiguous. Edge detection by stride was unstated. | EDGE §2.3.2: `P = max(256, tile_stride)`. Edge means flag or stride 4096/8192. |
| 8 | When a tampered sealed file also has a record-root tile, which check gets reported was not stated. | §10.9 rule 8. |
