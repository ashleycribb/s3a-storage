//! Checkpoint-Gated Compaction Test Suite
//!
//! Validates:
//! 1. Corrupt an input tile -> compaction refuses fail-closed, inputs survive, tile marked quarantined.
//! 2. Injected record drop or duplication -> During count/hash gate fails, nothing commits.
//! 3. Crash between write and verify -> old slot is still the live one.
//! 4. Duplicate input tile in same compaction generation -> Before-gate rejects fail-closed.
//! 5. Quarantined tile -> compaction refuses immediately with clear error.

use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use s3a_core::{
    FILE_HEADER_SIZE, HYPER_TILE_HEADER_SIZE, HYPER_TILE_SIZE,
};
use s3a_engine::{
    compactor_gates::{clear_quarantine, fault_injection, is_tile_quarantined, quarantine_path, quarantine_tile},
    recover_archive, Compactor, MmapReader, TelemetryRecord, TileType, TileWriter,
};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);
static FAULT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
        path.push(format!("s3a_gate_test_{}_{}_{}_{}.s3a", suffix, std::process::id(), ts, id));
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestTempFile {
    fn drop(&mut self) {
        clear_quarantine(&self.path);
        let _ = fs::remove_file(&self.path);
    }
}

/// Test 1: Corrupt an input tile, so compaction refuses and the inputs survive.
#[test]
fn test_corrupt_input_tile_refuses_compaction_and_inputs_survive() {
    let _lock = FAULT_LOCK.lock().unwrap();
    fault_injection::reset();
    let temp_input = TestTempFile::new("corrupt_input");
    let in_path = temp_input.path();
    let temp_out = TestTempFile::new("corrupt_out");
    let out_path = temp_out.path();

    // 1. Create input archive with 2 valid tiles
    let mut writer = TileWriter::create(in_path).unwrap();
    let rec1 = TelemetryRecord::new(100, 1, 1, 10.0);
    let rec2 = TelemetryRecord::new(200, 1, 1, 20.0);
    writer.write_hyper_tile(TileType::TELEMETRY, &[rec1], Some(&[100]), None).unwrap();
    writer.write_hyper_tile(TileType::TELEMETRY, &[rec2], Some(&[200]), None).unwrap();
    drop(writer);

    let initial_len = fs::metadata(in_path).unwrap().len();
    assert_eq!(initial_len, (FILE_HEADER_SIZE + 2 * HYPER_TILE_SIZE) as u64);

    // Verify input archive is initially healthy
    {
        let reader = MmapReader::open(in_path).unwrap();
        assert_eq!(reader.tile_count(), 2);
        assert!(reader.verify_checksums().is_ok());
    }

    // 2. Corrupt payload bytes of Tile 1 (offset: FILE_HEADER_SIZE + 1 * HYPER_TILE_SIZE + HYPER_TILE_HEADER_SIZE + 4)
    let corrupt_offset = (FILE_HEADER_SIZE + HYPER_TILE_SIZE + HYPER_TILE_HEADER_SIZE + 4) as u64;
    {
        let mut f = OpenOptions::new().write(true).open(in_path).unwrap();
        f.seek(SeekFrom::Start(corrupt_offset)).unwrap();
        f.write_all(&[0xDE, 0xAD, 0xBE, 0xEF]).unwrap();
        f.flush().unwrap();
        f.sync_all().unwrap();
    }

    // 3. Attempt compaction: must fail-closed!
    let comp_res = Compactor::compact(&[in_path], out_path);
    assert!(comp_res.is_err(), "Compaction must refuse when input tile CRC is corrupted");
    let err_msg = comp_res.unwrap_err().to_string();
    assert!(
        err_msg.contains("quarantined") || err_msg.contains("CRC mismatch"),
        "Error message must indicate failure/quarantine: {}",
        err_msg
    );

    // 4. Verify output was discarded and inputs survived completely
    assert!(!out_path.exists(), "Output file must be discarded on compaction failure");
    assert!(in_path.exists(), "Input file must survive intact");
    assert_eq!(
        fs::metadata(in_path).unwrap().len(),
        initial_len,
        "Input file length must not be truncated or deleted"
    );

    // 5. Verify the corrupted tile was marked quarantined
    let in_gen = {
        let r = MmapReader::open(in_path).unwrap();
        r.generation()
    };
    assert!(
        is_tile_quarantined(in_path, 2, in_gen).unwrap(),
        "Offending Tile 1 (tile_id 2) must be marked quarantined after retry failure"
    );

    // 6. Further compaction attempt must refuse immediately with clear error
    let second_comp = Compactor::compact(&[in_path], out_path);
    assert!(second_comp.is_err());
    let err_str = second_comp.unwrap_err().to_string();
    assert!(
        err_str.contains("quarantined"),
        "Subsequent compaction must clearly state tile is quarantined: {}",
        err_str
    );
    assert!(!out_path.exists(), "Output must not be created");
}

