# S3A Sync Protocol (S3A-SYNC)

Version: 0.1-draft · Status: Draft (docs only, no code) · ADR: [ADR-0006](../adr/0006-sync-protocol.md)

The key words MUST, MUST NOT, SHOULD, SHOULD NOT and MAY are to be read as described in RFC 2119.
All multi-byte integers are **little-endian**. Readers MUST ignore reserved bytes; writers MUST zero them.

This spec covers replication of S3A content between peers (desktop, server, edge, gate, relay).
It is separate from [docs/sync.md](../sync.md) (Explorer → LRS batching), which it does not replace.

## 1. Core protocol

### 1.1 Message envelope

Every message is one self-contained envelope:

| Off | Size | Field | Notes |
|----:|-----:|-------|-------|
| 0 | 4 | `magic` | `"S3AS"` |
| 4 | 2 | `version` | major<<8 \| minor. This draft = `0x0001`. Unknown major MUST be rejected with `E_VERSION`. |
| 6 | 2 | `msg_type` | see 1.2 |
| 8 | 2 | `flags` | bit0 signed, bit1 DTN profile, bit2 compressed; others reserved |
| 10 | 1 | `priority` | 0 = bulk … 3 = critical |
| 11 | 1 | reserved | zero |
| 12 | 16 | `msg_id` | random 128-bit; idempotency key |
| 28 | 16 | `stream_id` | sender stream (UUID) |
| 44 | 8 | `seq` | per-stream sequence counter (§2.4) |
| 52 | 8 | `created_us` | UTC microseconds |
| 60 | 4 | `lifetime_s` | 0 = no expiry. Expired messages MUST be dropped. |
| 64 | 4 | `body_len` | checked; MUST be ≤ negotiated max |
| 68 | 4 | `ext_len` | TLV extension block length |
| 72 | n | `ext` | TLVs (type u16, len u16, value); unknown types MUST be skipped |
| … | m | `body` | per `msg_type` |
| … | k | `sig_block` | present iff flags.bit0 (§1.3) |

Lengths MUST be validated with checked arithmetic; a violation is `E_MALFORMED`.

### 1.2 Message types and want/have negotiation

| Type | Name | Body |
|-----:|------|------|
| 1 | `HELLO` | supported versions, hash/sig algorithm IDs, max body, capabilities |
| 2 | `HAVE` | list of content hashes the sender holds (or a root + ranges) |
| 3 | `WANT` | list of content hashes the sender lacks |
| 4 | `CHUNK` | hash + bytes of one content-defined chunk (8/64/128 KiB CDC, per C2) |
| 5 | `PUSH` | manifest (ordered chunk hashes) for an object/tile set |
| 6 | `ACK` | `msg_id`s accepted |
| 7 | `RECEIPT` | signed custody receipt (§2.3) |
| 8 | `ERROR` | code + retryable + detail (§1.4) |
| 9 | `PROVE` / 10 `PROOF` | proof-of-possession challenge/response (§1.3) |

Negotiation: peers exchange `HELLO`, then the receiver sends `WANT` for hashes it does not hold.
A sender MUST NOT send chunks that were not wanted, except in the DTN profile (§2), where
unsolicited `PUSH`+`CHUNK` bundles are allowed. Chunks are addressed by content hash, so
transfers are deduplicated and resumable.

### 1.3 Hashes, keys and proof of possession

- Hashes are **purpose-tagged**: `H(tag ‖ data)` where `tag` names the use (`"s3a.chunk.v1"`,
  `"s3a.manifest.v1"`, …). Default hash: BLAKE3-256 (alg ID 1).
- Public content-hash exchange can leak what a peer holds. Peers MAY agree a **keyed** hash
  (BLAKE3 keyed mode with a per-session or per-tenant key) for `HAVE`/`WANT` lists.
- Before serving a chunk to a peer that only claims ownership (dedup across tenants), a server
  SHOULD send `PROVE {hash, nonce, range}`; the peer answers `PROOF = H_keyed(nonce, data[range])`.
  Knowledge of a hash alone MUST NOT grant access to its bytes.
- `sig_block` = `alg_id u16 ‖ key_id (32) ‖ sig_len u16 ‖ sig`. It covers the envelope header,
  `ext` and `body`. `alg_id` gives crypto agility: 1 = Ed25519, 2 = ML-DSA-65 (FIPS 204),
  3 = hybrid Ed25519+ML-DSA-65. Receivers MUST reject unknown or disallowed `alg_id` (`E_SIG`).

### 1.4 Errors

| Code | Name | Retryable | Meaning |
|-----:|------|:---------:|---------|
| 1 | `E_VERSION` | no | unsupported major version |
| 2 | `E_MALFORMED` | no | bad length/field |
| 3 | `E_SIG` | no | signature missing/invalid/disallowed algorithm |
| 4 | `E_EXPIRED` | no | lifetime elapsed |
| 5 | `E_HASH_MISMATCH` | yes | chunk bytes ≠ hash (resend) |
| 6 | `E_NOT_FOUND` | no | wanted hash unknown |
| 7 | `E_PROOF` | no | proof of possession failed |
| 8 | `E_BUSY` | yes | back off (ext TLV may carry retry-after) |
| 9 | `E_STORAGE` | yes | receiver out of space / I/O error |
| 10 | `E_SEQ_GAP` | yes | gap detected; resend from given seq |
| 11 | `E_REPLAY` | no | duplicate/old seq outside window |
| 12 | `E_AUTH` | no | key not authorised for stream |

Retryable errors SHOULD be retried with exponential backoff and jitter.

### 1.5 Idempotent push

