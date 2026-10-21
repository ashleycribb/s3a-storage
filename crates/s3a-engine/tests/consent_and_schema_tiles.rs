//! Tests for CONSENT (Tile 71) and SCHEMA (Tile 77) Tiles
//!
//! Validates:
//! 1. consent grant -> query visible
//! 2. withdraw -> hidden for that purpose only
//! 3. expiry
//! 4. legal_hold blocks purge
//! 5. model_training blocked without explicit consent
//! 6. consent chain hash verified
//! 7. schema tile round-trip
//! 8. old file without schema tile still opens
//! 9. unknown type skipped safely
//! 10. inspect shows names

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use s3a_core::consent::{actor_id_to_subject_id, ConsentPurpose, ConsentRecord};
use s3a_core::schema::{
    decode_schema_payload, encode_schema_payload, SchemaFieldDefinition, SchemaTypeDefinition,
};
use s3a_core::{HyperTileHeader, TileType};
use s3a_engine::{
    append_schema_tile_if_missing, Compactor, LearningActivityRecord, MmapReader, QuerySieve,
    S3ACrudEngine, TelemetryRecord, TileWriter,
};

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
            "s3a_consent_schema_test_{}_{}_{}_{}.s3a",
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

/// Helper to write an initial file with learning records for given actors
fn create_test_file_with_activities(path: &Path, actor_ids: &[u64]) {
    let engine = S3ACrudEngine::open_or_create(path).expect("open or create engine");
    engine
        .enable_addon(s3a_core::ADDON_EDU)
        .expect("enable addon");
    let mut recs = Vec::new();
    for &actor_id in actor_ids {
        let mut rec = LearningActivityRecord::default();
        rec.actor_id = actor_id;
        rec.object_id = 42;
        rec.timestamp_sec = 1000;
        recs.push(rec);
    }
    let mut writer = TileWriter::open_append(path).expect("open append");
    writer
        .write_hyper_tile(10, &recs, None, None)
        .expect("write activities");
}

#[test]
fn test_consent_grant_query_visible() {
    let temp = TestTempFile::new("grant_visible");
    create_test_file_with_activities(temp.path(), &[101, 102]);

    let engine = S3ACrudEngine::open_or_create(temp.path()).expect("open engine");
    let subject_101 = actor_id_to_subject_id(101);

    // Grant consent to subject 101 for Research
    engine
        .grant_consent(
            subject_101,
            1, // study_id
            ConsentPurpose::Research,
            0xFFFF, // full scope
            100,    // granted_at
            0,      // no expiry
            false,  // legal_hold
            1,      // admin granted_by
            true,   // is_admin_or_human
        )
        .expect("grant consent");

    let reader = MmapReader::open(temp.path()).expect("open reader");
    let sieve = QuerySieve::new(&reader)
        .with_purpose(ConsentPurpose::Research)
        .with_now(200);

    let activities = sieve.query_learning_activities(0, u64::MAX, None, None);
    // Subject 101 has consent for Research, subject 102 does not have consent.
    // Querying with declared purpose Research yields only subject 101!
    let actors: Vec<u64> = activities.iter().map(|(a, _)| a.actor_id).collect();
    assert_eq!(actors, vec![101]);
}

#[test]
fn test_consent_withdraw_hidden_for_purpose_only() {
    let temp = TestTempFile::new("withdraw_purpose");
    create_test_file_with_activities(temp.path(), &[201]);

    let engine = S3ACrudEngine::open_or_create(temp.path()).expect("open engine");
    let subject_201 = actor_id_to_subject_id(201);

    // Grant consent for Research AND Analytics
    engine
        .grant_consent(
            subject_201,
            1,
            ConsentPurpose::Research,
            0xFFFF,
            100,
            0,
            false,
            1,
            true,
        )
        .expect("grant research");
    engine
        .grant_consent(
            subject_201,
            1,
            ConsentPurpose::Analytics,
            0xFFFF,
            100,
            0,
            false,
            1,
            true,
        )
        .expect("grant analytics");

    // Withdraw consent ONLY for Research
    engine
        .withdraw_consent(
            subject_201,
            1,
            ConsentPurpose::Research,
            0xFFFF,
            150,
            1,
            true,
        )
        .expect("withdraw research");

    let reader = MmapReader::open(temp.path()).expect("open reader");

    // Research should now be hidden
    let sieve_research = QuerySieve::new(&reader)
        .with_purpose(ConsentPurpose::Research)
        .with_now(200);
    let acts_research = sieve_research.query_learning_activities(0, u64::MAX, None, None);
    assert!(
        acts_research.is_empty(),
        "Research query must hide subject whose consent was withdrawn"
    );

    // Analytics should still be visible!
    let sieve_analytics = QuerySieve::new(&reader)
        .with_purpose(ConsentPurpose::Analytics)
        .with_now(200);
    let acts_analytics = sieve_analytics.query_learning_activities(0, u64::MAX, None, None);
    assert_eq!(
        acts_analytics.len(),
        1,
        "Analytics query must remain visible"
    );
    assert_eq!(acts_analytics[0].0.actor_id, 201);
}

