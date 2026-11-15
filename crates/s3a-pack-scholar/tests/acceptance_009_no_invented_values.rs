//! Acceptance 009 follow-up (5): readers never invent values. Unknown means absent.
//!
//! Written before the fix. Do not edit. Checks go through serde JSON so they compile against
//! both today's types and the fixed ones (e.g. `title: Option<String>`): an absent field must be
//! missing or null in the JSON, never a placeholder string, 0, "" or the epoch.

use s3a_core::{
    FileHeader, HyperTileHeader, FILE_HEADER_SIZE, HYPER_TILE_HEADER_SIZE, HYPER_TILE_SIZE,
};
use s3a_pack_scholar::{
    AcademicPaperRecord, ResearchGraphEdgeRecord, ScholarGraphEdge, ScholarPaperRecord,
    ScholarTileType,
};

/// A pre-009 archive holding one legacy (layout v1) tile of fixed 64-byte records.
fn legacy_archive<T: bytemuck::Pod>(tile_type: u32, recs: &[T]) -> Vec<u8> {
    let bytes = bytemuck::cast_slice::<T, u8>(recs);
    let mut fh = FileHeader::new(1, 1_600_000_000);
    fh.addon_mask = s3a_core::ADDON_EDU;
    fh.header_crc32 = s3a_engine::compute_header_crc32(&fh);
    let mut buf = vec![0u8; FILE_HEADER_SIZE];
    buf[..s3a_core::FILE_HEADER_SLOT_SIZE].copy_from_slice(bytemuck::bytes_of(&fh));
    let mut h = HyperTileHeader::new(1, tile_type);
    h.record_layout_version = 1;
    h.record_count = recs.len() as u32;
    h.payload_bytes = bytes.len() as u32;
    h.data_crc32 = crc32c::crc32c(bytes);
    h.header_crc32 = s3a_engine::compute_tile_header_crc32(&h);
    let mut tile = vec![0u8; HYPER_TILE_SIZE];
    tile[..HYPER_TILE_HEADER_SIZE].copy_from_slice(bytemuck::bytes_of(&h));
    tile[HYPER_TILE_HEADER_SIZE..HYPER_TILE_HEADER_SIZE + bytes.len()].copy_from_slice(bytes);
    buf.extend_from_slice(&tile);
    buf
}

fn absent(v: &serde_json::Value, k: &str) -> bool {
    v.get(k).map_or(true, |x| x.is_null())
}

#[test]
fn v1_paper_has_title_and_venue_absent() {
    let rec = AcademicPaperRecord {
        paper_id: 123456789,
        timestamp_sec: 0,
        hilbert_index: 0,
        topic_x: 0.0,
        topic_y: 0.0,
        topic_z: 0.0,
        citation_count: 42,
        year: 2015,
        venue_id: 7,
        open_access_flag: 0,
        warning_count: 0,
        doi_prefix_hash: 0,
        flags: 0,
    };
    let buf = legacy_archive(ScholarTileType::ACADEMIC_PAPERS, &[rec]);
    let p = ScholarPaperRecord::get_from_bytes(&buf, "123456789")
        .unwrap()
        .expect("legacy paper found");
    let v = serde_json::to_value(&p).unwrap();
    assert_eq!(v["key"], "123456789");
    assert_eq!(
        v["year"], 2015,
        "a value the v1 record really holds is kept"
    );
    assert!(
        absent(&v, "title"),
        "v1 records hold no title; must be absent, got {}",
        v["title"]
    );
    assert!(
        absent(&v, "venue"),
        "v1 records hold only a venue_id hash, not a venue name; must be absent, got {}",
        v["venue"]
    );
}

fn legacy_edge(ts: u64) -> Vec<u8> {
    let mut e = ResearchGraphEdgeRecord::default();
    e.subject_hash = 0x1111;
    e.object_hash = 0x2222;
    e.predicate_id = 1; // cites
    e.timestamp_sec = ts;
    e.weight = 1.0;
    legacy_archive(ScholarTileType::RESEARCH_GRAPH, &[e])
}

#[test]
fn old_edge_has_provenance_actor_absent_and_no_epoch_timestamp() {
    let edges =
        ScholarGraphEdge::list_from_bytes(&legacy_edge(0), "0x0000000000001111", None).unwrap();
    assert_eq!(edges.len(), 1, "legacy edge is listed");
    let v = serde_json::to_value(&edges[0]).unwrap();
    assert!(
        absent(&v, "provenance_actor"),
        "legacy edges record no actor; must be absent, got {}",
        v["provenance_actor"]
    );
    assert!(
        absent(&v, "provenance_timestamp"),
        "timestamp_sec == 0 means unknown; must be absent, got {}",
        v["provenance_timestamp"]
    );
}

#[test]
fn old_edge_with_real_timestamp_keeps_it_as_utc_iso() {
    // 1_700_000_000 s = 2023-11-14T22:13:20Z. A real stored value is not invented, so it is kept.
    let edges =
        ScholarGraphEdge::list_from_bytes(&legacy_edge(1_700_000_000), "0x0000000000001111", None)
            .unwrap();
    let v = serde_json::to_value(&edges[0]).unwrap();
    assert_eq!(v["provenance_timestamp"], "2023-11-14T22:13:20Z");
    assert!(absent(&v, "provenance_actor"));
}

#[test]
fn missing_edge_schema_version_stays_absent() {
    let json = r#"{"source_key":"a","target_key":"b","edge_type":"cites","provenance_actor":"x","provenance_timestamp":"2026-01-01T00:00:00Z"}"#;
    let e: ScholarGraphEdge =
        serde_json::from_str(json).expect("an edge without schema_version still parses");
    let v = serde_json::to_value(&e).unwrap();
    assert!(
        absent(&v, "schema_version"),
        "schema_version was not given; must stay absent, got {}",
        v["schema_version"]
    );
}
