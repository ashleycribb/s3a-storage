# ADR-0006: Sync protocol with a DTN transfer profile (P1)

Status: Accepted — open questions resolved, approved by Ashley Cribb 2026-10-21

## Context
S3A needs one replication protocol for desktop, server, edge and gate peers. Lessons from Hugging Face
Xet (versioned messages, content-hash negotiation, proof of possession, retryable errors, idempotent
push) and from space data systems (CCSDS, Bundle Protocol v7, BPSec, CFDP, LunaNet) point to the same
design: self-contained, signed, content-addressed messages that tolerate delay and disconnection.

## Decision
- Adopt [S3A-SYNC 0.1-draft](../spec/S3A-SYNC.md): little-endian envelope, want/have by purpose-tagged
  content hash, keyed hashes and proof of possession, error table with retryable flag, `msg_id` idempotency.
- Define a DTN profile: signed bundles with lifetime/priority, BPv7 carriage, custody via signed receipts,
  per-stream sequence counters, chunk-hash resume, crypto-agile signatures (Ed25519 / ML-DSA / hybrid).
- Specify edge packet stores (RING/STOP) mapped to CCSDS APID + sequence count; implementation deferred.

## Consequences
- Docs only; no crate or file-format change in this PR.
- Implementation depends on C2 (CDC chunks) and the 1.0 hash/provenance freeze.

## Resolutions
Open questions 1–5 in S3A-SYNC §6 are Resolved — approved by Ashley Cribb 2026-10-21; see S3A-SYNC §7 for decisions and sources.
