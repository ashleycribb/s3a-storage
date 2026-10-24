use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use s3a_runledger::*;
use serde_json::json;

fn run(plan: &str, outcome: &str) -> Record {
    Record::Run { plan_id: plan.into(), input_hashes: vec!["aa".into()], code_hash: "c0ffee".into(), params: json!({"n": 3}), outcome: outcome.into(), timestamp: "2026-10-24T12:00:00-04:00".into() }
}
fn plan(id: &str) -> Record { Record::Plan { plan_id: id.into(), hypothesis: "h".into(), plan_hash: "p0".into() } }

fn sample() -> Ledger {
    let k = SigningKey::generate(&mut OsRng);
    let mut l = Ledger::default();
    l.append(&k, plan("P")).unwrap();
    l.append(&k, run("P", "positive")).unwrap();
    l.append(&k, Record::Amendment { plan_id: "P".into(), prior_seq: 0, reason: "outlier rule".into(), new_plan_hash: "p1".into() }).unwrap();
    l.append(&k, run("P", "null")).unwrap();
    l.append(&k, run("P", "failed")).unwrap();
    l
}

#[test] fn append_verify_roundtrip() {
    let l = sample();
    let l2 = Ledger::from_jsonl(&l.to_jsonl()).unwrap();
    assert_eq!(l2.entries.len(), 5);
}
#[test] fn tamper_rejected() {
    let mut l = sample();
    if let Record::Run { outcome, .. } = &mut l.entries[3].record { *outcome = "positive".into(); }
    assert_eq!(l.verify(), Err(LedgerError::BadSig(3)));
}
#[test] fn delete_rejected() {
    let mut l = sample(); l.entries.remove(1);
    assert!(l.verify().is_err());
}
#[test] fn delete_tail_detected_via_jsonl_edit() {
    let mut l = sample(); l.entries.remove(3);
    assert!(l.verify().is_err());
}
#[test] fn reorder_rejected() {
    let mut l = sample(); l.entries.swap(3, 4);
    assert!(l.verify().is_err());
}
#[test] fn amendment_unknown_plan_rejected() {
    let k = SigningKey::generate(&mut OsRng);
    let mut l = Ledger::default();
    l.append(&k, plan("P")).unwrap();
    let r = l.append(&k, Record::Amendment { plan_id: "Q".into(), prior_seq: 0, reason: "x".into(), new_plan_hash: "q".into() });
    assert!(matches!(r, Err(LedgerError::UnknownPlan(1, _))));
    assert_eq!(l.entries.len(), 1, "failed append must not write");
}
#[test] fn amendment_chain_must_reference_latest() {
    let k = SigningKey::generate(&mut OsRng);
    let mut l = sample();
    // latest version of P is seq 2; pointing at 0 again forks the plan
    let r = l.append(&k, Record::Amendment { plan_id: "P".into(), prior_seq: 0, reason: "fork".into(), new_plan_hash: "p2".into() });
    assert_eq!(r.err(), Some(LedgerError::BadAmendmentRef(5)));
    l.append(&k, Record::Amendment { plan_id: "P".into(), prior_seq: 2, reason: "ok".into(), new_plan_hash: "p2".into() }).unwrap();
}
#[test] fn run_under_unknown_plan_rejected() {
    let k = SigningKey::generate(&mut OsRng);
    let mut l = Ledger::default();
    assert!(l.append(&k, run("Z", "null")).is_err());
}
#[test] fn switched_plan_and_null_results_reported() {
    let r = sample().run_report();
    assert_eq!(r.len(), 3);
    assert_eq!((r[0].plan_version, r[0].switched), (0, false));
    assert_eq!((r[1].plan_version, r[1].plan_hash.as_str(), r[1].switched), (1, "p1", true));
    assert_eq!(r[1].outcome, "null");
    assert_eq!(r[2].outcome, "failed");
}
