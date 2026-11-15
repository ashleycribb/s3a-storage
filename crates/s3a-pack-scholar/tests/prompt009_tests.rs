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
    AcademicPaperRecord, ScholarAuthor, ScholarGraphEdge, ScholarPaperRecord, ScholarTileType,
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
        let mut q = self.path.as_os_str().to_os_string();
        q.push(".quarantine");
        let _ = fs::remove_file(PathBuf::from(q));
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
        title: "Attention Is All You Need: François & Müller Étude (NFC: e\u{0301} -> é)"
            .to_string(),
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

    // Seal archive with manifest and embedded schema
    let sealed_bytes = seal_buffer_with_manifest(&archive_bytes, 4, 2)
        .expect("seal_buffer_with_manifest should succeed");

    // Verify integrity of sealed archive
    let ver = verify_sealed_bytes(&sealed_bytes);
    assert!(ver.is_ok(), "Sealed archive integrity must verify cleanly");

    let mut paper = paper;
    paper.normalize();

    // Reopen and retrieve paper by canonical key
    let retrieved = ScholarPaperRecord::get_from_bytes(&sealed_bytes, "arxiv:1706.03762")
        .expect("get_from_bytes should succeed")
        .expect("paper must be found");

    assert_eq!(retrieved.key, paper.key);
    assert_eq!(retrieved.doi, paper.doi);
    assert_eq!(retrieved.aliases, paper.aliases);
    assert_eq!(retrieved.title, paper.title);
    assert_eq!(retrieved.authors, paper.authors);
    assert_eq!(retrieved.abstract_text, paper.abstract_text);
    assert_eq!(
        retrieved.abstract_text.as_ref().unwrap().len(),
        large_abstract.len()
    );
    assert_eq!(retrieved.venue, paper.venue);
    assert_eq!(retrieved.year, paper.year);
    assert_eq!(retrieved.language, paper.language);
    assert_eq!(retrieved.license, paper.license);
    assert_eq!(retrieved.source_url, paper.source_url);

    // Retrieve by DOI
    let by_doi = ScholarPaperRecord::get_from_bytes(&sealed_bytes, "10.48550/arXiv.1706.03762")
        .unwrap()
        .expect("paper must be found by DOI");
    assert_eq!(by_doi.key, paper.key);

    // Retrieve by alias
    let by_alias = ScholarPaperRecord::get_from_bytes(&sealed_bytes, "Attention Is All You Need")
        .unwrap()
        .expect("paper must be found by alias");
    assert_eq!(by_alias.key, paper.key);
}

