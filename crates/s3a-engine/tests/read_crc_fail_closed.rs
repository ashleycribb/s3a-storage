//! Fail-closed tile/header CRCs on read (durability audit fix #2).
//!
//! A zero CRC is corruption unless the tile or header is explicitly legacy:
//! - HyperTile: legacy iff `record_layout_version == 0` (same rule as the compaction gate).
//! - FileHeader: the unhashed single-slot fallback applies only when slot B was never written.

use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::mem::offset_of;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use s3a_core::{FileHeader, HyperTileHeader, FILE_HEADER_SIZE, HYPER_TILE_HEADER_SIZE};
use s3a_engine::{
    compute_header_crc32, resolve_active_header, MmapReader, S3AError, TelemetryRecord, TileType,
    TileWriter,
};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Tmp(PathBuf);
impl Tmp {
    fn new(tag: &str) -> Self {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut p = std::env::temp_dir();
        p.push(format!(
            "s3a_crc_fc_{}_{}_{}.s3a",
            tag,
            std::process::id(),
            id
        ));
        Tmp(p)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn write_one_tile(path: &Path) {
    let mut w = TileWriter::create(path).unwrap();
    let rec = TelemetryRecord::new(100, 1, 1, 10.0);
    w.write_hyper_tile(TileType::TELEMETRY, &[rec], Some(&[100]), None)
        .unwrap();
}

fn patch(path: &Path, offset: usize, bytes: &[u8]) {
    let mut f = OpenOptions::new().write(true).open(path).unwrap();
    f.seek(SeekFrom::Start(offset as u64)).unwrap();
    f.write_all(bytes).unwrap();
    f.sync_all().unwrap();
}

fn xor_byte(path: &Path, offset: usize, mask: u8) {
    let mut data = fs::read(path).unwrap();
    data[offset] ^= mask;
    fs::write(path, data).unwrap();
}

const TILE0: usize = FILE_HEADER_SIZE;
const HDR_CRC: usize = TILE0 + offset_of!(HyperTileHeader, header_crc32);
const LAYOUT_VER: usize = TILE0 + offset_of!(HyperTileHeader, record_layout_version);

#[test]
fn baseline_valid_archive_verifies() {
    let t = Tmp::new("ok");
    write_one_tile(t.path());
    MmapReader::open(t.path())
        .unwrap()
        .verify_checksums()
        .unwrap();
}

#[test]
fn zero_header_crc_on_non_legacy_tile_rejected_on_read() {
    let t = Tmp::new("zero_hdr");
    write_one_tile(t.path());
    patch(t.path(), HDR_CRC, &[0; 4]);
    let r = MmapReader::open(t.path()).unwrap();
    assert!(r.get_tile(0).unwrap().0.record_layout_version != 0);
    assert!(matches!(
        r.verify_checksums(),
        Err(S3AError::ChecksumMismatch { expected: 0, .. })
    ));
}

#[test]
fn zero_header_crc_on_legacy_tile_still_reads() {
    let t = Tmp::new("legacy");
    write_one_tile(t.path());
    patch(t.path(), LAYOUT_VER, &0u32.to_le_bytes());
    patch(t.path(), HDR_CRC, &[0; 4]);
    let r = MmapReader::open(t.path()).unwrap();
    r.verify_checksums().unwrap();
}

#[test]
fn flipped_bit_in_tile_header_rejected() {
    let t = Tmp::new("flip_hdr");
    write_one_tile(t.path());
    xor_byte(t.path(), LAYOUT_VER, 0x04);
    assert!(MmapReader::open(t.path())
        .unwrap()
        .verify_checksums()
        .is_err());
}

#[test]
fn flipped_bit_in_tile_payload_rejected() {
    let t = Tmp::new("flip_payload");
    write_one_tile(t.path());
    xor_byte(t.path(), TILE0 + HYPER_TILE_HEADER_SIZE, 0x01);
    assert!(MmapReader::open(t.path())
        .unwrap()
        .verify_checksums()
        .is_err());
}

#[test]
fn legacy_unhashed_single_slot_header_accepted() {
    let a = FileHeader::new(0, 1);
    assert_eq!(a.header_crc32, 0);
    let b: FileHeader = bytemuck::Zeroable::zeroed();
    let (h, slot) = resolve_active_header(&a, &b).unwrap();
    assert_eq!((h.header_crc32, slot), (0, 0));
}

#[test]
fn zero_crc_slot_a_rejected_when_slot_b_was_written() {
    let a = FileHeader::new(0, 1); // CRC zeroed
    let mut b = FileHeader::new(0, 1);
    b.generation = 2;
    b.header_crc32 = compute_header_crc32(&b) ^ 1; // written but corrupt
    assert!(matches!(
        resolve_active_header(&a, &b),
        Err(S3AError::CorruptedHeader)
    ));
}
