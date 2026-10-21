// Test fixtures intentionally use the literal 3.14 as arbitrary sample data.
#![allow(clippy::approx_constant)]
//! S3A Phase 1 Durability & Crash Recovery Fault-Injection Harness
//!
//! Validates:
//! 1. Dual A/B generation slot recovery on active slot corruption.
//! 2. Torn EOF truncation on partial tile append crash.
//! 3. Uncommitted full tile truncation when crash occurs before header slot sync.
//! 4. Mid-compact crash recovery: abandoned temp cleaned, live archive intact.
//! 5. Atomic replacement during compaction: tombstones purged, no orphan files.
//! 6. Compaction correctness: Scholar pack tiles (10-15) and unknown tiles (99)
//!    copied through intact, and multi-input addon_mask bitwise OR preserved.

use std::fs::{self, File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use s3a_core::{
    MarkRecord, ADDON_EDU, ADDON_FINTECH, FILE_HEADER_SIZE, FILE_HEADER_SLOT_A_OFFSET,
    HYPER_TILE_SIZE,
};
use s3a_engine::{
    clean_orphan_temps, recover_archive, unique_compact_temp_path, Compactor, HyperTileHeader,
    MmapReader, QuerySieve, S3ACrudEngine, TelemetryRecord, TileType, TileWriter,
};

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
            "s3a_durability_{}_{}_{}_{}.s3a",
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
        let _ = clean_orphan_temps(&self.path);
        let _ = fs::remove_file(&self.path);
    }
}

/// Scenario 1: Corrupt active generation slot bytes -> fallback to prior valid generation.
#[test]
fn test_crash_corrupt_active_slot_fallback() {
    let temp = TestTempFile::new("slot_fallback");
    let path = temp.path();

    // 1. Create archive (starts Gen 1 in Slot A) and commit tile 1 -> advances to Gen 2 in Slot B
    let mut writer = TileWriter::create(path).unwrap();
    let rec1 = TelemetryRecord::new(1000, 1, 100, 42.0);
    writer
        .write_hyper_tile(TileType::TELEMETRY, &[rec1], Some(&[1000]), None)
        .unwrap();
    drop(writer);

    let reader1 = MmapReader::open(path).unwrap();
    assert_eq!(reader1.generation(), 2);
    assert_eq!(reader1.tile_count(), 1);
    assert_eq!(reader1.active_slot(), 1); // Slot B
    drop(reader1);

    // 2. Open append and commit tile 2 -> advances to Gen 3 in Slot A
    let mut writer = TileWriter::open_append(path).unwrap();
    let rec2 = TelemetryRecord::new(2000, 1, 100, 84.0);
    writer
        .write_hyper_tile(TileType::TELEMETRY, &[rec2], Some(&[2000]), None)
        .unwrap();
    drop(writer);

    let reader2 = MmapReader::open(path).unwrap();
    assert_eq!(reader2.generation(), 3);
    assert_eq!(reader2.tile_count(), 2);
    assert_eq!(reader2.active_slot(), 0); // Slot A
    drop(reader2);

    // 3. Fault Injection: Corrupt active Slot A header bytes (invalidate CRC32C)
    {
        let mut f = OpenOptions::new().write(true).open(path).unwrap();
        f.seek(SeekFrom::Start((FILE_HEADER_SLOT_A_OFFSET + 12) as u64))
            .unwrap();
        f.write_all(&[0xDE, 0xAD, 0xBE, 0xEF]).unwrap();
        f.flush().unwrap();
        f.sync_all().unwrap();
    }

    // 4. Reopen and verify graceful fallback to Generation 2 (Slot B)
    let recovered_header = recover_archive(path).unwrap();
    assert_eq!(recovered_header.generation, 2);
    assert_eq!(recovered_header.tile_count, 1);

    let reader_fallback = MmapReader::open(path).unwrap();
    assert_eq!(reader_fallback.generation(), 2);
    assert_eq!(reader_fallback.tile_count(), 1);
    assert_eq!(reader_fallback.active_slot(), 1);
    assert!(reader_fallback.verify_checksums().is_ok());

    // 5. Append continues from Generation 2 -> advances to Generation 3 in Slot A
    let mut writer2 = TileWriter::open_append(path).unwrap();
    assert_eq!(writer2.tile_count(), 1);
    let rec3 = TelemetryRecord::new(3000, 1, 100, 99.0);
    writer2
        .write_hyper_tile(TileType::TELEMETRY, &[rec3], Some(&[3000]), None)
        .unwrap();
    drop(writer2);

    let reader3 = MmapReader::open(path).unwrap();
    assert_eq!(reader3.generation(), 3); // bumped from gen 2
    assert_eq!(reader3.tile_count(), 2);
    assert_eq!(reader3.active_slot(), 0); // Slot A
    assert!(reader3.verify_checksums().is_ok());
}