/// Test 2: Injected record drop or duplication, so the count/hash gate fails and nothing commits.
#[test]
fn test_injected_record_drop_or_duplication_fails_during_gate_and_nothing_commits() {
    let _lock = FAULT_LOCK.lock().unwrap();
    fault_injection::reset();
    let temp_input = TestTempFile::new("during_gate_input");
    let in_path = temp_input.path();
    let temp_out = TestTempFile::new("during_gate_out");
    let out_path = temp_out.path();

    // 1. Create input archive with 3 records
    let mut writer = TileWriter::create(in_path).unwrap();
    let r1 = TelemetryRecord::new(100, 1, 1, 10.0);
    let r2 = TelemetryRecord::new(200, 1, 2, 20.0);
    let r3 = TelemetryRecord::new(300, 1, 3, 30.0);
    writer.write_hyper_tile(TileType::TELEMETRY, &[r1, r2, r3], Some(&[100, 200, 300]), None).unwrap();
    drop(writer);

    let initial_len = fs::metadata(in_path).unwrap().len();

    // 2. Scenario A: Injected record DROP
    fault_injection::INJECT_RECORD_DROP.store(true, Ordering::SeqCst);
    let drop_res = Compactor::compact(&[in_path], out_path);
    assert!(drop_res.is_err(), "Compaction must fail when a record drop is injected");
    let drop_err = drop_res.unwrap_err().to_string();
    assert!(
        drop_err.contains("count mismatch") || drop_err.contains("During-Gate"),
        "Error must identify During-Gate mismatch: {}",
        drop_err
    );
    assert!(!out_path.exists(), "Output must be discarded after record drop failure");
    assert_eq!(fs::metadata(in_path).unwrap().len(), initial_len, "Input must survive untouched");

    // Clear quarantine from the drop test failure before scenario B
    clear_quarantine(in_path);
    fault_injection::reset();

    // 3. Scenario B: Injected record DUPLICATION
    fault_injection::INJECT_RECORD_DUPLICATION.store(true, Ordering::SeqCst);
    let dup_res = Compactor::compact(&[in_path], out_path);
    assert!(dup_res.is_err(), "Compaction must fail when a record duplication is injected");
    let dup_err = dup_res.unwrap_err().to_string();
    assert!(
        dup_err.contains("count mismatch") || dup_err.contains("During-Gate"),
        "Error must identify During-Gate mismatch: {}",
        dup_err
    );
    assert!(!out_path.exists(), "Output must be discarded after record duplication failure");
    assert_eq!(fs::metadata(in_path).unwrap().len(), initial_len, "Input must survive untouched");

    fault_injection::reset();
    clear_quarantine(in_path);

    // 4. Verify clean compaction succeeds once fault injection is reset (outputs telemetry + schema tile)
    let clean_res = Compactor::compact(&[in_path], out_path);
    assert!(clean_res.is_ok(), "Clean compaction must succeed without fault injection");
    let reader = MmapReader::open(out_path).unwrap();
    assert_eq!(reader.tile_count(), 2);
    assert!(reader.verify_checksums().is_ok());
}

