use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use s3a_core::{S3ACoordinate, ADDON_EDU};
use s3a_engine::{MmapReader, S3ACrudEngine};
use s3a_pack_scholar::{AiTraceRecord, HumanLrsRecord, ScholarCrudExt};

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
            "s3a_scholar_comp_{}_{}_{}_{}.s3a",
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
        let mut q = self.path.as_os_str().to_os_string();
        q.push(".quarantine");
        let _ = fs::remove_file(PathBuf::from(q));
    }
}

#[test]
fn test_scholar_session_uuid_join_between_tiles_10_and_11_identical_after_compaction() {
    let temp = TestTempFile::new("join_compaction");
    let path = temp.path();

    let engine = S3ACrudEngine::open_or_create(path).unwrap();
    engine.enable_addon(ADDON_EDU).unwrap();

    let session_uuid_1 = [0x1122_3344_5566_7788, 0x99AA_BBCC_DDEE_FF00];
    let session_uuid_2 = [0xCAFE_BABE_DEAD_BEEF, 0x0123_4567_89AB_CDEF];

    // Create Tile 10: Human decisions
    let human1 = HumanLrsRecord::new(
        session_uuid_1,
        1700000001,
        0x05E8_1001, // actor_hash
        10,          // verb_id: approved draft
        1.0,         // confidence
        S3ACoordinate::new(0, 2, 0),
        501,
    );
    let human2 = HumanLrsRecord::new(
        session_uuid_2,
        1700000002,
        0x05E8_1002,
        11, // verb_id: provided analogy
        0.85,
        S3ACoordinate::new(0, 2, 1),
        502,
    );
    engine.create_human_lrs(&[human1, human2]).unwrap();

    // Create Tile 11: AI trace logs
    let ai1 = AiTraceRecord::new(
        session_uuid_1,
        1700000003,
        0x05E8_2001, // agent_id
        100,         // tool_id
        0.98,        // verification_score
        S3ACoordinate::new(0, 1, 0),
        9999,
    );
    let ai2 = AiTraceRecord::new(
        session_uuid_2,
        1700000004,
        0x05E8_2002,
        101,
        0.95,
        S3ACoordinate::new(0, 1, 1),
        8888,
    );
    engine.create_ai_traces(&[ai1, ai2]).unwrap();

    // Query joined session bundle before compaction
    let (humans_before, ais_before) = engine.query_session_bundle(session_uuid_1).unwrap();
    assert_eq!(
        humans_before.len(),
        1,
        "Must find exactly 1 human decision for session 1"
    );
    assert_eq!(
        ais_before.len(),
        1,
        "Must find exactly 1 AI trace for session 1"
    );
    assert_eq!(humans_before[0].0.session_uuid, session_uuid_1);
    assert_eq!(ais_before[0].0.session_uuid, session_uuid_1);
    assert_eq!(
        humans_before[0].0.session_uuid,
        ais_before[0].0.session_uuid
    );

    // Also verify session 2 before compaction
    let (humans2_before, ais2_before) = engine.query_session_bundle(session_uuid_2).unwrap();
    assert_eq!(humans2_before.len(), 1);
    assert_eq!(ais2_before.len(), 1);

    // Execute Checkpoint-Gated Compaction via S3ACrudEngine
    let compacted_tile_count = engine.purge_and_compact().unwrap();
    assert!(
        compacted_tile_count >= 2,
        "Compacted archive must contain preserved domain tiles"
    );

    // Verify archive checksums pass post-compaction
    let reader = MmapReader::open(path).unwrap();
    assert!(
        reader.verify_checksums().is_ok(),
        "Post-compaction checksums must be valid"
    );

    // Query joined session bundle after compaction
    let (humans_after, ais_after) = engine.query_session_bundle(session_uuid_1).unwrap();
    assert_eq!(
        humans_after.len(),
        1,
        "Must find exactly 1 human decision for session 1 post-compaction"
    );
    assert_eq!(
        ais_after.len(),
        1,
        "Must find exactly 1 AI trace for session 1 post-compaction"
    );

    // Join result must be byte-for-byte and field-for-field identical!
    assert_eq!(
        humans_before[0].0, humans_after[0].0,
        "HumanLrsRecord must be identical before and after compaction"
    );
    assert_eq!(
        ais_before[0].0, ais_after[0].0,
        "AiTraceRecord must be identical before and after compaction"
    );
    assert_eq!(
        humans_after[0].0.session_uuid, ais_after[0].0.session_uuid,
        "Session UUID join must hold post-compaction"
    );

    // Check session 2 after compaction as well
    let (humans2_after, ais2_after) = engine.query_session_bundle(session_uuid_2).unwrap();
    assert_eq!(humans2_before[0].0, humans2_after[0].0);
    assert_eq!(ais2_before[0].0, ais2_after[0].0);
}