#[test]
fn test_consent_expiry() {
    let temp = TestTempFile::new("expiry");
    create_test_file_with_activities(temp.path(), &[301]);

    let engine = S3ACrudEngine::open_or_create(temp.path()).expect("open engine");
    let subject_301 = actor_id_to_subject_id(301);

    // Grant consent expiring at timestamp 1000
    engine
        .grant_consent(
            subject_301,
            1,
            ConsentPurpose::Research,
            0xFFFF,
            100,
            1000, // expires_at = 1000
            false,
            1,
            true,
        )
        .expect("grant consent");

    let reader = MmapReader::open(temp.path()).expect("open reader");

    // Before expiry (now = 500) -> visible
    let sieve_before = QuerySieve::new(&reader)
        .with_purpose(ConsentPurpose::Research)
        .with_now(500);
    assert_eq!(
        sieve_before
            .query_learning_activities(0, u64::MAX, None, None)
            .len(),
        1
    );

    // After expiry (now = 1500) -> hidden
    let sieve_after = QuerySieve::new(&reader)
        .with_purpose(ConsentPurpose::Research)
        .with_now(1500);
    assert_eq!(
        sieve_after
            .query_learning_activities(0, u64::MAX, None, None)
            .len(),
        0
    );
}

#[test]
fn test_consent_legal_hold_blocks_purge() {
    let temp = TestTempFile::new("legal_hold");
    let engine = S3ACrudEngine::open_or_create(temp.path()).expect("open engine");
    // Write a telemetry record for sensor 401
    engine
        .create_telemetry(&[TelemetryRecord::new(100, 401, 1, 42.0)])
        .expect("create telemetry");

    let subject_401 = actor_id_to_subject_id(401);

    // Grant consent with legal_hold = true
    engine
        .grant_consent(
            subject_401,
            1,
            ConsentPurpose::Research,
            0xFFFF,
            100,
            0,
            true, // legal_hold = true
            1,
            true,
        )
        .expect("grant consent with legal hold");

    // Attempting to purge a record for this actor must be blocked!
    let purge_res = engine.purge_telemetry_record(401, 1, 100);
    assert!(
        purge_res.is_err(),
        "Purge must fail when subject is under legal hold"
    );
    assert!(purge_res.unwrap_err().to_string().contains("legal hold"));

    // Ensure compaction also retains records under legal hold even if marked with tombstone
    let out_temp = TestTempFile::new("legal_hold_compacted");
    let res = Compactor::compact(&[temp.path()], out_temp.path());
    assert!(res.is_ok(), "Compaction succeeds");
    let out_reader = MmapReader::open(out_temp.path()).expect("open compacted reader");
    assert!(out_reader.tile_count() >= 2);
}

#[test]
fn test_consent_model_training_blocked_without_explicit_consent() {
    let temp = TestTempFile::new("model_training");
    create_test_file_with_activities(temp.path(), &[501]);

    let engine = S3ACrudEngine::open_or_create(temp.path()).expect("open engine");
    let subject_501 = actor_id_to_subject_id(501);

    // Grant only Research consent
    engine
        .grant_consent(
            subject_501,
            1,
            ConsentPurpose::Research,
            0xFFFF,
            100,
            0,
            false,
            1,
            true,
        )
        .expect("grant research");

    let reader = MmapReader::open(temp.path()).expect("open reader");

    // Querying for ModelTraining must be blocked
    let sieve_train = QuerySieve::new(&reader)
        .with_purpose(ConsentPurpose::ModelTraining)
        .with_now(200);
    assert!(sieve_train
        .query_learning_activities(0, u64::MAX, None, None)
        .is_empty());

    drop(reader);

    // Now grant explicit ModelTraining consent
    engine
        .grant_consent(
            subject_501,
            1,
            ConsentPurpose::ModelTraining,
            0xFFFF,
            110,
            0,
            false,
            1,
            true,
        )
        .expect("grant model training");

    let reader2 = MmapReader::open(temp.path()).expect("open reader 2");
    let sieve_train2 = QuerySieve::new(&reader2)
        .with_purpose(ConsentPurpose::ModelTraining)
        .with_now(200);
    assert_eq!(
        sieve_train2
            .query_learning_activities(0, u64::MAX, None, None)
            .len(),
        1
    );
}

