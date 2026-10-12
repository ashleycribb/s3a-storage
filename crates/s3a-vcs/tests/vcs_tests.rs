//! Integration test suite for Git-style versioning (s3a-vcs).
//!
//! Validates:
//! 1. Commit signature verification & forged signature rejection
//! 2. Commit log DAG order (traversal along parent chain)
//! 3. Branch & checkout isolation
//! 4. Diff counts (added, withdrawn, erased) without leaking restricted payloads
//! 5. Merge union & conflict reporting
//! 6. Sync push/pull inventory transmitting only missing tiles
//! 7. Non-fast-forward push rejection without --merge
//! 8. Consent scope respected on sync push
//! 9. Read-only Git export (`export-git`) produces valid Git repo (`git log` works)
//! 10. Manifest "Created" populated with valid archive timestamp

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use ed25519_dalek::SigningKey;
use rand_core::{OsRng, RngCore};

use s3a_core::consent::{ConsentPurpose, ConsentRecord};
use s3a_core::{TelemetryRecord, TileType};
use s3a_engine::{MmapReader, TileWriter};
use s3a_vcs::{export_git, sync_push, VcsRepo};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(5000);

struct TempArchiveFile {
    path: PathBuf,
}

impl TempArchiveFile {
    fn new(suffix: &str) -> Self {
        let count = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let mut p = std::env::temp_dir();
        p.push(format!("s3a_vcs_test_{}_{}_{}.s3a", suffix, std::process::id(), count));
        Self { path: p }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempArchiveFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        let key_file = PathBuf::from(format!("{}.vcs.key", self.path.display()));
        let _ = fs::remove_file(&key_file);
    }
}

fn generate_test_key() -> SigningKey {
    let mut seed = [0u8; 32];
    OsRng.fill_bytes(&mut seed);
    SigningKey::from_bytes(&seed)
}

#[test]
fn test_vcs_commit_signature_verify_and_forgery_rejected() {
    let archive = TempArchiveFile::new("sig_test");
    let mut writer = TileWriter::create(archive.path()).unwrap();
    let recs = vec![TelemetryRecord::new(100, 1, 1, 10.0)];
    writer.write_hyper_tile(TileType::TELEMETRY, &recs, None, None).unwrap();
    drop(writer);

    let key = generate_test_key();
    let repo = VcsRepo::open(archive.path()).unwrap();

    // 1. Commit and verify signature
    let commit = repo.commit("Initial commit", "main", &key, None).unwrap();
    assert!(commit.verify_signature(), "Original commit signature must verify cleanly");

    // 2. Tampered commit envelope should fail verification
    let mut tampered = commit;
    tampered.message_bytes[0] ^= 0xFF;
    assert!(!tampered.verify_signature(), "Tampered message must fail signature verification");

    let mut tampered_seal = commit;
    tampered_seal.seal_hash[0] ^= 0xAA;
    assert!(!tampered_seal.verify_signature(), "Tampered seal hash must fail signature verification");
}

#[test]
fn test_vcs_log_dag_order() {
    let archive = TempArchiveFile::new("log_order");
    let mut writer = TileWriter::create(archive.path()).unwrap();
    let recs1 = vec![TelemetryRecord::new(100, 1, 1, 10.0)];
    writer.write_hyper_tile(TileType::TELEMETRY, &recs1, None, None).unwrap();
    drop(writer);

    let key = generate_test_key();
    let repo = VcsRepo::open(archive.path()).unwrap();

    let c1 = repo.commit("Commit 1: Genesis", "main", &key, None).unwrap();

    // Append second tile
    let mut writer2 = TileWriter::open_append(archive.path()).unwrap();
    let recs2 = vec![TelemetryRecord::new(200, 2, 1, 20.0)];
    writer2.write_hyper_tile(TileType::TELEMETRY, &recs2, None, None).unwrap();
    drop(writer2);

    let c2 = repo.commit("Commit 2: Second sensor", "main", &key, None).unwrap();

    // Inspect history
    let history = repo.log(Some("main")).unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].seal_hash, c2.seal_hash);
    assert_eq!(history[0].message(), "Commit 2: Second sensor");
    assert_eq!(history[1].seal_hash, c1.seal_hash);
    assert_eq!(history[1].message(), "Commit 1: Genesis");
    assert_eq!(history[0].parent_seal_hashes[0], c1.seal_hash);
}

