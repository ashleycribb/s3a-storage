//! A3 regression tests: malformed files must yield None/Err, never panic (ADR-0004).
use s3a_edge::*;
use s3a_engine::*;
use std::path::PathBuf;

fn regression(name: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fuzz/regressions/tile_header")
        .join(name);
    std::fs::read(p).unwrap()
}

/// Exactly what fuzz/fuzz_targets/tile_header.rs does.
fn fuzz_path(bytes: &[u8]) {
    let dir = tempfile_dir();
    let path = dir.join("f.s3a");
    std::fs::write(&path, bytes).unwrap();
    if let Ok(r) = MmapReader::open(&path) {
        let _ = r.verify_checksums();
    }
    let _ = recover_archive(&path);
}

/// Fuzz path plus walking every tile through the Option and try_* accessors.
fn exercise(bytes: &[u8]) {
    let dir = tempfile_dir();
    let path = dir.join("f.s3a");
    std::fs::write(&path, bytes).unwrap();
    if let Ok(r) = MmapReader::open(&path) {
        let _ = r.verify_checksums();
        for i in 0..(r.tile_count() as usize).min(64) + 1 {
            let _ = r.get_promoted_tile(i);
            let _ = r.try_get_promoted_tile(i);
        }
    }
    let _ = recover_archive(&path);
}