`msg_id` is the idempotency key. A receiver MUST treat a repeated `msg_id` as a no-op and
re-send the original `ACK`/`RECEIPT`. Applying a `PUSH` MUST be atomic (all chunks present and
verified, then a single commit via the engine's atomic replace); partial state MUST NOT be visible.

## 2. DTN transfer profile

Enabled by `flags.bit1`. Intended for links with long delay, scheduled contacts or no end-to-end path.

1. **Self-contained bundles.** Each message MUST be signed and carry `lifetime_s` and `priority`,
   so it is valid when read days later from any medium.
2. **Bundle Protocol v7 carriage.** An envelope MAY be the payload block of a BPv7 bundle
   (RFC 9171). The bundle lifetime SHOULD equal `lifetime_s`; bundle priority SHOULD map from `priority`.
3. **Custody.** A sender MUST retain data until it holds a valid signed `RECEIPT` from the next hop
   covering the `msg_id` (and chunk hashes). Only then MAY it delete. Receipts are idempotent.
4. **Sequence counters.** `seq` is strictly increasing per `stream_id`. Receivers track the
   highest seq and a replay window (default 1024); gaps produce `E_SEQ_GAP`, duplicates outside
   the window `E_REPLAY`. Edge generation counters map onto `seq`.
5. **Per-block integrity.** Like BPSec (RFC 9172), each `CHUNK` is independently verifiable (hash +
   signed manifest), so integrity survives relays that cannot decrypt. Confidentiality MAY use
   per-block encryption; BPSec BIB/BCB MAY be used when carried in BPv7.
6. **Resumable transfer (CFDP-style).** After interruption the receiver sends `WANT` with only the
   missing chunk hashes of a manifest. No byte offsets are used.
7. **Media.** The profile MUST work over store-and-forward media: relay nodes, removable storage
   (a directory of `*.s3as` files, one envelope each) and `gate://` endpoints.

## 3. Edge packet stores (spec only; future edge-writer change)

An edge device MAY hold named **packet stores**:

| Property | Values |
|----------|--------|
| `name` | ≤ 16 bytes UTF-8 |
| `priority` | 0–3 (same as envelope) |
| `overflow` | `RING` (overwrite oldest) or `STOP` (refuse new, keep all) |
| `capacity` | bytes |
| `apid` | 11-bit CCSDS APID mapping |

- Each record carries a 14-bit per-store sequence count (CCSDS Space Packet style) widened into
  the envelope `seq`. `RING` overwrites MUST be reported as a gap, not hidden.
- Downlink requests select by store name and/or time range; highest priority stores drain first.
- Mapping to CCSDS Space Packets (CCSDS 133.0-B): APID = store `apid`, sequence count = low 14 bits.

## 4. Alignment and use cases

- **CCSDS / LunaNet.** The profile uses the same building blocks NASA LunaNet and CCSDS use
  (BPv7 store-and-forward, CFDP-like resume, APID/sequence telemetry), so S3A data can ride those
  networks without a bespoke gateway. No conformance claim is made by this draft.
- **IIoT / edge / fog.** The same properties (intermittent links, priority drain, custody, gaps
  visible) apply to factory sensors, vehicles and remote sites.

## 5. Non-goals

- Implementing BPv7, BPSec, CFDP or LTP; S3A rides on existing stacks.
- Routing, contact-plan scheduling or time/navigation services.
- Conflict resolution of concurrent writes (see D2/D3); judgements stay with humans.
- Changing the tile file format.

## 6. Open questions

1. Default signature algorithm at 1.0: Ed25519, ML-DSA, or hybrid?
2. Key distribution and revocation for offline peers.
3. Should the replay window size be negotiated?
4. CBOR vs fixed binary for `HELLO`/`HAVE` bodies.
5. Registration of a BPv7 service/endpoint ID for S3A.

## 7. Prior art and recommended resolutions

Questions in §6 stay **Open** until approved by Ashley Cribb. Recommendations only.

| # | What prior art chose | Recommended S3A answer |
|---|----------------------|------------------------|
| 1 | RFC 9173 default contexts are symmetric (BIB-HMAC-SHA2, BCB-AES-GCM); CCSDS SDLS (355.0-B) is AES-GCM with pre-shared keys; no CCSDS PQ default yet. NIST FIPS 204 finalised ML-DSA (2024). | Hybrid Ed25519+ML-DSA-65 (alg 3) for receipts/manifests; Ed25519 allowed for constrained links; HMAC-BLAKE3 MAY be used per hop. |
| 2 | RFC 9172 leaves key management out of scope; SDLS uses ground-managed keys with an extended-procedures key-management service (OTAR, activate/deactivate). | Pre-provisioned keys; rotation and revocation sent as signed, lifetime-bound sync messages; revocation lists carried as bundles. |
| 3 | SDLS anti-replay: per-SA sequence number and window, **configured** by management, not negotiated. | Configured per stream; advertised in `HELLO`; not negotiated. |
| 4 | BPv7 (RFC 9171) encodes bundles in CBOR (RFC 8949); CCSDS Space Packet (133.0-B) uses a fixed binary header. | Fixed binary envelope; deterministic CBOR for `HELLO`; fixed arrays of hashes for `HAVE`/`WANT`. |
| 5 | BPv7 endpoints use `dtn:` or `ipn:` schemes; `ipn` node/service numbers are IANA/SANA registered (RFC 9758). CCSDS 734.2-B profiles BPv7 for CCSDS. | Use `dtn://<node>/s3a` now; request an `ipn` service number from IANA/SANA before 1.0. |

Sources: RFC 9171, 9172, 9173, 5326, 9758 (rfc-editor.org); CCSDS 727.0-B, 133.0-B, 734.2-B, 355.0-B (public.ccsds.org); NASA LunaNet Interoperability Specification (nasa.gov); ECSS-E-ST-70-41C (ecss.nl); NIST FIPS 204 (csrc.nist.gov).
