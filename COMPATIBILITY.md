# S3A Compatibility Promise

S3A is meant for archives that must stay readable for decades. This promise is modelled on
SQLite's [long-term support statement](https://sqlite.org/lts.html), which commits to keeping
SQLite's on-disk format backwards compatible. It covers the on-disk format described in
[`docs/spec/S3A-TILE-SPEC.md`](docs/spec/S3A-TILE-SPEC.md), not Rust API signatures.

## 1. Within a major version (1.x)
- **Any 1.x reader opens any 1.x file.** A file written by any 1.x release stays readable by
  every later 1.x reader, with no conversion step.
- **New features only add.** New capabilities arrive only as new tile types, new flags, new
  optional schema/catalog entries, or a new `record_layout_version` for a tile type. An existing
  field's offset, size or meaning is never changed. Older readers skip what they don't know
  (TILE-SPEC §7.2) and still read everything else.
- **Unknown tiles are preserved.** Compaction, fusion, migration and repair tools copy unknown
  tile types through intact by default; dropping them needs an explicit user request (§7.2, ADR-0011).
- **Old layout versions stay readable.** A reader that supports layout version *n* of a tile type
  keeps supporting every earlier layout version of that type for the whole major version.

## 2. Reserved bytes
- Writers MUST write reserved and padding bytes as zero.
- Readers MUST ignore reserved bytes (TILE-SPEC §3.3) and MUST NOT reject a file because they are
  non-zero, unless the spec marks that field as checked (for example a CRC-covered field).
- A reserved region may be given a meaning in a later 1.x release only if zero keeps meaning
  "absent / old behavior".

## 3. Major versions
A new major version (2.0) is the only place an incompatible change may happen, and it requires:
1. **A migrator.** A tool that converts every 1.x file to the new format and verifies the result,
   following the `s3a-cli migrate` pattern (verify-after-write, never overwrite the input).
2. **A published deprecation window.** The incompatible change is announced in the spec and in
   `docs/api_changes/` at least one minor release, and at least 12 months, before 1.x writers stop
   being maintained. 1.x readers keep receiving fixes for the whole window.
3. **Readers keep reading 1.x.** A 2.x reader either opens 1.x files directly or points the user
   at the migrator; it never misreads them silently.

## 4. Signature algorithms
Signature and hash algorithms are identified by an algorithm field (S3A-SYNC, ADR-0006), not by
position or assumption.
- New algorithms are added as new table entries; an entry's id is never reused or renumbered.
- An algorithm may be marked **deprecated for new writes** (e.g. if it is weakened). Writers stop
  producing it; **readers keep verifying it on existing files**, reporting it as deprecated rather
  than failing, so old archives and proofs stay checkable. ADR-0010 (RSA verification of RFC 3161
  timestamps kept after dropping the `rsa` crate) is an example.
- Removing verification of an algorithm entirely is a major-version change under §3.

## 5. What is not covered
Experimental tile types or flags explicitly marked "experimental" in the spec, Rust crate APIs
(versioned separately under semver), and CLI output formatting.
