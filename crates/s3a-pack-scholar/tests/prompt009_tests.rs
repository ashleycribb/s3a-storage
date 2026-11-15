use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use s3a_core::{
    HyperTileHeader, TileType, FILE_HEADER_SIZE, HYPER_TILE_HEADER_SIZE, HYPER_TILE_SIZE,
};
use s3a_engine::lrs_store::{uuid_to_u64_pair, LrsStore};
use s3a_engine::sealed::{seal_buffer_with_manifest, verify_sealed_bytes};
use s3a_engine::MmapReader;
use s3a_pack_scholar::{
    AcademicPaperRecord, ResearchGraphEdgeRecord, ScholarAuthor, ScholarGraphEdge,
    ScholarPaperRecord, ScholarTileType,
};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TestTempFile {
    path: PathBuf,
}

impl TestTempFile {
    fn new(suffix: &str) -> Self {
        let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let mut path = std::env::temp_dir();
        path.push(format!(
            "s3a_prompt009_{}_{}_{}_{}.s3a",
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

#[test]
fn test_roundtrip_paper_non_ascii_and_20kb_abstract() {
    let large_abstract =
        "This is a detailed 20+ KB scientific abstract for testing large text continuation tiles. "
            .repeat(250); // ~22.5 KB
    assert!(large_abstract.len() > 20_000);

    let paper = ScholarPaperRecord {
        key: "arxiv:1706.03762".to_string(),
        doi: Some("10.48550/arXiv.1706.03762".to_string()),
        aliases: vec![
            "Attention Is All You Need".to_string(),
            "corpus:transformer-2017".to_string(),
        ],
        title: Some(
            "Attention Is All You Need: François & Müller Étude (NFC: e\u{0301} -> é)".to_string(),
        ),
        authors: vec![
            ScholarAuthor {
                name: "Ashish Vaswani".to_string(),
                orcid: Some("0000-0001-2345-6789".to_string()),
                affiliation: None,
            },
            ScholarAuthor {
                name: "Noam Shazeer".to_string(),
                orcid: None,
                affiliation: None,
            },
            ScholarAuthor {
                name: "Jürgen Schmidhuber".to_string(),
                orcid: Some("0000-0002-9876-5432".to_string()),
                affiliation: None,
            },
        ],
        abstract_text: Some(large_abstract.clone()),
        venue: Some("NeurIPS 2017".to_string()),
        year: Some(2017),
        language: Some("en".to_string()),
        license: Some("CC-BY-4.0".to_string()),
        source_url: Some("https://arxiv.org/abs/1706.03762".to_string()),
        published_date: Some("2017-06-12T00:00:00Z".to_string()),
    };

    // Append to in-memory archive
    let archive_bytes = paper
        .put_to_bytes(&[])
        .expect("put_to_bytes should succeed");

    // Seal archive with manifest
    let sealed_bytes = seal_buffer_with_manifest(&archive_bytes, 4, 2)
        .expect("seal_buffer_with_manifest should succeed");

    // Verify integrity of sealed archive
    let ver = verify_sealed_bytes(&sealed_bytes);
    assert!(ver.is_ok(), "Sealed archive integrity must verify cleanly");

    let mut paper = paper;
    paper.normalize();

    // Reopen and retrieve paper by key
    let retrieved = ScholarPaperRecord::get_from_bytes(&sealed_bytes, "arxiv:1706.03762")
        .expect("get_from_bytes should succeed")
        .expect("Paper must be retrievable from sealed bytes");

    assert_eq!(retrieved.key, paper.key);
    assert_eq!(retrieved.doi, paper.doi);
    assert_eq!(retrieved.title, paper.title);
    assert_eq!(retrieved.authors, paper.authors);
    assert_eq!(retrieved.abstract_text, paper.abstract_text);
    assert_eq!(retrieved.venue, paper.venue);
    assert_eq!(retrieved.year, paper.year);
    assert_eq!(retrieved.published_date, paper.published_date);

    let header_slot_a: s3a_core::FileHeader =
        bytemuck::pod_read_unaligned(&sealed_bytes[..s3a_core::FILE_HEADER_SLOT_SIZE]);
    let header_slot_b: s3a_core::FileHeader = bytemuck::pod_read_unaligned(
        &sealed_bytes[s3a_core::FILE_HEADER_SLOT_B_OFFSET
            ..s3a_core::FILE_HEADER_SLOT_B_OFFSET + s3a_core::FILE_HEADER_SLOT_SIZE],
    );
    let (active_header, _) =
        s3a_engine::resolve_active_header(&header_slot_a, &header_slot_b).unwrap();
    assert!(
        active_header.tile_count >= 2,
        "Expected at least 2 tiles for paper + seal, found {}",
        active_header.tile_count
    );

    // Also test very large text (140 KB > 64 KB hyper-tile capacity) spilling across continuation tiles
    let large_abstract = "Continuations in Hyper-Tiles. ".repeat(4700); // ~141 KB
    let multi_chunk_paper = ScholarPaperRecord {
        key: "arxiv:2301.large".to_string(),
        doi: Some("10.1000/large".to_string()),
        aliases: vec!["Large Paper".to_string()],
        title: Some("Paper with Large Abstract Spanning Continuation Tiles".to_string()),
        authors: vec![ScholarAuthor {
            name: "Author Multitile".to_string(),
            orcid: None,
            affiliation: None,
        }],
        abstract_text: Some(large_abstract.clone()),
        venue: Some("Large Venue".to_string()),
        year: Some(2026),
        language: Some("en".to_string()),
        license: Some("CC-BY-4.0".to_string()),
        source_url: None,
        published_date: Some("2026-11-15T12:00:00Z".to_string()),
    };
    let multi_buf = multi_chunk_paper.put_to_bytes(&[]).unwrap();
    let multi_sealed =
        s3a_engine::seal_buffer_v2(&multi_buf, &s3a_engine::SealOptionsV2::default()).unwrap();
    let slot_a: s3a_core::FileHeader =
        bytemuck::pod_read_unaligned(&multi_sealed[..s3a_core::FILE_HEADER_SLOT_SIZE]);
    let slot_b: s3a_core::FileHeader = bytemuck::pod_read_unaligned(
        &multi_sealed[s3a_core::FILE_HEADER_SLOT_B_OFFSET
            ..s3a_core::FILE_HEADER_SLOT_B_OFFSET + s3a_core::FILE_HEADER_SLOT_SIZE],
    );
    let (multi_hdr, _) = s3a_engine::resolve_active_header(&slot_a, &slot_b).unwrap();
    assert!(
        multi_hdr.tile_count >= 2,
        "140 KB abstract must span multiple continuation tiles, found {}",
        multi_hdr.tile_count
    );
    let retrieved_multi = ScholarPaperRecord::get_from_bytes(&multi_sealed, "arxiv:2301.large")
        .expect("get_from_bytes for large paper must succeed")
        .expect("large paper must be found");
    assert_eq!(retrieved_multi.abstract_text, Some(large_abstract));

    // Verify query by alias
    let retrieved_by_alias =
        ScholarPaperRecord::get_from_bytes(&sealed_bytes, "Attention Is All You Need")
            .expect("get_by_alias should succeed")
            .expect("Paper must be found by alias");
    assert_eq!(retrieved_by_alias.key, paper.key);

    // Verify query by DOI
    let retrieved_by_doi =
        ScholarPaperRecord::get_from_bytes(&sealed_bytes, "10.48550/arXiv.1706.03762")
            .expect("get_by_doi should succeed")
            .expect("Paper must be found by DOI");
    assert_eq!(retrieved_by_doi.key, paper.key);
}

#[test]
fn test_tampered_title_fails_integrity_and_positive_labels() {
    let paper = ScholarPaperRecord {
        key: "test:paper:001".to_string(),
        doi: Some("10.1000/182".to_string()),
        aliases: vec!["AliasPaper".to_string()],
        title: Some("Original Verifiable Title".to_string()),
        authors: vec![ScholarAuthor {
            name: "Alice Researcher".to_string(),
            orcid: None,
            affiliation: None,
        }],
        abstract_text: Some("Short abstract.".to_string()),
        venue: Some("Nature".to_string()),
        year: Some(2024),
        language: Some("en".to_string()),
        license: Some("CC0".to_string()),
        source_url: None,
        published_date: None,
    };

    let archive_bytes = paper.put_to_bytes(&[]).unwrap();
    let sealed_bytes = seal_buffer_with_manifest(&archive_bytes, 4, 2).unwrap();

    let mut tampered = sealed_bytes.clone();
    // Tamper the title text inside the payload
    let title_bytes = b"Original Verifiable Title";
    let pos = tampered
        .windows(title_bytes.len())
        .position(|w| w == title_bytes)
        .expect("Title bytes must exist in sealed buffer");
    tampered[pos] = b'X'; // Corrupt 'O' to 'X'

    // Integrity check must fail
    let ver = verify_sealed_bytes(&tampered);
    let integrity_ok = ver
        .as_ref()
        .map(|r| r.repaired_tiles.is_empty() && !r.warnings.iter().any(|w| w.contains("corrupt")))
        .unwrap_or(false);
    assert!(!integrity_ok, "Tampered archive integrity must fail");

    // Viewer verification check must report failure and no positive label
    let temp = TestTempFile::new("tampered");
    fs::write(temp.path(), &tampered).unwrap();

    let arch_ver = s3a_harness::viewer::check_archive_verification(temp.path());
    assert!(
        !arch_ver.integrity_ok,
        "Tampered archive must not be verified"
    );
    assert!(arch_ver.is_sealed, "Tampered archive claims sealed");

    let registry = s3a_harness::viewer::ViewerRegistry::new();
    let state = registry.load_archive(temp.path()).unwrap();
    assert_eq!(state.summary.status_badge, "Not verified");
    assert_eq!(
        state.summary.human_approval,
        "Sealed (integrity check failed)"
    );
}

#[test]
fn test_edges_survive_seal_and_append_only_retraction() {
    let edge1 = ScholarGraphEdge::new(
        "arxiv:1706.03762",
        "arxiv:1409.0473",
        "cites",
        "human_agent_1",
        "2026-11-15T10:00:00Z",
    );
    let edge2 = ScholarGraphEdge::new(
        "arxiv:1706.03762",
        "arxiv:1512.03385",
        "cites",
        "human_agent_1",
        "2026-11-15T10:05:00Z",
    );
    let edge3 = ScholarGraphEdge::new(
        "arxiv:1706.03762",
        "doi:10.1000/alias",
        "alias-of",
        "human_agent_1",
        "2026-11-15T10:10:00Z",
    );

    let mut buf = edge1.add_to_bytes(&[]).unwrap();
    buf = edge2.add_to_bytes(&buf).unwrap();
    buf = edge3.add_to_bytes(&buf).unwrap();

    // Query unsealed buffer
    let edges_cites =
        ScholarGraphEdge::list_from_bytes(&buf, "arxiv:1706.03762", Some("cites")).unwrap();
    assert_eq!(edges_cites.len(), 2);

    // Retract edge2 by its content-derived edge id
    let retraction =
        ScholarGraphEdge::retract(&edge2.edge_id(), "human_reviewer_2", "2026-11-15T11:00:00Z");
    buf = retraction.add_to_bytes(&buf).unwrap();

    // Seal buffer with manifest
    let sealed = seal_buffer_with_manifest(&buf, 4, 2).unwrap();
    assert!(verify_sealed_bytes(&sealed).is_ok());

    // Query sealed buffer: edge2 must be retracted
    let edges_after =
        ScholarGraphEdge::list_from_bytes(&sealed, "arxiv:1706.03762", Some("cites")).unwrap();
    assert_eq!(edges_after.len(), 1);
    assert_eq!(edges_after[0].target_key, "arxiv:1409.0473");
    assert_eq!(edges_after[0].edge_id(), edge1.edge_id());

    // Verify alias edge survives
    let aliases =
        ScholarGraphEdge::list_from_bytes(&sealed, "arxiv:1706.03762", Some("alias-of")).unwrap();
    assert_eq!(aliases.len(), 1);
    assert_eq!(aliases[0].target_key, "doi:10.1000/alias");
}

#[test]
fn test_embedded_registry_decodes_archive_without_pack_code() {
    let paper = ScholarPaperRecord {
        key: "arxiv:2106.09685".to_string(),
        doi: Some("10.48550/arXiv.2106.09685".to_string()),
        aliases: vec!["LoRA".to_string()],
        title: Some("LoRA: Low-Rank Adaptation of Large Language Models".to_string()),
        authors: vec![ScholarAuthor {
            name: "Edward J. Hu".to_string(),
            orcid: None,
            affiliation: None,
        }],
        abstract_text: Some("LoRA reduces the number of trainable parameters.".to_string()),
        venue: Some("ICLR 2022".to_string()),
        year: Some(2022),
        language: Some("en".to_string()),
        license: Some("MIT".to_string()),
        source_url: None,
        published_date: None,
    };
    let edge = ScholarGraphEdge::new(
        "arxiv:2106.09685",
        "arxiv:2005.14165",
        "cites",
        "ingest",
        "2026-11-15T00:00:00Z",
    );

    let mut buf = paper.put_to_bytes(&[]).unwrap();
    buf = edge.add_to_bytes(&buf).unwrap();

    // Seal with embedded schema catalog (Tile 77)
    let sealed =
        s3a_engine::seal_buffer_v2(&buf, &s3a_engine::SealOptionsV2::default().with_about())
            .unwrap();

    // Read the embedded registry using only core / engine reader (no Scholar pack types or built-in schemas needed)
    let info = s3a_engine::read_embedded_registry_from_bytes(&sealed)
        .expect("read_embedded_registry_from_bytes must succeed");

    assert_eq!(
        info.spec_version.as_deref(),
        Some(s3a_engine::sealed::spec_version())
    );
    assert!(info.engine_version.is_some());

    let paper_schema = info
        .types
        .iter()
        .find(|t| t.type_id == 12)
        .expect("Tile 12 schema must be embedded");
    assert_eq!(paper_schema.name, "academic_papers");
    assert_eq!(paper_schema.owning_pack, "Scholar Pack");

    // Layout v2 header fields
    let field_names: Vec<&str> = paper_schema
        .fields
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert!(field_names.contains(&"key_hash"));
    assert!(field_names.contains(&"sha256_hash"));
    assert!(field_names.contains(&"total_bytes"));
    assert!(field_names.contains(&"chunk_index"));
    assert!(field_names.contains(&"total_chunks"));
    assert!(field_names.contains(&"chunk_bytes"));
    assert!(field_names.contains(&"chunk_crc32"));
    assert!(field_names.contains(&"schema_version"));
    assert!(field_names.contains(&"reserved"));

    // Decode the paper record using ONLY the embedded schema fields (no Scholar pack structs)
    let chunk_bytes_field = paper_schema
        .fields
        .iter()
        .find(|f| f.name == "chunk_bytes")
        .unwrap();
    let record_header_len = paper_schema.record_size as usize; // 64

    let slot_a: s3a_core::FileHeader = bytemuck::pod_read_unaligned(
        &sealed[s3a_core::FILE_HEADER_SLOT_A_OFFSET
            ..s3a_core::FILE_HEADER_SLOT_A_OFFSET + s3a_core::FILE_HEADER_SLOT_SIZE],
    );
    let slot_b: s3a_core::FileHeader = bytemuck::pod_read_unaligned(
        &sealed[s3a_core::FILE_HEADER_SLOT_B_OFFSET
            ..s3a_core::FILE_HEADER_SLOT_B_OFFSET + s3a_core::FILE_HEADER_SLOT_SIZE],
    );
    let (active_hdr, _) = s3a_engine::resolve_active_header(&slot_a, &slot_b).unwrap();

    let mut decoded_json: Option<serde_json::Value> = None;
    for i in 0..active_hdr.tile_count as usize {
        let off = FILE_HEADER_SIZE + i * s3a_core::HYPER_TILE_SIZE;
        let h: s3a_core::HyperTileHeader =
            bytemuck::pod_read_unaligned(&sealed[off..off + HYPER_TILE_HEADER_SIZE]);
        if h.tile_type == 12 {
            let p0 = off + HYPER_TILE_HEADER_SIZE;
            let cb_off = p0 + chunk_bytes_field.byte_offset as usize;
            let chunk_bytes =
                u32::from_le_bytes(sealed[cb_off..cb_off + 4].try_into().unwrap()) as usize;
            let json_slice = &sealed[p0 + record_header_len..p0 + record_header_len + chunk_bytes];
            decoded_json = Some(serde_json::from_slice(json_slice).unwrap());
            break;
        }
    }
    let val = decoded_json.expect("Must decode paper using only embedded schema");
    assert_eq!(val["key"], "arxiv:2106.09685");
    assert_eq!(
        val["title"],
        "LoRA: Low-Rank Adaptation of Large Language Models"
    );

    // Mutation check: Must fail if schema embedding is turned off
    let sealed_without_about =
        s3a_engine::seal_buffer_v2(&buf, &s3a_engine::SealOptionsV2::default()).unwrap();
    let res = s3a_engine::read_embedded_registry_from_bytes(&sealed_without_about);
    assert!(
        res.is_err(),
        "Must fail when schema embedding is turned off"
    );
    assert!(res.unwrap_err().to_string().contains("not embedded"));
}

#[test]
fn test_recovery_picks_newest_valid_copy_by_sequence_number() {
    let temp = TestTempFile::new("seq_recovery");
    let path = temp.path();

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut header = s3a_core::FileHeader::new(2, now);
    header.addon_mask = s3a_core::ADDON_EDU;
    header.generation = 1;
    header.file_uuid = [111, 222];
    header.header_crc32 = s3a_engine::compute_header_crc32(&header);

    let mut buf = vec![0u8; FILE_HEADER_SIZE];
    buf[..s3a_core::FILE_HEADER_SLOT_SIZE].copy_from_slice(bytemuck::bytes_of(&header));

    // Tile 0: sequence 1, valid payload A
    let payload_a = b"Valid Payload from Sequence 1";
    let mut hdr1 = HyperTileHeader::new(100, TileType::TELEMETRY);
    hdr1.sequence_number = 1;
    hdr1.tile_index = 0;
    hdr1.payload_bytes = payload_a.len() as u32;
    hdr1.data_crc32 = crc32c::crc32c(payload_a);
    hdr1.header_crc32 = s3a_engine::compute_tile_header_crc32(&hdr1);

    let mut tile1 = vec![0u8; HYPER_TILE_SIZE];
    tile1[..HYPER_TILE_HEADER_SIZE].copy_from_slice(bytemuck::bytes_of(&hdr1));
    tile1[HYPER_TILE_HEADER_SIZE..HYPER_TILE_HEADER_SIZE + payload_a.len()]
        .copy_from_slice(payload_a);
    buf.extend_from_slice(&tile1);

    // Tile 1: sequence 2, corrupted payload B (data CRC mismatch)
    let payload_b = b"Corrupted Payload Sequence 2";
    let mut hdr2 = HyperTileHeader::new(100, TileType::TELEMETRY);
    hdr2.sequence_number = 2;
    hdr2.tile_index = 1;
    hdr2.payload_bytes = payload_b.len() as u32;
    hdr2.data_crc32 = 0xDEADBEEF; // Deliberate mismatch
    hdr2.header_crc32 = s3a_engine::compute_tile_header_crc32(&hdr2);

    let mut tile2 = vec![0u8; HYPER_TILE_SIZE];
    tile2[..HYPER_TILE_HEADER_SIZE].copy_from_slice(bytemuck::bytes_of(&hdr2));
    tile2[HYPER_TILE_HEADER_SIZE..HYPER_TILE_HEADER_SIZE + payload_b.len()]
        .copy_from_slice(payload_b);
    buf.extend_from_slice(&tile2);

    fs::write(path, &buf).unwrap();

    // MmapReader::open recovers the newest VALID copy
    let reader = MmapReader::open(path).expect("MmapReader::open should succeed");
    let tile_opt = reader.get_tile_by_id(100);
    assert!(tile_opt.is_some(), "Tile 100 must be recovered");
    let (h, p) = tile_opt.unwrap();
    assert_eq!(
        h.sequence_number, 1,
        "Recovery must pick newest VALID copy by sequence"
    );
    assert_eq!(&p[..payload_a.len()], payload_a);
}

#[test]
fn test_old_format_archive_compatibility() {
    let legacy_rec = AcademicPaperRecord {
        paper_id: 123456789,
        timestamp_sec: 0,
        hilbert_index: 0,
        topic_x: 0.0,
        topic_y: 0.0,
        topic_z: 0.0,
        citation_count: 42,
        year: 2015,
        venue_id: 0,
        open_access_flag: 0,
        warning_count: 0,
        doi_prefix_hash: 0,
        flags: 0,
    };
    let rec_bytes = bytemuck::bytes_of(&legacy_rec);

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut header = s3a_core::FileHeader::new(2, now);
    header.addon_mask = s3a_core::ADDON_EDU;
    header.header_crc32 = s3a_engine::compute_header_crc32(&header);

    let mut buf = vec![0u8; FILE_HEADER_SIZE];
    buf[..s3a_core::FILE_HEADER_SLOT_SIZE].copy_from_slice(bytemuck::bytes_of(&header));

    // Tile 0: legacy paper record
    let mut hdr = HyperTileHeader::new(1, ScholarTileType::ACADEMIC_PAPERS);
    hdr.record_layout_version = 1;
    hdr.record_count = 1;
    hdr.payload_bytes = rec_bytes.len() as u32;
    hdr.data_crc32 = crc32c::crc32c(rec_bytes);
    hdr.header_crc32 = s3a_engine::compute_tile_header_crc32(&hdr);

    let mut tile = vec![0u8; HYPER_TILE_SIZE];
    tile[..HYPER_TILE_HEADER_SIZE].copy_from_slice(bytemuck::bytes_of(&hdr));
    tile[HYPER_TILE_HEADER_SIZE..HYPER_TILE_HEADER_SIZE + rec_bytes.len()]
        .copy_from_slice(rec_bytes);
    buf.extend_from_slice(&tile);

    // Tile 1: legacy edge record
    let legacy_edge = ResearchGraphEdgeRecord::new(100, 200, 1, 0, 1, 0.95);
    let edge_rec_bytes = bytemuck::bytes_of(&legacy_edge);
    let mut edge_hdr = HyperTileHeader::new(2, ScholarTileType::RESEARCH_GRAPH);
    edge_hdr.record_layout_version = 1;
    edge_hdr.record_count = 1;
    edge_hdr.payload_bytes = edge_rec_bytes.len() as u32;
    edge_hdr.data_crc32 = crc32c::crc32c(edge_rec_bytes);
    edge_hdr.header_crc32 = s3a_engine::compute_tile_header_crc32(&edge_hdr);

    let mut edge_tile = vec![0u8; HYPER_TILE_SIZE];
    edge_tile[..HYPER_TILE_HEADER_SIZE].copy_from_slice(bytemuck::bytes_of(&edge_hdr));
    edge_tile[HYPER_TILE_HEADER_SIZE..HYPER_TILE_HEADER_SIZE + edge_rec_bytes.len()]
        .copy_from_slice(edge_rec_bytes);
    buf.extend_from_slice(&edge_tile);

    // Read using get_from_bytes
    let paper = ScholarPaperRecord::get_from_bytes(&buf, "123456789")
        .expect("get_from_bytes should succeed")
        .expect("paper should be found");

    assert_eq!(paper.key, "123456789");
    assert_eq!(paper.year, Some(2015));
    // Rule 5: Old v1 papers return title/venue absent (None), NOT invented values
    assert_eq!(paper.title, None);
    assert_eq!(paper.venue, None);
    assert_eq!(paper.abstract_text, None);
    assert_eq!(paper.license, None);
    assert!(paper.authors.is_empty());

    // Rule 5: Old edges carry only key hashes (rendered "0x{:016x}"), no actor and no
    // schema_version; their stored timestamp_sec (1 here) is reported as-is, never invented.
    let edges = ScholarGraphEdge::list_from_bytes(&buf, "0x0000000000000064", None).unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].target_key, "0x00000000000000c8");
    assert_eq!(edges[0].provenance_actor, None);
    assert_eq!(
        edges[0].provenance_timestamp.as_deref(),
        Some("1970-01-01T00:00:01Z")
    );
    assert_eq!(edges[0].schema_version, None);

    // Rule 5: Missing edge schema_version in JSON is absent (None), not 2
    let edge_json_no_ver = r#"{"source_key":"A","target_key":"B","edge_type":"cites"}"#;
    let parsed: ScholarGraphEdge = serde_json::from_str(edge_json_no_ver).unwrap();
    assert_eq!(parsed.schema_version, None);
}

#[test]
fn test_rebuild_indexes_matches_original_queries() {
    let temp = TestTempFile::new("rebuild_idx");
    let path = temp.path();

    // Create an LRS archive with statements
    let lrs = LrsStore::open_or_create(path).unwrap();
    let stmt_json = serde_json::json!({
        "id": "12345678-1234-5678-1234-567812345678",
        "actor": {
            "mbox": "mailto:alice@example.com",
            "name": "Alice"
        },
        "verb": {
            "id": "http://adlnet.gov/expapi/verbs/completed",
            "display": { "en-US": "completed" }
        },
        "object": {
            "id": "http://example.com/activities/course-101",
            "definition": { "name": { "en-US": "Course 101" } }
        },
        "timestamp": "2026-11-15T12:00:00Z"
    });

    let stmt_str = serde_json::to_string(&stmt_json).unwrap();
    lrs.store_statement(&stmt_str, &[]).unwrap();
    drop(lrs);

    // Query original archive before damage
    let uuid_pair = uuid_to_u64_pair("12345678-1234-5678-1234-567812345678").unwrap();
    let lrs_before = LrsStore::open_or_create(path).unwrap();
    let res_before = lrs_before.get_statement(uuid_pair, false).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&res_before.1).unwrap()["id"],
        "12345678-1234-5678-1234-567812345678"
    );
    drop(lrs_before);

