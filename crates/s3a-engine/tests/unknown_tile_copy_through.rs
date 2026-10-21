//! S3A-TILE-SPEC §7.2: unknown tile types WITHOUT `COPY_THROUGH_COMPACT` survive
//! compaction, tile fusion byte-for-byte (payload).

use std::path::PathBuf;

use s3a_core::{HyperTileHeader, TileType};
use s3a_engine::{Compactor, MmapReader, TileFusionEngine, TileWriter};

const UNKNOWN: u32 = 9999; // not registered; carries no flags at all

fn tmp(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("s3a_unk_{}_{}.s3a", name, std::process::id()));
    let _ = std::fs::remove_file(&p);
    p
}

fn payload() -> Vec<u8> {
    (0..1000u32).map(|i| (i * 31 % 251) as u8).collect()
}

fn write_input(p: &PathBuf) {
    assert!(s3a_core::registry::global_registry().get(UNKNOWN).is_none());
    let mut w = TileWriter::create(p).unwrap();
    let mut t = HyperTileHeader::new(0, TileType::TELEMETRY);
    t.record_count = 1;
    w.write_raw_hyper_tile(&t, &[7u8; 32]).unwrap();
    let mut u = HyperTileHeader::new(0, UNKNOWN);
    u.record_count = 0;
    w.write_raw_hyper_tile(&u, &payload()).unwrap();
}

fn assert_unknown_survives(out: &PathBuf) {
    let r = MmapReader::open(out).unwrap();
    r.verify_checksums().unwrap();
    let found: Vec<_> = (0..r.tile_count() as usize)
        .filter_map(|i| r.get_tile(i))
        .filter(|(h, _)| h.tile_type == UNKNOWN)
        .collect();
    assert_eq!(found.len(), 1, "unknown tile must be copied through");
    assert_eq!(found[0].1, &payload()[..], "payload must be byte-identical");
}

#[test]
fn unflagged_unknown_tile_survives_compaction() {
    let (i, o) = (tmp("c_in"), tmp("c_out"));
    write_input(&i);
    Compactor::compact(&[&i], &o).unwrap();
    assert_unknown_survives(&o);
}

#[test]
fn unflagged_unknown_tile_survives_tile_fusion() {
    let (i, o) = (tmp("f_in"), tmp("f_out"));
    write_input(&i);
    TileFusionEngine::fuse_telemetry_tiles(&[&i], &o).unwrap();
    assert_unknown_survives(&o);
}

#[test]
fn known_non_telemetry_tile_survives_tile_fusion() {
    let (i, o) = (tmp("k_in"), tmp("k_out"));
    write_input(&i);
    assert!(s3a_engine::append_schema_tile_if_missing(&i).unwrap());
    let src = MmapReader::open(&i).unwrap();
    let schema: Vec<u8> = (0..src.tile_count() as usize)
        .filter_map(|k| src.get_tile(k))
        .find(|(h, _)| h.tile_type == TileType::SCHEMA)
        .map(|(_, p)| p.to_vec())
        .expect("schema tile in input");
    drop(src);
    TileFusionEngine::fuse_telemetry_tiles(&[&i], &o).unwrap();
    let r = MmapReader::open(&o).unwrap();
    r.verify_checksums().unwrap();
    let out: Vec<_> = (0..r.tile_count() as usize)
        .filter_map(|k| r.get_tile(k))
        .filter(|(h, _)| h.tile_type == TileType::SCHEMA)
        .collect();
    assert_eq!(out.len(), 1, "known schema tile must be carried through");
    assert_eq!(
        out[0].1,
        &schema[..],
        "schema payload must be byte-identical"
    );
    assert!(r.catalog().is_ok());
}
