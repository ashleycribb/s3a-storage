//! Struct-vs-schema-vs-registry drift test (schema fix, Nov 18, 2026).
//!
//! The engine's `standard_schema_for_type`, the pack's registry descriptors,
//! and the pack structs must agree on record size and layout, or tour decodes
//! records through the wrong fields. Three drifts were found by the foundation
//! sweep on e79ecb39:
//!
//! - Type 20 (`note_claims`) said `record_size: 64` in the engine schema and
//!   in both registry descriptors, while `ScholarNoteClaimRecord` is 160
//!   bytes (Phase C extension). The two wrong values agreed with each other,
//!   so the `schema.rs` cross-check never fired.
//! - Type 32 (`citation_fingerprint`) described a stale fingerprint layout
//!   (`fingerprint_id`, `timestamp_sec`, `citation_hash`) that matches no
//!   struct in the codebase; the real struct is `CitationFingerprintRecord`.
//! - Types 12/13 exist in TWO layouts: v1 fixed POD records
//!   (`AcademicPaperRecord` / `ResearchGraphEdgeRecord`, see
//!   docs/spec/legacy-layouts.md) and v2 chunked blobs
//!   (`ScholarPaperBlobHeader` + JSON, see docs/spec/registry.md). The schema
//!   describes the CURRENT (v2) layout only — that is correct, and is pinned
//!   by the embedded-registry acceptance tests — so tour must select the
//!   decoder from the tile's declared `record_layout_version`
//!   (`is_scholar_pod_floor`), never from the schema shape. An earlier fix
//!   attempt made 12/13 opaque blobs; it broke the two embedded-registry
//!   acceptance tests, which require the v2 blob field list. This test pins
//!   the v2 blob field lists for 12/13, the full 20-field list for type 20,
//!   and the 9-field POD list for type 32.

use std::mem::size_of;

use s3a_core::registry::TileTypeRegistry;
use s3a_engine::standard_schema_for_type;
use s3a_pack_scholar::{
    register_pack_types, AcademicPaperRecord, AgentRegistryRecord, AiTraceRecord,
    CitationFingerprintRecord, HumanLrsRecord, ResearchGraphEdgeRecord,
    ScholarNoteClaimRecord, ScholarTileType, SrsWorkspaceRecord,
};

/// (type_id, struct size) for every Scholar type with a named record struct.
fn scholar_types() -> Vec<(u32, usize)> {
    vec![
        (ScholarTileType::HUMAN_LRS, size_of::<HumanLrsRecord>()),
        (ScholarTileType::AI_TRACE_LOG, size_of::<AiTraceRecord>()),
        (ScholarTileType::ACADEMIC_PAPERS, size_of::<AcademicPaperRecord>()),
        (ScholarTileType::RESEARCH_GRAPH, size_of::<ResearchGraphEdgeRecord>()),
        (ScholarTileType::SRS_WORKSPACE, size_of::<SrsWorkspaceRecord>()),
        (ScholarTileType::AGENT_REGISTRY, size_of::<AgentRegistryRecord>()),
        (ScholarTileType::NOTE_CLAIMS, size_of::<ScholarNoteClaimRecord>()),
        (
            ScholarTileType::CITATION_FINGERPRINT,
            size_of::<CitationFingerprintRecord>(),
        ),
    ]
}

/// Assert a schema's field list is exactly `(name, field_type, byte_offset,
/// byte_len)` in order. Field type codes: U16=2, U32=3, U64=4, F32=9,
/// STRING=11, BYTES=13.
fn assert_schema_fields(type_id: u32, expected: &[(&str, u16, u32, u32)]) {
    let schema = standard_schema_for_type(type_id);
    assert_eq!(
        schema.fields.len(),
        expected.len(),
        "type {}: schema field count {} != expected {}",
        type_id,
        schema.fields.len(),
        expected.len()
    );
    for (i, (name, ftype, off, len)) in expected.iter().enumerate() {
        let f = &schema.fields[i];
        assert_eq!(f.name, *name, "type {} field {}: name", type_id, i + 1);
        assert_eq!(
            f.field_type, *ftype,
            "type {} field {} ({}): field_type",
            type_id, i + 1, name
        );
        assert_eq!(
            f.byte_offset, *off,
            "type {} field {} ({}): byte_offset",
            type_id, i + 1, name
        );
        assert_eq!(
            f.byte_len, *len,
            "type {} field {} ({}): byte_len",
            type_id, i + 1, name
        );
    }
}

#[test]
fn scholar_schema_record_size_matches_struct() {
    for (type_id, size) in scholar_types() {
        let schema = standard_schema_for_type(type_id);
        assert_eq!(
            schema.record_size as usize, size,
            "type {}: engine schema record_size {} != size_of struct {}",
            type_id, schema.record_size, size
        );
    }
}

#[test]
fn scholar_registry_record_size_matches_struct() {
    let mut reg = TileTypeRegistry::new();
    register_pack_types(&mut reg).expect("register pack types");
    for (type_id, size) in scholar_types() {
        let desc = reg
            .get(type_id)
            .unwrap_or_else(|| panic!("type {} not registered", type_id));
        assert_eq!(
            desc.record_size, size,
            "type {}: registry record_size {} != size_of struct {}",
            type_id, desc.record_size, size
        );
    }
}

