//! M1 legacy migration tests. The fixture was written by the real v1.0 `TileWriter` from
//! commit 66f70c4 (2 telemetry tiles x 5 records); see docs/api_changes/2026-10-21-m1-migrate.md.
use s3a_engine::migrate::{migrate_file, LegacyVersion, FAIL_BEFORE_RENAME};
use s3a_engine::*;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());
const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/legacy_v1_0_telemetry.s3a"
);

fn tmpdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("s3a_m1_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn v1_0_fixture_is_rejected_by_current_reader() {
    let r = MmapReader::open(FIXTURE).unwrap();
    assert!(
        r.verify_checksums().is_err(),
        "v1.0 prefix-only tile CRC should fail current verify"
    );
}

#[test]
fn round_trip_v1_0_to_current() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let d = tmpdir("rt");
    let input = d.join("in.s3a");
    std::fs::copy(FIXTURE, &input).unwrap();
    let before = std::fs::read(&input).unwrap();
    let out = d.join("out.s3a");
    let rep = migrate_file(&input, &out, false).unwrap();
    assert_eq!(rep.source_version, LegacyVersion::V1_0);
    assert_eq!((rep.tile_count, rep.record_count), (2, 10));
    assert_eq!(
        std::fs::read(&input).unwrap(),
        before,
        "input must not change"
    );
    let r = MmapReader::open(&out).unwrap();
    r.verify_checksums().unwrap();
    assert_eq!(r.file_header().file_uuid, rep.new_file_uuid);
    for t in 0..2u64 {
        for i in 0..5u64 {
            let rec: TelemetryRecord = r
                .read_record_by_coordinate(&S3ACoordinate::new(0, t as u32, i as u32))
                .unwrap();
            assert_eq!(rec.sensor_id, 100 + i as u32);
            assert_eq!(rec.value, (t * 10 + i) as f64 * 1.5);
        }
    }
    // Fields after the v1.0 216-byte hull were shifted back 8 bytes into place.
    let (h, _) = r.get_tile(0).unwrap();
    assert_eq!(h.encoding.value_scale, 1.0);
    assert!(h.learned_index.slope > 0.0);
    assert_eq!((h.tile_index, h.record_layout_version), (0, 1));
    // Migrating an already-current file is refused.
    assert!(migrate_file(&out, &d.join("again.s3a"), false).is_err());
}

#[test]
fn corrupted_legacy_input_rejected() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let d = tmpdir("corrupt");
    let mut bytes = std::fs::read(FIXTURE).unwrap();
    bytes[FILE_HEADER_SIZE + HYPER_TILE_HEADER_SIZE + 3] ^= 0xFF; // flip a payload byte
    let input = d.join("in.s3a");
    std::fs::write(&input, &bytes).unwrap();
    let out = d.join("out.s3a");
    assert!(migrate_file(&input, &out, false).is_err());
    assert!(!out.exists());
    let mut bytes = std::fs::read(FIXTURE).unwrap();
    bytes.truncate(FILE_HEADER_SIZE + HYPER_TILE_SIZE + 10); // truncated second tile
    std::fs::write(&input, &bytes).unwrap();
    assert!(migrate_file(&input, &out, false).is_err());
    assert!(!out.exists());
    assert!(!d.join("out.s3a.migrate-tmp").exists());
}

#[test]
fn dry_run_writes_nothing() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let d = tmpdir("dry");
    let out = d.join("out.s3a");
    let rep = migrate_file(std::path::Path::new(FIXTURE), &out, true).unwrap();
    assert!(rep.dry_run);
    assert_eq!(std::fs::read_dir(&d).unwrap().count(), 0);
}

#[test]
fn crash_before_rename_leaves_no_partial_output() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let d = tmpdir("crash");
    let out = d.join("out.s3a");
    FAIL_BEFORE_RENAME.store(true, Ordering::SeqCst);
    let res = migrate_file(std::path::Path::new(FIXTURE), &out, false);
    FAIL_BEFORE_RENAME.store(false, Ordering::SeqCst);
    assert!(res.is_err());
    assert_eq!(
        std::fs::read_dir(&d).unwrap().count(),
        0,
        "no output and no temp file"
    );
}

#[test]
fn refuses_existing_output_and_in_place() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let d = tmpdir("exists");
    let input = d.join("in.s3a");
    std::fs::copy(FIXTURE, &input).unwrap();
    assert!(migrate_file(&input, &input, false).is_err());
    std::fs::write(d.join("out.s3a"), b"keep").unwrap();
    assert!(migrate_file(&input, &d.join("out.s3a"), false).is_err());
    assert_eq!(std::fs::read(d.join("out.s3a")).unwrap(), b"keep");
}

#[test]
fn v1_1_unaddressed_is_migrated() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let d = tmpdir("v11");
    let input = d.join("in.s3a");
    {
        let mut w = TileWriter::create_with_uuid(&input, [0, 0]).unwrap();
        let recs = [TelemetryRecord {
            timestamp: 5,
            sensor_id: 1,
            metric_id: 2,
            value: 3.0,
            flags: 0,
        }];
        w.write_hyper_tile(TileType::TELEMETRY, &recs, Some(&[5]), None)
            .unwrap();
    }
    let out = d.join("out.s3a");
    let rep = migrate_file(&input, &out, false).unwrap();
    assert_eq!(rep.source_version, LegacyVersion::V1_1Unaddressed);
    MmapReader::open(&out).unwrap().verify_checksums().unwrap();
}