/// Scenario 2: Torn EOF partial tile write -> truncate to committed size and resume writing.
#[test]
fn test_crash_torn_eof_partial_tile() {
    let temp = TestTempFile::new("torn_eof");
    let path = temp.path();

    // 1. Create archive with 1 committed tile
    let mut writer = TileWriter::create(path).unwrap();
    let rec1 = TelemetryRecord::new(200, 1, 1000, 10.0);
    writer
        .write_hyper_tile(TileType::TELEMETRY, &[rec1], Some(&[1000]), None)
        .unwrap();
    drop(writer);

    let expected_valid_size = FILE_HEADER_SIZE as u64 + (HYPER_TILE_SIZE as u64);
    assert_eq!(fs::metadata(path).unwrap().len(), expected_valid_size);

    // 2. Fault Injection: Simulate torn write appending 2048 partial bytes to EOF without header commit
    {
        let mut f = OpenOptions::new().append(true).open(path).unwrap();
        let garbage = [0xA5u8; 2048];
        f.write_all(&garbage).unwrap();
        f.flush().unwrap();
        f.sync_all().unwrap();
    }
    assert_eq!(
        fs::metadata(path).unwrap().len(),
        expected_valid_size + 2048
    );

    // 3. Recovery: verify recover_archive truncates the torn bytes
    let recovered_header = recover_archive(path).unwrap();
    assert_eq!(recovered_header.tile_count, 1);
    assert_eq!(fs::metadata(path).unwrap().len(), expected_valid_size);

    // 4. Open append resumes seamlessly
    let mut writer2 = TileWriter::open_append(path).unwrap();
    assert_eq!(writer2.tile_count(), 1);
    let rec2 = TelemetryRecord::new(200, 1, 2000, 20.0);
    writer2
        .write_hyper_tile(TileType::TELEMETRY, &[rec2], Some(&[2000]), None)
        .unwrap();
    drop(writer2);

    let reader = MmapReader::open(path).unwrap();
    assert_eq!(reader.tile_count(), 2);
    assert!(reader.verify_checksums().is_ok());
}

/// Scenario 3: Crash after full tile payload sync but before header slot commit.
#[test]
fn test_crash_uncommitted_tile_before_header_sync() {
    let temp = TestTempFile::new("uncommitted_tile");
    let path = temp.path();

    // 1. Create archive with 1 tile committed
    let mut writer = TileWriter::create(path).unwrap();
    let rec1 = TelemetryRecord::new(300, 1, 1000, 15.0);
    writer
        .write_hyper_tile(TileType::TELEMETRY, &[rec1], Some(&[1000]), None)
        .unwrap();
    drop(writer);

    let committed_size = FILE_HEADER_SIZE as u64 + (HYPER_TILE_SIZE as u64);

    // 2. Fault Injection: Append an entire 128KB tile block at EOF, but do not update header slot
    {
        let mut f = OpenOptions::new().append(true).open(path).unwrap();
        let uncommitted_tile = vec![0x33u8; HYPER_TILE_SIZE];
        f.write_all(&uncommitted_tile).unwrap();
        f.flush().unwrap();
        f.sync_all().unwrap();
    }
    assert_eq!(
        fs::metadata(path).unwrap().len(),
        committed_size + HYPER_TILE_SIZE as u64
    );

    // 3. Reader strictly honors resolve_active_header; uncommitted tile is invisible
    let reader = MmapReader::open(path).unwrap();
    assert_eq!(reader.tile_count(), 1);
    drop(reader);

    // 4. open_append truncates the uncommitted 128KB block and writes Tile 1 cleanly
    let mut writer2 = TileWriter::open_append(path).unwrap();
    assert_eq!(writer2.tile_count(), 1);
    assert_eq!(fs::metadata(path).unwrap().len(), committed_size);

    let rec2 = TelemetryRecord::new(300, 1, 2000, 25.0);
    writer2
        .write_hyper_tile(TileType::TELEMETRY, &[rec2], Some(&[2000]), None)
        .unwrap();
    drop(writer2);

    let reader2 = MmapReader::open(path).unwrap();
    assert_eq!(reader2.tile_count(), 2);
    assert!(reader2.verify_checksums().is_ok());
}

