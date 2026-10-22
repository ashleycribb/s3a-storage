# S3A for newcomers

This page is short on purpose. It links to the normative documents instead of repeating them. Every command below was run on this repository, and its output is pasted unedited, except where `…` marks a cut.

## What S3A is

An S3A archive is a single file:
- a 512-byte **file header** with two 64-byte header slots (A/B), so a crash during a commit leaves the older slot valid;
- then fixed-size **Hyper-Tiles** of 128 KiB (131,072 bytes). Each tile has a 512-byte header (type, record count, CRC32C checksums, generation, file UUID) and up to 130,560 payload bytes.

The tile **type** says what the payload holds, for example telemetry, embeddings, schema or manifest. A **sealed** file also gets a v2 footer at the end: a directory of tiles with SHA-256 hashes, a seal hash chain, and optional Reed-Solomon parity.

Normative sources:
- [`docs/spec/S3A-TILE-SPEC.md`](spec/S3A-TILE-SPEC.md): §2 file header, §3 tile header, §10 sealed format, §14 tile catalog.
- [`docs/spec/S3A-EDGE-PROFILE.md`](spec/S3A-EDGE-PROFILE.md): 4 KB / 8 KB micro-tiles for microcontrollers.
- [`docs/spec/registry.md`](spec/registry.md): tile type IDs.
- Decisions are recorded as ADRs in [`docs/adr/`](adr/).
- Terms are in the [glossary](GLOSSARY.md).

## 5-minute quickstart

You need a Rust toolchain (the workspace uses edition 2021) and Python 3.8 or newer. The transcript below used Python 3.13.5. Build the CLI once from the repo root:

```
cargo build -p s3a-cli -p s3a-spec-vectors
```

The CLI has no "create an empty archive" command yet, so this quickstart starts from a small committed test file. It is the conformance vector `valid_multi_tile`, which has 3 tiles: telemetry, embedding_128 and statement_index.

**1. Copy the sample file and check its header**

```
$ mkdir /tmp/s3a-quickstart
$ cp conformance/vectors-1.1/files/valid_multi_tile.s3a /tmp/s3a-quickstart/hello.s3a
$ ./target/debug/s3a-cli status /tmp/s3a-quickstart/hello.s3a
=========================================================================
                     S3A ARCHIVE STATUS REPORT                           
=========================================================================
Archive:                /tmp/s3a-quickstart/hello.s3a
File Size:              393728 bytes (384.50 KB)
Storage Version:        1
Committed Generation:   4
Active Metadata Slot:   Slot B
Header CRC32:           0xC2DB3BD3
Committed Tiles:        3
Capability Addon Mask:  0x0000 (none (pure general db))
=========================================================================
[exit 0]
```

**2. Seal it.** The input is never modified. `--about` records which spec and tool wrote the file (TILE-SPEC §14.5.4). `--record-roots` adds per-tile Merkle roots (tile 83, ADR-0012).

```
$ ./target/debug/s3a-cli seal /tmp/s3a-quickstart/hello.s3a /tmp/s3a-quickstart/hello.sealed.s3a --about --record-roots
Sealed S3A archive created successfully at '/tmp/s3a-quickstart/hello.sealed.s3a' (UTF-8 Plain-Text Manifest, self-describing about entry, Record-Root Tile (RFC 6962))
[exit 0]
```

**3. Verify it.** Exit codes are 0 ok, 2 failed, 3 record-root mismatch, 4 anchor failed.

```
$ ./target/debug/s3a-cli verify /tmp/s3a-quickstart/hello.sealed.s3a
Sealed verification SUCCESS: 6 tiles verified in '/tmp/s3a-quickstart/hello.sealed.s3a'
Record roots: OK (3 tiles)
[exit 0]
```

**4. Tour it.** Sealing put a plain-text manifest at floor 0, a schema tile, and the record-root tile into the file.

```
$ ./target/debug/s3a-cli tour /tmp/s3a-quickstart/hello.sealed.s3a
============================================================
 S3A ARCHIVE TOUR: /tmp/s3a-quickstart/hello.sealed.s3a
============================================================
 File Version:    2
 Footer Version:  v2
 Total Floors:    6
 Seal Status:     verified ✓ (Chain: ✓, Anchors: none)
 Consent Summary: open / unrestricted (no consent records)
 Schema Registry: 6 registered types
   - Type 0: telemetry
   - Type 1: embedding_128
   - Type 22: statement_index
   - Type 77: schema
   - Type 78: manifest
   - Type 83: record_root

--- PLAIN-TEXT MANIFEST ---
… (the manifest text, shown in step 5) …

--- FLOOR DIRECTORY ---
Floor  0: MANIFEST         (type 78, v1) | 0    recs | 131072 B | keys: none | hash: bf08c05a...
Floor  1: TELEMETRY        (type 0 , v1) | 2    recs | 131072 B | keys: none | hash: c3a8fe3a...
Floor  2: EMBEDDING_128    (type 1 , v1) | 2    recs | 131072 B | keys: none | hash: 30c6e5da...
Floor  3: STATEMENT_INDEX  (type 22, v1) | 2    recs | 131072 B | keys: none | hash: e9e12251...
Floor  4: SCHEMA           (type 77, v1) | 0    recs | 131072 B | keys: none | hash: 0479aa2a...
Floor  5: RECORD_ROOT      (type 83, v1) | 3    recs | 131072 B | keys: none | hash: 14423473...
[exit 0]
```