    // Damage index tile (Tile 22) in the file
    let mut bytes = std::fs::read(path).unwrap();
    let mut found_index = false;
    let slot_a: s3a_core::FileHeader = bytemuck::pod_read_unaligned(
        &bytes[s3a_core::FILE_HEADER_SLOT_A_OFFSET
            ..s3a_core::FILE_HEADER_SLOT_A_OFFSET + s3a_core::FILE_HEADER_SLOT_SIZE],
    );
    let slot_b: s3a_core::FileHeader = bytemuck::pod_read_unaligned(
        &bytes[s3a_core::FILE_HEADER_SLOT_B_OFFSET
            ..s3a_core::FILE_HEADER_SLOT_B_OFFSET + s3a_core::FILE_HEADER_SLOT_SIZE],
    );
    let (active_hdr, _) = s3a_engine::resolve_active_header(&slot_a, &slot_b).unwrap();
    for i in 0..active_hdr.tile_count as usize {
        let off = FILE_HEADER_SIZE + i * s3a_core::HYPER_TILE_SIZE;
        let h: s3a_core::HyperTileHeader =
            bytemuck::pod_read_unaligned(&bytes[off..off + HYPER_TILE_HEADER_SIZE]);
        if h.tile_type == TileType::STATEMENT_INDEX {
            found_index = true;
            for b in &mut bytes[off + HYPER_TILE_HEADER_SIZE..off + HYPER_TILE_HEADER_SIZE + 512] {
                *b = 0; // Zero out index records
            }
        }
    }
    assert!(found_index, "Statement index tile must exist");
    std::fs::write(path, &bytes).unwrap();