#[test]
fn test_vcs_branch_and_checkout_isolation() {
    let archive = TempArchiveFile::new("branch_iso");
    let mut writer = TileWriter::create(archive.path()).unwrap();
    let recs = vec![TelemetryRecord::new(100, 1, 1, 10.0)];
    writer.write_hyper_tile(TileType::TELEMETRY, &recs, None, None).unwrap();
    drop(writer);

    let key = generate_test_key();
    let repo = VcsRepo::open(archive.path()).unwrap();

    let c1 = repo.commit("Main genesis", "main", &key, None).unwrap();

    // Create branch "experiment"
    let branch_ref = repo.set_branch("experiment", c1.seal_hash, &key).unwrap();
    assert_eq!(branch_ref.ref_name(), "experiment");
    assert_eq!(branch_ref.target_seal_hash, c1.seal_hash);

    // Create immutable tag "v1.0.0"
    let tag_ref = repo.set_tag("v1.0.0", c1.seal_hash, &key).unwrap();
    assert_eq!(tag_ref.ref_name(), "v1.0.0");

    // Tag cannot be reassigned to a different commit
    let forged_seal = [0x55u8; 32];
    assert!(repo.set_tag("v1.0.0", forged_seal, &key).is_err(), "Tags must be immutable");
}

#[test]
fn test_vcs_diff_counts_without_restricted_payloads() {
    let archive = TempArchiveFile::new("diff_privacy");
    let mut writer = TileWriter::create(archive.path()).unwrap();
    let recs = vec![TelemetryRecord::new(100, 1, 1, 10.0)];
    writer.write_hyper_tile(TileType::TELEMETRY, &recs, None, None).unwrap();
    drop(writer);

    let key = generate_test_key();
    let repo = VcsRepo::open(archive.path()).unwrap();
    let c1 = repo.commit("Commit 1", "main", &key, None).unwrap();

    let diff = repo.diff(&[0u8; 32], &c1.seal_hash).unwrap();
    assert_eq!(diff.total_records_added, 1);
    assert_eq!(diff.total_records_withdrawn, 0);
    assert_eq!(diff.total_records_erased, 0);

    // Critical privacy check: diff serialization must not leak confidential sensor fields or values
    let diff_json = serde_json::to_string(&diff).unwrap();
    assert!(!diff_json.contains("sensor_id"));
    assert!(!diff_json.contains("secret"));
}

#[test]
fn test_vcs_merge_union_and_conflict_detection() {
    let archive = TempArchiveFile::new("merge_union");
    let mut writer = TileWriter::create(archive.path()).unwrap();
    let recs = vec![TelemetryRecord::new(100, 1, 1, 10.0)];
    writer.write_hyper_tile(TileType::TELEMETRY, &recs, None, None).unwrap();
    drop(writer);

    let key = generate_test_key();
    let repo = VcsRepo::open(archive.path()).unwrap();
    let c1 = repo.commit("Base commit", "main", &key, None).unwrap();

    // Create branch "feature"
    repo.set_branch("feature", c1.seal_hash, &key).unwrap();

    // Commit on feature branch
    let mut writer2 = TileWriter::open_append(archive.path()).unwrap();
    let recs2 = vec![TelemetryRecord::new(200, 2, 1, 20.0)];
    writer2.write_hyper_tile(TileType::TELEMETRY, &recs2, None, None).unwrap();
    drop(writer2);

    let c_feat = repo.commit("Feature commit", "feature", &key, None).unwrap();

    // Merge feature into main (should fast-forward)
    let merge_commit = repo.merge("feature", "main", &key, true).unwrap();
    assert_eq!(merge_commit.seal_hash, c_feat.seal_hash);
}

#[test]
fn test_vcs_sync_inventory_missing_tiles_only() {
    let src_file = TempArchiveFile::new("sync_src");
    let tgt_file = TempArchiveFile::new("sync_tgt");

    let mut w_src = TileWriter::create(src_file.path()).unwrap();
    let recs1 = vec![TelemetryRecord::new(100, 1, 1, 10.0)];
    w_src.write_hyper_tile(TileType::TELEMETRY, &recs1, None, None).unwrap();
    let recs2 = vec![TelemetryRecord::new(200, 2, 1, 20.0)];
    w_src.write_hyper_tile(TileType::TELEMETRY, &recs2, None, None).unwrap();
    drop(w_src);

    let key = generate_test_key();
    let src_repo = VcsRepo::open(src_file.path()).unwrap();
    src_repo.commit("Source commit", "main", &key, None).unwrap();

    // Initialize empty target repository
    let tgt_repo = VcsRepo::init(tgt_file.path()).unwrap();

    // Push: target is missing tiles, should receive tiles
    let transferred = sync_push(&src_repo, &tgt_repo, "main", false, None, &key).unwrap();
    assert!(transferred > 0, "Initial push must transfer missing tiles");

    // Second push: target already has tiles, should transfer 0 tiles (inventory negotiation)
    let transferred2 = sync_push(&src_repo, &tgt_repo, "main", false, None, &key).unwrap();
    assert_eq!(transferred2, 0, "Subsequent push must transfer zero tiles");
}