/// Test 3: Crash between write and verify, so the old slot is still the live one.
#[test]
fn test_crash_between_write_and_verify_old_slot_still_live() {
    let _lock = FAULT_LOCK.lock().unwrap();
    fault_injection::reset();
    let temp = TestTempFile::new("crash_between_write_verify");
    let path = temp.path();

    // 1. Create archive and commit tile 1 (starts in Slot A gen 1, commits tile 1 in Slot B gen 2)
    let mut writer = TileWriter::create(path).unwrap();
    let rec1 = TelemetryRecord::new(1000, 10, 1, 100.0);
    writer.write_hyper_tile(TileType::TELEMETRY, &[rec1], Some(&[1000]), None).unwrap();
    drop(writer);

    let reader_init = MmapReader::open(path).unwrap();
    let old_slot = reader_init.active_slot();
    let old_gen = reader_init.generation();
    let old_count = reader_init.tile_count();
    assert_eq!(old_slot, 1, "Initial active slot should be Slot B (1)");
    assert_eq!(old_gen, 2, "Initial generation should be 2");
    assert_eq!(old_count, 1, "Initial tile count should be 1");
    drop(reader_init);

    // 2. Open append and enable crash injection between tile write and header commit switch
    let mut writer2 = TileWriter::open_append(path).unwrap();
    fault_injection::INJECT_CRASH_BEFORE_COMMIT.store(true, Ordering::SeqCst);

    let rec2 = TelemetryRecord::new(2000, 10, 1, 200.0);
    let write_res = writer2.write_hyper_tile(TileType::TELEMETRY, &[rec2], Some(&[2000]), None);
    assert!(write_res.is_err(), "Write must abort on injected crash before commit");
    drop(writer2);
    fault_injection::reset();

    // 3. Inspect archive: old slot is still the live one!
    let reader_post_crash = MmapReader::open(path).unwrap();
    assert_eq!(reader_post_crash.active_slot(), old_slot, "Old slot must still be the active one!");
    assert_eq!(reader_post_crash.generation(), old_gen, "Old generation must remain active!");
    assert_eq!(reader_post_crash.tile_count(), old_count, "Old tile count must remain active!");
    assert!(reader_post_crash.verify_checksums().is_ok(), "Old committed data must be intact and verified");
    drop(reader_post_crash);

    // 4. Archive recovery should clean up the uncommitted bytes and keep old slot live
    let recovered = recover_archive(path).unwrap();
    assert_eq!(recovered.generation, old_gen);
    assert_eq!(recovered.tile_count, old_count);

    // 5. Subsequent append can proceed cleanly from the old slot
    let mut writer3 = TileWriter::open_append(path).unwrap();
    assert_eq!(writer3.active_slot(), old_slot);
    assert_eq!(writer3.tile_count(), old_count);
    let rec3 = TelemetryRecord::new(3000, 10, 1, 300.0);
    writer3.write_hyper_tile(TileType::TELEMETRY, &[rec3], Some(&[3000]), None).unwrap();
    drop(writer3);

    let reader_final = MmapReader::open(path).unwrap();
    assert_eq!(reader_final.generation(), old_gen + 1);
    assert_eq!(reader_final.tile_count(), old_count + 1);
    assert_eq!(reader_final.active_slot(), 1 - old_slot); // Switched to Slot A
    assert!(reader_final.verify_checksums().is_ok());
}

/// Test 4: Allow each input tile to be used only once per compaction generation.
#[test]
fn test_duplicate_input_tile_in_compaction_generation_rejected() {
    let _lock = FAULT_LOCK.lock().unwrap();
    fault_injection::reset();
    let temp = TestTempFile::new("dup_input");
    let in_path = temp.path();
    let temp_out = TestTempFile::new("dup_out");
    let out_path = temp_out.path();

    // Create 1 input tile
    let mut writer = TileWriter::create(in_path).unwrap();
    let rec = TelemetryRecord::new(100, 1, 1, 10.0);
    writer.write_hyper_tile(TileType::TELEMETRY, &[rec], Some(&[100]), None).unwrap();
    drop(writer);

    // Pass the same input path twice in input_paths:
    let res = Compactor::compact(&[in_path, in_path], out_path);
    assert!(res.is_err(), "Compaction must fail when the same input tile is consumed twice in one pass");
    let err = res.unwrap_err().to_string();
    assert!(
        err.contains("consumed more than once") || err.contains("duplicate input tile"),
        "Error must mention duplicate tile detection: {}",
        err
    );
    assert!(!out_path.exists(), "Output must be discarded");
}

/// Test 5: A zero header CRC on a non-legacy tile is rejected fail-closed.
#[test]
fn test_zero_header_crc_rejected_on_non_legacy_tile() {
    let _lock = FAULT_LOCK.lock().unwrap();
    fault_injection::reset();
    let temp_in = TestTempFile::new("zero_crc_in");
    let in_path = temp_in.path();
    let temp_out = TestTempFile::new("zero_crc_out");
    let out_path = temp_out.path();

    // 1. Create a valid tile (record_layout_version = 1 by default, which is non-legacy)
    let mut writer = TileWriter::create(in_path).unwrap();
    let rec = TelemetryRecord::new(100, 1, 1, 10.0);
    writer.write_hyper_tile(TileType::TELEMETRY, &[rec], Some(&[100]), None).unwrap();
    drop(writer);

    // 2. Corrupt header CRC by zeroing it out: header_crc32 is at offset 36 in HyperTileHeader
    let header_crc_offset = (FILE_HEADER_SIZE + 36) as u64;
    {
        let mut f = OpenOptions::new().write(true).open(in_path).unwrap();
        f.seek(SeekFrom::Start(header_crc_offset)).unwrap();
        f.write_all(&[0u8; 4]).unwrap();
        f.flush().unwrap();
        f.sync_all().unwrap();
    }

    // 3. Compaction must fail-closed!
    let res = Compactor::compact(&[in_path], out_path);
    assert!(res.is_err(), "Compactor must reject non-legacy tile with zero header CRC");
    let err_str = res.unwrap_err().to_string();
    assert!(
        err_str.contains("zero header CRC") || err_str.contains("header CRC mismatch") || err_str.contains("quarantined"),
        "Error must mention zero header CRC / mismatch: {}",
        err_str
    );
    assert!(!out_path.exists(), "Output must not be created");
}

