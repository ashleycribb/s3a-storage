//! Prompt 014 (prereq G1): gate approval only from explicit approve verbs.
//!
//! Rule under test: a human gate is `Approved` only when the recorded
//! decision carries an explicit approve verb (`VERB_ID_APPROVED` or
//! `VERB_ID_TA_APPROVED_DRAFT`). A numeric `decision_score` must never
//! approve a gate on its own, and must never veto an explicit approval.
//!
//! Fail-first status on main (a0d4976): `gate_not_approved_by_high_score_without_verb`
//! and `gate_rejected_by_reject_verb_high_score` FAIL because
//! `record_human_decision_with_gate` approves whenever
//! `decision_score >= 0.70`, regardless of the verb. The other two tests
//! pass on main and guard the approve-verb behaviour the fix must keep.
//!
//! Mutation check (after the fix lands): restore the
//! `decision_score >= 0.70` arm and confirm the first test fails again.

use s3a_core::{S3ACoordinate, ADDON_EDU};
use s3a_engine::S3ACrudEngine;
use s3a_harness::{GateStatus, ResearchLoopEngine};
use s3a_pack_scholar::xapi::{
    VERB_ID_APPROVED, VERB_ID_REJECTED, VERB_ID_REVIEWED, VERB_ID_TA_APPROVED_DRAFT,
};

struct TempArchive {
    path: std::path::PathBuf,
}

impl TempArchive {
    fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let mut path = std::env::temp_dir();
        path.push(format!(
            "s3a_gate_verbs_test_{}_{}_{}.s3a",
            std::process::id(),
            ts,
            id
        ));
        Self { path }
    }
}

impl Drop for TempArchive {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn setup() -> (
    TempArchive,
    S3ACrudEngine,
    ResearchLoopEngine,
    S3ACoordinate,
) {
    let vault = TempArchive::new();
    let engine = S3ACrudEngine::open_or_create(&vault.path).expect("open test vault");
    engine
        .enable_addon(ADDON_EDU)
        .expect("enable scholar addon");
    let loop_engine = ResearchLoopEngine::new([0xAAAA_BBBB, 0xCCCC_DDDD], 77);
    // A real AI-step coordinate, so the human decision links to a real trace.
    let ai_coord = loop_engine
        .record_ai_step(&engine, 1, 0.85, S3ACoordinate::new(0, 0, 0), 555)
        .expect("record AI step");
    (vault, engine, loop_engine, ai_coord)
}

fn decide_and_read_status(
    loop_engine: &ResearchLoopEngine,
    engine: &S3ACrudEngine,
    verb_id: u32,
    decision_score: f32,
    ai_coord: S3ACoordinate,
) -> GateStatus {
    let gate = loop_engine.request_human_gate("test_action", "prompt 014 test");
    assert_eq!(gate.status, GateStatus::Pending);
    loop_engine
        .record_human_decision_with_gate(
            engine,
            0x05E8_0001,
            verb_id,
            decision_score,
            ai_coord,
            0x9999_8888,
            Some(&gate.gate_id),
        )
        .expect("record human decision");
    loop_engine
        .get_gate_status(&gate.gate_id)
        .expect("gate must still exist")
        .status
}

#[test]
fn gate_not_approved_by_high_score_without_verb() {
    let (_vault, engine, loop_engine, ai_coord) = setup();
    // A human reviewed with high confidence but never said "approve".
    // A score of 0.95 must not manufacture an approval.
    let status = decide_and_read_status(&loop_engine, &engine, VERB_ID_REVIEWED, 0.95, ai_coord);
    assert_eq!(
        status,
        GateStatus::Rejected,
        "high score without an approve verb must not approve the gate"
    );
}

#[test]
fn gate_approved_by_approve_verb() {
    let (_vault, engine, loop_engine, ai_coord) = setup();
    // An explicit approve verb approves even when the score is below 0.70:
    // the verb decides, the score never vetoes a human approval.
    let status = decide_and_read_status(&loop_engine, &engine, VERB_ID_APPROVED, 0.50, ai_coord);
    assert_eq!(
        status,
        GateStatus::Approved,
        "explicit approve verb must approve the gate"
    );
}

#[test]
fn gate_rejected_by_reject_verb_high_score() {
    let (_vault, engine, loop_engine, ai_coord) = setup();
    // An explicit reject verb rejects even at a high score: the score must
    // not override what the human actually said.
    let status = decide_and_read_status(&loop_engine, &engine, VERB_ID_REJECTED, 0.95, ai_coord);
    assert_eq!(
        status,
        GateStatus::Rejected,
        "explicit reject verb must reject the gate despite a high score"
    );
}

#[test]
fn gate_approved_by_ta_approve_verb() {
    let (_vault, engine, loop_engine, ai_coord) = setup();
    // The TA's approve-draft verb is an explicit approval too; the fix must
    // keep it working alongside the plain approve verb.
    let status = decide_and_read_status(
        &loop_engine,
        &engine,
        VERB_ID_TA_APPROVED_DRAFT,
        0.50,
        ai_coord,
    );
    assert_eq!(
        status,
        GateStatus::Approved,
        "TA approve-draft verb must approve the gate"
    );
}
