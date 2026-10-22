//! Threat tests for signed share links / QR codes (S3A-VAULT §17).
use s3a_vault::share::*;
use s3a_vault::{GateAddress, KeyStore, MemoryKeyStore};

const NOW: u64 = 1_800_000_000;
const ALICE: [u8; 16] = [1; 16];
const MALLORY: [u8; 16] = [2; 16];
const BOB: [u8; 16] = [3; 16];

fn ks() -> MemoryKeyStore {
    let mut k = MemoryKeyStore::default();
    k.insert_keypair(&ALICE, &[11; 32]).unwrap();
    k.insert_keypair(&MALLORY, &[22; 32]).unwrap();
    k.insert_keypair(&BOB, &[33; 32]).unwrap();
    k
}
fn addr(k: &MemoryKeyStore, id: &[u8; 16]) -> String {
    GateAddress::from_public_key(&k.get_public_key(id).unwrap()).to_address_string()
}
fn tok(k: &MemoryKeyStore, id: &[u8; 16], aud: Option<String>) -> ShareToken {
    issue(k, id, [7; 32], NOW, 3600, [9; 16], aud).unwrap()
}
fn contacts(k: &MemoryKeyStore) -> Contacts {
    let mut c = Contacts::default();
    c.by_key.insert(
        hex::encode(k.get_public_key(&ALICE).unwrap()),
        "alice".into(),
    );
    c
}

#[test]
fn valid_link_verifies_and_shows_petname() {
    let k = ks();
    let t = tok(&k, &ALICE, None);
    let back = ShareToken::from_link(&t.to_link()).unwrap();
    assert_eq!(back, t);
    let r = open(
        &back,
        NOW + 10,
        None,
        Some(&[7; 32]),
        &mut SeenStore::default(),
    );
    assert_eq!(r, Ok(()));
    let d = describe(&back, &r, &contacts(&k), true);
    assert!(
        d.starts_with("VERIFIED") && d.contains("alice (pinned contact)"),
        "{d}"
    );
}

#[test]
fn forged_code_claiming_alice_key_rejected() {
    let k = ks();
    // Mallory builds a token with Alice's key/address but signs with her own key.
    let mut t = tok(&k, &ALICE, None);
    t.signature = k.sign(&MALLORY, &t.canonical_bytes()).unwrap();
    let t2 = ShareToken::from_link(&t.to_link()).unwrap();
    assert_eq!(verify(&t2, NOW, None, None), Err(Reject::BadSignature));
    // Zeroed / garbage signature.
    t.signature = [0; 64];
    assert_eq!(verify(&t, NOW, None, None), Err(Reject::BadSignature));
}

#[test]
fn swapped_sticker_shows_wrong_issuer_not_petname() {
    let k = ks();
    // Mallory's own, validly signed code pasted over Alice's.
    let t = tok(&k, &MALLORY, None);
    let r = verify(&t, NOW, None, None);
    assert_eq!(r, Ok(()));
    let d = describe(&t, &r, &contacts(&k), false);
    assert!(d.contains("UNKNOWN ISSUER") && !d.contains("alice"), "{d}");
    assert!(d.contains(&addr(&k, &MALLORY)));
    assert!(!d.contains(&addr(&k, &ALICE)));
}

#[test]
fn link_host_swapped_to_alice_address_rejected() {
    let k = ks();
    let t = tok(&k, &MALLORY, None);
    let link = t.to_link();
    let forged = link.replace(&addr(&k, &MALLORY), &addr(&k, &ALICE));
    assert_ne!(forged, link);
    assert_eq!(ShareToken::from_link(&forged), Err(Reject::AddressMismatch));
}

#[test]
fn embedded_pubkey_swapped_rejected() {
    let k = ks();
    let t = tok(&k, &MALLORY, None);
    let mut b = t.to_bytes();
    // Replace embedded key with Alice's: address bytes no longer match the key.
    let off = SHARE_DOMAIN.len() + 1 + 2 + 7;
    b[off..off + 32].copy_from_slice(&k.get_public_key(&ALICE).unwrap());
    assert_eq!(ShareToken::from_bytes(&b), Err(Reject::AddressMismatch));
}

#[test]
fn replay_rejected_second_time() {
    let k = ks();
    let t = tok(&k, &ALICE, None);
    let mut seen = SeenStore::default();
    assert_eq!(open(&t, NOW, None, None, &mut seen), Ok(()));
    assert_eq!(
        open(&t, NOW + 1, None, None, &mut seen),
        Err(Reject::Replay)
    );
    // Persisted store survives reload.
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("seen.json");
    seen.save(&p).unwrap();
    let mut s2 = SeenStore::load(&p).unwrap();
    assert_eq!(open(&t, NOW + 2, None, None, &mut s2), Err(Reject::Replay));
}

#[test]
fn failed_check_does_not_burn_nonce() {
    let k = ks();
    let t = tok(&k, &ALICE, None);
    let mut seen = SeenStore::default();
    assert_eq!(
        open(&t, NOW, None, Some(&[8; 32]), &mut seen),
        Err(Reject::ItemHashMismatch)
    );
    assert!(seen.seen.is_empty());
}

