//! Concurrency, merge rules, delegation grants, and human escalation tests for s3a-swarm.

use ed25519_dalek::SigningKey;
use rand_core::{OsRng, RngCore};
use s3a_pack_scholar::ScholarTileType;
use s3a_swarm::grant::{validate_commit_grant, GrantRejectionReason};
use s3a_swarm::human_queue::AskHumanQueue;
use s3a_swarm::merge::{RecordMergeOutcome, RepoRecordEntry};
use s3a_swarm::{run_swarm, SimulatedAgent};
use s3a_vcs::records::RESOLUTION_CHOICE_TARGET;

#[tokio::test]
async fn test_swarm_small_concurrency_and_signature_verification() {
    let temp_dir = std::env::temp_dir();
    let out_archive = temp_dir.join(format!("s3a_swarm_test_{}.s3a", std::process::id()));

    let agents = 12;
    let commits_per_agent = 4;
    let (metrics, _coordinator) = run_swarm(agents, commits_per_agent, false, Some(out_archive.clone()))
        .await
        .expect("Swarm run should succeed");

    assert_eq!(metrics.agents, 12);
    assert_eq!(metrics.commits, 48);
    assert!(metrics.verified_signatures > 0);
    assert!(metrics.commits_per_sec > 0.0);
    assert!(metrics.p50_latency_ms >= 0.0);
    assert!(metrics.p99_latency_ms >= 0.0);
    assert!(!metrics.final_seal_hash.is_empty());

    // Clean up test archive
    let _ = std::fs::remove_file(&out_archive);
}

#[tokio::test]
async fn test_swarm_shadow_mode_evaluation() {
    let agents = 8;
    let commits_per_agent = 3;
    let (metrics, _coordinator) = run_swarm(agents, commits_per_agent, true, None)
        .await
        .expect("Shadow swarm run should succeed");

    assert!(metrics.shadow_mode);
    assert_eq!(metrics.agents, 8);
    assert_eq!(metrics.commits, 24);
    assert!(metrics.auto_merged > 0);
}

#[test]
fn test_deterministic_merge_rules_lww_and_tie_breaking() {
    let entry_a = RepoRecordEntry {
        record_id: "shared_config_1".to_string(),
        tile_type: 1,
        type_name: "CONFIG".to_string(),
        payload_hash: [1u8; 32],
        summary: "Version 100".to_string(),
        committed_timestamp: 1000,
        commit_signature: [10u8; 64],
        author_gate_address: "gate://s3a-0001".to_string(),
    };

    // Newer timestamp wins (LWW)
    let entry_newer = RepoRecordEntry {
        record_id: "shared_config_1".to_string(),
        tile_type: 1,
        type_name: "CONFIG".to_string(),
        payload_hash: [2u8; 32],
        summary: "Version 101".to_string(),
        committed_timestamp: 1050,
        commit_signature: [5u8; 64], // even with lower signature
        author_gate_address: "gate://s3a-0002".to_string(),
    };
    match entry_a.merge_with(&entry_newer) {
        RecordMergeOutcome::SameRecordUpdated { winner_timestamp, rule_applied } => {
            assert_eq!(winner_timestamp, 1050);
            assert_eq!(rule_applied, "timestamp_lww");
        }
        other => panic!("Expected SameRecordUpdated, got {:?}", other),
    }

    // Older timestamp preserved existing
    let entry_older = RepoRecordEntry {
        record_id: "shared_config_1".to_string(),
        tile_type: 1,
        type_name: "CONFIG".to_string(),
        payload_hash: [3u8; 32],
        summary: "Version 99".to_string(),
        committed_timestamp: 950,
        commit_signature: [255u8; 64],
        author_gate_address: "gate://s3a-0003".to_string(),
    };
    match entry_a.merge_with(&entry_older) {
        RecordMergeOutcome::SameRecordPreservedExisting { rule_applied } => {
            assert_eq!(rule_applied, "timestamp_lww_existing_newer");
        }
        other => panic!("Expected SameRecordPreservedExisting, got {:?}", other),
    }

    // Identical timestamp: tie-break via lexicographical signature comparison
    let mut higher_sig = [10u8; 64];
    higher_sig[63] = 99;
    let entry_tie_higher_sig = RepoRecordEntry {
        record_id: "shared_config_1".to_string(),
        tile_type: 1,
        type_name: "CONFIG".to_string(),
        payload_hash: [4u8; 32],
        summary: "Version 100 Tied Higher Sig".to_string(),
        committed_timestamp: 1000,
        commit_signature: higher_sig,
        author_gate_address: "gate://s3a-0004".to_string(),
    };
    match entry_a.merge_with(&entry_tie_higher_sig) {
        RecordMergeOutcome::SameRecordUpdated { winner_timestamp, rule_applied } => {
            assert_eq!(winner_timestamp, 1000);
            assert_eq!(rule_applied, "signature_tie_break");
        }
        other => panic!("Expected SameRecordUpdated via signature tie break, got {:?}", other),
    }
}