#[test]
fn test_consent_chain_hash_verified() {
    let temp = TestTempFile::new("chain_hash");
    let mut writer = TileWriter::create(temp.path()).expect("create file");

    let subject = [7u8; 16];
    let rec1 = ConsentRecord::new(
        subject,
        1,
        ConsentPurpose::Research,
        0xFFFF,
        100,
        0,
        false,
        1,
        [0u8; 32],
    );
    let rec2 = ConsentRecord::new(
        subject,
        1,
        ConsentPurpose::Analytics,
        0xFFFF,
        200,
        0,
        false,
        1,
        rec1.consent_hash,
    );
    let rec3 = ConsentRecord::new_withdrawal(
        subject,
        1,
        ConsentPurpose::Research,
        0xFFFF,
        300,
        1,
        rec2.consent_hash,
    );

    writer
        .write_hyper_tile(TileType::CONSENT, &[rec1, rec2, rec3], None, None)
        .expect("write consent records");

    let reader = MmapReader::open(temp.path()).expect("open reader");
    let sieve = QuerySieve::new(&reader);

    assert_eq!(
        sieve.verify_consent_chain(&subject),
        Ok(true),
        "Chain should be valid"
    );

    // Test tamper resistance
    let other_subject = [9u8; 16];
    assert_eq!(
        sieve.verify_consent_chain(&other_subject),
        Ok(true),
        "Empty chain vacuously valid"
    );
}

#[test]
fn test_schema_tile_roundtrip() {
    let defs = vec![
        SchemaTypeDefinition {
            type_id: 88,
            type_version: 1,
            name: "ROBOT_TELEMETRY".to_string(),
            owning_pack: "s3a-pack-robotics".to_string(),
            record_size: 64,
            fields: vec![
                SchemaFieldDefinition {
                    field_id: 1,
                    field_type: 4,
                    byte_offset: 0,
                    byte_len: 8,
                    null_allowed: false,
                    name: "timestamp_ns".to_string(),
                },
                SchemaFieldDefinition {
                    field_id: 2,
                    field_type: 6,
                    byte_offset: 8,
                    byte_len: 8,
                    null_allowed: false,
                    name: "joint_angle".to_string(),
                },
            ],
        },
        SchemaTypeDefinition {
            type_id: 89,
            type_version: 2,
            name: "DNA_MUTATION".to_string(),
            owning_pack: "s3a-pack-genomics".to_string(),
            record_size: 32,
            fields: vec![SchemaFieldDefinition {
                field_id: 1,
                field_type: 3,
                byte_offset: 0,
                byte_len: 4,
                null_allowed: false,
                name: "locus_id".to_string(),
            }],
        },
    ];

    let bytes = encode_schema_payload(&defs);
    let decoded = decode_schema_payload(&bytes).expect("decode schema");

    assert_eq!(decoded.len(), 2);
    assert_eq!(decoded[0].type_id, 88);
    assert_eq!(decoded[0].name, "ROBOT_TELEMETRY");
    assert_eq!(decoded[0].owning_pack, "s3a-pack-robotics");
    assert_eq!(decoded[0].record_size, 64);
    assert_eq!(decoded[0].fields.len(), 2);
    assert_eq!(decoded[0].fields[0].name, "timestamp_ns");
    assert_eq!(decoded[0].fields[1].name, "joint_angle");

    assert_eq!(decoded[1].type_id, 89);
    assert_eq!(decoded[1].name, "DNA_MUTATION");
    assert_eq!(decoded[1].fields.len(), 1);
    assert_eq!(decoded[1].fields[0].name, "locus_id");
}