    // Verify: queries FAIL or miss when index tile is damaged!
    // (Proves that the test fails if rebuild_indexes is a no-op)
    let lrs_damaged = LrsStore::open_or_create(path).unwrap();
    let res_damaged = lrs_damaged.get_statement(uuid_pair, false);
    assert!(
        res_damaged.is_err(),
        "Querying damaged archive before rebuild must fail (proves rebuild_indexes is not a no-op)"
    );
    drop(lrs_damaged);

    // Rebuild indexes
    s3a_engine::rebuild_indexes(path).expect("rebuild_indexes must succeed");

    // Query rebuilt archive: compare ALL queries before and after
    let lrs_after = LrsStore::open_or_create(path).unwrap();
    let res_after = lrs_after.get_statement(uuid_pair, false).unwrap();
    let stmt_after: serde_json::Value = serde_json::from_str(&res_after.1).unwrap();
    assert_eq!(
        stmt_after["id"].as_str().unwrap(),
        "12345678-1234-5678-1234-567812345678"
    );
    assert_eq!(res_after.1, res_before.1);
}

#[test]
fn test_real_writer_sequence_monotonicity_and_rewrite_recovery() {
    let temp = TestTempFile::new("real_writer_seq");
    let path = temp.path();

    // 1. Initial write
    {
        let mut writer = s3a_engine::TileWriter::create(path).unwrap();
        let payload1 = b"Telemetry Record 1";
        let mut hdr = HyperTileHeader::new(1, TileType::TELEMETRY);
        hdr.payload_bytes = payload1.len() as u32;
        hdr.data_crc32 = crc32c::crc32c(payload1);
        writer.write_raw_hyper_tile(&hdr, payload1).unwrap();
        // First written tile gets sequence 1
    }

    // 2. Monotonicity across reopen
    {
        let mut writer = s3a_engine::TileWriter::open_append(path).unwrap();
        let payload2 = b"Telemetry Record 2";
        let mut hdr = HyperTileHeader::new(2, TileType::TELEMETRY);
        hdr.payload_bytes = payload2.len() as u32;
        hdr.data_crc32 = crc32c::crc32c(payload2);
        writer.write_raw_hyper_tile(&hdr, payload2).unwrap();
        // Second written tile gets sequence 2
    }

    // 3. Reopen again and check next sequence
    {
        let mut writer = s3a_engine::TileWriter::open_append(path).unwrap();
        // Rewrite tile 1 with updated payload and higher sequence
        let payload1_v2 = b"Updated Telemetry Record 1 (Seq 3)";
        let mut hdr = HyperTileHeader::new(1, TileType::TELEMETRY);
        hdr.payload_bytes = payload1_v2.len() as u32;
        hdr.data_crc32 = crc32c::crc32c(payload1_v2);
        writer.write_raw_hyper_tile(&hdr, payload1_v2).unwrap();
    }

    // 4. Verify recovery recovers newest valid copy by sequence number
    {
        let reader = MmapReader::open(path).unwrap();
        let tile1 = reader.get_tile_by_id(1).expect("Tile 1 must be found");
        assert_eq!(tile1.0.sequence_number, 3);
        assert_eq!(&tile1.1[..34], b"Updated Telemetry Record 1 (Seq 3)");
    }

    // 5. Checksum verification when reading sequence on open:
    // Tamper the header CRC of sequence 3: recovery must fall back to sequence 1!
    {
        let mut bytes = std::fs::read(path).unwrap();
        let slot_a: s3a_core::FileHeader = bytemuck::pod_read_unaligned(
            &bytes[s3a_core::FILE_HEADER_SLOT_A_OFFSET
                ..s3a_core::FILE_HEADER_SLOT_A_OFFSET + s3a_core::FILE_HEADER_SLOT_SIZE],
        );
        let slot_b: s3a_core::FileHeader = bytemuck::pod_read_unaligned(
            &bytes[s3a_core::FILE_HEADER_SLOT_B_OFFSET
                ..s3a_core::FILE_HEADER_SLOT_B_OFFSET + s3a_core::FILE_HEADER_SLOT_SIZE],
        );
        let (active_hdr, _) = s3a_engine::resolve_active_header(&slot_a, &slot_b).unwrap();
        let count = active_hdr.tile_count as usize;
        let last_off = FILE_HEADER_SIZE + (count - 1) * s3a_core::HYPER_TILE_SIZE;
        bytes[last_off + 28] ^= 0xFF; // Corrupt header_crc32 of tile 3
        std::fs::write(path, &bytes).unwrap();

        let reader = MmapReader::open(path).unwrap();
        let tile1 = reader.get_tile_by_id(1).expect("Tile 1 must fall back");
        assert_eq!(tile1.0.sequence_number, 1);
        assert_eq!(&tile1.1[..18], b"Telemetry Record 1");

        // Open with TileWriter and assert next_sequence ignores corrupted tile sequence 3!
        let writer = s3a_engine::TileWriter::open_append(path).unwrap();
        assert_eq!(writer.next_sequence(), 3); // Max valid sequence is 2, so next is 3!
    }
}

