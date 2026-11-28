//! Verifies that `docs/spec/scholar_pack_v1.schema.json` matches the
//! implemented Rust structs in `s3a_pack_scholar::records`.

use s3a_pack_scholar::{
    AcademicPaperRecord, AiTraceRecord, HumanLrsRecord, ResearchGraphEdgeRecord,
    ScholarNoteClaimRecord, SrsManifestHeader, SrsWorkspaceRecord, SCHOLAR_LAYOUT_VERSION_V1,
};
use serde_json::Value;
use std::mem::{align_of, offset_of, size_of};

const SCHEMA_JSON: &str = include_str!("../../../docs/spec/scholar_pack_v1.schema.json");

fn assert_field(fields: &[Value], name: &str, expected_offset: usize, expected_len: usize) {
    let f = fields
        .iter()
        .find(|v| v["name"].as_str() == Some(name))
        .unwrap_or_else(|| panic!("missing field '{name}' in scholar_pack_v1.schema.json"));
    assert_eq!(
        f["byte_offset"].as_u64().unwrap() as usize,
        expected_offset,
        "byte_offset mismatch for '{name}'"
    );
    assert_eq!(
        f["byte_len"].as_u64().unwrap() as usize,
        expected_len,
        "byte_len mismatch for '{name}'"
    );
}

