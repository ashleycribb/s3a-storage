//! Acceptance (gap 2): RFC 3161 tokens signed with RSA verify with the same code on native and
//! wasm32 (the wasm half runs in tests/smoke.mjs). Fixtures: tests/fixtures/test_tsa/, made
//! once by scripts/gen_test_tsa_fixtures.sh (test-only CA and TSA; keys deleted).
//!
//! Written before the fix. Do not edit.

use s3a_engine::{parse_pem_certificates, verify_rfc3161, AnchorProof};

const FX: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/test_tsa");

fn read(name: &str) -> Vec<u8> {
    std::fs::read(format!("{FX}/{name}")).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn imprint() -> [u8; 32] {
    let hex_s = String::from_utf8(read("message_imprint_sha256.hex")).unwrap();
    let v = hex::decode(hex_s.trim()).unwrap();
    v.try_into().unwrap()
}

fn proof(bytes: Vec<u8>) -> AnchorProof {
    AnchorProof {
        method: "rfc3161".into(),
        fingerprint: hex::encode(imprint()),
        proof_bytes: bytes,
        submitted_at: 0,
        status: "unverified".into(),
        uri: None,
        block_height: None,
        block_hash: None,
    }
}

fn roots() -> Vec<x509_cert::Certificate> {
    parse_pem_certificates(std::str::from_utf8(&read("test_root_ca.pem")).unwrap()).unwrap()
}

/// Flip one byte inside the SignerInfo signature (the last `sig_len` bytes of the token are
/// the signature for these tokens: SignerInfo has no unsigned attributes).
fn tampered(mut b: Vec<u8>) -> Vec<u8> {
    let n = b.len();
    b[n - 40] ^= 0x01;
    b
}

#[test]
fn pkcs1_v15_sha256_token_verifies() {
    let ok = verify_rfc3161(
        &proof(read("token_pkcs1_sha256.der")),
        &imprint(),
        Some(&roots()),
    );
    assert_eq!(ok, Ok(true));
}

#[test]
fn pss_sha256_token_verifies() {
    let ok = verify_rfc3161(
        &proof(read("token_pss_sha256.der")),
        &imprint(),
        Some(&roots()),
    );
    assert_eq!(ok, Ok(true));
}

#[test]
fn tampered_tokens_are_rejected() {
    for f in ["token_pkcs1_sha256.der", "token_pss_sha256.der"] {
        let r = verify_rfc3161(&proof(tampered(read(f))), &imprint(), Some(&roots()));
        assert!(
            r.is_err(),
            "{f}: a tampered signature must be an error, got {r:?}"
        );
    }
}

#[test]
fn wrong_imprint_or_untrusted_root_is_not_confirmed() {
    let mut other = imprint();
    other[0] ^= 1;
    let r = verify_rfc3161(
        &proof(read("token_pkcs1_sha256.der")),
        &other,
        Some(&roots()),
    );
    assert_ne!(r, Ok(true), "a different fingerprint must not confirm");
    let digicert = parse_pem_certificates(include_str!("fixtures/digicert_root_ca.pem")).unwrap();
    let r = verify_rfc3161(
        &proof(read("token_pss_sha256.der")),
        &imprint(),
        Some(&digicert),
    );
    assert!(
        r.is_err(),
        "a token from the test TSA must not chain to DigiCert, got {r:?}"
    );
}
