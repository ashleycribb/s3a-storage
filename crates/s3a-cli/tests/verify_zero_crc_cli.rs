//! `s3a-cli verify` must reject a non-legacy tile whose header CRC was zeroed (#161 rule),
//! and still reject the raw v1.0 fixture (migrate first).
use s3a_core::{HyperTileHeader, FILE_HEADER_SIZE};
use s3a_engine::{TelemetryRecord, TileType, TileWriter};
use std::io::{Seek, SeekFrom, Write};
use std::process::Command;

#[test]
fn cli_verify_rejects_non_legacy_zero_header_crc() {
    let bin = env!("CARGO_BIN_EXE_s3a-cli");
    let path = std::env::temp_dir().join(format!("s3a_cli_zero_crc_{}.s3a", std::process::id()));
    let _ = std::fs::remove_file(&path);
    {
        let mut w = TileWriter::create(&path).unwrap();
        w.write_hyper_tile(
            TileType::TELEMETRY,
            &[TelemetryRecord::new(100, 1, 1, 10.0)],
            Some(&[100]),
            None,
        )
        .unwrap();
    }
    let ok = Command::new(bin)
        .args(["verify", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        ok.status.success(),
        "{}",
        String::from_utf8_lossy(&ok.stderr)
    );

    let off = FILE_HEADER_SIZE + std::mem::offset_of!(HyperTileHeader, header_crc32);
    let mut f = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    f.seek(SeekFrom::Start(off as u64)).unwrap();
    f.write_all(&[0; 4]).unwrap();
    drop(f);
    let bad = Command::new(bin)
        .args(["verify", path.to_str().unwrap()])
        .output()
        .unwrap();
    let _ = std::fs::remove_file(&path);
    assert!(!bad.status.success(), "zeroed header CRC must fail verify");
    assert!(String::from_utf8_lossy(&bad.stderr).contains("Checksum mismatch"));
}

#[test]
fn cli_verify_still_rejects_raw_v1_0_fixture() {
    let bin = env!("CARGO_BIN_EXE_s3a-cli");
    let f = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../s3a-engine/tests/fixtures/legacy_v1_0_telemetry.s3a"
    );
    let out = Command::new(bin).args(["verify", f]).output().unwrap();
    assert!(!out.status.success());
}