#[test]
fn scholar_pack_v1_schema_matches_rust_layouts() {
    let doc: Value = serde_json::from_str(SCHEMA_JSON).expect("valid JSON");
    assert_eq!(
        doc["record_layout_version"].as_u64().unwrap() as u32,
        SCHOLAR_LAYOUT_VERSION_V1
    );

    let tiles = doc["tiles"].as_object().unwrap();

    // Tile 10: HumanLrsRecord
    let t10 = &tiles["10"];
    assert_eq!(t10["record_size"].as_u64().unwrap() as usize, size_of::<HumanLrsRecord>());
    assert_eq!(t10["align"].as_u64().unwrap() as usize, align_of::<HumanLrsRecord>());
    let f10 = t10["fields"].as_array().unwrap();
    assert_field(f10, "session_uuid", offset_of!(HumanLrsRecord, session_uuid), 16);
    assert_field(f10, "timestamp_sec", offset_of!(HumanLrsRecord, timestamp_sec), 8);
    assert_field(f10, "actor_hash", offset_of!(HumanLrsRecord, actor_hash), 8);
    assert_field(f10, "verb_id", offset_of!(HumanLrsRecord, verb_id), 4);
    assert_field(f10, "decision_score", offset_of!(HumanLrsRecord, decision_score), 4);
    assert_field(f10, "ai_trace_coord", offset_of!(HumanLrsRecord, ai_trace_coord), 16);
    assert_field(f10, "object_hash", offset_of!(HumanLrsRecord, object_hash), 4);
    assert_field(f10, "flags", offset_of!(HumanLrsRecord, flags), 4);

    // Tile 11: AiTraceRecord
    let t11 = &tiles["11"];
    assert_eq!(t11["record_size"].as_u64().unwrap() as usize, size_of::<AiTraceRecord>());
    assert_eq!(t11["align"].as_u64().unwrap() as usize, align_of::<AiTraceRecord>());
    let f11 = t11["fields"].as_array().unwrap();
    assert_field(f11, "session_uuid", offset_of!(AiTraceRecord, session_uuid), 16);
    assert_field(f11, "timestamp_sec", offset_of!(AiTraceRecord, timestamp_sec), 8);
    assert_field(f11, "agent_id", offset_of!(AiTraceRecord, agent_id), 8);
    assert_field(f11, "step_type", offset_of!(AiTraceRecord, step_type), 4);
    assert_field(f11, "confidence", offset_of!(AiTraceRecord, confidence), 4);
    assert_field(f11, "human_coord", offset_of!(AiTraceRecord, human_coord), 16);
    assert_field(f11, "hilbert_index", offset_of!(AiTraceRecord, hilbert_index), 4);
    assert_field(f11, "flags", offset_of!(AiTraceRecord, flags), 4);

    // Tile 12: AcademicPaperRecord
    let t12 = &tiles["12"];
    assert_eq!(t12["record_size"].as_u64().unwrap() as usize, size_of::<AcademicPaperRecord>());
    assert_eq!(t12["align"].as_u64().unwrap() as usize, align_of::<AcademicPaperRecord>());
    let f12 = t12["fields"].as_array().unwrap();
    assert_field(f12, "paper_id", offset_of!(AcademicPaperRecord, paper_id), 8);
    assert_field(f12, "timestamp_sec", offset_of!(AcademicPaperRecord, timestamp_sec), 8);
    assert_field(f12, "hilbert_index", offset_of!(AcademicPaperRecord, hilbert_index), 8);
    assert_field(f12, "topic_x", offset_of!(AcademicPaperRecord, topic_x), 4);
    assert_field(f12, "topic_y", offset_of!(AcademicPaperRecord, topic_y), 4);
    assert_field(f12, "topic_z", offset_of!(AcademicPaperRecord, topic_z), 4);
    assert_field(f12, "citation_count", offset_of!(AcademicPaperRecord, citation_count), 4);
    assert_field(f12, "year", offset_of!(AcademicPaperRecord, year), 2);
    assert_field(f12, "venue_id", offset_of!(AcademicPaperRecord, venue_id), 2);
    assert_field(f12, "open_access_flag", offset_of!(AcademicPaperRecord, open_access_flag), 2);
    assert_field(f12, "warning_count", offset_of!(AcademicPaperRecord, warning_count), 2);
    assert_field(f12, "doi_prefix_hash", offset_of!(AcademicPaperRecord, doi_prefix_hash), 8);
    assert_field(f12, "flags", offset_of!(AcademicPaperRecord, flags), 8);

    // Tile 13: ResearchGraphEdgeRecord
    let t13 = &tiles["13"];
    assert_eq!(t13["record_size"].as_u64().unwrap() as usize, size_of::<ResearchGraphEdgeRecord>());
    assert_eq!(t13["align"].as_u64().unwrap() as usize, align_of::<ResearchGraphEdgeRecord>());
    let f13 = t13["fields"].as_array().unwrap();
    assert_field(f13, "subject_hash", offset_of!(ResearchGraphEdgeRecord, subject_hash), 8);
    assert_field(f13, "object_hash", offset_of!(ResearchGraphEdgeRecord, object_hash), 8);
    assert_field(f13, "timestamp_sec", offset_of!(ResearchGraphEdgeRecord, timestamp_sec), 8);
    assert_field(f13, "hilbert_coord", offset_of!(ResearchGraphEdgeRecord, hilbert_coord), 8);
    assert_field(f13, "predicate_id", offset_of!(ResearchGraphEdgeRecord, predicate_id), 4);
    assert_field(f13, "weight", offset_of!(ResearchGraphEdgeRecord, weight), 4);
    assert_field(f13, "flags", offset_of!(ResearchGraphEdgeRecord, flags), 8);
    assert_field(f13, "reserved", offset_of!(ResearchGraphEdgeRecord, _reserved), 16);

    // Tile 14: SrsWorkspaceRecord
    let t14 = &tiles["14"];
    assert_eq!(t14["record_size"].as_u64().unwrap() as usize, size_of::<SrsWorkspaceRecord>());
    assert_eq!(t14["align"].as_u64().unwrap() as usize, align_of::<SrsWorkspaceRecord>());
    let f14 = t14["fields"].as_array().unwrap();
    assert_field(f14, "project_uuid", offset_of!(SrsWorkspaceRecord, project_uuid), 16);
    assert_field(f14, "question_hash", offset_of!(SrsWorkspaceRecord, question_hash), 8);
    assert_field(f14, "collection_mask", offset_of!(SrsWorkspaceRecord, collection_mask), 8);
    assert_field(f14, "topic_centroid_xyz", offset_of!(SrsWorkspaceRecord, topic_centroid_xyz), 12);
    assert_field(f14, "status", offset_of!(SrsWorkspaceRecord, status), 4);
    assert_field(f14, "pinned_works_count", offset_of!(SrsWorkspaceRecord, pinned_works_count), 4);
    assert_field(f14, "claims_count", offset_of!(SrsWorkspaceRecord, claims_count), 4);
    assert_field(f14, "flags", offset_of!(SrsWorkspaceRecord, flags), 8);

    // Tile 15: SrsManifestHeader
    let t15 = &tiles["15"];
    assert_eq!(t15["record_size"].as_u64().unwrap() as usize, size_of::<SrsManifestHeader>());
    assert_eq!(t15["align"].as_u64().unwrap() as usize, align_of::<SrsManifestHeader>());
    let f15 = t15["fields"].as_array().unwrap();
    assert_field(f15, "magic", offset_of!(SrsManifestHeader, magic), 8);
    assert_field(f15, "schema_version", offset_of!(SrsManifestHeader, schema_version), 4);
    assert_field(f15, "archive_flags", offset_of!(SrsManifestHeader, archive_flags), 4);
    assert_field(f15, "project_uuid", offset_of!(SrsManifestHeader, project_uuid), 16);
    assert_field(f15, "snapshot_uuid", offset_of!(SrsManifestHeader, snapshot_uuid), 16);
    assert_field(f15, "parent_snapshot_uuid", offset_of!(SrsManifestHeader, parent_snapshot_uuid), 16);
    assert_field(f15, "origin_session_uuid", offset_of!(SrsManifestHeader, origin_session_uuid), 16);
    assert_field(f15, "created_at_sec", offset_of!(SrsManifestHeader, created_at_sec), 8);
    assert_field(f15, "tile_count", offset_of!(SrsManifestHeader, tile_count), 4);
    assert_field(f15, "crc32c_checksum", offset_of!(SrsManifestHeader, crc32c_checksum), 4);
    assert_field(f15, "total_records", offset_of!(SrsManifestHeader, total_records), 8);

    // Tile 20: ScholarNoteClaimRecord
    let t20 = &tiles["20"];
    assert_eq!(t20["record_size"].as_u64().unwrap() as usize, size_of::<ScholarNoteClaimRecord>());
    assert_eq!(t20["align"].as_u64().unwrap() as usize, align_of::<ScholarNoteClaimRecord>());
    let f20 = t20["fields"].as_array().unwrap();
    assert_field(f20, "claim_id", offset_of!(ScholarNoteClaimRecord, claim_id), 8);
    assert_field(f20, "timestamp_sec", offset_of!(ScholarNoteClaimRecord, timestamp_sec), 8);
    assert_field(f20, "session_uuid", offset_of!(ScholarNoteClaimRecord, session_uuid), 16);
    assert_field(f20, "author_hash", offset_of!(ScholarNoteClaimRecord, author_hash), 8);
    assert_field(f20, "confidence", offset_of!(ScholarNoteClaimRecord, confidence), 4);
    assert_field(f20, "kind", offset_of!(ScholarNoteClaimRecord, kind), 2);
    assert_field(f20, "flags", offset_of!(ScholarNoteClaimRecord, flags), 2);
    assert_field(f20, "quote_sha256", offset_of!(ScholarNoteClaimRecord, quote_sha256), 32);
    assert_field(f20, "producer_id", offset_of!(ScholarNoteClaimRecord, producer_id), 8);
}