/// Scenario 4: Mid-compact crash where temp file exists but atomic replace was not called.
#[test]
fn test_crash_mid_compact_temp_abandoned() {
    let temp = TestTempFile::new("mid_compact");
    let live_path = temp.path();

    // 1. Create live archive with 2 telemetry records
    let engine = S3ACrudEngine::open_or_create(live_path).unwrap();
    let rec1 = TelemetryRecord::new(400, 1, 1000, 50.0);
    let rec2 = TelemetryRecord::new(400, 1, 2000, 60.0);
    engine.create_telemetry(&[rec1, rec2]).unwrap();

    let initial_tile_count = MmapReader::open(live_path).unwrap().tile_count();

    // 2. Fault Injection: Create an orphan compaction temp file beside the live file
    let orphan_temp = unique_compact_temp_path(live_path);
    {
        let mut f = File::create(&orphan_temp).unwrap();
        f.write_all(b"partial compaction garbage data").unwrap();
        f.flush().unwrap();
        f.sync_all().unwrap();
    }
    assert!(orphan_temp.exists());

    // 3. Recovery on open: live file is verified intact, orphan temp is cleaned
    let active_hdr = recover_archive(live_path).unwrap();
    assert_eq!(active_hdr.tile_count, initial_tile_count);
    assert!(
        !orphan_temp.exists(),
        "Orphan compaction temp must be deleted during archive recovery"
    );

    // 4. Live database remains readable and valid
    let reader = MmapReader::open(live_path).unwrap();
    assert_eq!(reader.tile_count(), initial_tile_count);
    assert!(reader.verify_checksums().is_ok());
}

/// Scenario 5: Atomic replace compact success: tombstones purged, no orphan files.
#[test]
fn test_atomic_replace_compact_success() {
    let temp = TestTempFile::new("atomic_compact");
    let live_path = temp.path();

    let engine = S3ACrudEngine::open_or_create(live_path).unwrap();

    // Write record A, record B, then tombstone record A
    let rec_a = TelemetryRecord::new(1000, 500, 1, 100.0);
    let rec_b = TelemetryRecord::new(2000, 500, 1, 200.0);
    engine.create_telemetry(&[rec_a, rec_b]).unwrap();

    let mut tomb_a = rec_a;
    tomb_a.mark_tombstone();
    engine.create_telemetry(&[tomb_a]).unwrap();

    // Purge and compact using atomic replace
    let new_tile_count = engine.purge_and_compact().unwrap();
    assert!(new_tile_count >= 1);

    // Verify reader sees only record B; tombstone A is purged
    let reader = MmapReader::open(live_path).unwrap();
    assert!(reader.verify_checksums().is_ok());

    let sieve = s3a_engine::QuerySieve::new(&reader);
    let results = sieve.query_telemetry(0, u64::MAX, Some(500), Some(1));
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].timestamp, 2000);
    assert_eq!(results[0].value, 200.0);

    // Verify no orphan files remain in directory
    let parent = live_path.parent().unwrap();
    let stem = live_path.file_stem().unwrap().to_str().unwrap();
    for entry in fs::read_dir(parent).unwrap().flatten() {
        let name = entry.file_name().into_string().unwrap();
        if name.starts_with(&format!("{}.compact.", stem)) {
            panic!("Found uncleaned compaction temp: {}", name);
        }
    }
}

