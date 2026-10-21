# ADR-0012: Lessons from Bitcoin's block design (per-record Merkle proofs)

- Status: Accepted (Merkle module + CLI); seal wiring implemented as record-root tile 83
- Date: 2026-10-21
- Inspiration: Andrej Karpathy's educational, from-scratch Bitcoin block code,
  [`cryptos/block.py`](https://github.com/karpathy/cryptos/blob/main/cryptos/block.py)

This is an educational note: what a Bitcoin block header gets right, how each idea maps onto
S3A, and what we deliberately do **not** copy. Every factual claim links to its source.

## 1. A small, fixed header that commits to everything

A Bitcoin block header is six fields — `nVersion`, `hashPrevBlock`, `hashMerkleRoot`, `nTime`,
`nBits`, `nNonce` — serialized in that order
([Bitcoin Core `src/primitives/block.h`](https://github.com/bitcoin/bitcoin/blob/master/src/primitives/block.h)),
80 bytes total, which `block.py` decodes field by field
([block.py](https://github.com/karpathy/cryptos/blob/main/cryptos/block.py)). The transactions
themselves are not in the header; they are bound to it through the Merkle root
([Bitcoin whitepaper §7](https://bitcoin.org/bitcoin.pdf)).

**S3A mapping.** Our v2 seal hash already commits to a compact header (prev hash, generation,
tile count, file UUID) plus the tile index entries
([`compute_seal_hash_v2`](../../crates/s3a-engine/src/sealed.rs)). What it does *not* yet commit
to is an individual record — the gap this ADR closes.

## 2. Every block points back to the one before

`hashPrevBlock` chains each header to its predecessor
([block.h](https://github.com/bitcoin/bitcoin/blob/master/src/primitives/block.h);
[whitepaper §3–4](https://bitcoin.org/bitcoin.pdf)), so rewriting history means rebuilding
everything after it.

**S3A mapping (already have it).** v2 seals carry `prev_seal_hash` and `verify_seal_chain`
rejects breaks and swapped generations ([sealed.rs](../../crates/s3a-engine/src/sealed.rs));
`s3a-vcs` commits carry `parent_seal_hashes`
([records.rs](../../crates/s3a-vcs/src/records.rs)).

## 3. Merkle roots and inclusion proofs

A Merkle tree lets a verifier check that one item is included using only a logarithmic number
of hashes ([whitepaper §7–8](https://bitcoin.org/bitcoin.pdf);
[RFC 6962 §2.1.1](https://datatracker.ietf.org/doc/html/rfc6962#section-2.1.1)).

**Bitcoin's weakness.** When a level has an odd number of nodes, Bitcoin duplicates the last
one, so two different transaction lists can produce the same root — CVE-2012-2459
([Bitcoin Core `src/consensus/merkle.cpp`, header comment](https://github.com/bitcoin/bitcoin/blob/master/src/consensus/merkle.cpp);
[NVD CVE-2012-2459](https://nvd.nist.gov/vuln/detail/CVE-2012-2459)).

**What S3A adopts: RFC 6962 Merkle Tree Hash.** Leaf = `SHA-256(0x00 || record)`, node =
`SHA-256(0x01 || left || right)`, split at the largest power of two below `n`, no duplication
([RFC 6962 §2.1](https://datatracker.ietf.org/doc/html/rfc6962#section-2.1)). Domain separation
means an interior node can never be passed off as a leaf, and `[a,b,c]` and `[a,b,c,c]` have
different roots (both are unit tests). Proof verification follows
[RFC 9162 §2.1.3.2](https://www.rfc-editor.org/rfc/rfc9162#section-2.1.3.2).

Implementation (this PR):
- [`s3a-edge/src/merkle.rs`](../../crates/s3a-edge/src/merkle.rs) — `no_std`, no allocation:
  `merkle_root`, `merkle_root_fixed`, `inclusion_proof_*`, `verify_inclusion`. Checked against
  the Certificate Transparency reference roots for 0–8 leaves
  ([merkle_tree_test.cc](https://github.com/google/certificate-transparency/blob/master/cpp/merkletree/merkle_tree_test.cc)).
- [`s3a-cli/src/proof_cli.rs`](../../crates/s3a-cli/src/proof_cli.rs):
  `s3a-cli prove <file> <tile> <record>` emits a self-contained JSON proof (scheme
  `s3a-merkle-rfc6962-sha256-v1`); `s3a-cli verify-proof <proof.json> [<file>]` verifies it,
  optionally against the file's recomputed tile root. A tile's records are its `payload_bytes`
  split into `record_count` equal slices.

## 4. Frozen byte layouts

Bitcoin's header field order and widths are fixed by its serializer
([block.h `SERIALIZE_METHODS`](https://github.com/bitcoin/bitcoin/blob/master/src/primitives/block.h)),
which is why any implementation — including a 100-line Python one
([block.py](https://github.com/karpathy/cryptos/blob/main/cryptos/block.py)) — can verify any block.
S3A makes the same promise in [`COMPATIBILITY.md`](../../COMPATIBILITY.md): fields are never
moved or redefined and unknown tiles are preserved. The proof scheme id above is versioned for the
same reason.

## 5. Validation is independent of trust

Anyone can recompute a header hash and check it against the target without trusting the sender
([whitepaper §5, §8](https://bitcoin.org/bitcoin.pdf)). For S3A, the equivalent is the
spec-only reader (future-proofing item 3) and proofs that verify with nothing but the JSON and a
root. The Bitcoin main-net genesis header is now an offline test vector for our header/PoW
seams (`target_from_nbits`, `extract_merkle_root_from_header`) in
[anchor.rs](../../crates/s3a-engine/src/anchor.rs); its hash and Merkle root are the values
asserted in [Bitcoin Core `chainparams.cpp`](https://github.com/bitcoin/bitcoin/blob/master/src/kernel/chainparams.cpp).

## 6. Not adopted: proof-of-work

Proof-of-work exists to reach agreement among untrusted strangers
([whitepaper §4](https://bitcoin.org/bitcoin.pdf)). S3A has no such consensus problem. We borrow
Bitcoin's timestamping instead, by anchoring seal fingerprints through
[OpenTimestamps](https://opentimestamps.org/) ([anchor.rs](../../crates/s3a-engine/src/anchor.rs)).

## Follow-up: wiring record roots into the seal (implemented)

Changing `compute_seal_hash_v2` would change every existing seal hash, so it is not done here.
Compatible plan, per [`COMPATIBILITY.md`](../../COMPATIBILITY.md) and
[ADR-0011](0011-unknown-tile-copy-through-default.md):
1. A new optional *record-root* tile listing `(tile_index, tile_id, record_count, root)` per tile,
   written before sealing so the existing seal hash covers it unchanged.
2. Readers that don't know the tile skip/copy it through; verifiers that do recompute each root.
3. Files without the tile verify exactly as today (old-file compatibility is tested in
   `crates/s3a-cli/tests/proof_cli_tests.rs` against the committed legacy v1.0 fixture).

**Implemented** ([`record_roots.rs`](../../crates/s3a-engine/src/record_roots.rs)): tile type `83`
(`record_root`, core extension cluster 70–86, next free id), 64-byte entries
`(tile_index, tile_type, tile_id, record_count, seal_generation, root)`. `s3a-cli seal --record-roots`
writes it; `s3a-cli verify` reports `Record roots: OK` or lists mismatches (exit 3);
`s3a-cli verify-proof <proof> <file> --against-seal` checks the proof root against the sealed entry.
Compactor and fusion drop the tile; each seal drops and optionally regenerates it.

**Opt-in, not default-on.** COMPATIBILITY.md §1 allows new optional tiles, but default-on would
change the tile count, and therefore the seal hash, that existing callers get from identical input
and seal options. Opt-in keeps every existing seal reproducible (tested against the pre-change hash).
It can become default in a later 1.x minor release with an `api_changes` note.

**Known limit.** A pre-0012 compactor treats type 83 as unknown and copies it through (ADR-0011).
Its output is unsealed, and `verify` flags the stale roots. Re-sealing drops and regenerates the tile.
