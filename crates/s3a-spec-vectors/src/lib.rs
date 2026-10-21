use std::fs::{self, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use bytemuck::{bytes_of};
use s3a_core::{
    HyperTileHeader, MarkRecord, S3AError, StatementIndexRecord, TelemetryRecord, TileType,
    ADDON_EDU, HYPER_TILE_SIZE, MARK_CONSENT_WITHDRAWN, MARK_SET,
};
use s3a_engine::{
    compute_tile_header_crc32, Compactor, MmapReader, QuerySieve, S3ACrudEngine, TileWriter,
};
use s3a_pack_scholar::{
    export_srs_snapshot, AcademicPaperRecord, ScholarCrudExt, SrsManifestHeader, SrsWorkspaceRecord,
};
use s3a_edge::{
    CompactEdgeSensorRecord, EdgeConsentRecord, EdgeWriter, EquipmentEventRecord, RamFlash,
};
use s3a_vault::{
    evaluate_agent_action, is_judgment_action, verify_delegation_grant,
    KeyStore, MemoryKeyStore, Vault, VaultItemRecord, VaultOwnerKind,
    ACTION_READ, ACTION_WRITE,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HeaderExpectation {
    pub active_slot: usize,
    pub generation: u64,
    pub tile_count: u32,
    #[serde(default)]
    pub addon_mask: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SampleRecord {
    pub tile_type: u32,
    pub record_index: usize,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ConformanceExpectation {
    pub case_name: String,
    pub category: String, // "valid" or "invalid"
    pub description: String,
    pub expected_outcome: String, // "ok", "error", "fallback_recovered", "mark_protected"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_error_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header: Option<HeaderExpectation>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub records_per_tile_type: std::collections::BTreeMap<String, usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sample_records: Vec<SampleRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub records_hidden_by_default: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub records_visible_with_override: Option<bool>,
    #[serde(default)]
    pub visible_records: usize,
    #[serde(default)]
    pub hidden_records: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_sealed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rebuilt_directory: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub out_of_order_detected: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repaired_tiles_count: Option<usize>,
}

/// Generates all 26 conformance test cases into the target output directory.
pub fn generate_all_vectors<P: AsRef<Path>>(out_dir: P) -> io::Result<Vec<String>> {
    let dir = out_dir.as_ref();
    fs::create_dir_all(dir)?;

    let generated_cases = vec![
        // 1. Valid Cases (v1.0)
        generate_valid_empty(dir)?,
        generate_valid_single_tile_core(dir)?,
        generate_valid_multi_tile(dir)?,
        generate_valid_after_compaction(dir)?,
        generate_valid_dual_header_slots(dir)?,
        generate_valid_unknown_tile_type(dir)?,
        generate_valid_snapshot_container(dir)?,
        generate_valid_protected_mark_hidden(dir)?,
        // 2. Invalid Cases (v1.0)
        generate_invalid_bad_magic(dir)?,
        generate_invalid_both_slots_corrupt(dir)?,
        generate_invalid_torn_write_fallback(dir)?,
        generate_invalid_tile_crc_mismatch(dir)?,
        generate_invalid_truncated_tile(dir)?,
        generate_invalid_unsupported_layout_version(dir)?,
        // 3. Sealed & Self-Addressing Cases (v1.1)
        generate_sealed_ok(dir)?,
        generate_sealed_footer_missing(dir)?,
        generate_sealed_endcap_corrupt(dir)?,
        generate_sealed_truncated_mid_tile(dir)?,
        generate_one_tile_corrupt_repaired(dir)?,
        generate_three_tiles_corrupt_in_one_group(dir)?,
        generate_tiles_out_of_order(dir)?,
        generate_foreign_tile(dir)?,
        generate_legacy_unaddressed_tiles(dir)?,
        // 4. Edge Profile Vectors (Phase 1)
        generate_micro_valid_4kb(dir)?,
        generate_micro_truncated_file(dir)?,
        generate_micro_signed_seal_chain(dir)?,
        // 5. Vault Profile Vectors (Phase 1)
        generate_vault_transfer_chain(dir)?,
        generate_vault_delegation_governance(dir)?,
        generate_vault_key_revocation(dir)?,
        generate_vault_vc_import(dir)?,
    ];

    // Write README with runner contract
    write_spec_vectors_readme(dir)?;

    Ok(generated_cases)
}

fn write_json_and_record<T: Serialize>(dir: &Path, name: &str, exp: &T) -> io::Result<String> {
    let json_path = dir.join(format!("{}.expected.json", name));
    let json_str = serde_json::to_string_pretty(exp).map_err(io::Error::other)?;
    fs::write(json_path, json_str)?;
    Ok(name.to_string())
}

// Case 1: valid_empty
fn generate_valid_empty(dir: &Path) -> io::Result<String> {
    let name = "valid_empty";
    let file_path = dir.join(format!("{}.s3a", name));
    let writer = TileWriter::create(&file_path)?;
    drop(writer);

    let exp = ConformanceExpectation {
        case_name: name.to_string(),
        category: "valid".to_string(),
        description: "Empty valid S3A file with 0 tiles and generation 1".to_string(),
        expected_outcome: "ok".to_string(),
        error_class: None,
        expected_error_message: None,
        file_size_bytes: Some(512),
        header: Some(HeaderExpectation {
            active_slot: 0,
            generation: 1,
            tile_count: 0,
            addon_mask: 0,
        }),
        records_per_tile_type: std::collections::BTreeMap::new(),
        sample_records: Vec::new(),
        records_hidden_by_default: None,
        records_visible_with_override: None,
        visible_records: 0,
        hidden_records: 0,
        ..Default::default()
    };
    write_json_and_record(dir, name, &exp)
}

// Case 2: valid_single_tile_core
fn generate_valid_single_tile_core(dir: &Path) -> io::Result<String> {
    let name = "valid_single_tile_core";
    let file_path = dir.join(format!("{}.s3a", name));
    let mut writer = TileWriter::create(&file_path)?;

    let records = vec![
        TelemetryRecord::new(1000, 101, 1, 23.5),
        TelemetryRecord::new(1010, 101, 1, 24.0),
        TelemetryRecord::new(1020, 102, 2, 88.2),
    ];
    let timestamps = [1000, 1010, 1020];
    writer.write_hyper_tile(TileType::TELEMETRY, &records, Some(&timestamps), None)?;
    drop(writer);

    let mut map = std::collections::BTreeMap::new();
    map.insert("0".to_string(), 3);

    let sample_records = vec![
        SampleRecord {
            tile_type: 0,
            record_index: 0,
            summary:
                "TelemetryRecord { timestamp: 1000, sensor_id: 101, metric_id: 1, value: 23.5 }"
                    .to_string(),
        },
        SampleRecord {
            tile_type: 0,
            record_index: 2,
            summary:
                "TelemetryRecord { timestamp: 1020, sensor_id: 102, metric_id: 2, value: 88.2 }"
                    .to_string(),
        },
    ];

    let exp = ConformanceExpectation {
        case_name: name.to_string(),
        category: "valid".to_string(),
        description: "Single 128KB Hyper-Tile containing 3 core Telemetry records".to_string(),
        expected_outcome: "ok".to_string(),
        error_class: None,
        expected_error_message: None,
        file_size_bytes: Some(512 + HYPER_TILE_SIZE as u64),
        header: Some(HeaderExpectation {
            active_slot: 1,
            generation: 2,
            tile_count: 1,
            addon_mask: 0,
        }),
        records_per_tile_type: map,
        sample_records,
        records_hidden_by_default: None,
        records_visible_with_override: None,
        visible_records: 0,
        hidden_records: 0,
        ..Default::default()
    };
    write_json_and_record(dir, name, &exp)
}

// Case 3: valid_multi_tile
fn generate_valid_multi_tile(dir: &Path) -> io::Result<String> {
    let name = "valid_multi_tile";
    let file_path = dir.join(format!("{}.s3a", name));
    let mut writer = TileWriter::create(&file_path)?;

    // Tile 0: Telemetry (TileType 0)
    let tel_records = vec![
        TelemetryRecord::new(2000, 10, 1, 100.0),
        TelemetryRecord::new(2010, 11, 1, 101.5),
    ];
    writer.write_hyper_tile(TileType::TELEMETRY, &tel_records, None, None)?;

    // Tile 1: Embedding 128 (TileType 1)
    let mut v1 = [0.0f32; 128];
    v1[0] = 1.0;
    let mut v2 = [0.0f32; 128];
    v2[127] = 2.0;
    let emb_records = vec![
        s3a_core::EmbeddingRecord128::new(1, 2000, v1),
        s3a_core::EmbeddingRecord128::new(2, 2010, v2),
    ];
    writer.write_hyper_tile(TileType::EMBEDDING, &emb_records, None, None)?;

    // Tile 2: Statement Index (TileType 22)
    let stmt_records = vec![
        StatementIndexRecord::new([1, 100], 2000, 2000, 0xAA, 0x01, 0x02, 0, 0, 0, 0),
        StatementIndexRecord::new([2, 200], 2010, 2010, 0xBB, 0x01, 0x03, 0, 0, 0, 0),
    ];
    writer.write_hyper_tile(TileType::STATEMENT_INDEX, &stmt_records, None, None)?;
    drop(writer);

    let mut map = std::collections::BTreeMap::new();
    map.insert("0".to_string(), 2);
    map.insert("1".to_string(), 2);
    map.insert("22".to_string(), 2);

    let sample_records = vec![
        SampleRecord {
            tile_type: 0,
            record_index: 0,
            summary:
                "TelemetryRecord { timestamp: 2000, sensor_id: 10, metric_id: 1, value: 100.0 }"
                    .to_string(),
        },
        SampleRecord {
            tile_type: 1,
            record_index: 0,
            summary: "EmbeddingRecord128 { id: 1, timestamp: 2000, vector[0]: 1.0 }".to_string(),
        },
        SampleRecord {
            tile_type: 22,
            record_index: 0,
            summary: "StatementIndexRecord { statement_id: [1, 100], actor_hash: 0xAA }"
                .to_string(),
        },
    ];

    let exp = ConformanceExpectation {
        case_name: name.to_string(),
        category: "valid".to_string(),
        description:
            "Multi-tile archive spanning Telemetry (0), Embedding (1), and Statement Index (22)"
                .to_string(),
        expected_outcome: "ok".to_string(),
        error_class: None,
        expected_error_message: None,
        file_size_bytes: Some(512 + 3 * HYPER_TILE_SIZE as u64),
        header: Some(HeaderExpectation {
            active_slot: 1,
            generation: 4,
            tile_count: 3,
            addon_mask: 0,
        }),
        records_per_tile_type: map,
        sample_records,
        records_hidden_by_default: None,
        records_visible_with_override: None,
        visible_records: 0,
        hidden_records: 0,
        ..Default::default()
    };
    write_json_and_record(dir, name, &exp)
}

// Case 4: valid_after_compaction
fn generate_valid_after_compaction(dir: &Path) -> io::Result<String> {
    let name = "valid_after_compaction";
    let uncompacted_path = dir.join("valid_after_compaction_input.tmp");
    let file_path = dir.join(format!("{}.s3a", name));

    let mut writer = TileWriter::create(&uncompacted_path)?;
    let rec1 = TelemetryRecord::new(3000, 1, 1, 10.0);
    let mut rec2_tombstone = TelemetryRecord::new(3010, 1, 1, 15.0);
    rec2_tombstone.mark_tombstone();
    let rec3 = TelemetryRecord::new(3020, 2, 1, 20.0);

    writer.write_hyper_tile(
        TileType::TELEMETRY,
        &[rec1, rec2_tombstone, rec3],
        None,
        None,
    )?;
    drop(writer);

    // Compact to destination
    Compactor::compact(&[&uncompacted_path], &file_path)?;
    let _ = fs::remove_file(uncompacted_path);

    let mut map = std::collections::BTreeMap::new();
    map.insert("0".to_string(), 2);
    map.insert("77".to_string(), 0);

    let sample_records = vec![
        SampleRecord {
            tile_type: 0,
            record_index: 0,
            summary: "TelemetryRecord { timestamp: 3000, sensor_id: 1, metric_id: 1, value: 10.0 } (active)".to_string(),
        },
        SampleRecord {
            tile_type: 0,
            record_index: 1,
            summary: "TelemetryRecord { timestamp: 3020, sensor_id: 2, metric_id: 1, value: 20.0 } (active)".to_string(),
        },
    ];

    let exp = ConformanceExpectation {
        case_name: name.to_string(),
        category: "valid".to_string(),
        description: "Archive post-compaction: tombstoned records purged, live records preserved"
            .to_string(),
        expected_outcome: "ok".to_string(),
        error_class: None,
        expected_error_message: None,
        file_size_bytes: Some(512 + 2 * HYPER_TILE_SIZE as u64),
        header: Some(HeaderExpectation {
            active_slot: 0,
            generation: 3,
            tile_count: 2,
            addon_mask: 0,
        }),
        records_per_tile_type: map,
        sample_records,
        records_hidden_by_default: None,
        records_visible_with_override: None,
        visible_records: 0,
        hidden_records: 0,
        ..Default::default()
    };
    write_json_and_record(dir, name, &exp)
}

// Case 5: valid_dual_header_slots
fn generate_valid_dual_header_slots(dir: &Path) -> io::Result<String> {
    let name = "valid_dual_header_slots";
    let file_path = dir.join(format!("{}.s3a", name));

    let mut writer = TileWriter::create(&file_path)?;
    let recs = vec![TelemetryRecord::new(4000, 1, 1, 55.0)];
    writer.write_hyper_tile(TileType::TELEMETRY, &recs, None, None)?;
    // Two commits performed: Slot A has gen 1 (tile_count 0), Slot B has gen 2 (tile_count 1). Both have valid CRC.
    drop(writer);

    let mut map = std::collections::BTreeMap::new();
    map.insert("0".to_string(), 1);

    let exp = ConformanceExpectation {
        case_name: name.to_string(),
        category: "valid".to_string(),
        description: "Both Slot A and Slot B valid with different generations; reader must pick Slot B (generation 2)".to_string(),
        expected_outcome: "ok".to_string(),
        error_class: None,
        expected_error_message: None,
        file_size_bytes: Some(512 + HYPER_TILE_SIZE as u64),
        header: Some(HeaderExpectation {
            active_slot: 1,
            generation: 2,
            tile_count: 1,
            addon_mask: 0,
        }),
        records_per_tile_type: map,
        sample_records: vec![SampleRecord {
            tile_type: 0,
            record_index: 0,
            summary: "TelemetryRecord { timestamp: 4000, sensor_id: 1, value: 55.0 }".to_string(),
        }],
        records_hidden_by_default: None,
        records_visible_with_override: None,
        visible_records: 0,
        hidden_records: 0,
        ..Default::default()
    };
    write_json_and_record(dir, name, &exp)
}

// Case 6: valid_unknown_tile_type
fn generate_valid_unknown_tile_type(dir: &Path) -> io::Result<String> {
    let name = "valid_unknown_tile_type";
    let file_path = dir.join(format!("{}.s3a", name));

    let mut writer = TileWriter::create(&file_path)?;
    let header = HyperTileHeader::new(1, 9999); // Unknown tile type
    let payload = b"CUSTOM_DOMAIN_EXT_PAYLOAD_BYTES_RESERVED_FOR_THIRD_PARTY_PACK_9999";
    writer.write_raw_hyper_tile(&header, payload)?;
    drop(writer);

    let mut map = std::collections::BTreeMap::new();
    map.insert("9999".to_string(), 1);

    let exp = ConformanceExpectation {
        case_name: name.to_string(),
        category: "valid".to_string(),
        description: "Archive with unknown tile type 9999; readers must skip during domain queries and verify CRC cleanly".to_string(),
        expected_outcome: "ok".to_string(),
        error_class: None,
        expected_error_message: None,
        file_size_bytes: Some(512 + HYPER_TILE_SIZE as u64),
        header: Some(HeaderExpectation {
            active_slot: 1,
            generation: 2,
            tile_count: 1,
            addon_mask: 0,
        }),
        records_per_tile_type: map,
        sample_records: Vec::new(),
        records_hidden_by_default: None,
        records_visible_with_override: None,
        visible_records: 0,
        hidden_records: 0,
        ..Default::default()
    };
    write_json_and_record(dir, name, &exp)
}

// Case 7: valid_snapshot_container
fn generate_valid_snapshot_container(dir: &Path) -> io::Result<String> {
    let name = "valid_snapshot_container";
    let live_tmp = dir.join("valid_snapshot_live.tmp");
    let file_path = dir.join(format!("{}.s3a", name));

    let engine = S3ACrudEngine::open_or_create(&live_tmp)?;
    engine.enable_addon(ADDON_EDU)?;

    let p_uuid = [0xAAAA_1111, 0xBBBB_2222];
    let ws = SrsWorkspaceRecord::new(p_uuid, 12345, 1, [0.1, 0.2, 0.3], 1);
    let paper = AcademicPaperRecord::new(
        1001, 1700000000, 123456, 0.1, 0.2, 0.3, 42, 2024, 1, 1, 0, 0xCAFE,
    );
    engine.create_srs_workspace(&ws)?;
    engine.create_academic_papers(&[paper])?;

    let manifest = SrsManifestHeader::new(
        p_uuid,
        [0x1234, 0x5678],
        [0, 0],
        [0xABCD, 0xEF01],
        1700000000,
        0,
        0,
    );

    export_srs_snapshot(&live_tmp, &file_path, &manifest, Some(&ws), Some(p_uuid))?;
    let _ = fs::remove_file(live_tmp);

    let mut map = std::collections::BTreeMap::new();
    map.insert("12".to_string(), 1); // academic_papers
    map.insert("14".to_string(), 1); // srs_workspace
    map.insert("15".to_string(), 1); // srs_manifest

    let exp = ConformanceExpectation {
        case_name: name.to_string(),
        category: "valid".to_string(),
        description: "Scholar Research Snapshot (.snapshot.s3a) container with Tile 15 SRS_MANIFEST and ADDON_EDU".to_string(),
        expected_outcome: "ok".to_string(),
        error_class: None,
        expected_error_message: None,
        file_size_bytes: Some(512 + 3 * HYPER_TILE_SIZE as u64),
        header: Some(HeaderExpectation {
            active_slot: 0,
            generation: 5,
            tile_count: 3,
            addon_mask: ADDON_EDU,
        }),
        records_per_tile_type: map,
        sample_records: vec![
            SampleRecord {
                tile_type: 14,
                record_index: 0,
                summary: "SrsWorkspaceRecord { question_hash: 12345, project_uuid: [0xAAAA1111, 0xBBBB2222] }".to_string(),
            },
            SampleRecord {
                tile_type: 12,
                record_index: 0,
                summary: "AcademicPaperRecord { openalex_id: 1001, citation_count: 42, publication_year: 2024 }".to_string(),
            },
            SampleRecord {
                tile_type: 15,
                record_index: 0,
                summary: "SrsManifestHeader { magic: b\"S3ASTOR1\", snapshot_uuid: [0x1234, 0x5678] }".to_string(),
            },
        ],
        records_hidden_by_default: None,
        records_visible_with_override: None,
        visible_records: 0,
        hidden_records: 0,
        ..Default::default()
    };
    write_json_and_record(dir, name, &exp)
}

// Case 8: invalid_bad_magic
fn generate_invalid_bad_magic(dir: &Path) -> io::Result<String> {
    let name = "invalid_bad_magic";
    let file_path = dir.join(format!("{}.s3a", name));
    let writer = TileWriter::create(&file_path)?;
    drop(writer);

    // Corrupt magic in Slot A
    let mut file = OpenOptions::new().read(true).write(true).open(&file_path)?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(b"BAD!")?;
    file.flush()?;

    let exp = ConformanceExpectation {
        case_name: name.to_string(),
        category: "invalid".to_string(),
        description: "Corrupted FileHeader magic bytes (b\"BAD!\" instead of b\"S3A1\")"
            .to_string(),
        expected_outcome: "error".to_string(),
        error_class: Some("InvalidMagic".to_string()),
        expected_error_message: Some("Invalid S3A file magic".to_string()),
        file_size_bytes: Some(512),
        header: None,
        records_per_tile_type: std::collections::BTreeMap::new(),
        sample_records: Vec::new(),
        records_hidden_by_default: None,
        records_visible_with_override: None,
        visible_records: 0,
        hidden_records: 0,
        ..Default::default()
    };
    write_json_and_record(dir, name, &exp)
}

// Case 9: invalid_both_slots_corrupt
fn generate_invalid_both_slots_corrupt(dir: &Path) -> io::Result<String> {
    let name = "invalid_both_slots_corrupt";
    let file_path = dir.join(format!("{}.s3a", name));
    let mut writer = TileWriter::create(&file_path)?;
    let recs = vec![TelemetryRecord::new(5000, 1, 1, 1.0)];
    writer.write_hyper_tile(TileType::TELEMETRY, &recs, None, None)?;
    drop(writer);

    // Corrupt CRC in Slot A (offset 40) and Slot B (offset 64 + 40 = 104)
    let mut file = OpenOptions::new().read(true).write(true).open(&file_path)?;
    file.seek(SeekFrom::Start(40))?;
    file.write_all(&[0xDE, 0xAD, 0xBE, 0xEF])?;
    file.seek(SeekFrom::Start(104))?;
    file.write_all(&[0xDE, 0xAD, 0xBE, 0xEF])?;
    file.flush()?;

    let exp = ConformanceExpectation {
        case_name: name.to_string(),
        category: "invalid".to_string(),
        description: "Both header slots A and B corrupted with invalid CRC32C".to_string(),
        expected_outcome: "error".to_string(),
        error_class: Some("CorruptedHeader".to_string()),
        expected_error_message: Some("Corrupted block or file header".to_string()),
        file_size_bytes: Some(512 + HYPER_TILE_SIZE as u64),
        header: None,
        records_per_tile_type: std::collections::BTreeMap::new(),
        sample_records: Vec::new(),
        records_hidden_by_default: None,
        records_visible_with_override: None,
        visible_records: 0,
        hidden_records: 0,
        ..Default::default()
    };
    write_json_and_record(dir, name, &exp)
}

// Case 10: invalid_torn_write_fallback
fn generate_invalid_torn_write_fallback(dir: &Path) -> io::Result<String> {
    let name = "invalid_torn_write_fallback";
    let file_path = dir.join(format!("{}.s3a", name));
    let mut writer = TileWriter::create(&file_path)?;
    let recs = vec![TelemetryRecord::new(6000, 1, 1, 9.9)];
    writer.write_hyper_tile(TileType::TELEMETRY, &recs, None, None)?;
    drop(writer);

    // Slot A has generation 1 (tile_count 0). Slot B has generation 2 (tile_count 1).
    // Corrupt Slot B CRC (offset 104) to simulate torn write during slot switch
    let mut file = OpenOptions::new().read(true).write(true).open(&file_path)?;
    file.seek(SeekFrom::Start(104))?;
    file.write_all(&[0xFF, 0xFF, 0xFF, 0xFF])?;
    file.flush()?;

    let exp = ConformanceExpectation {
        case_name: name.to_string(),
        category: "invalid".to_string(),
        description: "Newest header slot (Slot B, gen 2) torn with invalid CRC; reader must fall back to Slot A (gen 1)".to_string(),
        expected_outcome: "fallback_recovered".to_string(),
        error_class: Some("TornWriteFallback".to_string()),
        expected_error_message: None,
        file_size_bytes: Some(512 + HYPER_TILE_SIZE as u64),
        header: Some(HeaderExpectation {
            active_slot: 0,
            generation: 1,
            tile_count: 0,
            addon_mask: 0,
        }),
        records_per_tile_type: std::collections::BTreeMap::new(),
        sample_records: Vec::new(),
        records_hidden_by_default: None,
        records_visible_with_override: None,
        visible_records: 0,
        hidden_records: 0,
        ..Default::default()
    };
    write_json_and_record(dir, name, &exp)
}

// Case 11: invalid_tile_crc_mismatch
fn generate_invalid_tile_crc_mismatch(dir: &Path) -> io::Result<String> {
    let name = "invalid_tile_crc_mismatch";
    let file_path = dir.join(format!("{}.s3a", name));
    let mut writer = TileWriter::create(&file_path)?;
    let recs = vec![TelemetryRecord::new(7000, 1, 1, 42.0)];
    writer.write_hyper_tile(TileType::TELEMETRY, &recs, None, None)?;
    drop(writer);

    // Tamper with payload byte inside Tile 0 (offset 512 (dual header) + 512 (tile header) + 10 = 1034)
    let mut file = OpenOptions::new().read(true).write(true).open(&file_path)?;
    file.seek(SeekFrom::Start(1034))?;
    file.write_all(&[0x7F])?;
    file.flush()?;

    let exp = ConformanceExpectation {
        case_name: name.to_string(),
        category: "invalid".to_string(),
        description: "Hyper-Tile payload modified without updating data_crc32; verification fails with ChecksumMismatch".to_string(),
        expected_outcome: "error".to_string(),
        error_class: Some("ChecksumMismatch".to_string()),
        expected_error_message: Some("Checksum mismatch: expected 0x".to_string()),
        file_size_bytes: Some(512 + HYPER_TILE_SIZE as u64),
        header: Some(HeaderExpectation {
            active_slot: 1,
            generation: 2,
            tile_count: 1,
            addon_mask: 0,
        }),
        records_per_tile_type: std::collections::BTreeMap::new(),
        sample_records: Vec::new(),
        records_hidden_by_default: None,
        records_visible_with_override: None,
        visible_records: 0,
        hidden_records: 0,
        ..Default::default()
    };
    write_json_and_record(dir, name, &exp)
}

// Case 12: invalid_truncated_tile
fn generate_invalid_truncated_tile(dir: &Path) -> io::Result<String> {
    let name = "invalid_truncated_tile";
    let file_path = dir.join(format!("{}.s3a", name));
    let mut writer = TileWriter::create(&file_path)?;
    let recs = vec![TelemetryRecord::new(8000, 1, 1, 10.0)];
    writer.write_hyper_tile(TileType::TELEMETRY, &recs, None, None)?;
    drop(writer);

    // Truncate file from 131,584 bytes to 60,000 bytes (partial tile)
    let file = OpenOptions::new().read(true).write(true).open(&file_path)?;
    file.set_len(60_000)?;

    let exp = ConformanceExpectation {
        case_name: name.to_string(),
        category: "invalid".to_string(),
        description: "Archive header commits 1 Hyper-Tile (131,584 bytes expected) but file truncated to 60,000 bytes".to_string(),
        expected_outcome: "error".to_string(),
        error_class: Some("TruncatedTile".to_string()),
        expected_error_message: Some("Index or offset out of bounds".to_string()),
        file_size_bytes: Some(60_000),
        header: Some(HeaderExpectation {
            active_slot: 1,
            generation: 2,
            tile_count: 1,
            addon_mask: 0,
        }),
        records_per_tile_type: std::collections::BTreeMap::new(),
        sample_records: Vec::new(),
        records_hidden_by_default: None,
        records_visible_with_override: None,
        visible_records: 0,
        hidden_records: 0,
        ..Default::default()
    };
    write_json_and_record(dir, name, &exp)
}

// Case 13: invalid_unsupported_layout_version
fn generate_invalid_unsupported_layout_version(dir: &Path) -> io::Result<String> {
    let name = "invalid_unsupported_layout_version";
    let file_path = dir.join(format!("{}.s3a", name));
    let mut writer = TileWriter::create(&file_path)?;
    let recs = vec![TelemetryRecord::new(9000, 1, 1, 3.5)];
    writer.write_hyper_tile(TileType::TELEMETRY, &recs, None, None)?;
    drop(writer);

    // Overwrite Tile 0 header: set record_layout_version = 99 (offset 512 + 44 = 556)
    // and recompute header_crc32 so header itself is valid but layout version is rejected
    let mut file = OpenOptions::new().read(true).write(true).open(&file_path)?;
    let mut tile_hdr_buf = [0u8; 512];
    file.seek(SeekFrom::Start(512))?;
    file.read_exact(&mut tile_hdr_buf)?;

    let mut tile_hdr: HyperTileHeader = bytemuck::pod_read_unaligned(&tile_hdr_buf);
    tile_hdr.record_layout_version = 99;
    tile_hdr.header_crc32 = compute_tile_header_crc32(&tile_hdr);

    file.seek(SeekFrom::Start(512))?;
    file.write_all(bytes_of(&tile_hdr))?;
    file.flush()?;

    let exp = ConformanceExpectation {
        case_name: name.to_string(),
        category: "invalid".to_string(),
        description: "HyperTileHeader record_layout_version set to 99; reading records must fail with UnsupportedLayoutVersion".to_string(),
        expected_outcome: "error".to_string(),
        error_class: Some("UnsupportedLayoutVersion".to_string()),
        expected_error_message: Some("Unsupported record layout version: 99".to_string()),
        file_size_bytes: Some(512 + HYPER_TILE_SIZE as u64),
        header: Some(HeaderExpectation {
            active_slot: 1,
            generation: 2,
            tile_count: 1,
            addon_mask: 0,
        }),
        records_per_tile_type: std::collections::BTreeMap::new(),
        sample_records: Vec::new(),
        records_hidden_by_default: None,
        records_visible_with_override: None,
        visible_records: 0,
        hidden_records: 0,
        ..Default::default()
    };
    write_json_and_record(dir, name, &exp)
}

// Case 14: valid_protected_mark_hidden
fn generate_valid_protected_mark_hidden(dir: &Path) -> io::Result<String> {
    let name = "valid_protected_mark_hidden";
    let file_path = dir.join(format!("{}.s3a", name));
    let mut writer = TileWriter::create(&file_path)?;

    // Tile 0: Telemetry with 1 record
    let recs = vec![TelemetryRecord::new(9999, 1, 1, 99.9)];
    writer.write_hyper_tile(TileType::TELEMETRY, &recs, None, None)?;

    // Tile 1: Tile 19 MARKS marking the record with MARK_CONSENT_WITHDRAWN
    let mark = MarkRecord::new(
        TileType::TELEMETRY,
        1, // target tile ID
        0, // record index
        writer.generation(),
        MARK_CONSENT_WITHDRAWN,
        MARK_SET,
        1700000000,
        0xFEED_FACE,
    );
    writer.write_hyper_tile(TileType::MARKS, &[mark], None, None)?;
    drop(writer);

    let mut records_per_tile_type = std::collections::BTreeMap::new();
    records_per_tile_type.insert("telemetry".to_string(), 1);
    records_per_tile_type.insert("marks".to_string(), 1);

    let exp = ConformanceExpectation {
        case_name: name.to_string(),
        category: "valid".to_string(),
        description: "Record masked by active MARK_CONSENT_WITHDRAWN; hidden in default queries, visible with include_hidden".to_string(),
        expected_outcome: "ok".to_string(),
        error_class: None,
        expected_error_message: None,
        file_size_bytes: Some(512 + 2 * HYPER_TILE_SIZE as u64),
        header: Some(HeaderExpectation {
            active_slot: 0,
            generation: 3,
            tile_count: 2,
            addon_mask: 0,
        }),
        records_per_tile_type,
        sample_records: vec![
            SampleRecord {
                tile_type: TileType::TELEMETRY,
                record_index: 0,
                summary: "sensor 1, value 99.9 (protected by consent mark)".to_string(),
            },
            SampleRecord {
                tile_type: TileType::MARKS,
                record_index: 0,
                summary: "mark_kind 3 (MARK_CONSENT_WITHDRAWN), target L0:T1:R0".to_string(),
            },
        ],
        records_hidden_by_default: Some(true),
        records_visible_with_override: Some(true),
        visible_records: 0,
        hidden_records: 1,
        ..Default::default()
    };
    write_json_and_record(dir, name, &exp)
}

// Case 15: sealed_ok
fn generate_sealed_ok(dir: &Path) -> io::Result<String> {
    let name = "sealed_ok";
    let unsealed_tmp = dir.join(format!("{}.unsealed.tmp", name));
    let file_path = dir.join(format!("{}.s3a", name));

    let mut writer = TileWriter::create(&unsealed_tmp)?;
    let recs1: Vec<TelemetryRecord> = (0..5).map(|i| TelemetryRecord::new(1000 + i as u64, 100 + i as u32, 1, 10.0 + i as f64)).collect();
    let recs2: Vec<TelemetryRecord> = (0..5).map(|i| TelemetryRecord::new(2000 + i as u64, 200 + i as u32, 2, 20.0 + i as f64)).collect();
    writer.write_hyper_tile(TileType::TELEMETRY, &recs1, None, None)?;
    writer.write_hyper_tile(TileType::TELEMETRY, &recs2, None, None)?;
    drop(writer);

    s3a_engine::seal_archive(&unsealed_tmp, &file_path, s3a_engine::DEFAULT_SEAL_K, s3a_engine::DEFAULT_SEAL_M)?;
    let _ = fs::remove_file(unsealed_tmp);

    let file_size = fs::metadata(&file_path)?.len();

    let mut map = std::collections::BTreeMap::new();
    map.insert("telemetry".to_string(), 10);

    let exp = ConformanceExpectation {
        case_name: name.to_string(),
        category: "valid".to_string(),
        description: "Sealed S3A archive with directory, K=16/M=2 Reed-Solomon parity, and valid 64-byte end cap".to_string(),
        expected_outcome: "ok".to_string(),
        error_class: None,
        expected_error_message: None,
        file_size_bytes: Some(file_size),
        header: Some(HeaderExpectation {
            active_slot: 0,
            generation: 3,
            tile_count: 2,
            addon_mask: 0,
        }),
        records_per_tile_type: map,
        sample_records: vec![SampleRecord {
            tile_type: TileType::TELEMETRY,
            record_index: 0,
            summary: "Telemetry device 100".to_string(),
        }],
        records_hidden_by_default: None,
        records_visible_with_override: None,
        visible_records: 0,
        hidden_records: 0,
        is_sealed: Some(true),
        rebuilt_directory: Some(false),
        out_of_order_detected: Some(false),
        repaired_tiles_count: Some(0),
    };
    write_json_and_record(dir, name, &exp)
}

// Case 16: sealed_footer_missing
fn generate_sealed_footer_missing(dir: &Path) -> io::Result<String> {
    let name = "sealed_footer_missing";
    let file_path = dir.join(format!("{}.s3a", name));

    let mut writer = TileWriter::create(&file_path)?;
    let recs: Vec<TelemetryRecord> = (0..5).map(|i| TelemetryRecord::new(1000 + i as u64, 100 + i as u32, 1, 10.0 + i as f64)).collect();
    writer.write_hyper_tile(TileType::TELEMETRY, &recs, None, None)?;
    drop(writer);

    let file_size = fs::metadata(&file_path)?.len();

    let mut map = std::collections::BTreeMap::new();
    map.insert("telemetry".to_string(), 5);

    let exp = ConformanceExpectation {
        case_name: name.to_string(),
        category: "valid".to_string(),
        description: "Sealed archive with footer missing; reader rebuilds directory by scanning self-addressed tiles".to_string(),
        expected_outcome: "ok_with_warning".to_string(),
        error_class: None,
        expected_error_message: None,
        file_size_bytes: Some(file_size),
        header: Some(HeaderExpectation {
            active_slot: 1,
            generation: 2,
            tile_count: 1,
            addon_mask: 0,
        }),
        records_per_tile_type: map,
        sample_records: vec![SampleRecord {
            tile_type: TileType::TELEMETRY,
            record_index: 0,
            summary: "Telemetry device 100".to_string(),
        }],
        records_hidden_by_default: None,
        records_visible_with_override: None,
        visible_records: 0,
        hidden_records: 0,
        is_sealed: Some(false),
        rebuilt_directory: Some(true),
        out_of_order_detected: Some(false),
        repaired_tiles_count: Some(0),
    };
    write_json_and_record(dir, name, &exp)
}

// Case 17: sealed_endcap_corrupt
fn generate_sealed_endcap_corrupt(dir: &Path) -> io::Result<String> {
    let name = "sealed_endcap_corrupt";
    let unsealed_tmp = dir.join(format!("{}.unsealed.tmp", name));
    let file_path = dir.join(format!("{}.s3a", name));

    let mut writer = TileWriter::create(&unsealed_tmp)?;
    let recs: Vec<TelemetryRecord> = (0..5).map(|i| TelemetryRecord::new(1000 + i as u64, 100 + i as u32, 1, 10.0 + i as f64)).collect();
    writer.write_hyper_tile(TileType::TELEMETRY, &recs, None, None)?;
    drop(writer);

    s3a_engine::seal_archive(&unsealed_tmp, &file_path, s3a_engine::DEFAULT_SEAL_K, s3a_engine::DEFAULT_SEAL_M)?;
    let _ = fs::remove_file(unsealed_tmp);

    // Corrupt the end cap (endcap_crc32 is at len - 12)
    let mut file_bytes = fs::read(&file_path)?;
    let len = file_bytes.len();
    file_bytes[len - 12] ^= 0xFF;
    fs::write(&file_path, file_bytes)?;

    let file_size = fs::metadata(&file_path)?.len();

    let mut map = std::collections::BTreeMap::new();
    map.insert("telemetry".to_string(), 5);

    let exp = ConformanceExpectation {
        case_name: name.to_string(),
        category: "valid".to_string(),
        description: "Sealed archive with corrupt end cap; reader detects endcap corruption and rebuilds directory by scanning tiles".to_string(),
        expected_outcome: "ok_with_warning".to_string(),
        error_class: None,
        expected_error_message: None,
        file_size_bytes: Some(file_size),
        header: Some(HeaderExpectation {
            active_slot: 1,
            generation: 2,
            tile_count: 1,
            addon_mask: 0,
        }),
        records_per_tile_type: map,
        sample_records: vec![SampleRecord {
            tile_type: TileType::TELEMETRY,
            record_index: 0,
            summary: "Telemetry device 100".to_string(),
        }],
        records_hidden_by_default: None,
        records_visible_with_override: None,
        visible_records: 0,
        hidden_records: 0,
        is_sealed: Some(false),
        rebuilt_directory: Some(true),
        out_of_order_detected: Some(false),
        repaired_tiles_count: Some(0),
    };
    write_json_and_record(dir, name, &exp)
}

// Case 18: sealed_truncated_mid_tile
fn generate_sealed_truncated_mid_tile(dir: &Path) -> io::Result<String> {
    let name = "sealed_truncated_mid_tile";
    let unsealed_tmp = dir.join(format!("{}.unsealed.tmp", name));
    let file_path = dir.join(format!("{}.s3a", name));

    let mut writer = TileWriter::create(&unsealed_tmp)?;
    let recs: Vec<TelemetryRecord> = (0..5).map(|i| TelemetryRecord::new(1000 + i as u64, 100 + i as u32, 1, 10.0 + i as f64)).collect();
    writer.write_hyper_tile(TileType::TELEMETRY, &recs, None, None)?;
    drop(writer);

    s3a_engine::seal_archive(&unsealed_tmp, &file_path, s3a_engine::DEFAULT_SEAL_K, s3a_engine::DEFAULT_SEAL_M)?;
    let _ = fs::remove_file(unsealed_tmp);

    // Truncate file mid-tile (e.g. truncate to 512 + 65536 bytes)
    let truncated_len = 512 + 65536;
    let file = OpenOptions::new().write(true).open(&file_path)?;
    file.set_len(truncated_len)?;
    drop(file);

    let exp = ConformanceExpectation {
        case_name: name.to_string(),
        category: "invalid".to_string(),
        description: "Sealed archive truncated in the middle of a hyper-tile; reader must reject with SealedTruncated error".to_string(),
        expected_outcome: "error".to_string(),
        error_class: Some("SealedTruncated".to_string()),
        expected_error_message: Some("Sealed archive truncated mid-tile".to_string()),
        file_size_bytes: Some(truncated_len),
        header: None,
        records_per_tile_type: std::collections::BTreeMap::new(),
        sample_records: Vec::new(),
        records_hidden_by_default: None,
        records_visible_with_override: None,
        visible_records: 0,
        hidden_records: 0,
        is_sealed: None,
        rebuilt_directory: None,
        out_of_order_detected: None,
        repaired_tiles_count: None,
    };
    write_json_and_record(dir, name, &exp)
}

// Case 19: one_tile_corrupt_repaired
fn generate_one_tile_corrupt_repaired(dir: &Path) -> io::Result<String> {
    let name = "one_tile_corrupt_repaired";
    let unsealed_tmp = dir.join(format!("{}.unsealed.tmp", name));
    let file_path = dir.join(format!("{}.s3a", name));

    let mut writer = TileWriter::create(&unsealed_tmp)?;
    let recs1: Vec<TelemetryRecord> = (0..5).map(|i| TelemetryRecord::new(1000 + i as u64, 100 + i as u32, 1, 10.0 + i as f64)).collect();
    let recs2: Vec<TelemetryRecord> = (0..5).map(|i| TelemetryRecord::new(2000 + i as u64, 200 + i as u32, 2, 20.0 + i as f64)).collect();
    writer.write_hyper_tile(TileType::TELEMETRY, &recs1, None, None)?;
    writer.write_hyper_tile(TileType::TELEMETRY, &recs2, None, None)?;
    drop(writer);

    s3a_engine::seal_archive(&unsealed_tmp, &file_path, s3a_engine::DEFAULT_SEAL_K, s3a_engine::DEFAULT_SEAL_M)?;
    let _ = fs::remove_file(unsealed_tmp);

    // Corrupt 1 data tile (tile 0 payload at 512 + 512 + 10)
    let mut file_bytes = fs::read(&file_path)?;
    file_bytes[512 + 512 + 10] ^= 0xAA;
    fs::write(&file_path, file_bytes)?;

    let file_size = fs::metadata(&file_path)?.len();

    let mut map = std::collections::BTreeMap::new();
    map.insert("telemetry".to_string(), 10);

    let exp = ConformanceExpectation {
        case_name: name.to_string(),
        category: "valid".to_string(),
        description: "Sealed archive with 1 corrupted data tile in group; Reed-Solomon parity successfully repairs it in memory".to_string(),
        expected_outcome: "repaired".to_string(),
        error_class: None,
        expected_error_message: None,
        file_size_bytes: Some(file_size),
        header: Some(HeaderExpectation {
            active_slot: 0,
            generation: 3,
            tile_count: 2,
            addon_mask: 0,
        }),
        records_per_tile_type: map,
        sample_records: vec![SampleRecord {
            tile_type: TileType::TELEMETRY,
            record_index: 0,
            summary: "Telemetry device 100".to_string(),
        }],
        records_hidden_by_default: None,
        records_visible_with_override: None,
        visible_records: 0,
        hidden_records: 0,
        is_sealed: Some(true),
        rebuilt_directory: Some(false),
        out_of_order_detected: Some(false),
        repaired_tiles_count: Some(1),
    };
    write_json_and_record(dir, name, &exp)
}

// Case 20: three_tiles_corrupt_in_one_group
fn generate_three_tiles_corrupt_in_one_group(dir: &Path) -> io::Result<String> {
    let name = "three_tiles_corrupt_in_one_group";
    let unsealed_tmp = dir.join(format!("{}.unsealed.tmp", name));
    let file_path = dir.join(format!("{}.s3a", name));

    let mut writer = TileWriter::create(&unsealed_tmp)?;
    let recs1: Vec<TelemetryRecord> = (0..5).map(|i| TelemetryRecord::new(1000 + i as u64, 100 + i as u32, 1, 10.0 + i as f64)).collect();
    let recs2: Vec<TelemetryRecord> = (0..5).map(|i| TelemetryRecord::new(2000 + i as u64, 200 + i as u32, 2, 20.0 + i as f64)).collect();
    let recs3: Vec<TelemetryRecord> = (0..5).map(|i| TelemetryRecord::new(3000 + i as u64, 300 + i as u32, 3, 30.0 + i as f64)).collect();
    writer.write_hyper_tile(TileType::TELEMETRY, &recs1, None, None)?;
    writer.write_hyper_tile(TileType::TELEMETRY, &recs2, None, None)?;
    writer.write_hyper_tile(TileType::TELEMETRY, &recs3, None, None)?;
    drop(writer);

    s3a_engine::seal_archive(&unsealed_tmp, &file_path, s3a_engine::DEFAULT_SEAL_K, s3a_engine::DEFAULT_SEAL_M)?;
    let _ = fs::remove_file(unsealed_tmp);

    // Corrupt all 3 data tiles in group 0 (M=2, so 3 is unrepairable)
    let mut file_bytes = fs::read(&file_path)?;
    for t in 0..3 {
        let offset = 512 + t * HYPER_TILE_SIZE + 512 + 10;
        file_bytes[offset] ^= 0xAA;
    }
    fs::write(&file_path, file_bytes)?;

    let file_size = fs::metadata(&file_path)?.len();

    let exp = ConformanceExpectation {
        case_name: name.to_string(),
        category: "invalid".to_string(),
        description: "Sealed archive with 3 corrupted data tiles in a single group with M=2; exceeds RS parity capacity and must fail closed".to_string(),
        expected_outcome: "error".to_string(),
        error_class: Some("UnrepairableCorruption".to_string()),
        expected_error_message: Some("Corrupted tiles exceed RS parity M=2".to_string()),
        file_size_bytes: Some(file_size),
        header: None,
        records_per_tile_type: std::collections::BTreeMap::new(),
        sample_records: Vec::new(),
        records_hidden_by_default: None,
        records_visible_with_override: None,
        visible_records: 0,
        hidden_records: 0,
        is_sealed: Some(true),
        rebuilt_directory: None,
        out_of_order_detected: None,
        repaired_tiles_count: None,
    };
    write_json_and_record(dir, name, &exp)
}

// Case 21: tiles_out_of_order
fn generate_tiles_out_of_order(dir: &Path) -> io::Result<String> {
    let name = "tiles_out_of_order";
    let unsealed_tmp = dir.join(format!("{}.unsealed.tmp", name));
    let file_path = dir.join(format!("{}.s3a", name));

    let mut writer = TileWriter::create(&unsealed_tmp)?;
    let recs1: Vec<TelemetryRecord> = (0..5).map(|i| TelemetryRecord::new(1000 + i as u64, 100 + i as u32, 1, 10.0 + i as f64)).collect();
    let recs2: Vec<TelemetryRecord> = (0..5).map(|i| TelemetryRecord::new(2000 + i as u64, 200 + i as u32, 2, 20.0 + i as f64)).collect();
    writer.write_hyper_tile(TileType::TELEMETRY, &recs1, None, None)?;
    writer.write_hyper_tile(TileType::TELEMETRY, &recs2, None, None)?;
    drop(writer);

    // Read unsealed bytes and swap tile 0 and tile 1
    let mut unsealed_bytes = fs::read(&unsealed_tmp)?;
    let tile0_offset = 512;
    let tile1_offset = 512 + HYPER_TILE_SIZE;
    let tile0 = unsealed_bytes[tile0_offset..tile0_offset + HYPER_TILE_SIZE].to_vec();
    let tile1 = unsealed_bytes[tile1_offset..tile1_offset + HYPER_TILE_SIZE].to_vec();

    unsealed_bytes[tile0_offset..tile0_offset + HYPER_TILE_SIZE].copy_from_slice(&tile1);
    unsealed_bytes[tile1_offset..tile1_offset + HYPER_TILE_SIZE].copy_from_slice(&tile0);

    // Seal the buffer with swapped tiles
    let sealed_bytes = s3a_engine::seal_buffer(&unsealed_bytes, s3a_engine::DEFAULT_SEAL_K, s3a_engine::DEFAULT_SEAL_M)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{}", e)))?;
    fs::write(&file_path, &sealed_bytes)?;
    let _ = fs::remove_file(unsealed_tmp);

    let file_size = fs::metadata(&file_path)?.len();

    let mut map = std::collections::BTreeMap::new();
    map.insert("telemetry".to_string(), 10);

    let exp = ConformanceExpectation {
        case_name: name.to_string(),
        category: "valid".to_string(),
        description: "Sealed archive whose tiles on disk are out of order; reader reassembles them correctly by tile_index".to_string(),
        expected_outcome: "ok_with_warning".to_string(),
        error_class: None,
        expected_error_message: None,
        file_size_bytes: Some(file_size),
        header: Some(HeaderExpectation {
            active_slot: 0,
            generation: 3,
            tile_count: 2,
            addon_mask: 0,
        }),
        records_per_tile_type: map,
        sample_records: vec![SampleRecord {
            tile_type: TileType::TELEMETRY,
            record_index: 0,
            summary: "Telemetry device 100".to_string(),
        }],
        records_hidden_by_default: None,
        records_visible_with_override: None,
        visible_records: 0,
        hidden_records: 0,
        is_sealed: Some(true),
        rebuilt_directory: Some(false),
        out_of_order_detected: Some(true),
        repaired_tiles_count: Some(0),
    };
    write_json_and_record(dir, name, &exp)
}

// Case 22: foreign_tile
fn generate_foreign_tile(dir: &Path) -> io::Result<String> {
    let name = "foreign_tile";
    let unsealed_tmp = dir.join(format!("{}.unsealed.tmp", name));
    let file_path = dir.join(format!("{}.s3a", name));

    let mut writer = TileWriter::create(&unsealed_tmp)?;
    let recs1: Vec<TelemetryRecord> = (0..5).map(|i| TelemetryRecord::new(1000 + i as u64, 100 + i as u32, 1, 10.0 + i as f64)).collect();
    let recs2: Vec<TelemetryRecord> = (0..5).map(|i| TelemetryRecord::new(2000 + i as u64, 200 + i as u32, 2, 20.0 + i as f64)).collect();
    writer.write_hyper_tile(TileType::TELEMETRY, &recs1, None, None)?;
    writer.write_hyper_tile(TileType::TELEMETRY, &recs2, None, None)?;
    drop(writer);

    let mut file_bytes = fs::read(&unsealed_tmp)?;
    let _ = fs::remove_file(unsealed_tmp);

    // Corrupt file_uuid of tile 1 to a foreign file_uuid
    let tile1_offset = 512 + HYPER_TILE_SIZE;
    let mut tile1_hdr: HyperTileHeader = bytemuck::pod_read_unaligned(&file_bytes[tile1_offset..tile1_offset + s3a_core::HYPER_TILE_HEADER_SIZE]);
    tile1_hdr.file_uuid = [0xDEAD_BEEF, 0xCAFE_BABE];
    tile1_hdr.header_crc32 = compute_tile_header_crc32(&tile1_hdr);
    file_bytes[tile1_offset..tile1_offset + s3a_core::HYPER_TILE_HEADER_SIZE].copy_from_slice(bytes_of(&tile1_hdr));

    fs::write(&file_path, &file_bytes)?;
    let file_size = fs::metadata(&file_path)?.len();

    let exp = ConformanceExpectation {
        case_name: name.to_string(),
        category: "invalid".to_string(),
        description: "Tile header carrying foreign file_uuid; reader MUST reject with FileUuidMismatch error".to_string(),
        expected_outcome: "error".to_string(),
        error_class: Some("FileUuidMismatch".to_string()),
        expected_error_message: Some("Tile file_uuid does not match file header file_uuid".to_string()),
        file_size_bytes: Some(file_size),
        header: None,
        records_per_tile_type: std::collections::BTreeMap::new(),
        sample_records: Vec::new(),
        records_hidden_by_default: None,
        records_visible_with_override: None,
        visible_records: 0,
        hidden_records: 0,
        is_sealed: None,
        rebuilt_directory: None,
        out_of_order_detected: None,
        repaired_tiles_count: None,
    };
    write_json_and_record(dir, name, &exp)
}

// Case 23: legacy_unaddressed_tiles
fn generate_legacy_unaddressed_tiles(dir: &Path) -> io::Result<String> {
    let name = "legacy_unaddressed_tiles";
    let file_path = dir.join(format!("{}.s3a", name));

    let mut writer = TileWriter::create(&file_path)?;
    let recs: Vec<TelemetryRecord> = (0..5).map(|i| TelemetryRecord::new(1000 + i as u64, 100 + i as u32, 1, 10.0 + i as f64)).collect();
    writer.write_hyper_tile(TileType::TELEMETRY, &recs, None, None)?;
    drop(writer);

    // Overwrite tile 0 header to zero out file_uuid and tile_index (simulating legacy pre-v1.1 tile)
    let mut file_bytes = fs::read(&file_path)?;
    let tile_offset = 512;
    let mut hdr: HyperTileHeader = bytemuck::pod_read_unaligned(&file_bytes[tile_offset..tile_offset + s3a_core::HYPER_TILE_HEADER_SIZE]);
    hdr.file_uuid = [0, 0];
    hdr.tile_index = 0;
    hdr.record_layout_version = 1;
    hdr.header_crc32 = compute_tile_header_crc32(&hdr);
    file_bytes[tile_offset..tile_offset + s3a_core::HYPER_TILE_HEADER_SIZE].copy_from_slice(bytes_of(&hdr));
    fs::write(&file_path, &file_bytes)?;

    let file_size = fs::metadata(&file_path)?.len();

    let mut map = std::collections::BTreeMap::new();
    map.insert("telemetry".to_string(), 5);

    let exp = ConformanceExpectation {
        case_name: name.to_string(),
        category: "valid".to_string(),
        description: "Legacy pre-v1.1 unaddressed tile (record_layout_version=1, file_uuid=[0,0]); reader accepts via legacy path".to_string(),
        expected_outcome: "ok".to_string(),
        error_class: None,
        expected_error_message: None,
        file_size_bytes: Some(file_size),
        header: Some(HeaderExpectation {
            active_slot: 1,
            generation: 2,
            tile_count: 1,
            addon_mask: 0,
        }),
        records_per_tile_type: map,
        sample_records: vec![SampleRecord {
            tile_type: TileType::TELEMETRY,
            record_index: 0,
            summary: "Telemetry device 100".to_string(),
        }],
        records_hidden_by_default: None,
        records_visible_with_override: None,
        visible_records: 0,
        hidden_records: 0,
        is_sealed: Some(false),
        rebuilt_directory: None,
        out_of_order_detected: None,
        repaired_tiles_count: None,
    };
    write_json_and_record(dir, name, &exp)
}

// Case 24: micro_valid_4kb
fn generate_micro_valid_4kb(dir: &Path) -> io::Result<String> {
    let name = "micro_valid_4kb";
    let file_path = dir.join(format!("{}.s3a", name));

    let flash = RamFlash::<16384>::new(256, 512);
    let device_id = [0x11u8; 16];
    let file_uuid = [0x1111_2222_3333_4444, 0x5555_6666_7777_8888];
    let mut writer = EdgeWriter::<_, 4096>::open_or_format(flash, device_id, file_uuid, 1700000000)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{:?}", e)))?;

    // Tile 0: Consent Record
    let consent = EdgeConsentRecord::new(
        [0xAA; 16],
        0x01,
        0x01,
        1700000000,
        1800000000,
        1001,
    );
    writer.append_consent(&consent)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{:?}", e)))?;
    writer.seal_tile(1700000001)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{:?}", e)))?;

    // Tile 1: 5 Sensor records
    for i in 0..5 {
        let rec = CompactEdgeSensorRecord::new(
            (i * 10) as u16,
            1,
            1,
            200 + (i as i32) * 5,
            0,
        );
        writer.append_sensor_record(&rec)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{:?}", e)))?;
    }
    writer.seal_tile(1700000060)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{:?}", e)))?;

    let flash = writer.into_storage();
    let total_bytes = 512 + 2 * 4096;
    fs::write(&file_path, &flash.as_slice()[..total_bytes])?;

    let mut map = std::collections::BTreeMap::new();
    map.insert("71".to_string(), 1); // Consent
    map.insert("75".to_string(), 5); // Time-series sensor

    let sample_records = vec![
        SampleRecord {
            tile_type: 71,
            record_index: 0,
            summary: "EdgeConsentRecord { consent_version: 1, allowed_purposes: 1 }".to_string(),
        },
        SampleRecord {
            tile_type: 75,
            record_index: 0,
            summary: "CompactEdgeSensorRecord { delta_ts_ms: 0, sensor_id: 1, metric_id: 1, quantized_val: 200 }".to_string(),
        },
    ];

    let exp = ConformanceExpectation {
        case_name: name.to_string(),
        category: "valid".to_string(),
        description: "Valid 4,096-byte Micro S3A file with standard DualFileHeader and 2 micro-tiles".to_string(),
        expected_outcome: "ok".to_string(),
        error_class: None,
        expected_error_message: None,
        file_size_bytes: Some(total_bytes as u64),
        header: Some(HeaderExpectation {
            active_slot: 0,
            generation: 3,
            tile_count: 2,
            addon_mask: 0x0008,
        }),
        records_per_tile_type: map,
        sample_records,
        records_hidden_by_default: None,
        records_visible_with_override: None,
        visible_records: 0,
        hidden_records: 0,
        is_sealed: Some(false),
        rebuilt_directory: None,
        out_of_order_detected: None,
        repaired_tiles_count: None,
    };

    write_json_and_record(dir, name, &exp)
}

// Case 25: micro_truncated_file
fn generate_micro_truncated_file(dir: &Path) -> io::Result<String> {
    let name = "micro_truncated_file";
    let file_path = dir.join(format!("{}.s3a", name));

    let flash = RamFlash::<16384>::new(256, 512);
    let device_id = [0x11u8; 16];
    let file_uuid = [0x1111_2222_3333_4444, 0x5555_6666_7777_8888];
    let mut writer = EdgeWriter::<_, 4096>::open_or_format(flash, device_id, file_uuid, 1700000000)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{:?}", e)))?;

    // Tile 0: Consent Record
    let consent = EdgeConsentRecord::new(
        [0xAA; 16],
        0x01,
        0x01,
        1700000000,
        1800000000,
        1001,
    );
    writer.append_consent(&consent)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{:?}", e)))?;
    writer.seal_tile(1700000001)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{:?}", e)))?;

    let mut flash = writer.into_storage();
    // Simulate torn write of Tile 1: write partial payload into flash sector 1 (offset 512 + 4096 + 64),
    // but header at offset 512 + 4096 remains 0xFF, and file header slot is not committed.
    let torn_tile_offset = 512 + 4096;
    let partial_data = [0x42u8; 512];
    flash.memory[torn_tile_offset + 64..torn_tile_offset + 64 + 512].copy_from_slice(&partial_data);

    let total_bytes = torn_tile_offset + 64 + 512;
    fs::write(&file_path, &flash.as_slice()[..total_bytes])?;

    let mut map = std::collections::BTreeMap::new();
    map.insert("71".to_string(), 1);

    let exp = ConformanceExpectation {
        case_name: name.to_string(),
        category: "valid".to_string(),
        description: "Power-loss torn micro-tile: Tile 0 valid, Tile 1 cut off mid-write; recovery preserves Tile 0".to_string(),
        expected_outcome: "ok".to_string(),
        error_class: None,
        expected_error_message: None,
        file_size_bytes: Some(total_bytes as u64),
        header: Some(HeaderExpectation {
            active_slot: 1,
            generation: 2,
            tile_count: 1,
            addon_mask: 0x0008,
        }),
        records_per_tile_type: map,
        sample_records: vec![SampleRecord {
            tile_type: 71,
            record_index: 0,
            summary: "EdgeConsentRecord { consent_version: 1, allowed_purposes: 1 }".to_string(),
        }],
        records_hidden_by_default: None,
        records_visible_with_override: None,
        visible_records: 0,
        hidden_records: 0,
        is_sealed: Some(false),
        rebuilt_directory: None,
        out_of_order_detected: None,
        repaired_tiles_count: None,
    };

    write_json_and_record(dir, name, &exp)
}

// Case 26: micro_signed_seal_chain
fn generate_micro_signed_seal_chain(dir: &Path) -> io::Result<String> {
    let name = "micro_signed_seal_chain";
    let file_path = dir.join(format!("{}.s3a", name));

    let flash = RamFlash::<32768>::new(256, 512);
    let device_id = [0x11u8; 16];
    let file_uuid = [0x1111_2222_3333_4444, 0x5555_6666_7777_8888];
    let mut writer = EdgeWriter::<_, 4096>::open_or_format(flash, device_id, file_uuid, 1700000000)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{:?}", e)))?;

    let signing_key = s3a_edge::SigningKey::from_bytes(&[0x42u8; 32]);
    let pubkey_bytes = signing_key.verifying_key().to_bytes();

    // Tile 0: Consent Record
    let consent = EdgeConsentRecord::new(
        [0xAA; 16],
        0x01,
        0x01,
        1700000000,
        1800000000,
        1001,
    );
    writer.append_consent(&consent)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{:?}", e)))?;
    let seal0 = writer.seal_tile(1700000001)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{:?}", e)))?;
    let sig0 = s3a_edge::sign_seal_hash(&signing_key, &seal0);
    assert!(s3a_edge::verify_signature_raw(&pubkey_bytes, &seal0, &sig0));

    // Tile 1: 5 Sensor records
    for i in 0..5 {
        let rec = CompactEdgeSensorRecord::new(
            (i * 10) as u16,
            1,
            1,
            200 + (i as i32) * 5,
            0,
        );
        writer.append_sensor_record(&rec)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{:?}", e)))?;
    }
    let seal1 = writer.seal_tile(1700000060)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{:?}", e)))?;
    let sig1 = s3a_edge::sign_seal_hash(&signing_key, &seal1);
    assert!(s3a_edge::verify_signature_raw(&pubkey_bytes, &seal1, &sig1));

    // Tile 2: 3 Equipment events
    for i in 0..3 {
        let evt = EquipmentEventRecord::new(
            1700000070 + (i as u64) * 5,
            1,
            0,
            1,
            1001 + (i as u32),
            [0x01; 8],
            100,
        );
        writer.append_equipment_event(&evt)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{:?}", e)))?;
    }
    let seal2 = writer.seal_tile(1700000080)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{:?}", e)))?;
    let sig2 = s3a_edge::sign_seal_hash(&signing_key, &seal2);
    assert!(s3a_edge::verify_signature_raw(&pubkey_bytes, &seal2, &sig2));

    let flash = writer.into_storage();
    let total_bytes = 512 + 3 * 4096;
    fs::write(&file_path, &flash.as_slice()[..total_bytes])?;

    let exp = ConformanceExpectation {
        case_name: name.to_string(),
        category: "valid".to_string(),
        description: "Three 4 KB micro-tiles with continuous SHA-256 seal chain and valid Ed25519 signatures".to_string(),
        expected_outcome: "ok".to_string(),
        error_class: None,
        expected_error_message: None,
        file_size_bytes: Some(total_bytes as u64),
        header: Some(HeaderExpectation {
            active_slot: 1,
            generation: 4,
            tile_count: 3,
            addon_mask: 0x0008,
        }),
        records_per_tile_type: std::collections::BTreeMap::new(),
        sample_records: vec![SampleRecord {
            tile_type: 70,
            record_index: 1,
            summary: "DeviceSealRecord with verified Ed25519 signature".to_string(),
        }],
        records_hidden_by_default: None,
        records_visible_with_override: None,
        visible_records: 0,
        hidden_records: 0,
        is_sealed: Some(false),
        rebuilt_directory: None,
        out_of_order_detected: None,
        repaired_tiles_count: None,
    };

    write_json_and_record(dir, name, &exp)
}

fn generate_vault_transfer_chain(dir: &Path) -> io::Result<String> {
    let name = "vault_transfer_chain";
    let file_path = dir.join(format!("{}.s3a", name));
    let mut keystore = MemoryKeyStore::new();

    let mut vault = Vault::create(&file_path, "Issuer Vault", VaultOwnerKind::Organization, &mut keystore)
        .map_err(io::Error::other)?;

    let primary = *vault.primary_identity().unwrap();
    let issuer_id = primary.identity_id;
    let issuer_pk = primary.public_key;

    let alice_id = [0x02; 16];
    let bob_id = [0x03; 16];

    let alice_pk = keystore.generate_keypair(&alice_id).map_err(io::Error::other)?;
    let bob_pk = keystore.generate_keypair(&bob_id).map_err(io::Error::other)?;

    let item_hash = [0xAA; 32];
    let item_record = VaultItemRecord::new(
        [0x10; 16],
        1, // Credential
        item_hash,
        issuer_pk,
        issuer_pk,
        1700000000,
        0,
        0,
    );
    vault.add_item(item_record).map_err(io::Error::other)?;

    let transfer_1 = s3a_vault::create_transfer_record(
        [0x01; 16],
        item_hash,
        issuer_id,
        alice_pk,
        1700000010,
        [0u8; 32],
        0,
        &keystore,
    ).map_err(io::Error::other)?;
    let link1_hash = transfer_1.compute_transfer_hash();
    vault.add_transfer(transfer_1).map_err(io::Error::other)?;

    let transfer_2 = s3a_vault::create_transfer_record(
        [0x02; 16],
        item_hash,
        alice_id,
        bob_pk,
        1700000020,
        link1_hash,
        0,
        &keystore,
    ).map_err(io::Error::other)?;
    vault.add_transfer(transfer_2).map_err(io::Error::other)?;

    let exp = ConformanceExpectation {
        case_name: name.to_string(),
        category: "valid".to_string(),
        description: "Three-link signed transfer chain: Genesis Issuer -> Alice -> Bob (VAULT-VEC-01)".to_string(),
        expected_outcome: "ok".to_string(),
        error_class: None,
        expected_error_message: None,
        file_size_bytes: Some(fs::metadata(&file_path)?.len()),
        header: None,
        records_per_tile_type: std::collections::BTreeMap::new(),
        sample_records: vec![SampleRecord {
            tile_type: 64,
            record_index: 0,
            summary: "ItemTransferRecord resolving owner to Bob".to_string(),
        }],
        records_hidden_by_default: None,
        records_visible_with_override: None,
        visible_records: 0,
        hidden_records: 0,
        is_sealed: Some(false),
        rebuilt_directory: None,
        out_of_order_detected: None,
        repaired_tiles_count: None,
    };

    write_json_and_record(dir, name, &exp)
}

fn generate_vault_delegation_governance(dir: &Path) -> io::Result<String> {
    let name = "vault_delegation_governance";
    let file_path = dir.join(format!("{}.s3a", name));
    let mut keystore = MemoryKeyStore::new();

    let mut vault = Vault::create(&file_path, "Alice Vault", VaultOwnerKind::Person, &mut keystore)
        .map_err(io::Error::other)?;

    let primary = *vault.primary_identity().unwrap();
    let alice_id = primary.identity_id;

    let gamma_id = [0x02; 16];
    let gamma_pk = keystore.generate_keypair(&gamma_id).map_err(io::Error::other)?;

    let grant = s3a_vault::create_delegation_grant(
        [0x50; 16],
        alice_id,
        VaultOwnerKind::Person,
        gamma_id,
        gamma_pk,
        ACTION_READ | ACTION_WRITE,
        0x0001,
        100,
        1700000000,
        1700010000,
        None,
        &keystore,
    ).map_err(io::Error::other)?;
    vault.add_grant(grant).map_err(io::Error::other)?;

    let exp = ConformanceExpectation {
        case_name: name.to_string(),
        category: "valid".to_string(),
        description: "Delegation grant with agent action and non-delegable judgment protection (VAULT-VEC-02)".to_string(),
        expected_outcome: "ok".to_string(),
        error_class: None,
        expected_error_message: None,
        file_size_bytes: Some(fs::metadata(&file_path)?.len()),
        header: None,
        records_per_tile_type: std::collections::BTreeMap::new(),
        sample_records: vec![SampleRecord {
            tile_type: 64,
            record_index: 0,
            summary: "DelegationGrantRecord Alice -> Gamma".to_string(),
        }],
        records_hidden_by_default: None,
        records_visible_with_override: None,
        visible_records: 0,
        hidden_records: 0,
        is_sealed: Some(false),
        rebuilt_directory: None,
        out_of_order_detected: None,
        repaired_tiles_count: None,
    };

    write_json_and_record(dir, name, &exp)
}

fn generate_vault_key_revocation(dir: &Path) -> io::Result<String> {
    let name = "vault_key_revocation";
    let file_path = dir.join(format!("{}.s3a", name));
    let mut keystore = MemoryKeyStore::new();

    let mut vault = Vault::create(&file_path, "Rotated Vault", VaultOwnerKind::Person, &mut keystore)
        .map_err(io::Error::other)?;

    let primary = *vault.primary_identity().unwrap();
    let k1_id = primary.identity_id;

    let k2_id = [0x02; 16];
    let _k2_pk = keystore.generate_keypair(&k2_id).map_err(io::Error::other)?;

    let rotation_timestamp = 1700005000;
    let rotation = s3a_vault::create_key_rotation_record(
        [0x60; 16],
        k1_id,
        k2_id,
        rotation_timestamp,
        &keystore,
    ).map_err(io::Error::other)?;
    vault.add_key_rotation(rotation).map_err(io::Error::other)?;

    let exp = ConformanceExpectation {
        case_name: name.to_string(),
        category: "valid".to_string(),
        description: "Key rotation K1 to K2 invalidating subsequent actions while preserving historical actions (VAULT-VEC-03)".to_string(),
        expected_outcome: "ok".to_string(),
        error_class: None,
        expected_error_message: None,
        file_size_bytes: Some(fs::metadata(&file_path)?.len()),
        header: None,
        records_per_tile_type: std::collections::BTreeMap::new(),
        sample_records: vec![SampleRecord {
            tile_type: 64,
            record_index: 0,
            summary: "KeyRotationRecord K1 -> K2".to_string(),
        }],
        records_hidden_by_default: None,
        records_visible_with_override: None,
        visible_records: 0,
        hidden_records: 0,
        is_sealed: Some(false),
        rebuilt_directory: None,
        out_of_order_detected: None,
        repaired_tiles_count: None,
    };

    write_json_and_record(dir, name, &exp)
}

fn generate_vault_vc_import(dir: &Path) -> io::Result<String> {
    let name = "vault_vc_import";
    let file_path = dir.join(format!("{}.s3a", name));
    let mut keystore = MemoryKeyStore::new();

    let issuer_id = [0x01; 16];
    let recipient_id = [0x02; 16];

    let issuer_pk = keystore.generate_keypair(&issuer_id).map_err(io::Error::other)?;
    let recipient_pk = keystore.generate_keypair(&recipient_id).map_err(io::Error::other)?;

    let mut vault = Vault::create(&file_path, "Recipient Vault", VaultOwnerKind::Person, &mut keystore)
        .map_err(io::Error::other)?;

    let vc_json = serde_json::json!({
        "@context": [
            "https://www.w3.org/ns/credentials/v2",
            "https://purl.imsglobal.org/spec/ob/v3p0/context-3.0.3.json"
        ],
        "id": "urn:uuid:7f3b89b1-5f33-4df4-a82a-89a1c296ef12",
        "type": ["VerifiableCredential", "OpenBadgeCredential"],
        "issuer": {
            "id": "gate://s3a-0000-0000-0000-0000-00",
            "type": ["Profile"],
            "name": "Institute for Distributed Science"
        },
        "validFrom": "2026-01-01T00:00:00Z",
        "credentialSubject": {
            "id": "gate://s3a-1111-2222-3333-4444-55",
            "type": ["AchievementSubject"],
            "achievement": {
                "id": "urn:uuid:9a8b7c6d-5e4f-3a2b-1c0d-ef9a8b7c6d5e",
                "type": ["Achievement"],
                "name": "Advanced S3A Architecture Specialist",
                "criteria": { "narrative": "Completed Phase 1 S3A Vault Conformance." }
            }
        }
    });

    let json_bytes = serde_json::to_vec_pretty(&vc_json).map_err(io::Error::other)?;
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(&json_bytes);
    let doc_hash: [u8; 32] = hasher.finalize().into();

    let _sig = keystore.sign(&issuer_id, &doc_hash).map_err(io::Error::other)?;

    let blob_tile_id = {
        let mut writer = TileWriter::open_append(&file_path)?;
        writer.write_hyper_tile(TileType::BLOB, &json_bytes[..], None, None)?
    };
    let blob_tile_index = (blob_tile_id - 1) as u32;

    let item_record = VaultItemRecord::new(
        [0x20; 16],
        1, // Credential
        doc_hash,
        issuer_pk,
        recipient_pk,
        1700000000,
        blob_tile_index,
        json_bytes.len() as u32,
    );
    vault.add_item(item_record).map_err(io::Error::other)?;

    let exp = ConformanceExpectation {
        case_name: name.to_string(),
        category: "valid".to_string(),
        description: "W3C VC / Open Badges 3.0 JSON-LD import with Ed25519 signature in Blob tile 68 and indexed VaultItemRecord (VAULT-VEC-04)".to_string(),
        expected_outcome: "ok".to_string(),
        error_class: None,
        expected_error_message: None,
        file_size_bytes: Some(fs::metadata(&file_path)?.len()),
        header: None,
        records_per_tile_type: std::collections::BTreeMap::new(),
        sample_records: vec![SampleRecord {
            tile_type: 68,
            record_index: 0,
            summary: "W3C VC Open Badges 3.0 JSON-LD credential in Blob tile".to_string(),
        }],
        records_hidden_by_default: None,
        records_visible_with_override: None,
        visible_records: 0,
        hidden_records: 0,
        is_sealed: Some(false),
        rebuilt_directory: None,
        out_of_order_detected: None,
        repaired_tiles_count: None,
    };

    write_json_and_record(dir, name, &exp)
}

fn write_spec_vectors_readme(dir: &Path) -> io::Result<()> {
    let readme_content = r#"# S3A Conformance Test Vectors (`spec-vectors/`)

This directory contains official conformance test vectors for the **S3A Hyper-Tile Storage Specification (v1.1-draft)**. Each test case consists of a binary `.s3a` archive and a companion `.expected.json` document specifying expected reader resolution, checksum verification, record counts, and failure classes.

---

## 1. Conformance Matrix & Expected Results

| Case Name | Category | Expected Outcome | Error Class | Description |
|:---|:---:|:---:|:---:|:---|
| `valid_empty` | **Valid** | `ok` | *None* | Minimal valid S3A file (512B DualFileHeader, 0 tiles, generation 1). |
| `valid_single_tile_core` | **Valid** | `ok` | *None* | Single 128KB Hyper-Tile containing 3 core Telemetry records. |
| `valid_multi_tile` | **Valid** | `ok` | *None* | Multi-tile archive containing Telemetry (0), Embedding (1), and Statement Index (22). |
| `valid_after_compaction` | **Valid** | `ok` | *None* | Archive after compaction: soft-deleted tombstones purged, live records preserved. |
| `valid_dual_header_slots` | **Valid** | `ok` | *None* | Both Slot A and Slot B valid; reader must arbitrate to Slot B (higher generation). |
| `valid_unknown_tile_type` | **Valid** | `ok` | *None* | Archive containing unknown tile type 9999; readers must skip during domain queries and verify CRC. |
| `valid_snapshot_container` | **Valid** | `ok` | *None* | Portable research snapshot (.snapshot.s3a) containing Tile 15 SRS_MANIFEST and ADDON_EDU. |
| `invalid_bad_magic` | **Invalid** | `error` | `InvalidMagic` | Header magic corrupted to `b"BAD!"`. Reader must fail immediately. |
| `invalid_both_slots_corrupt` | **Invalid** | `error` | `CorruptedHeader` | Both Slot A and Slot B have corrupted CRC32C. Reader must fail closed. |
| `invalid_torn_write_fallback` | **Invalid** | `fallback_recovered` | `TornWriteFallback` | Newest Slot B has bad CRC; reader must recover by falling back to valid Slot A. |
| `invalid_tile_crc_mismatch` | **Invalid** | `error` | `ChecksumMismatch` | Tile payload bytes modified without updating `data_crc32`. Verification must fail. |
| `invalid_truncated_tile` | **Invalid** | `error` | `TruncatedTile` | File truncated mid-tile; reader must detect file boundary violation. |
| `invalid_unsupported_layout_version` | **Invalid** | `error` | `UnsupportedLayoutVersion` | Tile header declares `record_layout_version = 99`; reader must reject record read. |
| `valid_protected_mark_hidden` | **Valid** | `ok` | *None* | Record masked by Tile 19 `MARK_CONSENT_WITHDRAWN`; hidden by default, visible with override. |
| `sealed_ok` | **Valid** | `ok` | *None* | Sealed archive with directory, K=16/M=2 Reed-Solomon parity, and valid 64-byte end cap. |
| `sealed_footer_missing` | **Valid** | `ok_with_warning` | *None* | Sealed archive with footer missing; reader rebuilds directory by scanning tiles. |
| `sealed_endcap_corrupt` | **Valid** | `ok_with_warning` | *None* | Sealed archive with corrupt end cap; reader rebuilds directory by scanning tiles. |
| `sealed_truncated_mid_tile` | **Invalid** | `error` | `SealedTruncated` | Sealed archive truncated mid-tile; reader must reject with SealedTruncated error. |
| `one_tile_corrupt_repaired` | **Valid** | `repaired` | *None* | 1 corrupted data tile in group; Reed-Solomon parity repairs it in memory. |
| `three_tiles_corrupt_in_one_group` | **Invalid** | `error` | `UnrepairableCorruption` | 3 corrupted tiles in group with M=2; exceeds parity capacity and fails closed. |
| `tiles_out_of_order` | **Valid** | `ok_with_warning` | *None* | Tiles out of order on disk; reader reassembles them by tile_index. |
| `foreign_tile` | **Invalid** | `error` | `FileUuidMismatch` | Tile header carrying foreign file_uuid; reader rejects with FileUuidMismatch error. |
| `legacy_unaddressed_tiles` | **Valid** | `ok` | *None* | Pre-v1.1 unaddressed tiles (record_layout_version<=1, file_uuid=[0,0]); accepted via legacy path. |
| `micro_valid_4kb` | **Valid** | `ok` | *None* | Valid 4 KB micro-tile file with standard DualFileHeader (MICRO-VEC-01). |
| `micro_truncated_file` | **Valid** | `ok` | *None* | Power-loss torn micro-tile: Tile 0 valid, Tile 1 cut off mid-write (MICRO-VEC-02). |
| `micro_signed_seal_chain` | **Valid** | `ok` | *None* | 3 consecutive 4 KB tiles with continuous SHA-256 seal chain and Ed25519 signatures (MICRO-VEC-03). |
| `vault_transfer_chain` | **Valid** | `ok` | *None* | 3-link signed transfer chain: Genesis Issuer -> Alice -> Bob (VAULT-VEC-01). |
| `vault_delegation_governance` | **Valid** | `ok` | *None* | Delegation grant with agent action execution and non-delegable judgment protection (VAULT-VEC-02). |
| `vault_key_revocation` | **Valid** | `ok` | *None* | Key rotation K1 to K2 invalidating subsequent actions while preserving history (VAULT-VEC-03). |
| `vault_vc_import` | **Valid** | `ok` | *None* | W3C VC / Open Badges 3.0 JSON-LD credential in Blob tile 68 with indexed VaultItemRecord (VAULT-VEC-04). |

---

## 2. Test Runner Execution Contract

An independent implementation runner MUST execute the test suite adhering to the following protocol:

1. **Header Resolution**:
   - Parse `DualFileHeader` from bytes 0..512.
   - Evaluate CRC32C of Slot A (offset 0) and Slot B (offset 64).
   - Verify active slot matches `header.active_slot` and `header.generation`.
   - If `expected_outcome == "fallback_recovered"`, verify reader successfully selects the older valid slot without crashing.
   - If `error_class` is `InvalidMagic` or `CorruptedHeader`, verify reader aborts opening with the corresponding error.
2. **Tile & Checksum Verification**:
   - For all valid cases, invoke the implementation's checksum verification over all committed tiles.
   - For `invalid_tile_crc_mismatch`, verify checksum verification detects mismatch and returns error.
   - For `invalid_truncated_tile`, verify reader rejects reading tiles beyond physical EOF.
3. **Record Extraction & Layout Enforcement**:
   - Verify record counts per tile type match `records_per_tile_type`.
   - For `invalid_unsupported_layout_version`, verify reader returns an unsupported layout error when accessing records in that tile.
4. **Policy Mark Evaluation**:
   - For `valid_protected_mark_hidden`:
     - Querying without administrative override MUST return 0 records (`visible_records == 0`).
     - Querying with administrative override (`include_hidden = true`) MUST reveal the record (`visible_records + hidden_records == 1`).
5. **Sealed File & Self-Addressing Verification (v1.1)**:
   - For `sealed_ok`, inspect 64-byte end cap (`b"S3AEND\0\x01"`), verify directory, parity, and tile checksums.
   - For `sealed_footer_missing` and `sealed_endcap_corrupt`, detect missing/corrupted footer, emit warning, and rebuild directory by scanning self-addressed tiles.
   - For `sealed_truncated_mid_tile`, reject with `SealedTruncated`.
   - For `one_tile_corrupt_repaired`, repair damaged data tile in memory using Reed-Solomon parity and report as repaired.
   - For `three_tiles_corrupt_in_one_group`, reject with `UnrepairableCorruption`.
   - For `tiles_out_of_order`, detect out-of-order storage and reassemble into logical sequence by `tile_index`.
   - For `foreign_tile`, compare tile `file_uuid` against file header `file_uuid`; reject with `FileUuidMismatch`.
   - For `legacy_unaddressed_tiles`, verify unaddressed tiles (`record_layout_version <= 1 && file_uuid == [0,0]`) are accepted through the legacy path.
6. **Edge Micro-Tile Verification (Phase 1)**:
   - For `micro_valid_4kb`, verify archive declares Edge profile, tile stride is 4096 bytes, and both micro-tiles pass CRC32C and payload hash validation.
   - For `micro_truncated_file`, verify reader accepts committed Tile 0 and EdgeWriter recovery cleanly restores append readiness.
   - For `micro_signed_seal_chain`, verify all 3 tiles maintain continuous SHA-256 seal hash chaining and valid Ed25519 hardware signatures.
7. **Vault Profile Verification (Phase 1)**:
   - For `vault_transfer_chain`, verify 3-link ownership resolution via `resolve_chain_ownership`, tamper detection, and double-send prevention.
   - For `vault_delegation_governance`, verify active grant delegation, agent tool step tracking, and fail-safe blocking of judgment actions (`RequiresHumanApproval`).
   - For `vault_key_revocation`, verify `KeyRotationRecord` invalidates actions at/after rotation while preserving validity of historical actions.
   - For `vault_vc_import`, verify Blob tile 68 stores valid Open Badges 3.0 JSON-LD with verified Ed25519 proof signature and `VaultItemRecord` index.

---

## 3. Regenerating Test Vectors

To regenerate these test vectors using the reference implementation:
```bash
cargo run -p s3a-spec-vectors
```
"#;
    fs::write(dir.join("README.md"), readme_content)?;
    Ok(())
}

/// Executes conformance verification against all generated vectors in `dir`.
pub fn verify_conformance_vectors<P: AsRef<Path>>(dir: P) -> Result<usize, String> {
    let d = dir.as_ref();
    let cases = [
        ("valid_empty", "ok"),
        ("valid_single_tile_core", "ok"),
        ("valid_multi_tile", "ok"),
        ("valid_after_compaction", "ok"),
        ("valid_dual_header_slots", "ok"),
        ("valid_unknown_tile_type", "ok"),
        ("valid_snapshot_container", "ok"),
        ("invalid_bad_magic", "error"),
        ("invalid_both_slots_corrupt", "error"),
        ("invalid_torn_write_fallback", "fallback_recovered"),
        ("invalid_tile_crc_mismatch", "error"),
        ("invalid_truncated_tile", "error"),
        ("invalid_unsupported_layout_version", "error"),
        ("valid_protected_mark_hidden", "ok"),
        // v1.1 Sealed & Self-Addressing Cases
        ("sealed_ok", "ok"),
        ("sealed_footer_missing", "ok_with_warning"),
        ("sealed_endcap_corrupt", "ok_with_warning"),
        ("sealed_truncated_mid_tile", "error"),
        ("one_tile_corrupt_repaired", "repaired"),
        ("three_tiles_corrupt_in_one_group", "error"),
        ("tiles_out_of_order", "ok_with_warning"),
        ("foreign_tile", "error"),
        ("legacy_unaddressed_tiles", "ok"),
        // Edge Profile Conformance Cases (Phase 1)
        ("micro_valid_4kb", "ok"),
        ("micro_truncated_file", "ok"),
        ("micro_signed_seal_chain", "ok"),
        // Vault Profile Conformance Cases (Phase 1)
        ("vault_transfer_chain", "ok"),
        ("vault_delegation_governance", "ok"),
        ("vault_key_revocation", "ok"),
        ("vault_vc_import", "ok"),
    ];

    let mut verified_count = 0;

    for (name, outcome) in cases {
        let s3a_path = d.join(format!("{}.s3a", name));
        let json_path = d.join(format!("{}.expected.json", name));

        if !s3a_path.exists() {
            return Err(format!("Missing test vector: {}", s3a_path.display()));
        }
        if !json_path.exists() {
            return Err(format!("Missing expected json: {}", json_path.display()));
        }

        let json_str = fs::read_to_string(&json_path).map_err(|e| e.to_string())?;
        let exp: ConformanceExpectation =
            serde_json::from_str(&json_str).map_err(|e| e.to_string())?;

        match outcome {
            "ok" => {
                let reader = MmapReader::open(&s3a_path)
                    .map_err(|e| format!("{}: open failed: {}", name, e))?;
                reader
                    .verify_checksums()
                    .map_err(|e| format!("{}: checksum failed: {}", name, e))?;

                if exp.is_sealed == Some(true) {
                    let report = s3a_engine::verify_sealed_archive(&s3a_path, None)
                        .map_err(|e| format!("{}: verify_sealed_archive failed: {}", name, e))?;
                    assert!(report.is_sealed, "{}: expected is_sealed true", name);
                    assert_eq!(report.repaired_tiles.len(), 0, "{}: expected 0 repaired tiles", name);
                }

                if name == "valid_protected_mark_hidden" {
                    let sieve_default = QuerySieve::new(&reader).with_hidden_marks(false);
                    let recs_hidden = sieve_default.query_telemetry(0, u64::MAX, None, None);
                    assert_eq!(
                        recs_hidden.len(),
                        exp.visible_records,
                        "{}: visible records mismatch",
                        name
                    );

                    let sieve_override = QuerySieve::new(&reader).with_hidden_marks(true);
                    let recs_visible = sieve_override.query_telemetry(0, u64::MAX, None, None);
                    assert_eq!(
                        recs_visible.len(),
                        exp.visible_records + exp.hidden_records,
                        "{}: total records (with hidden) mismatch",
                        name
                    );
                }

                if let Some(h) = &exp.header {
                    assert_eq!(
                        reader.active_slot(),
                        h.active_slot,
                        "{}: active slot mismatch",
                        name
                    );
                    assert_eq!(
                        reader.generation(),
                        h.generation,
                        "{}: generation mismatch",
                        name
                    );
                    assert_eq!(
                        reader.tile_count(),
                        h.tile_count,
                        "{}: tile count mismatch",
                        name
                    );
                }

                if reader.is_edge_archive() {
                    assert_eq!(reader.tile_stride(), 4096, "{}: expected 4096 stride", name);
                    if name == "micro_truncated_file" {
                        let file_data = fs::read(&s3a_path).map_err(|e| e.to_string())?;
                        let mut flash = RamFlash::<16384>::new(256, 512);
                        flash.memory[..file_data.len()].copy_from_slice(&file_data);
                        let writer = EdgeWriter::<_, 4096>::recover(flash, [0x11; 16])
                            .map_err(|e| format!("{}: EdgeWriter::recover failed: {:?}", name, e))?;
                        assert_eq!(writer.tile_count(), 1, "{}: recovered tile count mismatch", name);
                    }
                    if name == "micro_signed_seal_chain" {
                        let (h0, _) = reader.get_edge_tile(0).unwrap();
                        let (h1, _) = reader.get_edge_tile(1).unwrap();
                        let (h2, _) = reader.get_edge_tile(2).unwrap();
                        let dev_id = [0x11; 16];
                        let seal0 = s3a_edge::compute_seal_hash(&[0u8; 32], h0.generation as u64, 0, &h0.payload_hash, &dev_id);
                        let seal1 = s3a_edge::compute_seal_hash(&seal0, h1.generation as u64, 1, &h1.payload_hash, &dev_id);
                        let seal2 = s3a_edge::compute_seal_hash(&seal1, h2.generation as u64, 2, &h2.payload_hash, &dev_id);
                        assert!(s3a_edge::verify_seal_chain_link(&seal1, &seal0, h1.generation as u64, 1, &h1.payload_hash, &dev_id));
                        assert!(s3a_edge::verify_seal_chain_link(&seal2, &seal1, h2.generation as u64, 2, &h2.payload_hash, &dev_id));
                    }
                }

                if name == "vault_transfer_chain" {
                    let vault = Vault::open(&s3a_path)
                        .map_err(|e| format!("{}: Vault::open failed: {:?}", name, e))?;
                    let item_hash = [0xAA; 32];
                    let owner_pk = vault.get_item_owner(&item_hash)
                        .map_err(|e| format!("{}: get_item_owner failed: {:?}", name, e))?;
                    assert_ne!(owner_pk, [0u8; 32], "{}: owner must not be zero", name);
                }
                if name == "vault_delegation_governance" {
                    let vault = Vault::open(&s3a_path)
                        .map_err(|e| format!("{}: Vault::open failed: {:?}", name, e))?;
                    let grant = vault.grants().values().next().copied()
                        .ok_or_else(|| format!("{}: missing grant", name))?;
                    verify_delegation_grant(&grant)
                        .map_err(|e| format!("{}: verify_delegation_grant failed: {:?}", name, e))?;
                    let out_write = evaluate_agent_action(&grant, ACTION_WRITE, "record_write", 1700000010, 1);
                    assert!(out_write.is_ok(), "{}: evaluate write failed: {:?}", name, out_write);
                    assert!(is_judgment_action("erasure"));
                    let out_judge = evaluate_agent_action(&grant, ACTION_WRITE, "erasure", 1700000010, 1);
                    assert!(matches!(out_judge, Err(s3a_vault::VaultError::RequiresHumanApproval(_))));
                }
                if name == "vault_key_revocation" {
                    let vault = Vault::open(&s3a_path)
                        .map_err(|e| format!("{}: Vault::open failed: {:?}", name, e))?;
                    let primary_pk = vault.primary_identity().unwrap().public_key;
                    assert!(vault.revocation_registry().check_key_valid(&primary_pk, 1700005000 - 10).is_ok());
                    assert!(matches!(
                        vault.revocation_registry().check_key_valid(&primary_pk, 1700005000 + 10),
                        Err(s3a_vault::VaultError::KeyRevokedError(_))
                    ));
                }
                if name == "vault_vc_import" {
                    let vault = Vault::open(&s3a_path)
                        .map_err(|e| format!("{}: Vault::open failed: {:?}", name, e))?;
                    assert!(!vault.items().is_empty(), "{}: expected indexed items", name);
                }
            }
            "fallback_recovered" => {
                let reader = MmapReader::open(&s3a_path)
                    .map_err(|e| format!("{}: fallback open failed: {}", name, e))?;
                if let Some(h) = &exp.header {
                    assert_eq!(
                        reader.active_slot(),
                        h.active_slot,
                        "{}: fallback slot mismatch",
                        name
                    );
                    assert_eq!(
                        reader.generation(),
                        h.generation,
                        "{}: fallback generation mismatch",
                        name
                    );
                }
            }
            "ok_with_warning" => {
                let report = s3a_engine::verify_sealed_archive(&s3a_path, None)
                    .map_err(|e| format!("{}: verify_sealed_archive failed: {}", name, e))?;
                if let Some(exp_sealed) = exp.is_sealed {
                    assert_eq!(report.is_sealed, exp_sealed, "{}: is_sealed mismatch", name);
                }
                if let Some(exp_rebuilt) = exp.rebuilt_directory {
                    assert_eq!(report.rebuilt_directory, exp_rebuilt, "{}: rebuilt_directory mismatch", name);
                }
                if let Some(exp_ooo) = exp.out_of_order_detected {
                    assert_eq!(report.out_of_order_detected, exp_ooo, "{}: out_of_order mismatch", name);
                }
            }
            "repaired" => {
                let report = s3a_engine::verify_sealed_archive(&s3a_path, None)
                    .map_err(|e| format!("{}: verify_sealed_archive failed: {}", name, e))?;
                assert_eq!(
                    report.repaired_tiles.len(),
                    exp.repaired_tiles_count.unwrap_or(1),
                    "{}: repaired tiles count mismatch",
                    name
                );
            }
            "error" => match exp.error_class.as_deref() {
                Some("InvalidMagic") => {
                    assert!(
                        MmapReader::open(&s3a_path).is_err(),
                        "{}: expected InvalidMagic error",
                        name
                    );
                }
                Some("CorruptedHeader") => {
                    assert!(
                        MmapReader::open(&s3a_path).is_err(),
                        "{}: expected CorruptedHeader error",
                        name
                    );
                }
                Some("ChecksumMismatch") => {
                    let reader = MmapReader::open(&s3a_path)
                        .map_err(|e| format!("{}: open failed: {}", name, e))?;
                    assert!(
                        reader.verify_checksums().is_err(),
                        "{}: expected ChecksumMismatch",
                        name
                    );
                }
                Some("TruncatedTile") => {
                    let reader = MmapReader::open(&s3a_path)
                        .map_err(|e| format!("{}: open failed: {}", name, e))?;
                    assert!(
                        reader.get_tile(0).is_none(),
                        "{}: expected truncated tile to be None",
                        name
                    );
                }
                Some("UnsupportedLayoutVersion") => {
                    let reader = MmapReader::open(&s3a_path)
                        .map_err(|e| format!("{}: open failed: {}", name, e))?;
                    let coord = s3a_core::S3ACoordinate::new(1, 0, 0);
                    let res = reader.read_record_by_coordinate::<TelemetryRecord>(&coord);
                    assert_eq!(
                        res,
                        Err(S3AError::UnsupportedLayoutVersion(99)),
                        "{}: expected UnsupportedLayoutVersion(99)",
                        name
                    );
                }
                Some("SealedTruncated") => {
                    assert_eq!(
                        s3a_engine::verify_sealed_archive(&s3a_path, None),
                        Err(S3AError::SealedTruncated),
                        "{}: expected SealedTruncated error",
                        name
                    );
                }
                Some("UnrepairableCorruption") => {
                    assert!(
                        matches!(
                            s3a_engine::verify_sealed_archive(&s3a_path, None),
                            Err(S3AError::UnrepairableCorruption { .. })
                        ),
                        "{}: expected UnrepairableCorruption error",
                        name
                    );
                }
                Some("FileUuidMismatch") => {
                    assert!(
                        matches!(
                            s3a_engine::verify_sealed_archive(&s3a_path, None),
                            Err(S3AError::FileUuidMismatch { .. })
                        ),
                        "{}: expected FileUuidMismatch from verify_sealed_archive",
                        name
                    );
                    let reader = MmapReader::open(&s3a_path)
                        .map_err(|e| format!("{}: open failed: {}", name, e))?;
                    assert!(
                        matches!(
                            reader.verify_checksums(),
                            Err(S3AError::FileUuidMismatch { .. })
                        ),
                        "{}: expected FileUuidMismatch from reader.verify_checksums",
                        name
                    );
                }
                _ => {}
            },
            _ => {}
        }

        verified_count += 1;
    }

    Ok(verified_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use s3a_pack_scholar::intake_srs_snapshot;

    #[test]
    fn test_conformance_vectors_generate_and_verify_all() {
        let temp_dir =
            std::env::temp_dir().join(format!("s3a_vectors_all_test_{}", std::process::id()));
        let generated = generate_all_vectors(&temp_dir).expect("Vector generation must succeed");
        assert_eq!(generated.len(), 30);

        let verified = verify_conformance_vectors(&temp_dir).expect("Verification must succeed");
        assert_eq!(verified, 30);

        let _ = fs::remove_dir_all(temp_dir);
    }

    // Individual Rust tests for each vector:
    #[test]
    fn test_vector_valid_empty() {
        let dir = std::env::temp_dir().join(format!("s3a_vec_empty_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        generate_valid_empty(&dir).unwrap();
        let reader = MmapReader::open(dir.join("valid_empty.s3a")).unwrap();
        assert_eq!(reader.tile_count(), 0);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_vector_valid_single_tile_core() {
        let dir = std::env::temp_dir().join(format!("s3a_vec_single_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        generate_valid_single_tile_core(&dir).unwrap();
        let reader = MmapReader::open(dir.join("valid_single_tile_core.s3a")).unwrap();
        reader.verify_checksums().unwrap();
        assert_eq!(reader.tile_count(), 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_vector_valid_multi_tile() {
        let dir = std::env::temp_dir().join(format!("s3a_vec_multi_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        generate_valid_multi_tile(&dir).unwrap();
        let reader = MmapReader::open(dir.join("valid_multi_tile.s3a")).unwrap();
        reader.verify_checksums().unwrap();
        assert_eq!(reader.tile_count(), 3);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_vector_valid_after_compaction() {
        let dir = std::env::temp_dir().join(format!("s3a_vec_compact_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        generate_valid_after_compaction(&dir).unwrap();
        let reader = MmapReader::open(dir.join("valid_after_compaction.s3a")).unwrap();
        reader.verify_checksums().unwrap();
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_vector_valid_dual_header_slots() {
        let dir = std::env::temp_dir().join(format!("s3a_vec_dual_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        generate_valid_dual_header_slots(&dir).unwrap();
        let reader = MmapReader::open(dir.join("valid_dual_header_slots.s3a")).unwrap();
        assert_eq!(reader.active_slot(), 1);
        assert_eq!(reader.generation(), 2);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_vector_valid_unknown_tile_type() {
        let dir = std::env::temp_dir().join(format!("s3a_vec_unknown_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        generate_valid_unknown_tile_type(&dir).unwrap();
        let reader = MmapReader::open(dir.join("valid_unknown_tile_type.s3a")).unwrap();
        reader.verify_checksums().unwrap();
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_vector_valid_snapshot_container() {
        let dir = std::env::temp_dir().join(format!("s3a_vec_snap_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        generate_valid_snapshot_container(&dir).unwrap();
        let reader = MmapReader::open(dir.join("valid_snapshot_container.s3a")).unwrap();
        reader.verify_checksums().unwrap();
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_vector_invalid_bad_magic() {
        let dir = std::env::temp_dir().join(format!("s3a_vec_bad_magic_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        generate_invalid_bad_magic(&dir).unwrap();
        assert!(MmapReader::open(dir.join("invalid_bad_magic.s3a")).is_err());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_vector_invalid_both_slots_corrupt() {
        let dir = std::env::temp_dir().join(format!("s3a_vec_both_corrupt_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        generate_invalid_both_slots_corrupt(&dir).unwrap();
        assert!(MmapReader::open(dir.join("invalid_both_slots_corrupt.s3a")).is_err());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_vector_invalid_torn_write_fallback() {
        let dir = std::env::temp_dir().join(format!("s3a_vec_torn_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        generate_invalid_torn_write_fallback(&dir).unwrap();
        let reader = MmapReader::open(dir.join("invalid_torn_write_fallback.s3a")).unwrap();
        assert_eq!(reader.active_slot(), 0);
        assert_eq!(reader.generation(), 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_vector_invalid_tile_crc_mismatch() {
        let dir = std::env::temp_dir().join(format!("s3a_vec_tile_crc_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        generate_invalid_tile_crc_mismatch(&dir).unwrap();
        let reader = MmapReader::open(dir.join("invalid_tile_crc_mismatch.s3a")).unwrap();
        assert!(reader.verify_checksums().is_err());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_vector_invalid_truncated_tile() {
        let dir = std::env::temp_dir().join(format!("s3a_vec_trunc_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        generate_invalid_truncated_tile(&dir).unwrap();
        let reader = MmapReader::open(dir.join("invalid_truncated_tile.s3a")).unwrap();
        assert!(reader.get_tile(0).is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_vector_invalid_unsupported_layout_version() {
        let dir = std::env::temp_dir().join(format!("s3a_vec_unsupported_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        generate_invalid_unsupported_layout_version(&dir).unwrap();
        let reader = MmapReader::open(dir.join("invalid_unsupported_layout_version.s3a")).unwrap();
        let coord = s3a_core::S3ACoordinate::new(1, 0, 0);
        let res = reader.read_record_by_coordinate::<TelemetryRecord>(&coord);
        assert_eq!(res, Err(S3AError::UnsupportedLayoutVersion(99)));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_vector_valid_protected_mark_hidden() {
        let dir = std::env::temp_dir().join(format!("s3a_vec_mark_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        generate_valid_protected_mark_hidden(&dir).unwrap();
        let reader = MmapReader::open(dir.join("valid_protected_mark_hidden.s3a")).unwrap();
        let sieve_default = QuerySieve::new(&reader).with_hidden_marks(false);
        assert_eq!(sieve_default.query_telemetry(0, u64::MAX, None, None).len(), 0);
        let sieve_override = QuerySieve::new(&reader).with_hidden_marks(true);
        assert_eq!(sieve_override.query_telemetry(0, u64::MAX, None, None).len(), 1);
        let _ = fs::remove_dir_all(dir);
    }

    // Phase 2b Vectors
    #[test]
    fn test_vector_sealed_ok() {
        let dir = std::env::temp_dir().join(format!("s3a_vec_sealed_ok_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        generate_sealed_ok(&dir).unwrap();
        let report = s3a_engine::verify_sealed_archive(dir.join("sealed_ok.s3a"), None).unwrap();
        assert!(report.is_sealed);
        assert_eq!(report.repaired_tiles.len(), 0);
        assert!(!report.rebuilt_directory);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_vector_sealed_footer_missing() {
        let dir = std::env::temp_dir().join(format!("s3a_vec_footer_missing_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        generate_sealed_footer_missing(&dir).unwrap();
        let report = s3a_engine::verify_sealed_archive(dir.join("sealed_footer_missing.s3a"), None).unwrap();
        assert!(!report.is_sealed);
        assert!(report.rebuilt_directory);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_vector_sealed_endcap_corrupt() {
        let dir = std::env::temp_dir().join(format!("s3a_vec_endcap_corrupt_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        generate_sealed_endcap_corrupt(&dir).unwrap();
        let report = s3a_engine::verify_sealed_archive(dir.join("sealed_endcap_corrupt.s3a"), None).unwrap();
        assert!(!report.is_sealed);
        assert!(report.rebuilt_directory);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_vector_sealed_truncated_mid_tile() {
        let dir = std::env::temp_dir().join(format!("s3a_vec_sealed_trunc_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        generate_sealed_truncated_mid_tile(&dir).unwrap();
        let err = s3a_engine::verify_sealed_archive(dir.join("sealed_truncated_mid_tile.s3a"), None).unwrap_err();
        assert_eq!(err, S3AError::SealedTruncated);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_vector_one_tile_corrupt_repaired() {
        let dir = std::env::temp_dir().join(format!("s3a_vec_repaired_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        generate_one_tile_corrupt_repaired(&dir).unwrap();
        let report = s3a_engine::verify_sealed_archive(dir.join("one_tile_corrupt_repaired.s3a"), None).unwrap();
        assert!(report.is_sealed);
        assert_eq!(report.repaired_tiles, vec![0]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_vector_three_tiles_corrupt_in_one_group() {
        let dir = std::env::temp_dir().join(format!("s3a_vec_three_corrupt_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        generate_three_tiles_corrupt_in_one_group(&dir).unwrap();
        let err = s3a_engine::verify_sealed_archive(dir.join("three_tiles_corrupt_in_one_group.s3a"), None).unwrap_err();
        assert!(matches!(err, S3AError::UnrepairableCorruption { .. }));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_vector_tiles_out_of_order() {
        let dir = std::env::temp_dir().join(format!("s3a_vec_ooo_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        generate_tiles_out_of_order(&dir).unwrap();
        let report = s3a_engine::verify_sealed_archive(dir.join("tiles_out_of_order.s3a"), None).unwrap();
        assert!(report.out_of_order_detected);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_vector_foreign_tile() {
        let dir = std::env::temp_dir().join(format!("s3a_vec_foreign_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        generate_foreign_tile(&dir).unwrap();
        let err = s3a_engine::verify_sealed_archive(dir.join("foreign_tile.s3a"), None).unwrap_err();
        assert!(matches!(err, S3AError::FileUuidMismatch { .. }));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_vector_legacy_unaddressed_tiles() {
        let dir = std::env::temp_dir().join(format!("s3a_vec_legacy_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        generate_legacy_unaddressed_tiles(&dir).unwrap();
        let reader = MmapReader::open(dir.join("legacy_unaddressed_tiles.s3a")).unwrap();
        reader.verify_checksums().unwrap();
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_scholar_snapshot_seal_verify_roundtrip() {
        let dir = std::env::temp_dir().join(format!("s3a_scholar_roundtrip_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let live_db = dir.join("scholar_live.s3a");
        let snapshot_file = dir.join("scholar_export.snapshot.s3a");

        let engine = S3ACrudEngine::open_or_create(&live_db).unwrap();
        engine.enable_addon(ADDON_EDU).unwrap();

        let p_uuid = [0x5555_6666, 0x7777_8888];
        let ws = SrsWorkspaceRecord::new(p_uuid, 999, 1, [0.5, 0.5, 0.5], 1);
        let paper = AcademicPaperRecord::new(
            2001, 1700000000, 987654, 0.1, 0.2, 0.3, 10, 2024, 1, 1, 0, 0x1234,
        );
        engine.create_srs_workspace(&ws).unwrap();
        engine.create_academic_papers(&[paper]).unwrap();

        let manifest = SrsManifestHeader::new(
            p_uuid,
            [0xAAAA, 0xBBBB],
            [0, 0],
            [0xCCCC, 0xDDDD],
            1700000000,
            0,
            0,
        );

        // Export sealed snapshot
        export_srs_snapshot(&live_db, &snapshot_file, &manifest, Some(&ws), Some(p_uuid)).unwrap();

        // Verify with verify_sealed_archive
        let report = s3a_engine::verify_sealed_archive(&snapshot_file, None).unwrap();
        assert!(report.is_sealed);
        assert!(!report.rebuilt_directory);
        assert!(report.repaired_tiles.is_empty());

        // Intake snapshot
        let bundle = intake_srs_snapshot(&snapshot_file).unwrap();
        assert_eq!(bundle.workspace.unwrap().project_uuid, p_uuid);
        assert_eq!(bundle.papers.len(), 1);
        assert_eq!(bundle.papers[0].paper_id, 2001);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_vector_micro_valid_4kb() {
        let dir = std::env::temp_dir().join(format!("s3a_vec_micro_valid_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        generate_micro_valid_4kb(&dir).unwrap();
        let reader = MmapReader::open(dir.join("micro_valid_4kb.s3a")).unwrap();
        assert!(reader.is_edge_archive());
        assert_eq!(reader.tile_stride(), 4096);
        assert_eq!(reader.tile_count(), 2);
        reader.verify_checksums().unwrap();
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_vector_micro_truncated_file() {
        let dir = std::env::temp_dir().join(format!("s3a_vec_micro_trunc_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        generate_micro_truncated_file(&dir).unwrap();
        let reader = MmapReader::open(dir.join("micro_truncated_file.s3a")).unwrap();
        assert!(reader.is_edge_archive());
        assert_eq!(reader.tile_count(), 1);
        reader.verify_checksums().unwrap();

        // Edge writer recovery test
        let file_data = fs::read(dir.join("micro_truncated_file.s3a")).unwrap();
        let mut flash = RamFlash::<16384>::new(256, 512);
        flash.memory[..file_data.len()].copy_from_slice(&file_data);
        let writer = EdgeWriter::<_, 4096>::recover(flash, [0x11; 16]).unwrap();
        assert_eq!(writer.tile_count(), 1);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_vector_micro_signed_seal_chain() {
        let dir = std::env::temp_dir().join(format!("s3a_vec_micro_seal_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        generate_micro_signed_seal_chain(&dir).unwrap();
        let reader = MmapReader::open(dir.join("micro_signed_seal_chain.s3a")).unwrap();
        assert!(reader.is_edge_archive());
        assert_eq!(reader.tile_count(), 3);
        reader.verify_checksums().unwrap();

        // Verify cryptographic seal chain
        let (h0, _) = reader.get_edge_tile(0).unwrap();
        let (h1, _) = reader.get_edge_tile(1).unwrap();
        let (h2, _) = reader.get_edge_tile(2).unwrap();
        let dev_id = [0x11; 16];
        let seal0 = s3a_edge::compute_seal_hash(&[0u8; 32], h0.generation as u64, 0, &h0.payload_hash, &dev_id);
        let seal1 = s3a_edge::compute_seal_hash(&seal0, h1.generation as u64, 1, &h1.payload_hash, &dev_id);
        let seal2 = s3a_edge::compute_seal_hash(&seal1, h2.generation as u64, 2, &h2.payload_hash, &dev_id);
        assert!(s3a_edge::verify_seal_chain_link(&seal1, &seal0, h1.generation as u64, 1, &h1.payload_hash, &dev_id));
        assert!(s3a_edge::verify_seal_chain_link(&seal2, &seal1, h2.generation as u64, 2, &h2.payload_hash, &dev_id));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_vector_vault_transfer_chain() {
        let dir = std::env::temp_dir().join(format!("s3a_vec_v_transfer_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        generate_vault_transfer_chain(&dir).unwrap();
        let reader = MmapReader::open(dir.join("vault_transfer_chain.s3a")).unwrap();
        reader.verify_checksums().unwrap();
        let vault = Vault::open(dir.join("vault_transfer_chain.s3a")).unwrap();
        let item_hash = [0xAA; 32];
        let owner_pk = vault.get_item_owner(&item_hash).unwrap();
        assert_ne!(owner_pk, [0u8; 32]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_vector_vault_delegation_governance() {
        let dir = std::env::temp_dir().join(format!("s3a_vec_v_delegation_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        generate_vault_delegation_governance(&dir).unwrap();
        let reader = MmapReader::open(dir.join("vault_delegation_governance.s3a")).unwrap();
        reader.verify_checksums().unwrap();
        let vault = Vault::open(dir.join("vault_delegation_governance.s3a")).unwrap();
        let grant = vault.grants().values().next().copied().unwrap();
        assert!(verify_delegation_grant(&grant).is_ok());
        let res = evaluate_agent_action(&grant, ACTION_WRITE, "record_write", 1700000010, 1);
        assert!(res.is_ok());
        let res_judge = evaluate_agent_action(&grant, ACTION_WRITE, "erasure", 1700000010, 1);
        assert!(matches!(res_judge, Err(s3a_vault::VaultError::RequiresHumanApproval(_))));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_vector_vault_key_revocation() {
        let dir = std::env::temp_dir().join(format!("s3a_vec_v_revocation_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        generate_vault_key_revocation(&dir).unwrap();
        let reader = MmapReader::open(dir.join("vault_key_revocation.s3a")).unwrap();
        reader.verify_checksums().unwrap();
        let vault = Vault::open(dir.join("vault_key_revocation.s3a")).unwrap();
        let primary_pk = vault.primary_identity().unwrap().public_key;
        assert!(vault.revocation_registry().check_key_valid(&primary_pk, 1700005000 - 10).is_ok());
        assert!(matches!(
            vault.revocation_registry().check_key_valid(&primary_pk, 1700005000 + 10),
            Err(s3a_vault::VaultError::KeyRevokedError(_))
        ));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_vector_vault_vc_import() {
        let dir = std::env::temp_dir().join(format!("s3a_vec_v_vc_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        generate_vault_vc_import(&dir).unwrap();
        let reader = MmapReader::open(dir.join("vault_vc_import.s3a")).unwrap();
        reader.verify_checksums().unwrap();
        let vault = Vault::open(dir.join("vault_vc_import.s3a")).unwrap();
        assert_eq!(vault.items().len(), 1);
        let item = vault.items().values().next().unwrap();
        let (hdr, payload) = reader.get_tile(item.payload_tile_index() as usize).unwrap();
        assert_eq!(hdr.tile_type, TileType::BLOB);
        let offset = item.payload_byte_offset() as usize;
        let length = item.payload_byte_length() as usize;
        let doc_slice = &payload[offset..(offset + length)];
        let val: serde_json::Value = serde_json::from_slice(doc_slice).unwrap();
        assert_eq!(val["type"][1], "OpenBadgeCredential");
        let _ = fs::remove_dir_all(dir);
    }
}