/// Scenario 6: Scholar pack tiles (10-15) and unknown tiles (99) survive compact across writer restart,
/// tombstones are preserved for pack tiles, and addon_mask bitwise OR is preserved.
#[test]
fn test_compact_preserves_scholar_tiles_and_addon_mask() {
    let input_a = TestTempFile::new("compact_input_a");
    let input_b = TestTempFile::new("compact_input_b");
    let out_file = TestTempFile::new("compact_output");

    // 1. Create Archive A with ADDON_EDU (0x0001)
    let mut writer_a = TileWriter::create(input_a.path()).unwrap();
    writer_a.enable_addon(ADDON_EDU).unwrap();

    // Write a Scholar Pack tile (Tile 12: Academic Papers) with 1 active record and 1 tombstone
    #[repr(C, align(8))]
    #[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
    struct MockPaperRecord {
        paper_id: u64,
        timestamp: u64,
        flags: u64,
        _pad: [u64; 5],
    }

    let paper_live = MockPaperRecord {
        paper_id: 101,
        timestamp: 1000,
        flags: 0,
        _pad: [0; 5],
    };
    let paper_tomb = MockPaperRecord {
        paper_id: 102,
        timestamp: 1001,
        flags: 1,
        _pad: [0; 5],
    };
    writer_a
        .write_hyper_tile_ext(
            12,
            &[paper_live, paper_tomb],
            Some(&[1000, 1001]),
            None,
            None,
        )
        .unwrap();

    // Write an arbitrary future domain pack tile (Tile 99)
    #[repr(C, align(8))]
    #[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
    struct MockUnknownRecord {
        data: [u64; 8],
    }
    let unknown_rec = MockUnknownRecord { data: [0x533A; 8] };
    let mut raw_hdr = HyperTileHeader::new(99, 99);
    raw_hdr.record_count = 1;
    raw_hdr.payload_bytes = std::mem::size_of::<MockUnknownRecord>() as u32;
    writer_a
        .write_raw_hyper_tile(&raw_hdr, bytemuck::bytes_of(&unknown_rec))
        .unwrap();
    drop(writer_a);

    // 2. Create Archive B with ADDON_FINTECH (0x0002) containing telemetry
    let mut writer_b = TileWriter::create(input_b.path()).unwrap();
    writer_b.enable_addon(ADDON_FINTECH).unwrap();
    let tel_rec = TelemetryRecord::new(1000, 600, 1, 3.14);
    writer_b
        .write_hyper_tile(TileType::TELEMETRY, &[tel_rec], Some(&[1000]), None)
        .unwrap();
    drop(writer_b);

    // 3. Compact Archive A and Archive B into output archive
    let compacted_tiles =
        Compactor::compact(&[input_a.path(), input_b.path()], out_file.path()).unwrap();
    assert!(compacted_tiles >= 3);

    // 4. Verify Output Archive
    let reader = MmapReader::open(out_file.path()).unwrap();
    assert!(reader.verify_checksums().is_ok());

    // Verify addon_mask is bitwise OR of Archive A (0x0001) and Archive B (0x0002) = 0x0003
    let expected_mask = ADDON_EDU | ADDON_FINTECH;
    assert_eq!(reader.file_header().addon_mask, expected_mask);

    // Verify Scholar Tile (12) survived intact with both active record and tombstone
    let (h12, p12) = reader.get_tile(1).unwrap();
    assert_eq!(h12.tile_type, 12);
    let papers: &[MockPaperRecord] = bytemuck::cast_slice(&p12[..h12.payload_bytes as usize]);
    assert_eq!(papers.len(), 2);
    assert_eq!(papers[0].paper_id, 101);
    assert_eq!(papers[0].flags, 0);
    assert_eq!(papers[1].paper_id, 102);
    assert_eq!(papers[1].flags, 1); // tombstone preserved

    // Verify Unknown Tile (99) survived intact
    let (h99, p99) = reader.get_tile(2).unwrap();
    assert_eq!(h99.tile_type, 99);
    let unknowns: &[MockUnknownRecord] = bytemuck::cast_slice(&p99[..h99.payload_bytes as usize]);
    assert_eq!(unknowns.len(), 1);
    assert_eq!(unknowns[0].data[0], 0x533A);
}

