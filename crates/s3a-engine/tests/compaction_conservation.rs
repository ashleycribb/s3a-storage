//! TILE-SPEC §6.1.7 conservation: compaction must never output fewer live records than it read.

use s3a_core::{TelemetryRecord, TileType};
use s3a_engine::{Compactor, MmapReader, TileWriter};
use std::path::PathBuf;

fn tmp(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("s3a_cons_{name}_{}.s3a", std::process::id()));
    let _ = std::fs::remove_file(&p);
    p
}

fn live_telemetry(p: &PathBuf) -> Vec<TelemetryRecord> {
    let r = MmapReader::open(p).unwrap();
    let mut v = Vec::new();
    for i in 0..r.tile_count() as usize {
        let (h, pl) = r.get_tile(i).unwrap();
        if h.tile_type == TileType::TELEMETRY {
            let recs: &[TelemetryRecord] = bytemuck::cast_slice(pl);
            v.extend(recs.iter().filter(|r| !r.is_tombstone()).copied());
        }
    }
    v
}

#[test]
fn same_key_different_values_are_both_kept() {
    let input = tmp("dupkey_in");
    let mut w = TileWriter::create(&input).unwrap();
    let a = TelemetryRecord::new(100, 1, 1, 1.5);
    let b = TelemetryRecord::new(100, 1, 1, 2.5); // same (sensor, metric, ts), different value
    w.write_hyper_tile(TileType::TELEMETRY, &[a, b], None, None)
        .unwrap();
    drop(w);
    assert_eq!(live_telemetry(&input).len(), 2);
    let out = tmp("dupkey_out");
    Compactor::compact(&[input], out.clone()).unwrap();
    let got = live_telemetry(&out);
    assert_eq!(got.len(), 2, "distinct live records were dropped: {got:?}");
}

#[test]
fn tombstone_deletes_earlier_same_key_record_only() {
    let input = tmp("tomb_in");
    let mut w = TileWriter::create(&input).unwrap();
    let a = TelemetryRecord::new(100, 1, 1, 1.0);
    let mut t = TelemetryRecord::new(100, 1, 1, 0.0);
    t.mark_tombstone();
    let other = TelemetryRecord::new(200, 1, 1, 3.0);
    w.write_hyper_tile(TileType::TELEMETRY, &[a, t, other], None, None)
        .unwrap();
    drop(w);
    let out = tmp("tomb_out");
    Compactor::compact(&[input], out.clone()).unwrap();
    assert_eq!(live_telemetry(&out), vec![other]);
}

#[test]
fn all_tombstoned_input_compacts_to_zero_live_records() {
    // conformance valid_schema_catalog: its only telemetry record has flags 0x1111...,
    // so FLAG_TOMBSTONE (bit 0) is set. Zero live records in, zero out: not data loss.
    let src = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../conformance/vectors-1.1/files/valid_schema_catalog.s3a"
    ));
    let r = MmapReader::open(&src).unwrap();
    assert_eq!(s3a_engine::count_live_records(&r), 0);
    drop(r);
    let out = tmp("catalog_out");
    Compactor::compact(&[src], out.clone()).unwrap();
    assert_eq!(
        s3a_engine::count_live_records(&MmapReader::open(&out).unwrap()),
        0
    );
}

#[test]
fn conservation_gate_refuses_fewer_records() {
    assert!(s3a_engine::conservation_check(3, 3, 0).is_ok());
    assert!(s3a_engine::conservation_check(2, 3, 1).is_ok());
    let e = s3a_engine::conservation_check(1, 3, 0).unwrap_err();
    assert!(e.to_string().contains("Compaction refused"));
}

#[test]
fn sealed_file_with_manifest_compacts_and_drops_manifest() {
    let input = tmp("sealed_src");
    let mut w = TileWriter::create(&input).unwrap();
    let recs = [
        TelemetryRecord::new(1, 1, 1, 1.0),
        TelemetryRecord::new(2, 1, 1, 2.0),
    ];
    w.write_hyper_tile(TileType::TELEMETRY, &recs, None, None)
        .unwrap();
    drop(w);
    let sealed = s3a_engine::seal_buffer_v2(
        &std::fs::read(&input).unwrap(),
        &s3a_engine::SealOptionsV2::default()
            .with_manifest()
            .with_embed_spec()
            .with_record_roots(),
    )
    .unwrap();
    let sp = tmp("sealed");
    std::fs::write(&sp, &sealed).unwrap();
    // The sealer now gives its own tiles unique tile_ids.
    let r = MmapReader::open(&sp).unwrap();
    let mut ids: Vec<u64> = (0..r.tile_count() as usize)
        .map(|i| r.get_tile(i).unwrap().0.tile_id)
        .collect();
    let n = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), n, "duplicate tile_ids in sealed file");
    drop(r);
    let out = tmp("sealed_out");
    Compactor::compact(&[sp.clone()], out.clone()).unwrap();
    assert!(!PathBuf::from(format!("{}.quarantine", sp.display())).exists());
    assert_eq!(live_telemetry(&out), recs.to_vec());
    let r = MmapReader::open(&out).unwrap();
    let types: Vec<u32> = (0..r.tile_count() as usize)
        .map(|i| r.get_tile(i).unwrap().0.tile_type)
        .collect();
    assert!(!types.contains(&TileType::MANIFEST), "{types:?}");
    assert!(!types.contains(&TileType::RECORD_ROOT));
    assert!(
        types.contains(&TileType::SPECIFICATION),
        "content tile kept"
    );
}

#[test]
fn old_sealed_file_with_duplicate_tile_ids_still_compacts() {
    // Sealers before this fix wrote manifest/record-root tiles with tile_id 0 and the
    // spec tile with id 1. The vectors-1.2 file was written that way.
    let src = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../conformance/vectors-1.2/files/about_sealed_self_describing.s3a"
    ));
    let copy = tmp("old_sealed");
    std::fs::copy(&src, &copy).unwrap();
    let before = s3a_engine::count_live_records(&MmapReader::open(&copy).unwrap());
    let out = tmp("old_sealed_out");
    Compactor::compact(&[copy], out.clone()).unwrap();
    assert!(s3a_engine::count_live_records(&MmapReader::open(&out).unwrap()) >= before);
}

#[test]
fn truncated_and_edge_inputs_are_refused_not_emptied() {
    for name in [
        "invalid_truncated_tile",
        "sealed_truncated_mid_tile",
        "micro_valid_4kb",
        "micro_truncated_file",
    ] {
        let src = PathBuf::from(format!(
            "{}/../../conformance/vectors-1.1/files/{name}.s3a",
            env!("CARGO_MANIFEST_DIR")
        ));
        let copy = tmp(name);
        std::fs::copy(&src, &copy).unwrap();
        let out = tmp(&format!("{name}_out"));
        let r = Compactor::compact(&[copy.clone()], out.clone());
        if let Ok(n) = r {
            panic!("{name}: compaction wrote {n} tiles instead of refusing");
        }
        assert!(!out.exists(), "{name}: output left behind");
        assert!(
            !PathBuf::from(format!("{}.quarantine", copy.display())).exists(),
            "{name}: a refusal must not quarantine"
        );
    }
}