#[test]
fn test_tampered_title_fails_integrity_and_positive_labels() {
    let paper = ScholarPaperRecord {
        key: "test:paper:001".to_string(),
        doi: Some("10.1000/182".to_string()),
        aliases: vec!["AliasPaper".to_string()],
        title: "Original Verifiable Title".to_string(),
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
    let edge2 = ScholarGraphEdge {
        source_key: "arxiv:1409.0473".to_string(),
        target_key: "arxiv:1706.03762".to_string(),
        edge_type: "cited-by".to_string(),
        provenance_actor: "system_indexer".to_string(),
        provenance_timestamp: "2026-11-15T10:05:00Z".to_string(),
        schema_version: 2,
        is_retraction: false,
        retracted_edge_id: None,
    };
    let edge3 = ScholarGraphEdge {
        source_key: "corpus:item_99".to_string(),
        target_key: "arxiv:1706.03762".to_string(),
        edge_type: "alias-of".to_string(),
        provenance_actor: "curator_bob".to_string(),
        provenance_timestamp: "2026-11-15T10:10:00Z".to_string(),
        schema_version: 2,
        is_retraction: false,
        retracted_edge_id: None,
    };

    let mut buf = edge1.add_to_bytes(&[]).unwrap();
    buf = edge2.add_to_bytes(&buf).unwrap();
    buf = edge3.add_to_bytes(&buf).unwrap();

    // Query before retraction
    let edges_before = ScholarGraphEdge::list_from_bytes(&buf, "arxiv:1706.03762", None).unwrap();
    assert_eq!(edges_before.len(), 3);

    // Append retraction for edge 1 (append-only: new edge record, not a delete)
    let retraction = ScholarGraphEdge::new_retraction(
        "arxiv:1706.03762",
        "arxiv:1409.0473",
        "cites",
        "curator_charlie",
        "2026-11-15T11:00:00Z",
    );
    buf = retraction.add_to_bytes(&buf).unwrap();

    // Seal the archive
    let sealed = seal_buffer_with_manifest(&buf, 4, 2).unwrap();
    let ver = verify_sealed_bytes(&sealed);
    assert!(ver.is_ok());

    // Query after retraction
    let edges_after = ScholarGraphEdge::list_from_bytes(&sealed, "arxiv:1706.03762", None).unwrap();
    // Edge 1 was retracted, so only edge 2 and edge 3 should be active
    assert_eq!(edges_after.len(), 2);
    assert!(!edges_after.iter().any(|e| e.edge_type == "cites"));
    assert!(edges_after.iter().any(|e| e.edge_type == "cited-by"));
    assert!(edges_after.iter().any(|e| e.edge_type == "alias-of"));
}

#[test]
fn test_embedded_registry_decodes_archive_without_pack_code() {
    let paper = ScholarPaperRecord {
        key: "arxiv:2106.09685".to_string(),
        doi: Some("10.48550/arXiv.2106.09685".to_string()),
        aliases: vec!["LoRA".to_string()],
        title: "LoRA: Low-Rank Adaptation of Large Language Models".to_string(),
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

    let sealed = seal_buffer_with_manifest(&buf, 4, 2).unwrap();

    // Read the embedded registry using only core / engine reader (no Scholar pack types needed)
    let info = s3a_engine::read_embedded_registry_from_bytes(&sealed)
        .expect("read_embedded_registry_from_bytes must succeed");

    assert_eq!(info.spec_version, s3a_engine::sealed::spec_version());

    let paper_schema = info
        .types
        .iter()
        .find(|t| t.type_id == 12)
        .expect("Tile 12 schema must be embedded");
    assert_eq!(paper_schema.name, "academic_papers");
    assert_eq!(paper_schema.owning_pack, "Scholar Pack");
    let field_names: Vec<&str> = paper_schema
        .fields
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert!(field_names.contains(&"paper_id"));
    assert!(field_names.contains(&"citation_count"));
    assert!(field_names.contains(&"year"));
    assert!(field_names.contains(&"doi_prefix_hash"));
    assert!(field_names.contains(&"flags"));

    let edge_schema = info
        .types
        .iter()
        .find(|t| t.type_id == 13)
        .expect("Tile 13 schema must be embedded");
    assert_eq!(edge_schema.name, "research_graph");
    assert_eq!(edge_schema.owning_pack, "Scholar Pack");
    let edge_field_names: Vec<&str> = edge_schema.fields.iter().map(|f| f.name.as_str()).collect();
    assert!(edge_field_names.contains(&"subject_hash"));
    assert!(edge_field_names.contains(&"object_hash"));
    assert!(edge_field_names.contains(&"predicate_id"));
    assert!(edge_field_names.contains(&"flags"));
}

#[test]
fn test_recovery_picks_newest_valid_copy_by_sequence_number() {
    let temp = TestTempFile::new("seq_recovery");
    let path = temp.path();

    // Create an unsealed archive with two tiles that share the same tile_id (simulating an update),
    // where sequence 2 has higher sequence number than sequence 1.
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
    // Tile with tile_id 100 should resolve to sequence 1 because sequence 2 failed CRC
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
    // Construct an archive containing a legacy 64-byte AcademicPaperRecord (version 1)
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
    let mut header = s3a_core::FileHeader::new(1, now);
    header.addon_mask = s3a_core::ADDON_EDU;
    header.header_crc32 = s3a_engine::compute_header_crc32(&header);

    let mut buf = vec![0u8; FILE_HEADER_SIZE];
    buf[..s3a_core::FILE_HEADER_SLOT_SIZE].copy_from_slice(bytemuck::bytes_of(&header));

    let mut hdr = HyperTileHeader::new(1, ScholarTileType::ACADEMIC_PAPERS);
    hdr.record_layout_version = 1; // Legacy layout
    hdr.record_count = 1;
    hdr.payload_bytes = rec_bytes.len() as u32;
    hdr.data_crc32 = crc32c::crc32c(rec_bytes);
    hdr.header_crc32 = s3a_engine::compute_tile_header_crc32(&hdr);

    let mut tile = vec![0u8; HYPER_TILE_SIZE];
    tile[..HYPER_TILE_HEADER_SIZE].copy_from_slice(bytemuck::bytes_of(&hdr));
    tile[HYPER_TILE_HEADER_SIZE..HYPER_TILE_HEADER_SIZE + rec_bytes.len()]
        .copy_from_slice(rec_bytes);
    buf.extend_from_slice(&tile);

    // Read using get_from_bytes
    let paper = ScholarPaperRecord::get_from_bytes(&buf, "123456789")
        .expect("get_from_bytes should succeed")
        .expect("paper should be found");

    assert_eq!(paper.key, "123456789");
    assert_eq!(paper.year, Some(2015));
    // New fields should be absent (None), NOT "" or 0
    assert_eq!(paper.title, "Paper 123456789");
    assert_eq!(paper.abstract_text, None);
    assert_eq!(paper.venue, None);
    assert_eq!(paper.license, None);
    assert!(paper.authors.is_empty());
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

    // Query original archive
    let lrs_before = LrsStore::open_or_create(path).unwrap();
    let uuid_pair = uuid_to_u64_pair("12345678-1234-5678-1234-567812345678").unwrap();
    let res_before = lrs_before.get_statement(uuid_pair, false).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&res_before.1).unwrap()["id"],
        "12345678-1234-5678-1234-567812345678"
    );
    drop(lrs_before);

    // Rebuild indexes
    s3a_engine::rebuild_indexes(path).expect("rebuild_indexes must succeed");

    // Query rebuilt archive
    let lrs_after = LrsStore::open_or_create(path).unwrap();
    let res_after = lrs_after.get_statement(uuid_pair, false).unwrap();
    let stmt_after: serde_json::Value = serde_json::from_str(&res_after.1).unwrap();
    assert_eq!(
        stmt_after["id"].as_str().unwrap(),
        "12345678-1234-5678-1234-567812345678"
    );
}

#[test]
fn test_wasm_and_native_produce_identical_bytes() {
    let paper = ScholarPaperRecord {
        key: "10.1234/test.paper".to_string(),
        doi: Some("10.1234/test.paper".to_string()),
        aliases: vec!["test-alias".to_string()],
        title: "Attention Is All You Need".to_string(),
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
        title: "Second Paper".to_string(),
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
