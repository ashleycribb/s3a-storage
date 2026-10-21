//! B1: self-describing catalog (Schema tile 77) reader contract.

use std::path::PathBuf;

use s3a_core::schema::{encode_schema_payload, SchemaFieldDefinition, SchemaTypeDefinition};
use s3a_core::{
    HyperTileHeader, TileType, FILE_HEADER_SIZE, HYPER_TILE_HEADER_SIZE, HYPER_TILE_SIZE,
};
use s3a_engine::schema::{catalog_mismatches, registry_catalog, CatalogError, CatalogSource};
use s3a_engine::{append_schema_tile_if_missing, MmapReader, TileWriter};

fn tmp(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("s3a_b1_{}_{}.s3a", name, std::process::id()));
    let _ = std::fs::remove_file(&p);
    p
}

fn write_telemetry_file(p: &PathBuf) {
    let mut w = TileWriter::create(p).unwrap();
    let mut h = HyperTileHeader::new(0, TileType::TELEMETRY);
    h.record_count = 1;
    w.write_raw_hyper_tile(&h, &[7u8; 32]).unwrap();
}

#[test]
fn b1_roundtrip_embedded_catalog() {
    let p = tmp("roundtrip");
    write_telemetry_file(&p);
    assert!(append_schema_tile_if_missing(&p).unwrap());
    let r = MmapReader::open(&p).unwrap();
    r.verify_checksums().unwrap();
    let cat = r.catalog().unwrap();
    assert_eq!(cat.source, CatalogSource::Embedded { tile_index: 1 });
    let t = cat.get(TileType::TELEMETRY).expect("telemetry in catalog");
    assert_eq!(t.name, "telemetry");
    assert!(!t.fields.is_empty());
    assert!(
        cat.get(TileType::SCHEMA).is_some(),
        "catalog describes itself"
    );
    let _ = std::fs::remove_file(&p);
}

#[test]
fn b1_file_without_schema_tile_falls_back_to_registry() {
    let p = tmp("nofallback");
    write_telemetry_file(&p);
    let r = MmapReader::open(&p).unwrap();
    let cat = r.catalog().unwrap();
    assert_eq!(cat.source, CatalogSource::Registry);
    assert_eq!(cat, registry_catalog());
    assert_eq!(cat.get(TileType::TELEMETRY).unwrap().name, "telemetry");
    // Old API unchanged.
    assert!(r.get_schema_definitions().is_empty());
    let _ = std::fs::remove_file(&p);
}

fn schema_payload_offset(tile_index: usize) -> usize {
    FILE_HEADER_SIZE + tile_index * HYPER_TILE_SIZE + HYPER_TILE_HEADER_SIZE
}

#[test]
fn b1_corrupt_schema_tile_rejected_by_tile_crc() {
    let p = tmp("corrupt_tile");
    write_telemetry_file(&p);
    append_schema_tile_if_missing(&p).unwrap();
    let mut bytes = std::fs::read(&p).unwrap();
    bytes[schema_payload_offset(1) + 40] ^= 0xFF; // inside first type record
    std::fs::write(&p, &bytes).unwrap();
    let r = MmapReader::open(&p).unwrap();
    match r.catalog() {
        Err(CatalogError::Tile(c)) => assert_eq!(c.tile, 1),
        other => panic!("expected tile CRC error, got {:?}", other),
    }
    let _ = std::fs::remove_file(&p);
}

#[test]
fn b1_corrupt_schema_payload_rejected_by_schema_crc() {
    // Tile CRC recomputed over a tampered payload: schema_crc32 must still catch it.
    let p = tmp("corrupt_payload");
    let defs = vec![SchemaTypeDefinition {
        type_id: 70_000,
        type_version: 1,
        name: "vendor_x".into(),
        owning_pack: "vendor".into(),
        record_size: 8,
        fields: vec![SchemaFieldDefinition {
            field_id: 1,
            field_type: 4,
            byte_offset: 0,
            byte_len: 8,
            null_allowed: false,
            name: "id".into(),
        }],
    }];
    let mut payload = encode_schema_payload(&defs);
    payload[40] ^= 0x01;
    let mut w = TileWriter::create(&p).unwrap();
    let mut h = HyperTileHeader::new(0, TileType::SCHEMA);
    h.record_layout_version = 1;
    w.write_raw_hyper_tile(&h, &payload).unwrap();
    drop(w);
    let r = MmapReader::open(&p).unwrap();
    r.verify_checksums().unwrap(); // tile-level CRC is consistent
    match r.catalog() {
        Err(CatalogError::Payload {
            tile_index: 0,
            error,
        }) => {
            assert!(
                matches!(error, s3a_core::S3AError::ChecksumMismatch { .. }),
                "{:?}",
                error
            )
        }
        other => panic!("expected schema_crc32 error, got {:?}", other),
    }
    let _ = std::fs::remove_file(&p);
}

#[test]
fn b1_decoder_never_panics_on_garbage_or_misalignment() {
    let defs = registry_catalog().types;
    let good = encode_schema_payload(&defs);
    // Misaligned copy decodes identically.
    let mut buf = vec![0u8; good.len() + 1];
    buf[1..].copy_from_slice(&good);
    assert_eq!(
        s3a_core::schema::decode_schema_payload(&buf[1..]).unwrap(),
        defs
    );
    // Truncations and bit flips return Err or Ok, never panic.
    for cut in 0..good.len().min(512) {
        let _ = s3a_core::schema::decode_schema_payload(&good[..cut]);
    }
    for i in 0..good.len().min(512) {
        let mut b = good.clone();
        b[i] ^= 0xA5;
        let _ = s3a_core::schema::decode_schema_payload(&b);
    }
}

#[test]
fn b1_embedded_catalog_matches_registry() {
    let p = tmp("matches");
    write_telemetry_file(&p);
    append_schema_tile_if_missing(&p).unwrap();
    let r = MmapReader::open(&p).unwrap();
    let cat = r.catalog().unwrap();
    assert!(
        catalog_mismatches(&cat).is_empty(),
        "{:?}",
        catalog_mismatches(&cat)
    );
    // Registry catalog is self-consistent and names every registered type.
    let reg = registry_catalog();
    assert!(catalog_mismatches(&reg).is_empty());
    let g = s3a_core::registry::global_registry();
    assert_eq!(reg.types.len(), g.descriptors().len());
    for d in g.descriptors() {
        assert_eq!(reg.get(d.tile_type).unwrap().name, d.name);
    }
    let _ = std::fs::remove_file(&p);
}

#[test]
fn b1_mismatch_detected() {
    let mut cat = registry_catalog();
    let t = cat
        .types
        .iter_mut()
        .find(|t| t.type_id == TileType::TELEMETRY)
        .unwrap();
    t.record_size += 1;
    let m = catalog_mismatches(&cat);
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].1, "record_size");
}