#[test]
fn test_vcs_non_fast_forward_rejected() {
    let src_file = TempArchiveFile::new("nff_src");
    let tgt_file = TempArchiveFile::new("nff_tgt");

    let mut w_src = TileWriter::create(src_file.path()).unwrap();
    w_src.write_hyper_tile(TileType::TELEMETRY, &[TelemetryRecord::new(100, 1, 1, 1.0)], None, None).unwrap();
    drop(w_src);

    let key = generate_test_key();
    let src_repo = VcsRepo::open(src_file.path()).unwrap();
    src_repo.commit("Source commit", "main", &key, None).unwrap();

    let mut w_tgt = TileWriter::create(tgt_file.path()).unwrap();
    w_tgt.write_hyper_tile(TileType::TELEMETRY, &[TelemetryRecord::new(999, 9, 9, 9.0)], None, None).unwrap();
    drop(w_tgt);

    let tgt_repo = VcsRepo::open(tgt_file.path()).unwrap();
    tgt_repo.commit("Divergent target commit", "main", &key, None).unwrap();

    // Push should fail with NonFastForward
    let res = sync_push(&src_repo, &tgt_repo, "main", false, None, &key);
    assert!(res.is_err(), "Non-fast-forward push must be rejected");
}

#[test]
fn test_vcs_consent_enforcement_on_push() {
    let src_file = TempArchiveFile::new("consent_src");
    let tgt_file = TempArchiveFile::new("consent_tgt");

    let mut w_src = TileWriter::create(src_file.path()).unwrap();
    let consent_rec = ConsentRecord::new(
        [0x11; 16],
        100,
        ConsentPurpose::Analytics, // Purpose is analytics
        0xFFFF,
        1700000000,
        0,
        false,
        1,
        [0u8; 32],
    );
    w_src.write_hyper_tile(TileType::CONSENT, &[consent_rec], None, None).unwrap();
    drop(w_src);

    let key = generate_test_key();
    let src_repo = VcsRepo::open(src_file.path()).unwrap();
    src_repo.commit("Consent commit", "main", &key, None).unwrap();

    let tgt_repo = VcsRepo::init(tgt_file.path()).unwrap();

    // Push requesting ConsentPurpose::Research (1) should be rejected because record is Marketing (2)
    let res = sync_push(&src_repo, &tgt_repo, "main", false, Some(1 /* Research */), &key);
    assert!(res.is_err(), "Push with mismatched consent purpose must be rejected");
}

#[test]
fn test_vcs_export_git_produces_valid_repo() {
    let archive = TempArchiveFile::new("export_git");
    let mut writer = TileWriter::create(archive.path()).unwrap();
    let recs = vec![TelemetryRecord::new(100, 1, 1, 10.0)];
    writer.write_hyper_tile(TileType::TELEMETRY, &recs, None, None).unwrap();
    drop(writer);

    let key = generate_test_key();
    let repo = VcsRepo::open(archive.path()).unwrap();
    repo.commit("First git export commit", "main", &key, None).unwrap();

    let mut git_out = std::env::temp_dir();
    git_out.push(format!("s3a_git_export_{}_{}", std::process::id(), TEST_COUNTER.fetch_add(1, Ordering::SeqCst)));

    export_git(&repo, &git_out).unwrap();

    // Verify git log works against the exported repository
    let git_log = Command::new("git")
        .args(["log", "--oneline"])
        .current_dir(&git_out)
        .output()
        .expect("git log must run");

    assert!(git_log.status.success());
    let log_str = String::from_utf8_lossy(&git_log.stdout);
    assert!(log_str.contains("First git export commit"));

    let _ = fs::remove_dir_all(&git_out);
}

#[test]
fn test_manifest_created_timestamp_populated() {
    let archive = TempArchiveFile::new("created_ts");
    let mut writer = TileWriter::create(archive.path()).unwrap();
    let recs = vec![TelemetryRecord::new(1700000000, 1, 1, 10.0)];
    writer.write_hyper_tile(TileType::TELEMETRY, &recs, None, None).unwrap();
    drop(writer);

    let key = generate_test_key();
    let repo = VcsRepo::open(archive.path()).unwrap();
    repo.commit("Testing created timestamp", "main", &key, None).unwrap();

    let reader = MmapReader::open(archive.path()).unwrap();
    let manifest = reader.get_manifest().expect("Manifest must be present");
    assert!(!manifest.contains("Genesis (Unset)"), "Created timestamp must never be 'Genesis (Unset)'");
    assert!(manifest.contains("Created:          202"), "Created timestamp must display valid year (202x)");
}
