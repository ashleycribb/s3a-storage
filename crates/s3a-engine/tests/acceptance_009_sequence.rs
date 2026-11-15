//! Acceptance 009 follow-up (7): the real writers assign strictly increasing sequence numbers,
//! across reopen, and only valid tiles count toward the next number.
//!
//! Written before the fix. Do not edit.

use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;

use s3a_core::{
    HyperTileHeader, TelemetryRecord, TileType, FILE_HEADER_SIZE, HYPER_TILE_HEADER_SIZE,
    HYPER_TILE_SIZE,
};
use s3a_engine::{MmapReader, TileWriter};

struct Tmp(PathBuf);
impl Tmp {
    fn new(tag: &str) -> Self {
        Tmp(std::env::temp_dir().join(format!("acc009_seq_{}_{}.s3a", tag, std::process::id())))
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn rec(i: u64) -> [TelemetryRecord; 1] {
    [TelemetryRecord::new(1_700_000_000_000 + i, 1, 1, i as f64)]
}

fn raw(w: &mut TileWriter, i: u8) {
    let payload = vec![i; 32];
    let mut h = HyperTileHeader::new(0, TileType::TELEMETRY);
    h.payload_bytes = payload.len() as u32;
    w.write_raw_hyper_tile(&h, &payload).unwrap();
}

/// (sequence_number, header CRC ok) for every committed tile, read by the real reader.
fn seqs(path: &PathBuf) -> Vec<u32> {
    let r = MmapReader::open(path).unwrap();
    (0..r.tile_count() as usize)
        .map(|i| {
            r.try_get_tile(i)
                .map(|(h, _)| h.sequence_number)
                .unwrap_or_else(|e| panic!("tile {i} invalid: {e:?}"))
        })
        .collect()
}

#[test]
fn writer_sequence_is_strictly_increasing_across_reopen() {
    let t = Tmp::new("reopen");
    {
        let mut w = TileWriter::create(&t.0).unwrap();
        for i in 0..3 {
            w.write_hyper_tile(TileType::TELEMETRY, &rec(i), None, None)
                .unwrap();
        }
    }
    {
        let mut w = TileWriter::open_append(&t.0).unwrap();
        w.write_hyper_tile(TileType::TELEMETRY, &rec(3), None, None)
            .unwrap();
        raw(&mut w, 4);
    }
    {
        let mut w = TileWriter::open_append(&t.0).unwrap();
        raw(&mut w, 5);
    }
    assert_eq!(
        seqs(&t.0),
        vec![1, 2, 3, 4, 5, 6],
        "every write gets the next sequence number, across two reopens"
    );
}

#[test]
fn append_tile_to_buffer_continues_writer_sequence() {
    let t = Tmp::new("buf");
    {
        let mut w = TileWriter::create(&t.0).unwrap();
        w.write_hyper_tile(TileType::TELEMETRY, &rec(0), None, None)
            .unwrap();
        w.write_hyper_tile(TileType::TELEMETRY, &rec(1), None, None)
            .unwrap();
    }
    let b = s3a_engine::append_tile_to_buffer(
        &fs::read(&t.0).unwrap(),
        TileType::TELEMETRY,
        1,
        &[7u8; 32],
    )
    .unwrap();
    fs::write(&t.0, &b).unwrap();
    {
        let mut w = TileWriter::open_append(&t.0).unwrap();
        raw(&mut w, 8);
    }
    assert_eq!(seqs(&t.0), vec![1, 2, 3, 4]);
}

/// Sets tile `idx`'s sequence_number; if `resign` the header CRC is recomputed (tile stays valid).
fn poke_seq(path: &PathBuf, idx: usize, seq: u32, resign: bool) {
    let mut b = fs::read(path).unwrap();
    let off = FILE_HEADER_SIZE + idx * HYPER_TILE_SIZE;
    let mut h: HyperTileHeader =
        bytemuck::pod_read_unaligned(&b[off..off + HYPER_TILE_HEADER_SIZE]);
    h.sequence_number = seq;
    if resign {
        h.header_crc32 = s3a_engine::compute_tile_header_crc32(&h);
    }
    b[off..off + HYPER_TILE_HEADER_SIZE].copy_from_slice(bytemuck::bytes_of(&h));
    fs::write(path, &b).unwrap();
}

#[test]
fn corrupt_tile_header_does_not_drive_the_next_sequence_number() {
    let t = Tmp::new("corrupt");
    {
        let mut w = TileWriter::create(&t.0).unwrap();
        for i in 0..3 {
            w.write_hyper_tile(TileType::TELEMETRY, &rec(i), None, None)
                .unwrap();
        }
    }
    // Tile 1's header is damaged (sequence bytes changed, CRC no longer matches).
    poke_seq(&t.0, 1, u32::MAX, false);
    let r = catch_unwind(AssertUnwindSafe(|| {
        let mut w = TileWriter::open_append(&t.0).unwrap();
        raw(&mut w, 9);
    }));
    assert!(
        r.is_ok(),
        "reopening a file with one corrupt tile header must not panic"
    );
    let reader = MmapReader::open(&t.0).unwrap();
    let last = reader
        .try_get_tile(3)
        .expect("new tile valid")
        .0
        .sequence_number;
    assert_eq!(
        last, 4,
        "next sequence = max over VALID tiles (3) + 1; a corrupt header must be ignored"
    );
}

#[test]
fn exhausted_sequence_is_an_error_not_a_wrap_or_panic() {
    let t = Tmp::new("exhaust");
    {
        let mut w = TileWriter::create(&t.0).unwrap();
        w.write_hyper_tile(TileType::TELEMETRY, &rec(0), None, None)
            .unwrap();
    }
    poke_seq(&t.0, 0, u32::MAX, true); // valid tile already at the last sequence number
    let r = catch_unwind(AssertUnwindSafe(|| {
        let opened = TileWriter::open_append(&t.0);
        match opened {
            Err(e) => Err(e),
            Ok(mut w) => {
                let mut h = HyperTileHeader::new(0, TileType::TELEMETRY);
                h.payload_bytes = 4;
                w.write_raw_hyper_tile(&h, &[1, 2, 3, 4]).map(|_| ())
            }
        }
    }));
    let r = r.unwrap_or_else(|_| panic!("sequence exhaustion must return io::Error, not panic"));
    assert!(
        r.is_err(),
        "writing past u32::MAX must fail, never wrap to 0"
    );
    let b = catch_unwind(|| {
        s3a_engine::append_tile_to_buffer(
            &fs::read(&t.0).unwrap(),
            TileType::TELEMETRY,
            1,
            &[0u8; 8],
        )
    });
    let b = b.unwrap_or_else(|_| panic!("append_tile_to_buffer must not panic on exhaustion"));
    assert!(b.is_err(), "append_tile_to_buffer past u32::MAX must fail");
}
