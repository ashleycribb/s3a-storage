//! Committee decision tests: independent of tracing consent, signed, chained, exported.
use crate::agent_api::WasmAgentSnapshot as Inner;
use crate::bindgen::{WasmAgentSnapshot, NOT_RECORDED};
use ed25519_dalek::{Signer, SigningKey};
use s3a_core::MARK_CONSENT_WITHDRAWN;
use s3a_pack_scholar::{verify_committee_log, ScholarTileType};
use wasm_bindgen::JsValue;

const TS: f64 = 1_761_000_000.0;

fn sess() -> WasmAgentSnapshot {
    WasmAgentSnapshot::new_session("0123456789abcdef0123456789abcdef").unwrap()
}

// Test-only key. Real reviewer keys are held by the web app's vault, never by the library.
fn reviewer() -> SigningKey {
    SigningKey::from_bytes(&[42u8; 32])
}

fn decide(
    s: &mut WasmAgentSnapshot,
    sk: &SigningKey,
    verdict: u32,
    comment: &str,
) -> Result<u32, JsValue> {
    let msg = s.committee_signing_payload(verdict, "prof-a", "submission-1", comment, TS)?;
    let sig = sk.sign(&msg).to_bytes();
    s.record_committee_decision(
        verdict,
        "prof-a",
        "submission-1",
        comment,
        TS,
        &sk.verifying_key().to_bytes(),
        &sig,
    )
}

fn human(s: &mut WasmAgentSnapshot) -> u32 {
    s.record_human(
        s3a_pack_scholar::xapi::VERB_ID_SEARCHED,
        JsValue::NULL,
        1,
        None,
    )
    .unwrap()
}

#[test]
fn decisions_recorded_in_every_consent_state() {
    let sk = reviewer();
    // never asked
    let mut s = sess();
    assert_eq!(s.consent_state(), "none");
    assert_eq!(decide(&mut s, &sk, 1, "ok").unwrap(), 0);
    assert_eq!(human(&mut s), NOT_RECORDED);
    // declined
    s.set_consent(false, 1, "study-1").unwrap();
    assert_eq!(decide(&mut s, &sk, 2, "fix ch.3").unwrap(), 1);
    assert_eq!(human(&mut s), NOT_RECORDED);
    assert!(s.inner.human_decisions.is_empty());
    // granted
    s.set_consent(true, 1, "study-1").unwrap();
    assert_eq!(decide(&mut s, &sk, 3, "").unwrap(), 2);
    assert_ne!(human(&mut s), NOT_RECORDED);
    // withdrawn
    s.set_consent(false, 1, "study-1").unwrap();
    assert_eq!(s.consent_state(), "withdrawn");
    assert_eq!(decide(&mut s, &sk, 1, "final").unwrap(), 3);
    assert_eq!(s.inner.committee_decisions.len(), 4);
    // decisions never land in the trace tile
    assert_eq!(s.inner.human_decisions.len(), 1);
    // withdrawal marks only trace tiles, never Tile 96
    assert!(!s
        .inner
        .policy_marks
        .iter()
        .any(|m| m.mark_kind == MARK_CONSENT_WITHDRAWN
            && m.target_tile_type == ScholarTileType::COMMITTEE_DECISION));
    let pk = sk.verifying_key().to_bytes();
    verify_committee_log(&s.inner.committee_decisions, |_, _| Some(pk)).unwrap();
    for i in 0..4 {
        s.verify_committee_decision(i, &pk).unwrap();
    }
    assert!(s.committee_decision_matches(1, "prof-a", "submission-1", "fix ch.3"));
    assert!(!s.committee_decision_matches(1, "prof-a", "submission-1", "edited"));
    let json = s.list_committee_decisions();
    assert!(json.contains("\"verdict_name\":\"revise\"") && json.contains("\"chain_ok\":true"));
    assert!(!json.contains("fix ch.3")); // comment text is never stored
}

