//! Regression tests for the tour-side usability fixes (Nov 18, 2026):
//! - B5: tour's lobby never claims "verified" from the seal chain alone
//!   (payload checksums are `verify`'s job; floor detail checks the tile
//!   it displays and reports the result).
//! - N4: one canonical floor index (position in the archive) used by the
//!   floor directory, `--floor N`, `--json`, and cross-section matches.
//! - B4: honest displayed numbers ("—" for unrecorded hashes / empty key
//!   ranges instead of zeros and `u64::MAX..0` sentinels).
//! - B3: Scholar records decode into named fields instead of hex blobs.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use s3a_cli::tour::{tour_file, TourOptions};
use s3a_core::{
    TelemetryRecord, TileType, ADDON_EDU, FILE_HEADER_SIZE, HYPER_TILE_HEADER_SIZE, HYPER_TILE_SIZE,
};
use s3a_engine::{
    seal_archive_v1, seal_archive_v2, verify_sealed_archive, SealOptionsV2, TileWriter,
};
use s3a_pack_scholar::{
    AcademicPaperRecord, CitationFingerprintRecord, ScholarNoteClaimRecord, ScholarTileType,
    CLAIM_KIND_EMPIRICAL, CONTENT_KIND_PDF,
};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(7000);

struct TempArchiveGuard {
    path: PathBuf,
}

