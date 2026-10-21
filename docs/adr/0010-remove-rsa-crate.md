# ADR 0010: Remove the `rsa` crate (RUSTSEC-2023-0071)

Status: Accepted — 2026-10-21

## Context
`cargo audit` flagged `rsa` 0.9 with [RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071.html)
("Marvin" timing side channel), which has no fixed release. The advisory concerns operations
that use the RSA **private key** (decryption/signing); see the advisory text.

In S3A, `rsa` was used in exactly one production place: `s3a-engine/src/anchor.rs`
`verify_signature_over_data`, which **verifies** third-party RSA PKCS#1 v1.5 signatures
(SHA-256/384/512) on RFC 3161 timestamp tokens and their X.509 certificate chains
(e.g. DigiCert TSA). S3A never signs with RSA. The only private-key use was in tests that
forge tokens to prove forgeries are rejected.

Because these are third-party TSA signatures, they cannot be swapped to the approved
Ed25519 / ML-DSA-65 scheme; S3A must keep verifying RSA to keep existing anchor proofs valid.

## Decision
- Replace `rsa` verification with [`ring` 0.17](https://docs.rs/ring/0.17/ring/signature/index.html)
  `RSA_PKCS1_2048_8192_SHA{256,384,512}` (verification-only API). `ring` was already in
  `Cargo.lock` (via rustls), so no new crate enters the tree and no existing dep is bumped.
- Tests sign with `ring::signature::RsaKeyPair` loaded from two fixed, throwaway 2048-bit
  keys in `tests/fixtures/test_only_rsa2048_{a,b}.der` (ring cannot generate RSA keys).
  These keys protect nothing and must never be used outside tests.
- No on-disk or proof format changes.

## Consequences
- `rsa` is gone from `Cargo.lock`; RUSTSEC-2023-0071 no longer reported.
- `ring` rejects RSA moduli under 2048 bits. Public TSAs and CA/B-compliant roots use ≥2048,
  but a proof from a legacy <2048-bit TSA would now fail verification.
- Signatures S3A itself produces follow the approved plan: hybrid Ed25519 + ML-DSA-65 by
  default, Ed25519-only on constrained links.
