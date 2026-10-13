//! Shadow branch evaluation layer.
//!
//! When `--shadow` is active, commits land on isolated shadow branches (`shadow/{agent}/{seq}`)
//! and are only promoted to `main` if an automated eval passes.

use s3a_vcs::records::VcsCommitRecord;
use crate::merge::RepoRecordEntry;

/// Outcome of automated shadow evaluation.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ShadowEvalOutcome {
    pub passed: bool,
    pub eval_name: String,
    pub score: f64,
    pub rationale: String,
}

/// Evaluates a candidate commit on a shadow branch before potential promotion.
pub fn evaluate_shadow_commit(
    commit: &VcsCommitRecord,
    records: &[RepoRecordEntry],
) -> ShadowEvalOutcome {
    // 1. Invariant 1: Valid cryptographic signature
    if !commit.verify_signature() {
        return ShadowEvalOutcome {
            passed: false,
            eval_name: "signature_verification".to_string(),
            score: 0.0,
            rationale: "Commit signature failed strict Ed25519 verification".to_string(),
        };
    }

    // 2. Invariant 2: Non-empty commit payload
    if records.is_empty() {
        return ShadowEvalOutcome {
            passed: false,
            eval_name: "non_empty_payload".to_string(),
            score: 0.0,
            rationale: "Commit has zero record entries".to_string(),
        };
    }

    // 3. Invariant 3: Commit message integrity
    let msg = commit.message();
    if msg.is_empty() {
        return ShadowEvalOutcome {
            passed: false,
            eval_name: "message_present".to_string(),
            score: 0.2,
            rationale: "Empty commit message not allowed in production branch".to_string(),
        };
    }

    // 4. Invariant 4: Record format validity (all payload hashes non-zero)
    for r in records {
        if r.payload_hash == [0u8; 32] {
            return ShadowEvalOutcome {
                passed: false,
                eval_name: "payload_integrity".to_string(),
                score: 0.5,
                rationale: format!("Record '{}' has null payload hash", r.record_id),
            };
        }
    }

    ShadowEvalOutcome {
        passed: true,
        eval_name: "s3a_swarm_shadow_eval".to_string(),
        score: 1.0,
        rationale: "All invariants passed: signature verified, payload structured, non-empty".to_string(),
    }
}