#[test]
fn test_epistemic_disagreements_never_auto_resolve_and_route_to_human() {
    let claim_a = RepoRecordEntry {
        record_id: "claim_quantum_speedup".to_string(),
        tile_type: ScholarTileType::NOTE_CLAIMS,
        type_name: "SCHOLAR_NOTE_CLAIMS".to_string(),
        payload_hash: [10u8; 32],
        summary: "Finding A: 10x speedup observed".to_string(),
        committed_timestamp: 2000,
        commit_signature: [1u8; 64],
        author_gate_address: "gate://s3a-lab-a".to_string(),
    };

    let claim_b = RepoRecordEntry {
        record_id: "claim_quantum_speedup".to_string(),
        tile_type: ScholarTileType::NOTE_CLAIMS,
        type_name: "SCHOLAR_NOTE_CLAIMS".to_string(),
        payload_hash: [20u8; 32],
        summary: "Finding B: No speedup within noise".to_string(),
        committed_timestamp: 2100, // even though later timestamp
        commit_signature: [255u8; 64],
        author_gate_address: "gate://s3a-lab-b".to_string(),
    };

    // MUST NOT auto-resolve
    match claim_a.merge_with(&claim_b) {
        RecordMergeOutcome::EpistemicConflictNeedsHuman { record_id, target_version, source_version } => {
            assert_eq!(record_id, "claim_quantum_speedup");
            assert!(target_version.contains("10x speedup"));
            assert!(source_version.contains("No speedup"));
        }
        other => panic!("Epistemic conflict must route to human, but got {:?}", other),
    }

    // Queue and resolve
    let queue = AskHumanQueue::new();
    let cid = queue.enqueue(
        "claim_quantum_speedup",
        ScholarTileType::NOTE_CLAIMS,
        "SCHOLAR_NOTE_CLAIMS",
        "Finding A: 10x speedup observed",
        "Finding B: No speedup within noise",
        "gate://s3a-lab-a",
        "gate://s3a-lab-b",
    );

    assert_eq!(queue.pending_count(), 1);
    let pending = queue.list_pending();
    assert_eq!(pending[0].conflict_id, cid);

    // Resolve with signed human supervisor key
    let mut seed = [0u8; 32];
    OsRng.fill_bytes(&mut seed);
    let supervisor_key = SigningKey::from_bytes(&seed);

    let resolution = queue.resolve(
        &cid,
        RESOLUTION_CHOICE_TARGET,
        "Replicated finding A with independent calibration",
        &supervisor_key,
    ).expect("Resolution must succeed");

    assert!(resolution.verify_signature());
    assert_eq!(queue.pending_count(), 0);
}

#[test]
fn test_grant_scope_enforcement_and_rejection() {
    let mut seed = [0u8; 32];
    OsRng.fill_bytes(&mut seed);
    let root_key = SigningKey::from_bytes(&seed);

    let agent = SimulatedAgent::new(7, &root_key);

    let valid_paths = vec!["records/agent_7/rec_1".to_string()];
    let invalid_paths = vec![
        "records/agent_7/rec_1".to_string(),
        "system/admin/unauthorized_write".to_string(),
    ];

    let now = 1000;
    // Valid write inside grant
    assert!(validate_commit_grant(&agent.grant, &agent.allowed_prefix, &valid_paths, now, 0).is_ok());

    // Invalid write outside grant
    match validate_commit_grant(&agent.grant, &agent.allowed_prefix, &invalid_paths, now, 0) {
        Err(GrantRejectionReason::UnauthorizedPath { path, allowed_prefix }) => {
            assert_eq!(path, "system/admin/unauthorized_write");
            assert_eq!(allowed_prefix, "records/agent_7/");
        }
        other => panic!("Expected UnauthorizedPath rejection, got {:?}", other),
    }
}

#[test]
fn test_bad_signature_rejection() {
    let mut seed = [0u8; 32];
    OsRng.fill_bytes(&mut seed);
    let root_key = SigningKey::from_bytes(&seed);

    let agent = SimulatedAgent::new(99, &root_key);
    let mut bundle = agent.prepare_commit(0, [0u8; 32]);

    // Initial signature is valid
    assert!(bundle.commit.verify_signature(), "Original commit signature must be valid");

    // Corrupt signature
    bundle.commit.signature[0] ^= 0xff;
    bundle.commit.signature[63] ^= 0xaa;
    assert!(!bundle.commit.verify_signature(), "Tampered signature must fail verification");
}