/// Scenario 7: Marks tile (Tile 19) metadata survives compaction:
/// - Target tile IDs are remapped to their new output tile IDs.
/// - Active marks are preserved; cleared marks are dropped/resolved.
/// - QuerySieve hides quarantined records by default, unless opt-in with_hidden_marks(true) is used.
#[test]
fn test_marks_survive_compaction_and_remap() {
    let input_file = TestTempFile::new("marks_compact_input");
    let out_file = TestTempFile::new("marks_compact_output");

    // 1. Create Archive with Telemetry tile (Tile 0) containing 2 records
    let mut writer = TileWriter::create(input_file.path()).unwrap();
    let rec0 = TelemetryRecord::new(1000, 1, 10, 10.0);
    let rec1 = TelemetryRecord::new(1001, 1, 10, 20.0);
    let tel_tid = writer
        .write_hyper_tile(
            TileType::TELEMETRY,
            &[rec0, rec1],
            Some(&[1000, 1001]),
            None,
        )
        .unwrap();

    // 2. Add MARKS tile (Tile 1) with active and cleared marks targeting Tile 0
    let m_quarantine = MarkRecord::new(
        TileType::TELEMETRY,
        tel_tid, // target_tile_id
        0,       // record_index 0
        1,       // generation
        s3a_core::MARK_QUARANTINE,
        s3a_core::MARK_SET,
        1005,
        0xABCD,
    );
    let m_legal_hold = MarkRecord::new(
        TileType::TELEMETRY,
        tel_tid,
        s3a_core::MARK_TARGET_ALL_RECORDS,
        1,
        s3a_core::MARK_LEGAL_HOLD,
        s3a_core::MARK_SET,
        1006,
        0xABCD,
    );
    let m_review_set = MarkRecord::new(
        TileType::TELEMETRY,
        tel_tid,
        1,
        1,
        s3a_core::MARK_NEEDS_REVIEW,
        s3a_core::MARK_SET,
        1007,
        0xABCD,
    );
    let m_review_clear = MarkRecord::new(
        TileType::TELEMETRY,
        tel_tid,
        1,
        1,
        s3a_core::MARK_NEEDS_REVIEW,
        s3a_core::MARK_CLEAR,
        1008,
        0xABCD,
    );

    writer
        .write_hyper_tile(
            TileType::MARKS,
            &[m_quarantine, m_legal_hold, m_review_set, m_review_clear],
            None,
            None,
        )
        .unwrap();
    drop(writer);

    // 3. Compact into output archive (1 telemetry tile + 1 marks tile + 1 schema tile)
    let compacted_tiles = Compactor::compact(&[input_file.path()], out_file.path()).unwrap();
    assert_eq!(compacted_tiles, 3);

    // 4. Verify output archive
    let reader = MmapReader::open(out_file.path()).unwrap();
    assert!(reader.verify_checksums().is_ok());

    let (h0, _) = reader.get_tile(0).unwrap();
    let expected_new_tid = h0.tile_id;

    let mut sieve = QuerySieve::new(&reader);
    let active_marks = sieve.active_marks();
    // 2 active marks survive: quarantine on rec 0, legal hold on all records.
    // The review mark was cleared so it should not be active.
    assert_eq!(active_marks.len(), 2);

    let q_mark = active_marks
        .iter()
        .find(|m| m.mark_kind == s3a_core::MARK_QUARANTINE)
        .unwrap();
    assert_eq!(q_mark.target_tile_id, expected_new_tid); // remapped to new tile ID
    assert_eq!(q_mark.record_index, 0);
    assert_eq!(q_mark.actor_hash, 0xABCD);

    let lh_mark = active_marks
        .iter()
        .find(|m| m.mark_kind == s3a_core::MARK_LEGAL_HOLD)
        .unwrap();
    assert_eq!(lh_mark.target_tile_id, expected_new_tid);
    assert_eq!(lh_mark.record_index, s3a_core::MARK_TARGET_ALL_RECORDS);

    // 5. Query records: by default, record 0 is quarantined, so hidden!
    let default_records = sieve.query_telemetry_with_coords(0, u64::MAX, None, None);
    assert_eq!(default_records.len(), 1);
    assert_eq!(default_records[0].0.timestamp, 1001); // only rec1 returned

    // Opt-in: with_hidden_marks(true) reveals all records
    sieve.set_include_hidden(true);
    let all_records = sieve.query_telemetry_with_coords(0, u64::MAX, None, None);
    assert_eq!(all_records.len(), 2);
}
