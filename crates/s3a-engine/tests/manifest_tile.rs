//! Tests for Plain-Text UTF-8 Manifest Tile (Tile 78) and Embedded Specification Tile (Tile 79)
//!
//! Validates:
//! 1. Manifest present and plain-text readable at byte offset 1024 after seal.
//! 2. Manifest matches tile list (floors, type names, record counts).
//! 3. No restricted data / personal identifiers leak (aggregate counts only).
//! 4. Old files without manifest open and verify cleanly.
//! 5. Embedded compressed specification tile (--embed-spec).
//! 6. MmapReader get_manifest and get_embedded_spec helpers.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use s3a_core::consent::ConsentPurpose;
use s3a_core::{HyperTileHeader, TileType, FILE_HEADER_SIZE, HYPER_TILE_HEADER_SIZE, HYPER_TILE_SIZE};
use s3a_engine::sealed::{
    seal_archive_v2, seal_archive_with_manifest, verify_sealed_archive,
    SealOptionsV2, SPEC_MARKDOWN,
};
use s3a_engine::{MmapReader, S3ACrudEngine, TelemetryRecord, TileWriter};

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
            "s3a_manifest_test_{}_{}_{}_{}.s3a",
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
fn test_manifest_present_and_plain_text_readable_after_seal() {
    let raw = TestTempFile::new("plain_raw");
    let sealed = TestTempFile::new("plain_sealed");

    let mut writer = TileWriter::create(raw.path()).unwrap();
    let recs: Vec<TelemetryRecord> = (0..5)
        .map(|i| TelemetryRecord::new(1000 + i as u64, 10 + i as u32, 1, 42.0 + i as f64))
        .collect();
    writer.write_hyper_tile(TileType::TELEMETRY, &recs, None, None).unwrap();
    drop(writer);

    let opts = SealOptionsV2 {
        enable_parity: false,
        k: 16,
        m: 2,
        prev_seal_hash: [0u8; 32],
        seal_generation: 1,
        write_manifest: true,
        embed_spec: false,
        title: Some("Biomedical Telemetry Trial".to_string()),
        creator: Some("gate://vault-alpha-1234".to_string()),
    };
    seal_archive_v2(raw.path(), sealed.path(), &opts).unwrap();

    let bytes = fs::read(sealed.path()).unwrap();
    assert!(bytes.len() >= FILE_HEADER_SIZE + HYPER_TILE_SIZE);

    // At offset 512, Tile 0 HyperTileHeader
    let hdr_slice = &bytes[FILE_HEADER_SIZE..FILE_HEADER_SIZE + HYPER_TILE_HEADER_SIZE];
    let hdr: &HyperTileHeader = bytemuck::from_bytes(hdr_slice);
    assert_eq!(hdr.tile_type, TileType::MANIFEST);
    assert_eq!(hdr.tile_index, 0);

    // At offset 1024 (512 + 512), plain-text UTF-8 manifest begins
    let payload_offset = FILE_HEADER_SIZE + HYPER_TILE_HEADER_SIZE;
    let payload_len = hdr.payload_bytes as usize;
    assert!(payload_len > 100);
    let manifest_str = std::str::from_utf8(&bytes[payload_offset..payload_offset + payload_len])
        .expect("Payload must be valid UTF-8 plain text");

    // Verify required contents per specification
    assert!(manifest_str.contains("THIS IS AN S3A FILE"));
    assert!(manifest_str.contains("https://github.com/ashleycribb/s3a-storage/blob/main/docs/spec/S3A-TILE-SPEC.md"));
    assert!(manifest_str.contains("Title / Purpose:  Biomedical Telemetry Trial"));
    assert!(manifest_str.contains("Creator:          gate://vault-alpha-1234"));
    assert!(manifest_str.contains("how to read: s3a-cli tour <file> or the S3A reader app"));
    assert!(manifest_str.contains("SEAL INTEGRITY:"));
    assert!(manifest_str.contains("Seal Status:      SEALED"));
    assert!(manifest_str.contains("TILE DIRECTORY:"));
    assert!(manifest_str.contains("manifest"));
    assert!(manifest_str.contains("telemetry"));

    // Verify reader opens cleanly
    let reader = MmapReader::open(sealed.path()).unwrap();
    assert_eq!(reader.tile_count(), 2); // 1 manifest + 1 telemetry
    let manifest_from_reader = reader.get_manifest().expect("Must read manifest via MmapReader");
    assert_eq!(manifest_from_reader, manifest_str);

    // Verify sealed verification
    let report = verify_sealed_archive(sealed.path(), None).unwrap();
    assert!(report.is_sealed);
    assert_eq!(report.tile_count, 2);
    assert_eq!(report.repaired_tiles.len(), 0);
}

#[test]
fn test_manifest_matches_tile_list_and_record_counts() {
    let raw = TestTempFile::new("matches_raw");
    let sealed = TestTempFile::new("matches_sealed");

    let mut writer = TileWriter::create(raw.path()).unwrap();
    let recs1: Vec<TelemetryRecord> = (0..12)
        .map(|i| TelemetryRecord::new(100 + i as u64, 1, 1, 1.0))
        .collect();
    let recs2: Vec<TelemetryRecord> = (0..7)
        .map(|i| TelemetryRecord::new(200 + i as u64, 2, 2, 2.0))
        .collect();
    writer.write_hyper_tile(TileType::TELEMETRY, &recs1, None, None).unwrap();
    writer.write_hyper_tile(TileType::TELEMETRY, &recs2, None, None).unwrap();
    drop(writer);

    seal_archive_with_manifest(raw.path(), sealed.path(), 16, 2, false).unwrap();

    let reader = MmapReader::open(sealed.path()).unwrap();
    let manifest = reader.get_manifest().unwrap();

    // Floor 0: manifest (0 records)
    // Floor 1: telemetry (12 records)
    // Floor 2: telemetry (7 records)
    assert!(manifest.contains("Floor  Type ID  Type Name             Records    Tile Size"));
    assert!(manifest.contains("0      78       manifest              0"));
    assert!(manifest.contains("1      0        telemetry             12"));
    assert!(manifest.contains("2      0        telemetry             7"));
    assert!(manifest.contains("Total Floors: 3"));
}