fn tempfile_dir() -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let d = std::env::temp_dir().join(format!(
        "s3a_a3_{}_{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn regression_slice_oob_lib861_no_panic() {
    exercise(&regression("slice_oob_lib861.bin"));
}

#[test]
fn regression_add_overflow_lib865_no_panic() {
    fuzz_path(&regression("add_overflow_lib865.bin"));
}

#[test]
fn regression_add_overflow_lib865_walk_tiles_no_panic() {
    exercise(&regression("add_overflow_lib865.bin"));
}

#[test]
fn regression_unaligned_from_bytes_no_panic() {
    exercise(&regression("unaligned_from_bytes.bin"));
}

fn edge_file() -> Vec<u8> {
    let flash = RamFlash::<{ 64 * 1024 }>::new(256, 512);
    let mut w =
        EdgeWriter::<_, 4096>::open_or_format(flash, [9; 16], [1, 2], 1_700_000_000).unwrap();
    for i in 0..5 {
        w.append_sensor_record(&CompactEdgeSensorRecord::new(i, 1, 1, 3000, QUALITY_OK))
            .unwrap();
    }
    w.seal_tile(1_700_000_010).unwrap();
    let n = w.tile_count() as usize;
    w.storage().as_slice()[..512 + n * 4096].to_vec()
}

#[test]
fn edge_crc_corruption_detected() {
    let bytes = edge_file();
    let dir = tempfile_dir();
    let path = dir.join("e.s3a");
    std::fs::write(&path, &bytes).unwrap();
    let r = MmapReader::open(&path).unwrap();
    let last = r.tile_count() as usize - 1;
    let (h, _) = r.try_get_edge_tile(last).expect("clean tile reads");
    assert_ne!(h.flags & TILE_FLAG_CRC_CHAIN, 0);
    let off = r.header_extent() + last * r.tile_stride();
    drop(r);

    // Flip a payload byte -> data CRC mismatch.
    let mut d = bytes.clone();
    d[off + COMMON_TILE_HEADER_SIZE] ^= 0xFF;
    std::fs::write(&path, &d).unwrap();
    let r = MmapReader::open(&path).unwrap();
    let e = r.try_get_edge_tile(last).unwrap_err();
    assert!(
        matches!(e.reason, CorruptReason::DataCrcMismatch { .. }),
        "{e}"
    );
    assert!(r.get_edge_tile(last).is_none());
    drop(r);

    // Flip a header byte (record_count region) -> header CRC mismatch.
    let mut h = bytes.clone();
    h[off + 8] ^= 0x01;
    std::fs::write(&path, &h).unwrap();
    let r = MmapReader::open(&path).unwrap();
    assert!(r.try_get_edge_tile(last).is_err());
}

#[test]
fn truncated_file_rejected_by_open() {
    let dir = tempfile_dir();
    let path = dir.join("t.s3a");
    {
        let mut w = TileWriter::create(&path).unwrap();
        let h = HyperTileHeader::new(1, 1);
        w.write_raw_hyper_tile(&h, &[7u8; 32]).unwrap();
    }
    let full = std::fs::read(&path).unwrap();
    let r = MmapReader::open(&path).unwrap();
    assert!(r.tile_count() >= 1);
    drop(r);
    // Cut off the last byte of the final tile: header now claims more than the file holds.
    std::fs::write(&path, &full[..full.len() - 1]).unwrap();
    // Opens (leading tiles stay readable), but the size mismatch is reported.
    let r = MmapReader::open(&path).unwrap();
    let last = r.tile_count() as usize - 1;
    let e = r.validate_file_size().unwrap_err();
    assert!(matches!(e.reason, CorruptReason::SlotPastEof { .. }), "{e}");
    assert!(r.get_tile(last).is_none());
    assert!(matches!(
        r.try_get_tile(last).unwrap_err().reason,
        CorruptReason::SlotPastEof { .. }
    ));
    assert_eq!(r.verify_checksums(), Err(S3AError::OutOfBounds));
}

fn open_edge(bytes: &[u8]) -> (MmapReader, usize, usize) {
    let dir = tempfile_dir();
    let path = dir.join("e.s3a");
    std::fs::write(&path, bytes).unwrap();
    let r = MmapReader::open(&path).unwrap();
    let last = r.tile_count() as usize - 1;
    let off = r.header_extent() + last * r.tile_stride();
    (r, last, off)
}

#[test]
fn edge_tile_valid_reads_with_matching_crcs() {
    let bytes = edge_file();
    let (r, last, _) = open_edge(&bytes);
    let (h, payload) = r.try_get_edge_tile(last).expect("valid tile reads");
    assert_eq!(crc32c::crc32c(payload), h.data_crc32);
    assert!(r.get_edge_tile(last).is_some());
}

#[test]
fn edge_tile_one_flipped_payload_byte_is_typed_data_crc_error() {
    let bytes = edge_file();
    let (_, last, off) = open_edge(&bytes);
    let mut d = bytes.clone();
    d[off + COMMON_TILE_HEADER_SIZE] ^= 0x01;
    let (r, last2, _) = open_edge(&d);
    assert_eq!(last, last2);
    let e = r.try_get_edge_tile(last).unwrap_err();
    assert_eq!(e.tile, last);
    assert!(
        matches!(e.reason, CorruptReason::DataCrcMismatch { .. }),
        "{e}"
    );
    assert!(r.get_edge_tile(last).is_none());
    assert!(r.try_get_promoted_tile(last).is_err());
}

/// Clearing TILE_FLAG_CRC_CHAIN/SEALED must not switch verification off.
#[test]
fn edge_tile_cleared_crc_flag_does_not_bypass_verification() {
    let bytes = edge_file();
    let (_, last, off) = open_edge(&bytes);

    // Flags cleared, payload byte flipped, header CRC left stale -> header CRC mismatch.
    let mut d = bytes.clone();
    d[off + 4] = 0;
    d[off + 5] = 0;
    d[off + COMMON_TILE_HEADER_SIZE] ^= 0x01;
    let (r, _, _) = open_edge(&d);
    assert!(r.try_get_edge_tile(last).is_err());
    assert!(r.get_edge_tile(last).is_none());

    // Attacker also recomputes the header CRC: the non-zero data CRC still catches the flip.
    let mut d2 = d.clone();
    let hdr: CommonTileHeader =
        bytemuck::pod_read_unaligned(&d2[off..off + COMMON_TILE_HEADER_SIZE]);
    let fixed = compute_common_tile_header_crc32(&hdr);
    d2[off + 56..off + 60].copy_from_slice(&fixed.to_le_bytes());
    let (r, _, _) = open_edge(&d2);
    let e = r.try_get_edge_tile(last).unwrap_err();
    assert!(
        matches!(e.reason, CorruptReason::DataCrcMismatch { .. }),
        "{e}"
    );
}
