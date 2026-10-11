//! Comprehensive Integration and Read-Bound Tests for `s3a-cli tour`
//!
//! Validates:
//! 1. Lobby on v1 and v2 files
//! 2. Floor detail with schema fields and records
//! 3. Array slice reads only covered chunks (verified with TrackingReader)
//! 4. Cross-section ordering across floors
//! 5. Consent-withheld counts (withholding participants without active research consent)
//! 6. Bytes-read bound (reads strictly bounded to footer or requested tile)
//! 7. CLI binary execution with plain text and --json

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use s3a_cli::tour::{tour_file, TourOptions};
use s3a_core::array::{ArrayDataType, ArrayTileBuilder};
use s3a_core::consent::{ConsentPurpose, ConsentRecord};
use s3a_core::schema::encode_schema_payload;
use s3a_core::{
    TelemetryRecord, TileType, FILE_HEADER_SIZE,
    HYPER_TILE_HEADER_SIZE, HYPER_TILE_SIZE,
};
use s3a_engine::{
    seal_archive_v1, seal_archive_v2, standard_schema_for_type, SealOptionsV2, TileWriter,
};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(1000);

struct TempArchiveGuard {
    path: PathBuf,
}

impl TempArchiveGuard {
    fn new(label: &str) -> Self {
        let count = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let mut p = std::env::temp_dir();
        p.push(format!("s3a_tour_test_{}_{}_{}.s3a", label, std::process::id(), count));
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

/// A reader tracking every read range `(file_offset, byte_count)` to mathematically prove read bounds.
struct TrackingReader<R> {
    inner: R,
    pub read_ranges: Vec<(u64, usize)>,
}

impl<R: Seek> TrackingReader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            read_ranges: Vec::new(),
        }
    }
}

impl<R: Read + Seek> Read for TrackingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let pos = self.inner.stream_position()?;
        let n = self.inner.read(buf)?;
        if n > 0 {
            self.read_ranges.push((pos, n));
        }
        Ok(n)
    }
}

impl<R: Seek> Seek for TrackingReader<R> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

#[test]
fn test_tour_lobby_v1_and_v2() {
    let raw = TempArchiveGuard::new("lobby_raw");
    let sealed_v1 = TempArchiveGuard::new("lobby_v1");
    let sealed_v2 = TempArchiveGuard::new("lobby_v2");

    // 1. Create raw file with telemetry and consent
    let mut writer = TileWriter::create(raw.path()).unwrap();
    let recs = vec![
        TelemetryRecord::new(100, 1, 1, 10.5),
        TelemetryRecord::new(101, 1, 2, 20.5),
    ];
    writer.write_hyper_tile(TileType::TELEMETRY, &recs, None, None).unwrap();

    let sub_a = [1u8; 16];
    let consent_a = ConsentRecord::new(sub_a, 10, ConsentPurpose::Research, 0, 100, 0, false, 1, [0u8; 32]);
    writer.write_hyper_tile(TileType::CONSENT, &[consent_a], None, None).unwrap();
    drop(writer);

    // 2. Seal as v1
    seal_archive_v1(raw.path(), sealed_v1.path(), 16, 2).unwrap();

    let mut file_v1 = fs::File::open(sealed_v1.path()).unwrap();
    let len_v1 = file_v1.metadata().unwrap().len();
    let opts = TourOptions::default();
    let report_v1 = tour_file(&mut file_v1, len_v1, sealed_v1.path().to_str().unwrap(), &opts).unwrap();

    assert_eq!(report_v1.lobby.footer_version, "v1");
    assert!(report_v1.lobby.seal_status.is_sealed);
    assert_eq!(report_v1.lobby.tile_count, 2);
    assert_eq!(report_v1.lobby.consent_summary.active_research_count, 1);

    // 3. Seal as v2
    let opts_v2 = SealOptionsV2 {
        enable_parity: false,
        k: 16,
        m: 2,
        prev_seal_hash: [0u8; 32],
        seal_generation: 1,
    };
    seal_archive_v2(raw.path(), sealed_v2.path(), &opts_v2).unwrap();

    let mut file_v2 = fs::File::open(sealed_v2.path()).unwrap();
    let len_v2 = file_v2.metadata().unwrap().len();
    let report_v2 = tour_file(&mut file_v2, len_v2, sealed_v2.path().to_str().unwrap(), &opts).unwrap();

    assert_eq!(report_v2.lobby.footer_version, "v2");
    assert!(report_v2.lobby.seal_status.is_sealed);
    assert!(report_v2.lobby.seal_status.hash_chain_verified);
    assert_eq!(report_v2.lobby.tile_count, 2);
    assert!(report_v2.lobby.schema_names.contains_key(&TileType::TELEMETRY));
    assert!(report_v2.lobby.schema_names.contains_key(&TileType::CONSENT));
}