#[test]
fn test_scholar_validation_and_edge_retraction_by_id() {
    // 1. Empty title validation
    let empty_title_paper = ScholarPaperRecord {
        key: "test:empty:title".to_string(),
        doi: None,
        aliases: vec![],
        title: Some("   ".to_string()),
        authors: vec![],
        abstract_text: None,
        venue: None,
        year: None,
        language: None,
        license: None,
        source_url: None,
        published_date: None,
    };
    let res = empty_title_paper.put_to_bytes(&[]);
    assert!(res.is_err(), "Empty paper title must be rejected");

    // 2. Non-UTC date validation on paper
    let non_utc_paper = ScholarPaperRecord {
        key: "test:non:utc".to_string(),
        doi: None,
        aliases: vec![],
        title: Some("Valid Title".to_string()),
        authors: vec![],
        abstract_text: None,
        venue: None,
        year: None,
        language: None,
        license: None,
        source_url: None,
        published_date: Some("2026-11-15T12:00:00+05:00".to_string()), // Non-UTC offset
    };
    assert!(
        non_utc_paper.put_to_bytes(&[]).is_err(),
        "Non-UTC date must be rejected"
    );

    // 3. Empty provenance actor validation on edge
    let empty_actor_edge = ScholarGraphEdge::new("A", "B", "cites", "", "2026-11-15T12:00:00Z");
    assert!(
        empty_actor_edge.add_to_bytes(&[]).is_err(),
        "Empty actor must be rejected"
    );

    // 4. Non-UTC timestamp on edge
    let non_utc_edge = ScholarGraphEdge::new(
        "A",
        "B",
        "cites",
        "actor",
        "2026-11-15 12:00:00", // Missing T and Z
    );
    assert!(
        non_utc_edge.add_to_bytes(&[]).is_err(),
        "Non-ISO timestamp must be rejected"
    );

    // 5. Retraction targeting specific edge_id and re-adding same edge later
    let edge1 = ScholarGraphEdge::new("node-X", "node-Y", "cites", "actor", "2026-11-15T10:00:00Z");

    let mut buf = edge1.add_to_bytes(&[]).unwrap();
    let list1 = ScholarGraphEdge::list_from_bytes(&buf, "node-X", Some("cites")).unwrap();
    assert_eq!(list1.len(), 1);

    // Retract edge1 by targeting its specific edge id
    let ret = ScholarGraphEdge::retract(&edge1.edge_id(), "actor", "2026-11-15T11:00:00Z");
    buf = ret.add_to_bytes(&buf).unwrap();

    let list_after_ret = ScholarGraphEdge::list_from_bytes(&buf, "node-X", Some("cites")).unwrap();
    assert_eq!(list_after_ret.len(), 0, "Edge must be retracted");

    // Re-add the same relation later; its later timestamp gives it a distinct edge id
    let edge2 = ScholarGraphEdge::new("node-X", "node-Y", "cites", "actor", "2026-11-15T12:00:00Z");
    assert_ne!(edge1.edge_id(), edge2.edge_id());
    buf = edge2.add_to_bytes(&buf).unwrap();

    let list_after_readd =
        ScholarGraphEdge::list_from_bytes(&buf, "node-X", Some("cites")).unwrap();
    assert_eq!(
        list_after_readd.len(),
        1,
        "Re-added edge with distinct edge_id must be visible"
    );
    assert_eq!(list_after_readd[0].edge_id(), edge2.edge_id());

    // 6. Malformed length field handling (never panic)
    let bsz = std::mem::size_of::<s3a_pack_scholar::ScholarPaperBlobHeader>();
    let mut bad_payload = vec![0u8; bsz + 32];
    let bad_hdr = s3a_pack_scholar::ScholarPaperBlobHeader {
        key_hash: 1,
        sha256_hash: [0u8; 32],
        total_bytes: 100,
        chunk_index: 0,
        total_chunks: 1,
        chunk_bytes: 0xFFFF_0000, // Deliberately exceeds payload length
        chunk_crc32: 0,
        schema_version: 2,
        _reserved: 0,
    };
    bad_payload[..bsz].copy_from_slice(bytemuck::bytes_of(&bad_hdr));
    let mut tile_hdr = HyperTileHeader::new(1, ScholarTileType::ACADEMIC_PAPERS);
    tile_hdr.record_layout_version = 2;
    tile_hdr.payload_bytes = bad_payload.len() as u32;
    tile_hdr.data_crc32 = crc32c::crc32c(&bad_payload);
    tile_hdr.header_crc32 = s3a_engine::compute_tile_header_crc32(&tile_hdr);

    // Call get_from_tiles with malformed chunk_bytes - must not panic!
    let res = ScholarPaperRecord::get_from_tiles(
        std::iter::once((&tile_hdr, bad_payload.as_slice())),
        "test",
    );
    assert!(
        res.is_err(),
        "Malformed length must return an error (not None, not a panic)"
    );
}