impl TempArchiveGuard {
    fn new(label: &str) -> Self {
        let count = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let mut p = std::env::temp_dir();
        p.push(format!(
            "s3a_tour_ux_{}_{}_{}.s3a",
            label,
            std::process::id(),
            count
        ));
        Self { path: p }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempArchiveGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn register_scholar_types() {
    s3a_core::registry::with_global_registry_mut(|reg| {
        let _ = s3a_pack_scholar::register_pack_types(reg);
    });
}

/// Writes one telemetry tile per timestamp and returns the raw archive path.
fn write_telemetry_archive(path: &Path, count: u32) {
    let mut writer = TileWriter::create(path).unwrap();
    for i in 0..count {
        let recs = vec![TelemetryRecord::new(100 + i as u64, 1, 1, (i * 10) as f64)];
        writer
            .write_hyper_tile(TileType::TELEMETRY, &recs, None, None)
            .unwrap();
    }
    drop(writer);
}

fn seal_with_manifest(raw: &Path, sealed: &Path) {
    let mut opts = SealOptionsV2::default();
    opts.write_manifest = true;
    seal_archive_v2(raw, sealed, &opts).unwrap();
}

fn open_report(path: &Path, opts: &TourOptions) -> s3a_cli::tour::TourReport {
    let mut file = fs::File::open(path).unwrap();
    let len = file.metadata().unwrap().len();
    tour_file(&mut file, len, path.to_str().unwrap(), opts).unwrap()
}

/// Flip one byte inside the first data tile's record payload. With
/// `write_manifest`, the file layout is: header, manifest tile, data tiles.
fn flip_payload_byte(path: &Path) {
    let mut bytes = fs::read(path).unwrap();
    let off = FILE_HEADER_SIZE + HYPER_TILE_SIZE + HYPER_TILE_HEADER_SIZE + 10;
    bytes[off] ^= 0xFF;
    fs::write(path, bytes).unwrap();
}

#[test]
fn tour_lobby_never_says_verified() {
    let raw = TempArchiveGuard::new("lobby_raw");
    let sealed = TempArchiveGuard::new("lobby_v2");
    write_telemetry_archive(raw.path(), 2);
    seal_with_manifest(raw.path(), sealed.path());

    // Good archive: chain valid, but tour must not say "verified" — it did
    // not re-check payloads.
    let rep = open_report(sealed.path(), &TourOptions::default());
    assert!(rep.lobby.seal_status.is_sealed);
    assert!(rep.lobby.seal_status.hash_chain_verified);
    assert!(!rep.lobby.seal_status.payloads_checked);
    assert!(
        !rep.lobby
            .seal_status
            .summary
            .to_lowercase()
            .contains("verified"),
        "lobby summary must not claim verified: {}",
        rep.lobby.seal_status.summary
    );
    assert!(
        rep.lobby.seal_status.summary.contains("not re-checked"),
        "lobby summary must point at verify: {}",
        rep.lobby.seal_status.summary
    );

    // Corrupted archive (one payload byte flipped): verify fails, and tour
    // still must not say "verified" — this is the evaluation's byte-flip.
    let corrupt = TempArchiveGuard::new("lobby_corrupt");
    fs::copy(sealed.path(), corrupt.path()).unwrap();
    flip_payload_byte(corrupt.path());
    assert!(
        verify_sealed_archive(corrupt.path(), None).is_err(),
        "verify must fail on the byte-flipped archive"
    );
    let rep_bad = open_report(corrupt.path(), &TourOptions::default());
    assert!(
        !rep_bad
            .lobby
            .seal_status
            .summary
            .to_lowercase()
            .contains("verified"),
        "corrupt archive lobby must not claim verified: {}",
        rep_bad.lobby.seal_status.summary
    );
    // The chain itself is intact (the flip is in payload bytes, not the
    // footer), which is exactly why the summary must not say "verified".
    assert!(rep_bad.lobby.seal_status.hash_chain_verified);
}

#[test]
fn tour_floor_detail_checks_payload() {
    let raw = TempArchiveGuard::new("payload_raw");
    let sealed = TempArchiveGuard::new("payload_v2");
    write_telemetry_archive(raw.path(), 2);
    seal_with_manifest(raw.path(), sealed.path());

    // Floor 1 is the first telemetry tile (floor 0 is the manifest).
    let detail = open_report(
        sealed.path(),
        &TourOptions {
            floor: Some(1),
            ..Default::default()
        },
    )
    .floor_detail
    .unwrap();
    assert!(detail.payload_ok, "good tile payload must verify");
    assert!(
        detail.payload_check.contains("SHA-256"),
        "payload check must name the hash comparison: {}",
        detail.payload_check
    );

    let corrupt = TempArchiveGuard::new("payload_corrupt");
    fs::copy(sealed.path(), corrupt.path()).unwrap();
    flip_payload_byte(corrupt.path());
    let detail_bad = open_report(
        corrupt.path(),
        &TourOptions {
            floor: Some(1),
            ..Default::default()
        },
    )
    .floor_detail
    .unwrap();
    assert!(
        !detail_bad.payload_ok,
        "flipped tile payload must fail the floor check"
    );
    assert!(
        detail_bad.payload_check.contains("mismatch"),
        "payload check must name the mismatch: {}",
        detail_bad.payload_check
    );
}

#[test]
fn tour_floor_index_is_canonical_position() {
    let raw = TempArchiveGuard::new("floor_raw");
    let sealed = TempArchiveGuard::new("floor_v2");
    write_telemetry_archive(raw.path(), 2);
    seal_with_manifest(raw.path(), sealed.path());

    // The sealed footer records the manifest AND the first data tile both
    // as tile_index 0 (the sealer does not renumber). Tour must still show
    // one index per floor, in position order.
    let rep = open_report(sealed.path(), &TourOptions::default());
    let floors = rep.floors.unwrap();
    assert_eq!(floors.len(), 3);
    let indices: Vec<u32> = floors.iter().map(|f| f.index).collect();
    assert_eq!(indices, vec![0, 1, 2], "floor indices must be positions");
    assert_eq!(floors[0].type_name, "manifest");
    assert_eq!(floors[1].type_name, "telemetry");
    assert_eq!(floors[2].type_name, "telemetry");

    // Every floor is addressable, including the last one (the old code
    // errored "Floor index 4 out of range" on a 5-floor demo archive
    // because it addressed by tile_index).
    for n in 0..3u32 {
        let detail = open_report(
            sealed.path(),
            &TourOptions {
                floor: Some(n),
                ..Default::default()
            },
        )
        .floor_detail
        .unwrap();
        assert_eq!(detail.index, n);
    }
}

#[test]
fn tour_scholar_demo_decodes_records() {
    register_scholar_types();
    let dir = std::env::temp_dir().join(format!(
        "s3a_tour_ux_demo_{}_{}",
        std::process::id(),
        TEST_COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    fs::create_dir_all(&dir).unwrap();
    let bin = env!("CARGO_BIN_EXE_s3a-cli");
    let raw = dir.join("demo.s3a");
    let sealed = dir.join("demo-sealed.s3a");

    let seed = Command::new(bin)
        .args(["scholar", "demo-seed", raw.to_str().unwrap()])
        .current_dir(&dir)
        .output()
        .expect("demo-seed failed");
    assert!(seed.status.success(), "demo-seed failed");

    let seal = Command::new(bin)
        .args(["seal", raw.to_str().unwrap(), sealed.to_str().unwrap()])
        .current_dir(&dir)
        .output()
        .expect("seal failed");
    assert!(seal.status.success(), "seal failed");

    // N4: five floors, canonical 0..4, in manifest order.
    let rep = open_report(&sealed, &TourOptions::default());
    let floors = rep.floors.unwrap();
    assert_eq!(floors.len(), 5);
    assert_eq!(
        floors.iter().map(|f| f.index).collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4]
    );
    assert_eq!(
        floors
            .iter()
            .map(|f| f.type_name.as_str())
            .collect::<Vec<_>>(),
        vec![
            "manifest",
            "academic_papers",
            "citation_fingerprint",
            "note_claims",
            "schema"
        ]
    );

    // B4: empty tiles (manifest, schema) have no key range and a real hash;
    // the raw u64::MAX..0 sentinels must not leak into the display field.
    assert_eq!(floors[0].key_range, None);
    assert_eq!(floors[4].key_range, None);
    assert_eq!(
        floors[1].key_range.as_deref(),
        Some("1600000000..1600000000")
    );
    assert_ne!(floors[0].payload_hash_short, "00000000...");

    // B3: floor 1 is the paper, decoded into named fields (not hex).
    let paper = open_report(
        &sealed,
        &TourOptions {
            floor: Some(1),
            ..Default::default()
        },
    )
    .floor_detail
    .unwrap();
    assert_eq!(paper.type_name, "academic_papers");
    assert!(paper.payload_ok);
    let rec = &paper.records[0];
    assert!(rec.get("record_data").is_none(), "must not be a hex blob");
    assert_eq!(rec["paper_id"], 1001);
    assert_eq!(rec["year"], 2024);
    assert_eq!(rec["citation_count"], 10);
    assert_eq!(rec["topic"]["x"], 0.1);
    assert_eq!(rec["open_access"], true);
    assert_eq!(rec["published"], "2020-09-13T12:26:40Z");
    assert!(paper
        .record_note
        .as_deref()
        .unwrap_or("")
        .contains("hashes only"));

    // B3: floor 2 is the citation fingerprint; floor 3 the claim (160-byte
    // record — the old code sliced it at the stale 64-byte registry size
    // and showed a truncated hex blob).
    let fp = open_report(
        &sealed,
        &TourOptions {
            floor: Some(2),
            ..Default::default()
        },
    )
    .floor_detail
    .unwrap();
    assert_eq!(fp.records[0]["content_kind"], "PDF");
    assert_eq!(fp.records[0]["paper_id"], 1001);
    assert!(fp.records[0]["content_sha256"].as_str().unwrap().len() == 64);

    let claim = open_report(
        &sealed,
        &TourOptions {
            floor: Some(3),
            ..Default::default()
        },
    )
    .floor_detail
    .unwrap();
    assert_eq!(claim.records[0]["claim_id"], 42);
    assert_eq!(claim.records[0]["kind"], "empirical");
    assert_eq!(claim.records[0]["passage_page"], 4);
    assert!(claim.records[0]["quote_sha256"].as_str().unwrap().len() == 64);
    assert!(claim.records[0]
        .get("verification_statement_uuid")
        .is_some());

    // N4: the last floor (schema) is addressable — the old code errored.
    let schema = open_report(
        &sealed,
        &TourOptions {
            floor: Some(4),
            ..Default::default()
        },
    )
    .floor_detail
    .unwrap();
    assert_eq!(schema.type_name, "schema");

    // B3 via cross-section: searching by a decoded field name works and
    // reports canonical floor indices.
    let cross = open_report(
        &sealed,
        &TourOptions {
            cross: Some("paper_id=1001".to_string()),
            ..Default::default()
        },
    )
    .cross_section
    .unwrap();
    assert_eq!(cross.match_count, 2);
    assert_eq!(
        cross
            .matches
            .iter()
            .map(|m| m.floor_index)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn tour_scholar_records_decode_from_library_tiles() {
    register_scholar_types();
    let raw = TempArchiveGuard::new("scholar_raw");
    let sealed = TempArchiveGuard::new("scholar_v2");

    let paper =
        AcademicPaperRecord::new(1001, 1600000000, 7, 0.5, 0.25, 0.125, 3, 2023, 2, 0, 1, 0);
    let fp = CitationFingerprintRecord::from_content(
        1001,
        "10.1000/1001",
        b"%PDF-1.4 test content",
        CONTENT_KIND_PDF,
        "https://example.org/papers/test.pdf",
        2,
        1600000000,
    );
    let claim = ScholarNoteClaimRecord::new(
        42,
        1600000000,
        [0, 0],
        0xdeadbeef,
        0.99,
        CLAIM_KIND_EMPIRICAL,
        "neural topological memory achieves O(1) recall",
    );

    let mut writer = TileWriter::create(raw.path()).unwrap();
    writer.enable_addon(ADDON_EDU).unwrap();
    writer
        .write_hyper_tile(ScholarTileType::ACADEMIC_PAPERS, &[paper], None, None)
        .unwrap();
    writer
        .write_hyper_tile(ScholarTileType::CITATION_FINGERPRINT, &[fp], None, None)
        .unwrap();
    writer
        .write_hyper_tile(ScholarTileType::NOTE_CLAIMS, &[claim], None, None)
        .unwrap();
    drop(writer);
    seal_archive_v2(raw.path(), sealed.path(), &SealOptionsV2::default()).unwrap();

    let rep = open_report(sealed.path(), &TourOptions::default());
    let floors = rep.floors.unwrap();
    assert_eq!(floors.len(), 3);
    assert_eq!(
        floors.iter().map(|f| f.index).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );

    let paper_detail = open_report(
        sealed.path(),
        &TourOptions {
            floor: Some(0),
            ..Default::default()
        },
    )
    .floor_detail
    .unwrap();
    assert_eq!(paper_detail.records[0]["hilbert_index"], 7);
    assert_eq!(paper_detail.records[0]["year"], 2023);
    assert_eq!(paper_detail.records[0]["warning_count"], 1);
    assert_eq!(paper_detail.records[0]["open_access"], false);

    let claim_detail = open_report(
        sealed.path(),
        &TourOptions {
            floor: Some(2),
            ..Default::default()
        },
    )
    .floor_detail
    .unwrap();
    assert_eq!(claim_detail.records[0]["claim_id"], 42);
    assert_eq!(claim_detail.records[0]["author_hash"], "0xdeadbeef");
    assert_eq!(claim_detail.records[0]["confidence"], 0.99);
    assert_eq!(claim_detail.records[0]["passage_page"], 0);
}

#[test]
fn tour_unsealed_honest_numbers() {
    register_scholar_types();
    let dir = std::env::temp_dir().join(format!(
        "s3a_tour_ux_unsealed_{}_{}",
        std::process::id(),
        TEST_COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    fs::create_dir_all(&dir).unwrap();
    let bin = env!("CARGO_BIN_EXE_s3a-cli");
    let raw = dir.join("demo.s3a");
    let seed = Command::new(bin)
        .args(["scholar", "demo-seed", raw.to_str().unwrap()])
        .current_dir(&dir)
        .output()
        .expect("demo-seed failed");
    assert!(seed.status.success(), "demo-seed failed");

    // B4: unsealed floors record no payload hash — show "—", not zeros.
    let rep = open_report(&raw, &TourOptions::default());
    for f in rep.floors.unwrap() {
        assert_eq!(f.payload_hash_short, "—", "floor {}", f.type_name);
    }
    assert_eq!(rep.lobby.seal_status.summary, "unsealed (live database)");

    // Floor detail on an unsealed tile: hash "—", payload CRCs still check.
    let detail = open_report(
        &raw,
        &TourOptions {
            floor: Some(0),
            ..Default::default()
        },
    )
    .floor_detail
    .unwrap();
    assert_eq!(detail.payload_hash, "—");
    assert!(detail.payload_ok, "writer-set CRCs must verify");
    assert_eq!(detail.records[0]["paper_id"], 1001);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn tour_v1_lobby_never_says_verified() {
    let raw = TempArchiveGuard::new("v1_raw");
    let sealed = TempArchiveGuard::new("v1_sealed");
    write_telemetry_archive(raw.path(), 2);
    seal_archive_v1(raw.path(), sealed.path(), 16, 2).unwrap();

    let rep = open_report(sealed.path(), &TourOptions::default());
    assert_eq!(rep.lobby.footer_version, "v1");
    assert!(rep.lobby.seal_status.is_sealed);
    // V1 never verified a hash chain (it only checks the directory endcap
    // CRC), so it must not claim the archive is verified either.
    assert!(!rep.lobby.seal_status.hash_chain_verified);
    assert!(
        !rep.lobby
            .seal_status
            .summary
            .to_lowercase()
            .contains("verified"),
        "v1 lobby must not claim verified: {}",
        rep.lobby.seal_status.summary
    );

    // Floor detail still checks the tile CRCs (no payload hash exists in v1).
    let detail = open_report(
        sealed.path(),
        &TourOptions {
            floor: Some(0),
            ..Default::default()
        },
    )
    .floor_detail
    .unwrap();
    assert!(detail.payload_ok);
    assert!(
        detail.payload_check.contains("no payload hash recorded"),
        "v1 payload check must say the hash is absent: {}",
        detail.payload_check
    );
}