#[test]
fn test_tour_floor_detail() {
    let raw = TempArchiveGuard::new("detail_raw");
    let sealed = TempArchiveGuard::new("detail_v2");

    let mut writer = TileWriter::create(raw.path()).unwrap();
    let recs = vec![
        TelemetryRecord::new(100, 10, 1, 42.0),
        TelemetryRecord::new(200, 20, 2, 84.0),
    ];
    writer.write_hyper_tile(TileType::TELEMETRY, &recs, None, None).unwrap();

    // Floor 1: Schema tile
    let telem_schema = standard_schema_for_type(TileType::TELEMETRY);
    let schema_payload = encode_schema_payload(&[telem_schema]);
    writer.write_hyper_tile(TileType::SCHEMA, &schema_payload, None, None).unwrap();
    drop(writer);

    seal_archive_v2(raw.path(), sealed.path(), &SealOptionsV2::default()).unwrap();

    let mut file = fs::File::open(sealed.path()).unwrap();
    let len = file.metadata().unwrap().len();

    let opts = TourOptions {
        floor: Some(0),
        ..Default::default()
    };
    let report = tour_file(&mut file, len, sealed.path().to_str().unwrap(), &opts).unwrap();

    let detail = report.floor_detail.expect("floor_detail must be populated");
    assert_eq!(detail.index, 0);
    assert_eq!(detail.type_id, TileType::TELEMETRY);
    assert_eq!(detail.type_name, "telemetry");
    assert_eq!(detail.record_count, 2);
    assert_eq!(detail.fields.len(), 5);
    assert_eq!(detail.fields[0].name, "timestamp");
    assert_eq!(detail.fields[1].name, "sensor_id");
    assert_eq!(detail.records.len(), 2);
    assert_eq!(detail.withheld_count, 0);
}

#[test]
fn test_tour_array_slice_reads_only_covered_chunks() {
    let raw = TempArchiveGuard::new("array_slice_raw");
    let sealed = TempArchiveGuard::new("array_slice_v2");

    let mut writer = TileWriter::create(raw.path()).unwrap();

    // Chunk 0: rows [0..4], cols [0..4] (coords [0, 0])
    let mut chunk0 = ArrayTileBuilder::new(2, ArrayDataType::Int32, &[4, 4], &[0, 0]).unwrap();
    chunk0.set_axis_names("rows,cols");
    let data0: Vec<i32> = (0..16).map(|i| 100 + i as i32).collect();
    chunk0.set_data(&data0).unwrap();
    let payload0 = chunk0.build();
    writer.write_hyper_tile(TileType::ARRAY, &payload0, None, None).unwrap();

    // Chunk 1: rows [0..4], cols [4..8] (coords [0, 4])
    let mut chunk1 = ArrayTileBuilder::new(2, ArrayDataType::Int32, &[4, 4], &[0, 4]).unwrap();
    chunk1.set_axis_names("rows,cols");
    let data1: Vec<i32> = (0..16).map(|i| 200 + i as i32).collect();
    chunk1.set_data(&data1).unwrap();
    let payload1 = chunk1.build();
    writer.write_hyper_tile(TileType::ARRAY, &payload1, None, None).unwrap();

    drop(writer);

    seal_archive_v2(raw.path(), sealed.path(), &SealOptionsV2::default()).unwrap();

    let file = fs::File::open(sealed.path()).unwrap();
    let len = file.metadata().unwrap().len();
    let mut tracker = TrackingReader::new(file);

    // Query slice covering strictly Chunk 0 only: rows 0..2, cols 0..2
    let opts = TourOptions {
        slice: Some("rows=0..2,cols=0..2".to_string()),
        ..Default::default()
    };
    let report = tour_file(&mut tracker, len, sealed.path().to_str().unwrap(), &opts).unwrap();

    let slice = report.slice_result.expect("slice_result must be populated");
    assert_eq!(slice.chunks_evaluated, 2);
    assert_eq!(slice.chunks_covered, 1);
    assert_eq!(slice.chunks_skipped, 1);
    assert_eq!(slice.total_elements, 4);

    // Chunk 1 element data starts after ArrayTileHeader (320 bytes)
    let chunk1_elements_start = (FILE_HEADER_SIZE + HYPER_TILE_SIZE + HYPER_TILE_HEADER_SIZE + s3a_core::array::ArrayTileHeader::SIZE) as u64;
    let chunk1_payload_end = (FILE_HEADER_SIZE + 2 * HYPER_TILE_SIZE) as u64;

    // Verify Chunk 1 element data bytes were NEVER touched
    for &(pos, count) in &tracker.read_ranges {
        let read_end = pos + count as u64;
        let overlaps_chunk1 = pos < chunk1_payload_end && read_end > chunk1_elements_start;
        assert!(
            !overlaps_chunk1,
            "Read at pos {} (len {}) touched unneeded chunk 1 element data [{}..{}]!",
            pos, count, chunk1_elements_start, chunk1_payload_end
        );
    }
}