#[test]
fn test_wasm_and_native_produce_identical_bytes() {
    let paper = ScholarPaperRecord {
        key: "10.1234/test.paper".to_string(),
        doi: Some("10.1234/test.paper".to_string()),
        aliases: vec!["test-alias".to_string()],
        title: Some("Attention Is All You Need".to_string()),
        authors: vec![ScholarAuthor {
            name: "Ashish Vaswani".to_string(),
            orcid: Some("0000-0001-2345-6789".to_string()),
            affiliation: None,
        }],
        abstract_text: Some("The dominant sequence transduction models...".to_string()),
        venue: Some("NeurIPS".to_string()),
        year: Some(2017),
        language: Some("en".to_string()),
        license: Some("CC-BY-4.0".to_string()),
        source_url: Some("https://arxiv.org/abs/1706.03762".to_string()),
        published_date: None,
    };

    let base = paper.put_to_bytes(&[]).unwrap();

    let paper2 = ScholarPaperRecord {
        key: "10.5678/another.paper".to_string(),
        doi: Some("10.5678/another.paper".to_string()),
        aliases: vec![],
        title: Some("Second Paper".to_string()),
        authors: vec![],
        abstract_text: None,
        venue: None,
        year: Some(2024),
        language: None,
        license: None,
        source_url: None,
        published_date: None,
    };

    let native_bytes = paper2.put_to_bytes(&base).unwrap();
    let paper2_json = serde_json::to_string(&paper2).unwrap();
    let wasm_bytes = s3a_wasm_snapshot::bindgen::put_paper(&base, &paper2_json).unwrap();

    // Verify native and WASM produce byte-identical output
    assert_eq!(
        native_bytes, wasm_bytes,
        "WASM and native put_paper must produce identical bytes"
    );
}