/// Test 6: Quarantining generation N doesn't block the same tile ID in generation N+1.
#[test]
fn test_quarantining_generation_n_does_not_block_generation_n_plus_1() {
    let _lock = FAULT_LOCK.lock().unwrap();
    fault_injection::reset();
    let temp_in = TestTempFile::new("gen_scoping_in");
    let in_path = temp_in.path();
    let temp_out = TestTempFile::new("gen_scoping_out");
    let out_path = temp_out.path();

    // Quarantine tile 1 for generation 2
    quarantine_tile(in_path, 1, 2).unwrap();
    assert!(
        is_tile_quarantined(in_path, 1, 2).unwrap(),
        "Tile 1 gen 2 must be quarantined"
    );
    assert!(
        !is_tile_quarantined(in_path, 1, 3).unwrap(),
        "Tile 1 gen 3 must NOT be quarantined"
    );

    // Write a tile into in_path such that the active archive generation reaches gen 3:
    // TileWriter::create starts at gen 1, commits tile 1 at gen 2, commits tile 2 at gen 3.
    let mut writer = TileWriter::create(in_path).unwrap();
    let r1 = TelemetryRecord::new(100, 1, 1, 10.0);
    let r2 = TelemetryRecord::new(200, 1, 2, 20.0);
    writer.write_hyper_tile(TileType::TELEMETRY, &[r1], Some(&[100]), None).unwrap();
    writer.write_hyper_tile(TileType::TELEMETRY, &[r2], Some(&[200]), None).unwrap();
    drop(writer);

    let active_gen = {
        let r = MmapReader::open(in_path).unwrap();
        r.generation()
    };
    assert_eq!(active_gen, 3, "Archive active generation should be 3");

    // Compaction of generation 3 must succeed without being blocked by generation 2's quarantine!
    let res = Compactor::compact(&[in_path], out_path);
    assert!(
        res.is_ok(),
        "Compaction must succeed for generation 3 even if generation 2 was quarantined: {:?}",
        res.err()
    );
    assert!(out_path.exists());
    let reader = MmapReader::open(out_path).unwrap();
    assert_eq!(reader.tile_count(), 2);
    assert!(reader.verify_checksums().is_ok());
}

/// Test 7: A corrupt sidecar line returns an error instead of being skipped.
#[test]
fn test_corrupt_sidecar_line_returns_error() {
    let _lock = FAULT_LOCK.lock().unwrap();
    fault_injection::reset();
    let temp_in = TestTempFile::new("corrupt_sidecar_in");
    let in_path = temp_in.path();
    let temp_out = TestTempFile::new("corrupt_sidecar_out");
    let out_path = temp_out.path();

    // Create a valid archive
    let mut writer = TileWriter::create(in_path).unwrap();
    let r1 = TelemetryRecord::new(100, 1, 1, 10.0);
    writer.write_hyper_tile(TileType::TELEMETRY, &[r1], Some(&[100]), None).unwrap();
    drop(writer);

    // Corrupt the quarantine sidecar with malformed unparseable content
    let q_path = quarantine_path(in_path);
    {
        let mut f = OpenOptions::new().create(true).write(true).truncate(true).open(&q_path).unwrap();
        writeln!(f, "CORRUPT_LINE_WITHOUT_COLON_OR_VALID_INTS").unwrap();
        f.flush().unwrap();
    }

    // Checking quarantine must fail with io::ErrorKind::InvalidData
    let check_res = is_tile_quarantined(in_path, 1, 1);
    assert!(check_res.is_err(), "is_tile_quarantined must fail on corrupt sidecar line");
    let err = check_res.unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert!(err.to_string().contains("Corrupted quarantine sidecar"));

    // Compactor must fail closed when checking quarantine on corrupt sidecar
    let comp_res = Compactor::compact(&[in_path], out_path);
    assert!(comp_res.is_err(), "Compaction must fail closed on corrupt quarantine sidecar");
    let comp_err = comp_res.unwrap_err().to_string();
    assert!(
        comp_err.contains("Corrupted quarantine sidecar"),
        "Compactor error must report corrupt quarantine sidecar: {}",
        comp_err
    );
    assert!(!out_path.exists(), "Output must not be created");
}
