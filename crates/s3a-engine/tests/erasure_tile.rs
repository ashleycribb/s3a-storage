//! Comprehensive tests for Right-to-be-Forgotten Erasure Tile (Tile 73 / 0x0049)
//!
//! Validates:
//! 1. erase -> record gone after compaction
//! 2. receipt remains and verifies
//! 3. legal_hold blocks purge/erasure
//! 4. chain hash verified
//! 5. seal verify passes after purge
//! 6. re-erasing same record is idempotent
//! 7. human/admin authorization gate (AI agents rejected)

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use s3a_core::consent::{actor_id_to_subject_id, ConsentPurpose};
use s3a_core::erasure::ErasureReason;
use s3a_core::TileType;
use s3a_engine::{
    seal_archive, verify_sealed_archive, MmapReader, QuerySieve, S3ACrudEngine, TelemetryRecord,
};
use sha2::{Digest, Sha256};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TestTempFile {
    path: PathBuf,
}

impl TestTempFile {
    fn new(suffix: &str) -> Self {
        let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let mut path = std::env::temp_dir();
        path.push(format!(
            "s3a_erasure_test_{}_{}_{}_{}.s3a",
            suffix,
            std::process::id(),
            ts,
            id
        ));
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestTempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn create_sample_telemetry(
    sensor_id: u32,
    metric_id: u32,
    timestamp: u64,
    value: f64,
) -> TelemetryRecord {
    let mut rec = TelemetryRecord::default();
    rec.sensor_id = sensor_id;
    rec.metric_id = metric_id;
    rec.timestamp = timestamp;
    rec.value = value;
    rec
}

#[test]
fn test_erase_record_gone_after_compaction_and_receipt_remains_and_verifies() {
    let tmp = TestTempFile::new("erase_purge");
    let engine = S3ACrudEngine::open_or_create(tmp.path()).unwrap();

    let rec1 = create_sample_telemetry(101, 1, 1000, 42.0);
    let rec2 = create_sample_telemetry(102, 1, 1001, 84.0);
    let tile_id = engine.create_telemetry(&[rec1, rec2]).unwrap();

    // Verify both records are visible initially
    let reader = MmapReader::open(tmp.path()).unwrap();
    let sieve = QuerySieve::new(&reader);
    let read_initial = sieve.query_telemetry(0, 2000, None, None);
    assert_eq!(read_initial.len(), 2);
    drop(reader);

    // Compute SHA-256 hash of rec1
    let mut hasher = Sha256::new();
    hasher.update(bytemuck::bytes_of(&rec1));
    let rec1_hash: [u8; 32] = hasher.finalize().into();

    // Human/admin records erasure for rec1
    let receipt = engine
        .record_erasure(
            TileType::TELEMETRY,
            tile_id,
            0, // record index 0
            rec1_hash,
            ErasureReason::Withdrawal,
            9999, // Compliance officer ID
            1050, // Requested at
            1055, // Executed at
            true, // is_admin_or_human
        )
        .unwrap();

    assert!(receipt.verify_hash());

    // Immediately after erasure receipt, sieve hides the record even before compaction
    let reader = MmapReader::open(tmp.path()).unwrap();
    let sieve = QuerySieve::new(&reader);
    let read_hidden = sieve.query_telemetry(0, 2000, None, None);
    assert_eq!(read_hidden.len(), 1);
    assert_eq!(read_hidden[0].sensor_id, 102);
    drop(reader);

    // Perform compaction / purge
    let compacted_tiles = engine.purge_and_compact().unwrap();
    assert!(compacted_tiles >= 2); // telemetry tile + erasure receipt tile (+ schema tile)

    // After compaction, rec1 is physically gone
    let reader = MmapReader::open(tmp.path()).unwrap();
    let sieve = QuerySieve::new(&reader);
    let read_after = sieve.query_telemetry(0, 2000, None, None);
    assert_eq!(read_after.len(), 1);
    assert_eq!(read_after[0].sensor_id, 102);
    drop(reader);

    // Verify erasure receipt remains and verifies in the compacted archive
    let reader = MmapReader::open(tmp.path()).unwrap();
    let sieve = QuerySieve::new(&reader);
    let erasures = sieve.query_erasure_records(Some(&rec1_hash));
    assert_eq!(erasures.len(), 1);
    assert_eq!(erasures[0].erased_record_hash, rec1_hash);
    assert_eq!(erasures[0].authority, 9999);
    assert!(erasures[0].verify_hash());
    assert!(sieve.verify_erasure_chain().unwrap());
}

#[test]
fn test_legal_hold_blocks_erasure() {
    let tmp = TestTempFile::new("legal_hold_blocks");
    let engine = S3ACrudEngine::open_or_create(tmp.path()).unwrap();

    let sensor_id = 201u32;
    let subject_id = actor_id_to_subject_id(sensor_id as u64);

    // Participant grants consent WITH legal_hold active
    engine
        .grant_consent(
            subject_id,
            777, // study_id
            ConsentPurpose::Research,
            0,
            1000,
            0,
            true, // legal_hold = true!
            8888,
            true,
        )
        .unwrap();

    let rec1 = create_sample_telemetry(sensor_id, 1, 1000, 99.0);
    let tile_id = engine.create_telemetry(&[rec1]).unwrap();

    let mut hasher = Sha256::new();
    hasher.update(bytemuck::bytes_of(&rec1));
    let rec1_hash: [u8; 32] = hasher.finalize().into();

    // Attempting to record erasure while under legal hold must fail with PermissionDenied
    let res = engine.record_erasure(
        TileType::TELEMETRY,
        tile_id,
        0,
        rec1_hash,
        ErasureReason::Withdrawal,
        9999,
        1050,
        1055,
        true,
    );
    assert!(res.is_err());
    let err = res.err().unwrap();
    assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
    assert!(err.to_string().contains("legal hold"));

    // Also verify purge_telemetry_record is blocked
    let purge_res = engine.purge_telemetry_record(sensor_id, 1, 1000);
    assert!(purge_res.is_err());
    assert_eq!(
        purge_res.err().unwrap().kind(),
        std::io::ErrorKind::PermissionDenied
    );
}

#[test]
fn test_erasure_chain_hash_verified() {
    let tmp = TestTempFile::new("erasure_chain");
    let engine = S3ACrudEngine::open_or_create(tmp.path()).unwrap();

    let rec1 = create_sample_telemetry(301, 1, 1000, 1.0);
    let rec2 = create_sample_telemetry(302, 1, 1001, 2.0);
    let rec3 = create_sample_telemetry(303, 1, 1002, 3.0);
    let tile_id = engine.create_telemetry(&[rec1, rec2, rec3]).unwrap();

    let hash_of = |rec: &TelemetryRecord| -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(bytemuck::bytes_of(rec));
        hasher.finalize().into()
    };

    let e1 = engine
        .record_erasure(
            TileType::TELEMETRY,
            tile_id,
            0,
            hash_of(&rec1),
            ErasureReason::Withdrawal,
            9001,
            1100,
            1105,
            true,
        )
        .unwrap();
    assert_eq!(e1.prev_erasure_hash, [0u8; 32]); // Genesis

    let e2 = engine
        .record_erasure(
            TileType::TELEMETRY,
            tile_id,
            1,
            hash_of(&rec2),
            ErasureReason::RetentionExpiry,
            9001,
            1200,
            1205,
            true,
        )
        .unwrap();
    assert_eq!(e2.prev_erasure_hash, e1.erasure_hash); // Chained

    let e3 = engine
        .record_erasure(
            TileType::TELEMETRY,
            tile_id,
            2,
            hash_of(&rec3),
            ErasureReason::LegalRequest,
            9001,
            1300,
            1305,
            true,
        )
        .unwrap();
    assert_eq!(e3.prev_erasure_hash, e2.erasure_hash); // Chained

    let reader = MmapReader::open(tmp.path()).unwrap();
    let sieve = QuerySieve::new(&reader);
    assert!(sieve.verify_erasure_chain().unwrap());
}

#[test]
fn test_re_erasing_same_record_is_idempotent() {
    let tmp = TestTempFile::new("idempotent_erasure");
    let engine = S3ACrudEngine::open_or_create(tmp.path()).unwrap();

    let rec = create_sample_telemetry(401, 1, 1000, 123.0);
    let tile_id = engine.create_telemetry(&[rec]).unwrap();

    let mut hasher = Sha256::new();
    hasher.update(bytemuck::bytes_of(&rec));
    let rec_hash: [u8; 32] = hasher.finalize().into();

    let first = engine
        .record_erasure(
            TileType::TELEMETRY,
            tile_id,
            0,
            rec_hash,
            ErasureReason::Withdrawal,
            9999,
            1000,
            1005,
            true,
        )
        .unwrap();

    // Call again with same target
    let second = engine
        .record_erasure(
            TileType::TELEMETRY,
            tile_id,
            0,
            rec_hash,
            ErasureReason::Withdrawal,
            9999,
            1000,
            1005,
            true,
        )
        .unwrap();

    assert_eq!(first, second);

    // Verify only 1 erasure record was recorded in the archive
    let reader = MmapReader::open(tmp.path()).unwrap();
    let sieve = QuerySieve::new(&reader);
    let all = sieve.query_erasure_records(None);
    assert_eq!(all.len(), 1);
}

#[test]
fn test_seal_verify_passes_after_purge() {
    let tmp = TestTempFile::new("seal_after_purge");
    let engine = S3ACrudEngine::open_or_create(tmp.path()).unwrap();

    let rec1 = create_sample_telemetry(501, 1, 1000, 10.0);
    let rec2 = create_sample_telemetry(502, 1, 1001, 20.0);
    let tile_id = engine.create_telemetry(&[rec1, rec2]).unwrap();

    let mut hasher = Sha256::new();
    hasher.update(bytemuck::bytes_of(&rec1));
    let rec1_hash: [u8; 32] = hasher.finalize().into();

    engine
        .record_erasure(
            TileType::TELEMETRY,
            tile_id,
            0,
            rec1_hash,
            ErasureReason::Withdrawal,
            9999,
            1050,
            1055,
            true,
        )
        .unwrap();

    // Compact archive
    engine.purge_and_compact().unwrap();

    // Seal archive to a new sealed file
    let sealed_tmp = TestTempFile::new("sealed_out");
    let seal_res = seal_archive(tmp.path(), sealed_tmp.path(), 16, 2);
    assert!(
        seal_res.is_ok(),
        "Seal must succeed after compaction and purge"
    );

    // Verify sealed archive passes 100%
    let report = verify_sealed_archive(sealed_tmp.path(), None).unwrap();
    assert!(report.is_sealed);
    assert!(report.warnings.is_empty());
}

#[test]
fn test_erasure_ai_agent_authorization_rejected() {
    let tmp = TestTempFile::new("auth_rejection");
    let engine = S3ACrudEngine::open_or_create(tmp.path()).unwrap();

    let rec = create_sample_telemetry(601, 1, 1000, 50.0);
    let tile_id = engine.create_telemetry(&[rec]).unwrap();

    let mut hasher = Sha256::new();
    hasher.update(bytemuck::bytes_of(&rec));
    let rec_hash: [u8; 32] = hasher.finalize().into();

    // Autonomous AI agent (is_admin_or_human = false) attempted write
    let res = engine.record_erasure(
        TileType::TELEMETRY,
        tile_id,
        0,
        rec_hash,
        ErasureReason::Withdrawal,
        1234, // Agent ID
        1000,
        1005,
        false, // AI Agent!
    );

    assert!(res.is_err());
    let err = res.err().unwrap();
    assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
    assert!(err.to_string().contains("never AI agents"));
}
