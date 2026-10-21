# ADR-0009: Edge packet stores

Status: proposed (spec only) · Date: 2026-10-21

## Decision
Edge devices hold named packet stores, one per sync stream, each with a fixed micro-tile range,
a priority, and an overflow mode `RING` (overwrite oldest, report a gap) or `STOP` (refuse new,
keep all). Stored data is released only after a signed custody `RECEIPT` from the next hop.
Priority and custody live in the S3A envelope, not in BPv7, because BPv7 removed both
(RFC 9171 Appendix A, https://www.rfc-editor.org/rfc/rfc9171.txt).

## Why
Bounded and circular FIFO stores are the established spacecraft pattern
(CCSDS 873.0-M-1 §2.3.3.1, https://ccsds.org/Pubs/873x0m1s.pdf). Release markers are append-only
bit programs, so they fit the existing power-loss model (S3A-EDGE-PROFILE §3.3, §4).

## Consequences
No file-format or code change now. A later edge-writer PR implements §16 of S3A-EDGE-PROFILE.md.