#[test]
fn test_tour_cross_section_ordering() {
    let raw = TempArchiveGuard::new("cross_raw");
    let sealed = TempArchiveGuard::new("cross_v2");

    let mut writer = TileWriter::create(raw.path()).unwrap();

    // Floor 0: sensor 10 and 20
    let recs0 = vec![
        TelemetryRecord::new(100, 10, 1, 1.0),
        TelemetryRecord::new(101, 20, 1, 2.0),
    ];
    writer.write_hyper_tile(TileType::TELEMETRY, &recs0, None, None).unwrap();

    // Floor 1: sensor 10 and 30
    let recs1 = vec![
        TelemetryRecord::new(200, 10, 1, 3.0),
        TelemetryRecord::new(201, 30, 1, 4.0),
    ];
    writer.write_hyper_tile(TileType::TELEMETRY, &recs1, None, None).unwrap();

    // Floor 2: sensor 10 and 40
    let recs2 = vec![
        TelemetryRecord::new(300, 10, 1, 5.0),
        TelemetryRecord::new(301, 40, 1, 6.0),
    ];
    writer.write_hyper_tile(TileType::TELEMETRY, &recs2, None, None).unwrap();

    drop(writer);

    seal_archive_v2(raw.path(), sealed.path(), &SealOptionsV2::default()).unwrap();

    let mut file = fs::File::open(sealed.path()).unwrap();
    let len = file.metadata().unwrap().len();

    let opts = TourOptions {
        cross: Some("sensor_id=10".to_string()),
        ..Default::default()
    };
    let report = tour_file(&mut file, len, sealed.path().to_str().unwrap(), &opts).unwrap();

    let cross = report.cross_section.expect("cross_section must be populated");
    assert_eq!(cross.match_count, 3);
    assert_eq!(cross.withheld_count, 0);

    // Assert strictly monotonic floor index ordering across results
    assert_eq!(cross.matches[0].floor_index, 0);
    assert_eq!(cross.matches[1].floor_index, 1);
    assert_eq!(cross.matches[2].floor_index, 2);
}

#[test]
fn test_tour_consent_withheld_counts() {
    let raw = TempArchiveGuard::new("consent_raw");
    let sealed = TempArchiveGuard::new("consent_v2");

    let sub_a = [0x11u8; 16];
    let sub_b = [0x22u8; 16];

    let mut writer = TileWriter::create(raw.path()).unwrap();

    // Floor 0: Participant records
    // Sub A: granted research consent
    // Sub B: withdrawn research consent
    let grant_a = ConsentRecord::new(sub_a, 1, ConsentPurpose::Research, 0, 100, 0, false, 1, [0u8; 32]);
    let withdraw_b = ConsentRecord::new_withdrawal(sub_b, 1, ConsentPurpose::Research, 0, 200, 1, [0u8; 32]);
    writer.write_hyper_tile(TileType::CONSENT, &[grant_a, withdraw_b], None, None).unwrap();

    drop(writer);

    seal_archive_v2(raw.path(), sealed.path(), &SealOptionsV2::default()).unwrap();

    let mut file = fs::File::open(sealed.path()).unwrap();
    let len = file.metadata().unwrap().len();

    // 1. Lobby consent summary
    let opts_lobby = TourOptions::default();
    let rep_lobby = tour_file(&mut file, len, sealed.path().to_str().unwrap(), &opts_lobby).unwrap();
    assert_eq!(rep_lobby.lobby.consent_summary.active_research_count, 1);
    assert_eq!(rep_lobby.lobby.consent_summary.withdrawn_count, 1);

    // 2. Floor detail with withholding
    let opts_detail = TourOptions {
        floor: Some(0),
        ..Default::default()
    };
    let rep_detail = tour_file(&mut file, len, sealed.path().to_str().unwrap(), &opts_detail).unwrap();
    let detail = rep_detail.floor_detail.unwrap();
    assert_eq!(detail.records.len(), 1); // Only Sub A is visible
    assert_eq!(detail.withheld_count, 1); // Sub B was withheld
    assert_eq!(detail.records[0]["subject_id"], hex::encode(sub_a));
}

