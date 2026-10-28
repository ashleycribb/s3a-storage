# Edge seal records: how they are signed and checked

Implementation note, not normative. It records what the reference code does today, checked
against `spec/S3A-EDGE-PROFILE.md` §6.1–§6.3, so a second reader can be written without reading
the source. Where the spec is silent, that is called out. The spec text is unchanged.

## What is signed

For tile `n` of an edge archive the device computes

```
payload_hash_n = SHA-256(tile payload bytes)            // payload_len bytes after the 64-byte tile header
seal_hash_n    = SHA-256("S3AEDGE1" || prev_seal_hash || generation_n (u64 LE)
                          || tile_index_n (u32 LE) || payload_hash_n || device_id (16 bytes))
sigma_n        = Ed25519_Sign(device_private_key, seal_hash_n)   // over the raw 32 bytes, no further hashing or prefix
```

`prev_seal_hash` is 32 zero bytes for tile 0, otherwise `seal_hash_{n-1}`. Code:
`s3a_edge::compute_seal_hash`, `s3a_edge::sign_seal_hash`; writer: `EdgeWriter::seal_tile`.
This matches spec §6.1 (chain) and §6.2 (signature over the 32-byte `seal_hash`).

`generation_n` is the tile header's `generation` (u32 widened to u64). When a tile's generation is 0
the reference readers use `file_header.generation - tile_count + n + 1` (`MmapReader::edge_seal_chain`,
`import_edge_archive`). The spec does not mention this fallback.

## Where it is stored

A signature lives in a `DeviceSealRecord` (144 bytes) inside a seal tile (type 70). A seal tile
is a tile like any other, so it has its own `seal_hash` and is itself chained. Fields:

| Field | Meaning |
|---|---|
| `tile_index` | index of the tile this record signs (not the seal tile's own index) |
| `generation` | generation of that tile |
| `seal_hash` | `seal_hash_n` for that tile |
| `prev_seal_hash` | `seal_hash_{n-1}` (zeros for tile 0) |
| `signature` | `sigma_n` |
| `record_crc32` | CRC32C over bytes 0..140 |

The device public key comes from the file's `DeviceIdentityRecord` (96 bytes; tile type 64 or
legacy 33; CRC32C over bytes 0..92; non-zero `device_id`). Seal records are optional: a device may
write none (the committed `micro_signed_seal_chain` vector signs in memory but stores none).

## What a verifier checks, in order

1. The identity record exists and passes its CRC. (`MmapReader::edge_device_identity`)
2. The tile holding the seal record passes its CRC. (`try_get_edge_tile`)
3. `record.verify()` (record CRC).
4. `record.seal_hash` equals the recomputed chain hash for `record.tile_index`.
   (`MmapReader::edge_seal_chain`)
5. `Ed25519_Verify(identity.public_key, record.seal_hash, record.signature)`.
   (`s3a_edge::verify_signature_raw`)
6. Optionally, `identity.public_key` is one the caller already trusts (pinned).

## Limits worth knowing

- **Self-asserted key.** The public key comes from the same file it signs. Steps 1–5 prove the
  file is internally consistent and that whoever wrote the seal records held that key. They do
  not prove who the key belongs to; only step 6 does, and it needs a key obtained elsewhere.
- **Verify, not verify_strict.** `s3a_edge::verify_signature` uses `ed25519_dalek::Verifier::verify`
  (the non-strict check in `ed25519_dalek`; it does not reject small-order keys or nonces the way `verify_strict` does). The vault crate uses `verify_strict`. The spec
  does not say which. Both accept every signature the reference writer produces.
- **`import_edge_archive` does not verify signatures.** It recomputes the chain from tile data
  and then reports `seal_chain_verified: true`; it never reads seal records or checks a signature.
  That flag means the chain could be recomputed, not that it was signed. Left unchanged here.