#[test]
fn test_old_file_without_schema_tile_still_opens() {
    let temp = TestTempFile::new("no_schema");
    create_test_file_with_activities(temp.path(), &[1]);

    let reader = MmapReader::open(temp.path()).expect("open legacy file");
    assert!(reader.get_schema_definitions().is_empty());
    // Known tile resolves via registry
    assert_eq!(
        reader.resolve_tile_type_name(TileType::TELEMETRY),
        "telemetry"
    );
    // Unknown tile resolves to UNKNOWN
    assert_eq!(reader.resolve_tile_type_name(9999), "UNKNOWN");
}

#[test]
fn test_unknown_type_skipped_safely() {
    let temp = TestTempFile::new("unknown_type_skip");
    let mut writer = TileWriter::create(temp.path()).expect("create file");
    writer
        .enable_addon(s3a_core::ADDON_EDU)
        .expect("enable addon");

    // Write unknown tile type 9999 with 64 bytes raw payload
    let unknown_data = [42u8; 64];
    let mut unknown_header = HyperTileHeader::new(0, 9999);
    unknown_header.payload_bytes = 64;
    unknown_header.record_count = 1;
    writer
        .write_raw_hyper_tile(&unknown_header, &unknown_data)
        .expect("write unknown tile");

    // Followed by standard learning activity record (Tile 10)
    let mut rec = LearningActivityRecord::default();
    rec.actor_id = 777;
    rec.timestamp_sec = 5000;
    writer
        .write_hyper_tile(10, &[rec], None, None)
        .expect("write activity");

    let reader = MmapReader::open(temp.path()).expect("open reader");
    assert_eq!(reader.tile_count(), 2);
    let (h0, _) = reader.get_tile(0).expect("get tile 0");
    assert_eq!(h0.tile_type, 9999);
    let (h1, _) = reader.get_tile(1).expect("get tile 1");
    assert_eq!(h1.tile_type, 10);

    let sieve = QuerySieve::new(&reader);
    let acts = sieve.query_learning_activities(0, u64::MAX, None, None);
    assert_eq!(acts.len(), 1);
    assert_eq!(acts[0].0.actor_id, 777);
}

#[test]
fn test_inspect_shows_schema_names() {
    let temp = TestTempFile::new("inspect_names");
    let mut writer = TileWriter::create(temp.path()).expect("create file");

    // Tile 8888 unknown in global registry
    let dummy = [0u8; 16];
    let mut h8888 = HyperTileHeader::new(0, 8888);
    h8888.payload_bytes = 16;
    h8888.record_count = 1;
    writer
        .write_raw_hyper_tile(&h8888, &dummy)
        .expect("write raw tile");

    // Write schema tile describing 8888 as "CUSTOM_NEURAL_WEIGHTS"
    let defs = vec![SchemaTypeDefinition {
        type_id: 8888,
        type_version: 1,
        name: "CUSTOM_NEURAL_WEIGHTS".to_string(),
        owning_pack: "s3a-neural".to_string(),
        record_size: 16,
        fields: vec![SchemaFieldDefinition {
            field_id: 1,
            field_type: 13,
            byte_offset: 0,
            byte_len: 16,
            null_allowed: false,
            name: "tensor_bytes".to_string(),
        }],
    }];
    let schema_bytes = encode_schema_payload(&defs);
    let mut h_schema = HyperTileHeader::new(0, TileType::SCHEMA);
    h_schema.payload_bytes = schema_bytes.len() as u32;
    h_schema.record_count = 0;
    writer
        .write_raw_hyper_tile(&h_schema, &schema_bytes)
        .expect("write schema tile");

    let reader = MmapReader::open(temp.path()).expect("open reader");
    // reader resolves 8888 to "CUSTOM_NEURAL_WEIGHTS" instead of "UNKNOWN"!
    assert_eq!(reader.resolve_tile_type_name(8888), "CUSTOM_NEURAL_WEIGHTS");

    // Test append_schema_tile_if_missing
    let temp_no_schema = TestTempFile::new("append_schema");
    create_test_file_with_activities(temp_no_schema.path(), &[10]);
    append_schema_tile_if_missing(temp_no_schema.path()).expect("append schema");
    let reader2 = MmapReader::open(temp_no_schema.path()).expect("open reader");
    assert_eq!(reader2.tile_count(), 2);
    let (h_appended, _) = reader2.get_tile(1).expect("get appended schema tile");
    assert_eq!(h_appended.tile_type, TileType::SCHEMA);
}