#[test]
fn test_tour_bytes_read_bound() {
    let raw = TempArchiveGuard::new("bound_raw");
    let sealed = TempArchiveGuard::new("bound_v2");

    let mut writer = TileWriter::create(raw.path()).unwrap();
    for i in 0..5 {
        let recs = vec![TelemetryRecord::new(100 + i, 1, 1, (i * 10) as f64)];
        writer.write_hyper_tile(TileType::TELEMETRY, &recs, None, None).unwrap();
    }
    drop(writer);

    seal_archive_v2(raw.path(), sealed.path(), &SealOptionsV2::default()).unwrap();

    let file = fs::File::open(sealed.path()).unwrap();
    let len = file.metadata().unwrap().len();

    // Test A: Lobby / Floor list reads ONLY the footer!
    let mut tracker = TrackingReader::new(file);
    let opts_lobby = TourOptions::default();
    let rep = tour_file(&mut tracker, len, sealed.path().to_str().unwrap(), &opts_lobby).unwrap();
    assert_eq!(rep.lobby.tile_count, 5);

    // Index offset is at `file_len - index_len - endcap_size`
    let data_len = (FILE_HEADER_SIZE + 5 * HYPER_TILE_SIZE) as u64;
    for &(pos, count) in &tracker.read_ranges {
        assert!(
            pos >= data_len,
            "Lobby touched tile body data at pos {} (len {})! Expected >= footer offset {}",
            pos, count, data_len
        );
    }

    // Test B: `--floor 2` reads ONLY the footer + Floor 2!
    let file2 = fs::File::open(sealed.path()).unwrap();
    let mut tracker2 = TrackingReader::new(file2);
    let opts_floor = TourOptions {
        floor: Some(2),
        ..Default::default()
    };
    let rep2 = tour_file(&mut tracker2, len, sealed.path().to_str().unwrap(), &opts_floor).unwrap();
    assert_eq!(rep2.floor_detail.unwrap().index, 2);

    let floor_2_start = (FILE_HEADER_SIZE + 2 * HYPER_TILE_SIZE) as u64;
    let floor_2_end = floor_2_start + HYPER_TILE_SIZE as u64;

    for &(pos, count) in &tracker2.read_ranges {
        let in_footer = pos >= data_len;
        let in_floor_2 = pos >= floor_2_start && (pos + count as u64) <= floor_2_end;
        assert!(
            in_footer || in_floor_2,
            "Floor 2 read touched unrelated tile at pos {} (len {})! Expected footer (>= {}) or floor 2 [{}..{}]",
            pos, count, data_len, floor_2_start, floor_2_end
        );
    }
}

#[test]
fn test_tour_cli_binary_and_json() {
    let raw = TempArchiveGuard::new("cli_raw");
    let sealed = TempArchiveGuard::new("cli_v2");

    let mut writer = TileWriter::create(raw.path()).unwrap();
    let recs = vec![TelemetryRecord::new(100, 1, 1, 42.0)];
    writer.write_hyper_tile(TileType::TELEMETRY, &recs, None, None).unwrap();
    drop(writer);

    seal_archive_v2(raw.path(), sealed.path(), &SealOptionsV2::default()).unwrap();

    let bin = env!("CARGO_BIN_EXE_s3a-cli");

    // 1. Plain text lobby
    let out_plain = Command::new(bin)
        .args(["tour", sealed.path().to_str().unwrap()])
        .output()
        .expect("tour execution failed");
    assert!(out_plain.status.success());
    let str_plain = String::from_utf8_lossy(&out_plain.stdout);
    assert!(str_plain.contains("S3A ARCHIVE TOUR"));
    assert!(str_plain.contains("Footer Version:  v2"));
    assert!(str_plain.contains("Floor  0: TELEMETRY"));

    // 2. JSON lobby
    let out_json = Command::new(bin)
        .args(["tour", "--json", sealed.path().to_str().unwrap()])
        .output()
        .expect("tour json failed");
    assert!(out_json.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out_json.stdout).expect("valid json output");
    assert_eq!(v["ok"], true);
    assert_eq!(v["lobby"]["footer_version"], "v2");
    assert_eq!(v["floors"][0]["type_name"], "telemetry");

    // 3. Detail --floor 0 --json
    let out_detail = Command::new(bin)
        .args(["tour", "--json", "--floor", "0", sealed.path().to_str().unwrap()])
        .output()
        .expect("tour floor detail failed");
    assert!(out_detail.status.success());
    let v_det: serde_json::Value = serde_json::from_slice(&out_detail.stdout).expect("valid json output");
    assert_eq!(v_det["floor_detail"]["type_name"], "telemetry");
    assert_eq!(v_det["floor_detail"]["records"][0]["sensor_id"], 1);

    // 4. History --history --json
    let out_hist = Command::new(bin)
        .args(["tour", "--json", "--history", sealed.path().to_str().unwrap()])
        .output()
        .expect("tour history failed");
    assert!(out_hist.status.success());
    let v_hist: serde_json::Value = serde_json::from_slice(&out_hist.stdout).expect("valid json output");
    assert_eq!(v_hist["history"][0]["generation"], 1);
}