**5. Read the manifest without any S3A tool.** Tile 0's payload starts at byte 1024 and is plain UTF-8 text:

```
$ tail -c +1025 /tmp/s3a-quickstart/hello.sealed.s3a | head -c 800
================================================================================
                           THIS IS AN S3A FILE
      S3A Hyper-Tile Storage Archive (Immutable Cryptographic Container)
 Specification: https://github.com/ashleycribb/s3a-storage/blob/main/docs/spec/S3A-TILE-SPEC.md
 Spec version 1.5-draft, SHA-256 of spec text: aa21a617e1effeb94bef117bd6e293151a0a2d035bf90f43eaca218fa51c9f2f
================================================================================

ARCHIVE METADATA:
  Title / Purpose:  S3A Storage Archive
  Creator:          None (Unassigned)
  File UUID:        80a4c304aaefb6ea254d47d778db3bf0
  Created:          2026-10-22T03:12:50Z
  Sealed:           2026-10-22T04:04:41Z

TILE DIRECTORY:
  Floor  Type ID  Type Name             Records    Tile Size
```

## Reading a file with the Python reader

[`readers/python/s3a_reader`](../readers/python/README.md) is a second, independent reader. It is pure Python standard library and was written from the spec text. It verifies headers, CRCs, the seal, parity repair and record roots, and prints the conformance report:

```
$ PYTHONPATH=readers/python python3 -m s3a_reader read /tmp/s3a-quickstart/hello.sealed.s3a | python3 -m json.tool
{
    "about": {
        "spec_sha256": "aa21a617e1effeb94bef117bd6e293151a0a2d035bf90f43eaca218fa51c9f2f",
        "spec_url": "https://github.com/ashleycribb/s3a-storage/blob/main/docs/spec/S3A-TILE-SPEC.md",
        "spec_version": "1.5-draft",
        "writer": "s3a-engine 0.1.0"
    },
    "active_slot": 0,
    "embedded_spec": "absent",
    "generation": 5,
    "is_sealed": true,
    "outcome": "ok",
    "record_roots": "ok",
    "repaired_tiles": 0,
    "tile_count": 6,
    "tile_payload_sha256": {
        … one SHA-256 per tile, keyed by tile type …
    },
    "tile_types": {
        "0": 1,
        "1": 1,
        "22": 1,
        "77": 1,
        "78": 1,
        "83": 1
    }
}
[exit 0]
```

## Conformance suite

[`conformance/`](../conformance/README.md) has versioned test vectors any implementation can run. Your program needs two subcommands: `read <file>`, which prints the JSON report, and `merkle`, which reads a proof case on stdin. Then run the stdlib runner. `--level writer` also runs the rewrite cases, which call `<impl> rewrite <in> <out>`.

```
$ PYTHONPATH=readers/python python3 conformance/run.py --impl "python3 -m s3a_reader" conformance/vectors-1.3
vectors-1.3: 66 passed, 0 failed
[exit 0]

$ cargo build -q -p s3a-spec-vectors --bin s3a-conformance && python3 conformance/run.py --impl target/debug/s3a-conformance --level writer conformance/vectors-1.3
vectors-1.3: 68 passed, 0 failed
[exit 0]
```

Suites are frozen once published. New cases go into a new `vectors-1.N` directory, and each version is a superset of the one before. CI runs both readers on every suite.

## Where to go next

- How a crash-safe commit works: [`docs/durability.md`](durability.md), [ADR-0001](adr/0001-atomic-replace-durability.md).
- Unknown tiles and reserved bytes (what a reader must ignore or reject): [ADR-0011](adr/0011-unknown-tile-copy-through-default.md), [ADR-0013](adr/0013-unknown-tiles-and-reserved-bytes.md).
- Schema tile and layout versions: [ADR-0008](adr/0008-schema-tile-catalog.md), [`docs/schema-evolution.md`](schema-evolution.md).
- Domain packs (Scholar, GIS, robotics, …): [`docs/pack-contract.md`](pack-contract.md).