#[test]
fn type_12_schema_is_the_v2_blob_layout() {
    // docs/spec/registry.md tile 12 v2: ScholarPaperBlobHeader + chunk_data.
    // The embedded-registry acceptance tests decode a sealed paper using
    // ONLY these fields; the v1 POD layout lives in legacy-layouts.md and is
    // selected by the tile's record_layout_version, not by this schema.
    let schema = standard_schema_for_type(ScholarTileType::ACADEMIC_PAPERS);
    assert_eq!(schema.type_version, 2);
    assert_eq!(schema.record_size, 64);
    assert_schema_fields(
        ScholarTileType::ACADEMIC_PAPERS,
        &[
            ("key_hash", 4, 0, 8),
            ("sha256_hash", 13, 8, 32),
            ("total_bytes", 3, 40, 4),
            ("chunk_index", 2, 44, 2),
            ("total_chunks", 2, 46, 2),
            ("chunk_bytes", 3, 48, 4),
            ("chunk_crc32", 3, 52, 4),
            ("schema_version", 3, 56, 4),
            ("reserved", 3, 60, 4),
            ("chunk_data", 11, 64, 0),
        ],
    );
}

#[test]
fn type_13_schema_is_the_v2_blob_layout() {
    // docs/spec/registry.md tile 13 v2: blob header + edge_json.
    let schema = standard_schema_for_type(ScholarTileType::RESEARCH_GRAPH);
    assert_eq!(schema.type_version, 2);
    assert_eq!(schema.record_size, 64);
    assert_schema_fields(
        ScholarTileType::RESEARCH_GRAPH,
        &[
            ("source_hash", 4, 0, 8),
            ("target_hash", 4, 8, 8),
            ("total_bytes", 3, 16, 4),
            ("schema_version", 3, 20, 4),
            ("payload_crc32", 3, 24, 4),
            ("is_retraction", 3, 28, 4),
            ("reserved", 13, 32, 32),
            ("edge_json", 11, 64, 0),
        ],
    );
}

#[test]
fn type_20_schema_lists_all_20_fields() {
    // All 11 base fields plus the 9 Phase C fields (bytes 64..160), matching
    // ScholarNoteClaimRecord field for field.
    let schema = standard_schema_for_type(ScholarTileType::NOTE_CLAIMS);
    assert_eq!(schema.type_version, 1);
    assert_eq!(schema.record_size, 160);
    assert_schema_fields(
        ScholarTileType::NOTE_CLAIMS,
        &[
            ("claim_id", 4, 0, 8),
            ("timestamp_sec", 4, 8, 8),
            ("session_uuid", 13, 16, 16),
            ("author_hash", 4, 32, 8),
            ("confidence", 9, 40, 4),
            ("kind", 2, 44, 2),
            ("flags", 2, 46, 2),
            ("keyword_bloom", 3, 48, 4),
            ("text_hash", 3, 52, 4),
            ("embedding_tile_id", 3, 56, 4),
            ("embedding_offset", 3, 60, 4),
            ("citation_fingerprint_id", 4, 64, 8),
            ("passage_page", 3, 72, 4),
            ("passage_char_offset", 3, 76, 4),
            ("passage_char_length", 3, 80, 4),
            ("reserved_passage", 3, 84, 4),
            ("quote_sha256", 13, 88, 32),
            ("producer_id", 4, 120, 8),
            ("verification_statement_uuid", 13, 128, 16),
            ("reserved", 13, 144, 16),
        ],
    );
}

#[test]
fn type_32_schema_matches_fingerprint_struct() {
    // Matches CitationFingerprintRecord field for field; the previous list
    // (fingerprint_id, timestamp_sec, citation_hash) matched no struct.
    let schema = standard_schema_for_type(ScholarTileType::CITATION_FINGERPRINT);
    assert_eq!(schema.type_version, 1);
    assert_eq!(schema.record_size, 96);
    assert_schema_fields(
        ScholarTileType::CITATION_FINGERPRINT,
        &[
            ("paper_id", 4, 0, 8),
            ("doi_hash", 4, 8, 8),
            ("content_sha256", 13, 16, 32),
            ("content_kind", 3, 48, 4),
            ("crossref_version", 3, 52, 4),
            ("fetched_at", 4, 56, 8),
            ("source_url_hash", 4, 64, 8),
            ("flags", 4, 72, 8),
            ("reserved", 13, 80, 16),
        ],
    );
}

#[test]
fn note_claims_is_160_everywhere() {
    // The regression that motivated this file: the Phase C extension grew
    // `ScholarNoteClaimRecord` to 160 bytes while the schema and both
    // registry descriptors still said 64.
    assert_eq!(size_of::<ScholarNoteClaimRecord>(), 160);
    assert_eq!(
        standard_schema_for_type(ScholarTileType::NOTE_CLAIMS).record_size,
        160
    );
    let mut reg = TileTypeRegistry::new();
    register_pack_types(&mut reg).expect("register pack types");
    assert_eq!(
        reg.get(ScholarTileType::NOTE_CLAIMS)
            .unwrap()
            .record_size,
        160
    );
}
