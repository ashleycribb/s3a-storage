//! S3A VCS Integration for Autonomous Harness.
//! Gating and human escalation for VCS merge operations.

use crate::decision::{
    DecisionEngine, DecisionKind, DecisionRequest, DecisionValue, OperationalDecisionType,
};
use s3a_vcs::{VcsError, VcsMergeResult, VcsRepo};

/// Executes a branch merge within the autonomous harness environment.
///
/// Mandatory Safety Invariant:
/// When a merge conflict involves claim, evidence, citation_fingerprint, or ai_trace records,
/// the harness NEVER auto-resolves or picks a side. It strictly raises `ask_human`
/// with both target and source versions presented for human supervisor resolution.
pub fn harness_merge(
    repo: &VcsRepo,
    source_branch: &str,
    target_branch: &str,
    signing_key: &ed25519_dalek::SigningKey,
    decision_engine: &DecisionEngine,
) -> Result<VcsMergeResult, VcsError> {
    match repo.merge(source_branch, target_branch, signing_key, false)? {
        VcsMergeResult::ConflictNeedsHuman {
            conflicting_records,
            message,
        } => {
            // Format supervisor prompt presenting BOTH versions for each conflicting record
            let mut question = format!("Claim Merge Conflict: {}\n", message);
            for c in &conflicting_records {
                question.push_str(&format!(
                    "Record: {} ({})\n  Version A ({}): {}\n  Version B ({}): {}\n",
                    c.record_id,
                    c.type_name,
                    target_branch,
                    c.target_version,
                    source_branch,
                    c.source_version
                ));
            }

            let req = DecisionRequest {
                decision_type: OperationalDecisionType::ToolSelection,
                question: question.clone(),
                context: format!("Conflicting records: {:?}", conflicting_records),
                kind: DecisionKind::Choice {
                    options: vec!["ask_human".to_string()],
                },
                safe_default: DecisionValue::Choice("ask_human".to_string()),
                is_claim_judgment: true,
            };

            let decision = decision_engine.evaluate(&req, false, false);
            assert_eq!(
                decision.action_taken, "ask_human",
                "Claim conflicts must route to ask_human"
            );
            assert!(decision.routed_to_human);

            Ok(VcsMergeResult::ConflictNeedsHuman {
                conflicting_records,
                message: question,
            })
        }
        other => Ok(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DecisionThresholds, LocalStore, MockModel, SafeDefaults};
    use s3a_core::TileType;
    use s3a_engine::TileWriter;
    use s3a_pack_scholar::records::{ScholarNoteClaimRecord, CLAIM_KIND_EMPIRICAL};
    use s3a_pack_scholar::ScholarTileType;
    use std::sync::Arc;

    #[test]
    fn test_harness_merge_routes_claim_conflict_to_ask_human() {
        let temp_dir = std::env::temp_dir();
        let path = temp_dir.join(format!("s3a_harness_merge_test_{}.s3a", std::process::id()));
        let mut writer = TileWriter::create(&path).unwrap();
        writer.enable_addon(s3a_core::ADDON_EDU).unwrap();
        let base_rec = vec![s3a_core::TelemetryRecord::new(100, 1, 1, 1.0)];
        writer
            .write_hyper_tile(TileType::TELEMETRY, &base_rec, None, None)
            .unwrap();
        drop(writer);

        let key = ed25519_dalek::SigningKey::from_bytes(&[42u8; 32]);

        let repo = VcsRepo::open(&path).unwrap();
        let base_commit = repo.commit("Base commit", "main", &key, None).unwrap();
        repo.set_branch("feature", base_commit.seal_hash, &key)
            .unwrap();

        // Main claim
        let mut w_main = TileWriter::open_append(&path).unwrap();
        let claim_a = vec![ScholarNoteClaimRecord::new(
            1,
            100,
            [0, 99],
            1,
            0.9,
            CLAIM_KIND_EMPIRICAL,
            "Finding A: 90% accuracy",
        )];
        w_main
            .write_hyper_tile(ScholarTileType::NOTE_CLAIMS, &claim_a, None, None)
            .unwrap();
        drop(w_main);
        repo.commit("Claim A", "main", &key, None).unwrap();

        // Feature claim
        let mut w_feat = TileWriter::open_append(&path).unwrap();
        let claim_b = vec![ScholarNoteClaimRecord::new(
            2,
            101,
            [0, 99],
            1,
            0.5,
            CLAIM_KIND_EMPIRICAL,
            "Finding B: 40% accuracy",
        )];
        w_feat
            .write_hyper_tile(ScholarTileType::NOTE_CLAIMS, &claim_b, None, None)
            .unwrap();
        drop(w_feat);
        repo.commit("Claim B", "feature", &key, None).unwrap();

        let mock_model = Arc::new(MockModel::new("mock-v1"));
        let store = Arc::new(LocalStore::new(path.clone(), false));
        let engine = DecisionEngine::new(
            mock_model,
            None,
            DecisionThresholds::default(),
            SafeDefaults::default(),
            store,
            "test-vcs-session",
        );

        let result = harness_merge(&repo, "feature", "main", &key, &engine).unwrap();
        match result {
            VcsMergeResult::ConflictNeedsHuman {
                message,
                conflicting_records,
            } => {
                assert_eq!(conflicting_records.len(), 1);
                assert!(message.contains("Version A (main):"));
                assert!(message.contains("Version B (feature):"));
            }
            _ => panic!("Expected ConflictNeedsHuman"),
        }

        let _ = std::fs::remove_file(&path);
    }
}
