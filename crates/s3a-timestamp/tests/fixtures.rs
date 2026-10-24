//! Offline tests on real responses fetched once (2026-10-24) for SHA-256("s3a fixture").
use s3a_timestamp::*;
use sha2::{Digest, Sha256};
const FREE: &[u8] = include_bytes!("fixtures/freetsa.tsr");
const SECT: &[u8] = include_bytes!("fixtures/sectigo.tsr");
const MODA: &[u8] = include_bytes!("fixtures/aimoda.tsr");
fn d() -> [u8; 32] { Sha256::digest(include_bytes!("fixtures/data.txt")).into() }
fn h(s: &str) -> Vec<u8> { (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect() }
fn tok(r: &[u8]) -> Vec<u8> { token_from_response(r).unwrap() }

#[test] fn valid_tokens_accepted() {
    for (r, n) in [(FREE, "629176B994471144"), (SECT, "6AB01BD16EAEE1BB"), (MODA, "602A34300242F0F9")] {
        let i = verify_token(&tok(r), &d(), Some(&h(n))).unwrap();
        assert!(i.gen_time.starts_with("20261024164"), "{}", i.gen_time);
    }
}
#[test] fn wrong_digest_rejected() { let mut x = d(); x[0] ^= 1; assert_eq!(verify_token(&tok(FREE), &x, None).unwrap_err(), Error::Imprint); }
#[test] fn nonce_mismatch_rejected() { assert_eq!(verify_token(&tok(SECT), &d(), Some(&[1, 2, 3])).unwrap_err(), Error::Nonce); }
#[test] fn corrupted_token_rejected() {
    let t = tok(FREE);
    // flip a byte of the genTime inside TSTInfo -> signed messageDigest mismatch
    let p = t.windows(8).position(|w| w == b"20261024").unwrap();
    let mut c = t.clone(); c[p + 9] ^= 1; assert_eq!(verify_token(&c, &d(), None).unwrap_err(), Error::MessageDigest);
    assert!(verify_token(&t[..t.len() - 10], &d(), None).is_err());
    let mut c = t.clone(); c[1] ^= 0x40; assert!(verify_token(&c, &d(), None).is_err());
}
#[test] fn request_is_wellformed() {
    let r = build_request(&d(), &h("629176B994471144"));
    assert_eq!(r[0], 0x30); assert!(r.windows(32).any(|w| w == d()));
}
#[test] fn k_of_n_policy_and_file_roundtrip() {
    let a = Anchor { digest: d(), tokens: vec![tok(FREE), tok(SECT), tok(MODA)] };
    let b = Anchor::from_bytes(&a.to_bytes()).unwrap(); assert_eq!(a, b);
    assert_eq!(b.verify(DEFAULT_K).unwrap().len(), 3);
    assert!(b.verify(4).is_err());
    // duplicate tokens from one TSA count once
    let dup = Anchor { digest: d(), tokens: vec![tok(FREE), tok(FREE)] };
    assert_eq!(dup.verify(2).unwrap_err(), Error::Policy { have: 1, need: 2 });
    // anchor for a different (e.g. truncated-ledger) head fails
    let mut w = a.clone(); w.digest[5] ^= 1; assert_eq!(w.verify(1).unwrap_err(), Error::Policy { have: 0, need: 1 });
    assert!(Anchor::from_bytes(&a.to_bytes()[..50]).is_err());
}