#[test]
fn errors_are_never_silent() {
    let sk = reviewer();
    let pk = sk.verifying_key().to_bytes();
    let mut s = sess();
    for v in [0u32, 4] {
        assert!(s.committee_signing_payload(v, "r", "s", "", TS).is_err());
    }
    assert!(s.committee_signing_payload(1, "", "s", "", TS).is_err());
    assert!(s.committee_signing_payload(1, "r", "  ", "", TS).is_err());
    assert!(s.committee_signing_payload(1, "r", "s", "", 0.0).is_err());
    let msg = s
        .committee_signing_payload(1, "prof-a", "submission-1", "ok", TS)
        .unwrap();
    let sig = sk.sign(&msg).to_bytes();
    // verdict differs from what was signed
    assert!(s
        .record_committee_decision(2, "prof-a", "submission-1", "ok", TS, &pk, &sig)
        .is_err());
    // comment differs
    assert!(s
        .record_committee_decision(1, "prof-a", "submission-1", "OK", TS, &pk, &sig)
        .is_err());
    // wrong key
    let other = SigningKey::from_bytes(&[7u8; 32])
        .verifying_key()
        .to_bytes();
    assert!(s
        .record_committee_decision(1, "prof-a", "submission-1", "ok", TS, &other, &sig)
        .is_err());
    // malformed lengths
    assert!(s
        .record_committee_decision(1, "prof-a", "submission-1", "ok", TS, &pk[..31], &sig)
        .is_err());
    assert!(s
        .record_committee_decision(1, "prof-a", "submission-1", "ok", TS, &pk, &sig[..63])
        .is_err());
    assert!(s.inner.committee_decisions.is_empty());
    // correct one succeeds; replaying the same signature fails (chain head moved)
    assert!(s
        .record_committee_decision(1, "prof-a", "submission-1", "ok", TS, &pk, &sig)
        .is_ok());
    assert!(s
        .record_committee_decision(1, "prof-a", "submission-1", "ok", TS, &pk, &sig)
        .is_err());
}

#[test]
fn tamper_detection() {
    let sk = reviewer();
    let pk = sk.verifying_key().to_bytes();
    let mut s = sess();
    decide(&mut s, &sk, 1, "a").unwrap();
    decide(&mut s, &sk, 3, "b").unwrap();
    // verdict flip
    let mut t = s.inner.committee_decisions.clone();
    t[1].verdict = 1;
    assert!(t[1].verify_signature(&pk).is_err());
    // chain break
    let mut t = s.inner.committee_decisions.clone();
    t[0].timestamp_sec += 1;
    assert!(s3a_pack_scholar::verify_committee_chain(&t).is_err());
    // wrong key
    assert!(s
        .verify_committee_decision(
            0,
            &SigningKey::from_bytes(&[9u8; 32])
                .verifying_key()
                .to_bytes()
        )
        .is_err());
}

#[test]
fn export_bytes_roundtrip_keeps_chain() {
    let sk = reviewer();
    let mut s = sess();
    s.set_consent(true, 1, "study-1").unwrap();
    decide(&mut s, &sk, 1, "a").unwrap();
    s.set_consent(false, 1, "study-1").unwrap();
    decide(&mut s, &sk, 2, "b").unwrap();
    let bytes = s.export_bytes().unwrap();
    let back = Inner::from_bytes(&bytes).unwrap();
    assert_eq!(back.committee_decisions, s.inner.committee_decisions);
    let pk = sk.verifying_key().to_bytes();
    verify_committee_log(&back.committee_decisions, |_, _| Some(pk)).unwrap();
    // reload via the JS wrapper and keep appending to the same chain
    let mut w = WasmAgentSnapshot::from_bytes(&bytes).unwrap();
    assert_eq!(decide(&mut w, &sk, 3, "c").unwrap(), 2);
    verify_committee_log(&w.inner.committee_decisions, |_, _| Some(pk)).unwrap();
}

#[test]
fn export_delta_roundtrip_not_consent_gated() {
    let sk = reviewer();
    let mut a = sess();
    let base = a.export_bytes().unwrap();
    decide(&mut a, &sk, 1, "a").unwrap();
    decide(&mut a, &sk, 2, "b").unwrap();
    let delta = a.export_delta().unwrap();
    // receiver has never consented to tracing; decisions still apply
    let mut b = Inner::from_bytes(&base).unwrap();
    b.manifest.origin_session_uuid = a.inner.manifest.origin_session_uuid;
    b.manifest.snapshot_uuid = a.inner.manifest.snapshot_uuid;
    assert!(!b.tracing_allowed());
    assert_eq!(b.apply_delta_bytes(&delta).unwrap(), 2);
    assert_eq!(b.committee_decisions, a.inner.committee_decisions);

    // a delta that does not extend the receiver's chain is rejected whole
    let mut c = Inner::from_bytes(&base).unwrap();
    c.manifest.origin_session_uuid = a.inner.manifest.origin_session_uuid;
    c.manifest.snapshot_uuid = a.inner.manifest.snapshot_uuid;
    let mut x = sess();
    decide(&mut x, &sk, 1, "first").unwrap();
    let _ = x.export_delta().unwrap();
    decide(&mut x, &sk, 2, "second").unwrap(); // prev_hash points at "first"
    let orphan = x.export_delta().unwrap();
    let mut packet = s3a_pack_scholar::SrsSyncDeltaPacket::parse_bytes(&orphan).unwrap();
    packet.header.delta_seq = 1;
    let orphan = packet.export_bytes();
    assert!(c.apply_delta_bytes(&orphan).is_err());
    assert!(c.committee_decisions.is_empty());
}