#[test]
fn tampered_payload_rejected() {
    let k = ks();
    let t = tok(&k, &ALICE, None);
    let wire = t.to_bytes();
    let canon_len = t.canonical_bytes().len();
    let sig_off = SHARE_DOMAIN.len() + 1 + 2 + 7 + 32 + 9;
    for i in [
        sig_off,
        sig_off + 32 + 3,
        sig_off + 32 + 8 + 7,
        canon_len - 2,
        canon_len + 5,
    ] {
        let mut b = wire.clone();
        b[i] ^= 0x01;
        let r = ShareToken::from_bytes(&b).and_then(|t| verify(&t, NOW, None, None));
        assert!(r.is_err(), "byte {i} flip accepted");
    }
    // Extended expiry is caught by the signature.
    let mut t2 = t.clone();
    t2.expires_at += 1000;
    assert_eq!(verify(&t2, NOW, None, None), Err(Reject::BadSignature));
    // Truncated / trailing bytes.
    assert!(ShareToken::from_bytes(&wire[..wire.len() - 1]).is_err());
    let mut longer = wire.clone();
    longer.push(0);
    assert!(ShareToken::from_bytes(&longer).is_err());
    // Non-canonical base64url char in the link.
    let mut link = t.to_link();
    link.push('=');
    assert!(ShareToken::from_link(&link).is_err());
}

#[test]
fn expired_and_future_rejected() {
    let k = ks();
    let t = tok(&k, &ALICE, None);
    assert_eq!(verify(&t, NOW + 3600, None, None), Err(Reject::Expired));
    assert_eq!(
        verify(&t, NOW - MAX_CLOCK_SKEW - 1, None, None),
        Err(Reject::NotYetValid)
    );
    let long = issue(&k, &ALICE, [7; 32], NOW, u64::MAX, [1; 16], None).unwrap();
    assert_eq!(long.expires_at - long.issued_at, MAX_LIFETIME);
}

#[test]
fn audience_enforced() {
    let k = ks();
    let t = tok(&k, &ALICE, Some(addr(&k, &BOB)));
    assert_eq!(verify(&t, NOW, Some(&addr(&k, &BOB)), None), Ok(()));
    assert!(matches!(
        verify(&t, NOW, Some(&addr(&k, &MALLORY)), None),
        Err(Reject::WrongAudience(_))
    ));
    assert!(matches!(
        verify(&t, NOW, None, None),
        Err(Reject::WrongAudience(_))
    ));
}

#[test]
fn item_hash_checked() {
    let k = ks();
    let t = tok(&k, &ALICE, None);
    assert_eq!(verify(&t, NOW, None, Some(&[7; 32])), Ok(()));
    assert_eq!(
        verify(&t, NOW, None, Some(&[6; 32])),
        Err(Reject::ItemHashMismatch)
    );
}

#[test]
fn registry_sig_alg_enforced() {
    let k = ks();
    for alg in ["none", "ml-dsa-44", "rsa"] {
        let mut t = tok(&k, &ALICE, None);
        t.sig_alg = alg.into();
        t.signature = k.sign(&ALICE, &t.canonical_bytes()).unwrap();
        let t = ShareToken::from_link(&t.to_link()).unwrap();
        assert!(
            matches!(verify(&t, NOW, None, None), Err(Reject::Algorithm(_))),
            "{alg}"
        );
    }
}

#[test]
fn qr_png_round_trip() {
    let k = ks();
    let link = tok(&k, &ALICE, None).to_link();
    let png = qr_png(&link, 4).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("code.png");
    std::fs::write(&p, &png).unwrap();
    let t = token_from_input(p.to_str().unwrap()).unwrap();
    assert_eq!(t.to_link(), link);
    assert!(qr_svg(&link).unwrap().starts_with("<?xml"));
}

#[test]
fn qr_phone_screenshot_scaled_blurred() {
    use image::{imageops, GrayImage, Luma};
    let k = ks();
    let link = tok(&k, &ALICE, Some(addr(&k, &BOB))).to_link();
    let png = qr_png(&link, 6).unwrap();
    let img = image::load_from_memory(&png).unwrap().to_luma8();
    // Non-integer downscale, gaussian blur, off-white background, contrast loss, offset.
    let (w, h) = img.dimensions();
    let small = imageops::resize(&img, w * 7 / 10, h * 7 / 10, imageops::FilterType::Triangle);
    let blurred = imageops::blur(&small, 1.2);
    let mut canvas =
        GrayImage::from_pixel(blurred.width() + 137, blurred.height() + 211, Luma([225]));
    for (x, y, p) in blurred.enumerate_pixels() {
        let v = 40 + (p.0[0] as u32 * 170 / 255) as u8 + ((x * 7 + y * 13) % 9) as u8;
        canvas.put_pixel(x + 61, y + 97, Luma([v]));
    }
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("screenshot.png");
    canvas.save(&p).unwrap();
    let t = token_from_input(p.to_str().unwrap()).unwrap();
    assert_eq!(t.to_link(), link);
    assert_eq!(
        verify(&t, NOW, Some(&addr(&k, &BOB)), Some(&[7; 32])),
        Ok(())
    );
}

#[test]
fn image_without_qr_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("blank.png");
    image::GrayImage::from_pixel(200, 200, image::Luma([255]))
        .save(&p)
        .unwrap();
    assert!(matches!(
        token_from_input(p.to_str().unwrap()),
        Err(Reject::Malformed(_))
    ));
}

#[test]
fn base64url_rfc4648_vectors() {
    for (i, o) in [
        ("", ""),
        ("f", "Zg"),
        ("fo", "Zm8"),
        ("foo", "Zm9v"),
        ("foob", "Zm9vYg"),
        ("fooba", "Zm9vYmE"),
        ("foobar", "Zm9vYmFy"),
    ] {
        assert_eq!(b64url_encode(i.as_bytes()), o);
        assert_eq!(b64url_decode(o).unwrap(), i.as_bytes());
    }
    assert!(b64url_decode("Zh").is_none()); // non-zero trailing bits
    assert!(b64url_decode("Zm+v").is_none());
}