#[test]
fn test_manifest_no_restricted_data_leaks_counts_only() {
    let raw = TestTempFile::new("privacy_raw");
    let sealed = TestTempFile::new("privacy_sealed");

    let engine = S3ACrudEngine::open_or_create(raw.path()).unwrap();

    // Secret subject identifier and confidential parameters
    let secret_subject_uuid = [0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE, 0xBA, 0xBE, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
    let secret_subject_hex = hex::encode(secret_subject_uuid);

    engine
        .grant_consent(
            secret_subject_uuid,
            9999, // study_id
            ConsentPurpose::Research,
            0xFFFF,
            1700000000,
            0,
            false,
            0xABCD,
            true,
        )
        .unwrap();

    // Now withdraw consent
    engine
        .withdraw_consent(
            secret_subject_uuid,
            9999,
            ConsentPurpose::Research,
            0xFFFF,
            1700001000,
            0xABCD,
            true,
        )
        .unwrap();
    drop(engine);

    seal_archive_with_manifest(raw.path(), sealed.path(), 16, 2, false).unwrap();

    let reader = MmapReader::open(sealed.path()).unwrap();
    let manifest = reader.get_manifest().unwrap();

    // Manifest MUST have aggregate counts
    assert!(manifest.contains("Total Consents:   1"));
    assert!(manifest.contains("Withdrawn:        1"));
    assert!(manifest.contains("Policy Notice:    Protected under S3A consent governance (aggregate counts only; zero payload leaks)"));

    // CRITICAL PRIVACY ASSERTION: The secret subject UUID, hex string, or private identifiers MUST NEVER leak into the manifest!
    assert!(!manifest.contains(&secret_subject_hex));
    assert!(!manifest.contains("deadbeefcafebabe"));
    assert!(!manifest.contains("9999")); // study_id not leaked in text
}

#[test]
fn test_old_files_without_manifest_open_and_verify() {
    let raw = TestTempFile::new("old_raw");
    let sealed = TestTempFile::new("old_sealed");

    let mut writer = TileWriter::create(raw.path()).unwrap();
    let recs = vec![TelemetryRecord::new(500, 1, 1, 100.0)];
    writer.write_hyper_tile(TileType::TELEMETRY, &recs, None, None).unwrap();
    drop(writer);

    // Explicitly seal without manifest (simulating legacy v1.0/v1.1 file)
    let opts = SealOptionsV2 {
        enable_parity: true,
        k: 16,
        m: 2,
        prev_seal_hash: [0u8; 32],
        seal_generation: 1,
        write_manifest: false,
        embed_spec: false,
        title: None,
        creator: None,
    };
    seal_archive_v2(raw.path(), sealed.path(), &opts).unwrap();

    let reader = MmapReader::open(sealed.path()).unwrap();
    assert_eq!(reader.tile_count(), 1);
    assert_eq!(reader.get_manifest(), None); // No manifest in old archive

    let report = verify_sealed_archive(sealed.path(), None).unwrap();
    assert!(report.is_sealed);
    assert_eq!(report.tile_count, 1);
    assert!(report.warnings.is_empty());
}

#[test]
fn test_embedded_spec_tile_roundtrip() {
    let raw = TestTempFile::new("spec_raw");
    let sealed = TestTempFile::new("spec_sealed");

    let mut writer = TileWriter::create(raw.path()).unwrap();
    let recs = vec![TelemetryRecord::new(100, 1, 1, 5.0)];
    writer.write_hyper_tile(TileType::TELEMETRY, &recs, None, None).unwrap();
    drop(writer);

    let opts = SealOptionsV2 {
        enable_parity: false,
        k: 16,
        m: 2,
        prev_seal_hash: [0u8; 32],
        seal_generation: 1,
        write_manifest: true,
        embed_spec: true,
        title: Some("Specification Container".to_string()),
        creator: None,
    };
    seal_archive_v2(raw.path(), sealed.path(), &opts).unwrap();

    let reader = MmapReader::open(sealed.path()).unwrap();
    assert_eq!(reader.tile_count(), 3); // 0: Manifest, 1: Specification, 2: Telemetry

    let (hdr0, _) = reader.get_tile(0).unwrap();
    assert_eq!(hdr0.tile_type, TileType::MANIFEST);

    let (hdr1, _) = reader.get_tile(1).unwrap();
    assert_eq!(hdr1.tile_type, TileType::SPECIFICATION);

    let (hdr2, _) = reader.get_tile(2).unwrap();
    assert_eq!(hdr2.tile_type, TileType::TELEMETRY);

    let decompressed_spec = reader.get_embedded_spec().expect("Must decompress embedded spec");
    assert!(decompressed_spec.starts_with("# S3A Hyper-Tile Storage Format Specification"));
    assert_eq!(decompressed_spec, SPEC_MARKDOWN);

    let report = verify_sealed_archive(sealed.path(), None).unwrap();
    assert!(report.is_sealed);
    assert_eq!(report.tile_count, 3);
}
