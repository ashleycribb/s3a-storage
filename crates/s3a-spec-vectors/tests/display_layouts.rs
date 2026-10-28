//! Display layouts are metadata only: they must match the real structs, resolve every record in
//! the committed vectors, and must not leak into the sealing path.

use std::path::PathBuf;

use s3a_core::display_layout::{
    core_display_layouts, ft, validate_layout, DisplayLayoutError, DisplayLayoutRegistry,
};
use s3a_core::lrs::StatementIndexRecord;
use s3a_core::schema::{SchemaFieldDefinition, SchemaTypeDefinition};
use s3a_core::TileType;
use s3a_engine::display_layouts::{
    edge_display_layouts, edge_display_registry, engine_display_layouts, engine_display_registry,
};
use s3a_engine::record_roots::RecordRootEntry;
use s3a_engine::MmapReader;

fn field<'a>(l: &'a SchemaTypeDefinition, name: &str) -> &'a SchemaFieldDefinition {
    l.fields
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("no field {name}"))
}
fn raw<'a>(rec: &'a [u8], l: &SchemaTypeDefinition, name: &str) -> &'a [u8] {
    let f = field(l, name);
    &rec[f.byte_offset as usize..(f.byte_offset + f.byte_len) as usize]
}
fn uint(rec: &[u8], l: &SchemaTypeDefinition, name: &str) -> u64 {
    let mut b = [0u8; 8];
    let r = raw(rec, l, name);
    b[..r.len()].copy_from_slice(r);
    u64::from_le_bytes(b)
}

fn all_registries() -> Vec<(&'static str, Vec<SchemaTypeDefinition>)> {
    vec![
        ("engine", engine_display_layouts()),
        ("edge", edge_display_layouts()),
        ("vault", s3a_vault::display_layouts()),
        ("scholar", s3a_pack_scholar::display_layouts()),
    ]
}

#[test]
fn every_builtin_layout_is_valid_and_registers() {
    for (set, layouts) in all_registries() {
        let reg = DisplayLayoutRegistry::from_layouts(layouts.clone())
            .unwrap_or_else(|e| panic!("{set}: {e}"));
        assert_eq!(reg.layouts().len(), layouts.len());
        for l in &layouts {
            let ids: Vec<u16> = l.fields.iter().map(|f| f.field_id).collect();
            assert_eq!(
                ids,
                (1..=l.fields.len() as u16).collect::<Vec<_>>(),
                "{set}/{}",
                l.name
            );
            assert!(l.fields.iter().all(|f| !f.null_allowed), "{set}/{}", l.name);
        }
    }
    assert_eq!(core_display_layouts().len(), 1);
}

#[test]
fn layouts_cover_the_requested_record_types() {
    let e = engine_display_registry();
    assert_eq!(
        e.find(TileType::STATEMENT_INDEX, None, Some(64))
            .unwrap()
            .name,
        "statement_index"
    );
    assert_eq!(
        e.find(TileType::RECORD_ROOT, None, Some(64)).unwrap().name,
        "record_root"
    );
    let s = s3a_pack_scholar::display_registry();
    for (t, size, name) in [
        (12, 64, "academic_paper"),
        (14, 64, "srs_workspace"),
        (15, 512, "srs_manifest"),
    ] {
        assert_eq!(s.find(t, None, Some(size)).unwrap().name, name);
    }
    let v = s3a_vault::display_registry();
    let mut sizes: Vec<u32> = v.layouts().iter().map(|l| l.record_size).collect();
    sizes.sort();
    assert_eq!(
        sizes,
        vec![64, 80, 96, 160, 192, 220, 240],
        "one layout per vault record size"
    );
    for l in v.layouts() {
        assert_eq!(v.find(64, None, Some(l.record_size)).unwrap().name, l.name);
    }
    let edge = edge_display_registry();
    assert_eq!(edge.find(71, None, Some(64)).unwrap().name, "edge_consent");
    assert_eq!(
        edge.find(64, None, Some(32)).unwrap().name,
        "equipment_event"
    );
    assert_eq!(
        edge.find(64, None, Some(96)).unwrap().name,
        "device_identity"
    );
}

#[test]
fn offsets_decode_the_real_structs() {
    // statement index
    let l = &core_display_layouts()[0];
    let rec = StatementIndexRecord {
        statement_id: [0x0102030405060708, 0x1112131415161718],
        stored_timestamp_ms: 111,
        timestamp_ms: 222,
        actor_hash: 333,
        verb_hash: 444,
        activity_hash: 555,
        registration_hash: 666,
        body_tile_id: 777,
        body_offset: 888,
        flags: 999,
    };
    let b = bytemuck::bytes_of(&rec);
    assert_eq!(l.record_size as usize, b.len());
    for (n, v) in [
        ("stored_timestamp_ms", 111),
        ("timestamp_ms", 222),
        ("actor_hash", 333),
        ("verb_hash", 444),
        ("activity_hash", 555),
        ("registration_hash", 666),
        ("body_tile_id", 777),
        ("body_offset", 888),
        ("flags", 999),
    ] {
        assert_eq!(uint(b, l, n), v, "{n}");
    }
    assert_eq!(raw(b, l, "statement_id"), &b[..16]);

    // record root
    let l = s3a_engine::display_layouts::record_root_layout();
    let rec = RecordRootEntry {
        tile_index: 7,
        tile_type: 22,
        tile_id: 9,
        record_count: 11,
        seal_generation: 13,
        root: [0xAB; 32],
    };
    let b = bytemuck::bytes_of(&rec);
    assert_eq!(l.record_size as usize, b.len());
    for (n, v) in [
        ("tile_index", 7),
        ("tile_type", 22),
        ("tile_id", 9),
        ("record_count", 11),
        ("seal_generation", 13),
    ] {
        assert_eq!(uint(b, &l, n), v, "{n}");
    }
    assert_eq!(raw(b, &l, "root"), &[0xAB; 32]);

    // scholar
    let reg = s3a_pack_scholar::display_registry();
    let l = reg.find(12, None, None).unwrap();
    let rec = s3a_pack_scholar::AcademicPaperRecord::new(
        5, 6, 7, 1.5, 2.5, 3.5, 40, 2024, 9, 1, 2, 0xD01,
    );
    let b = bytemuck::bytes_of(&rec);
    assert_eq!(l.record_size as usize, b.len());
    assert_eq!(uint(b, l, "paper_id"), 5);
    assert_eq!(uint(b, l, "citation_count"), 40);
    assert_eq!(uint(b, l, "year"), 2024);
    assert_eq!(
        f32::from_le_bytes(raw(b, l, "topic_y").try_into().unwrap()),
        2.5
    );
    assert_eq!(uint(b, l, "flags"), rec.flags);
    let l = reg.find(15, None, None).unwrap();
    let rec = s3a_pack_scholar::SrsManifestHeader::new(
        [1, 2],
        [3, 4],
        [5, 6],
        [7, 8],
        1_700_000_000,
        3,
        99,
    );
    let b = bytemuck::bytes_of(&rec);
    assert_eq!(l.record_size as usize, b.len());
    assert_eq!(raw(b, l, "magic"), b"S3ASTOR1");
    assert_eq!(uint(b, l, "created_at_sec"), 1_700_000_000);
    assert_eq!(uint(b, l, "tile_count"), rec.tile_count as u64);
    assert_eq!(
        raw(b, l, "snapshot_uuid"),
        bytemuck::bytes_of(&rec.snapshot_uuid)
    );

    // edge
    let reg = edge_display_registry();
    let l = reg.find(71, None, Some(64)).unwrap();
    let rec = s3a_edge::EdgeConsentRecord::new([0xAA; 16], 5, 6, 7, 8, 9);
    let b = bytemuck::bytes_of(&rec);
    assert_eq!(l.record_size as usize, b.len());
    assert_eq!(
        (
            uint(b, l, "allowed_purposes"),
            uint(b, l, "sharing_scope"),
            uint(b, l, "granted_at"),
            uint(b, l, "expires_at"),
            uint(b, l, "admin_actor_id")
        ),
        (5, 6, 7, 8, 9)
    );
    assert_eq!(uint(b, l, "record_crc32"), rec.record_crc32 as u64);
    let l = reg.find(70, None, None).unwrap();
    let rec = s3a_edge::DeviceSealRecord::new(3, 4, [1; 32], [2; 32], [3; 64]);
    let b = bytemuck::bytes_of(&rec);
    assert_eq!(l.record_size as usize, b.len());
    assert_eq!((uint(b, l, "tile_index"), uint(b, l, "generation")), (3, 4));
    assert_eq!(raw(b, l, "signature"), &[3u8; 64]);
    assert_eq!(uint(b, l, "record_crc32"), rec.record_crc32 as u64);

    // vault
    let reg = s3a_vault::display_registry();
    let l = reg.find(64, None, Some(240)).unwrap();
    let rec = s3a_vault::ItemTransferRecord {
        transfer_id: [9; 16],
        item_hash: [1; 32],
        from_pubkey: [2; 32],
        to_pubkey: [3; 32],
        timestamp: 12345u64.to_le_bytes(),
        prev_transfer_hash: [4; 32],
        transfer_mode: 2,
        _pad: [0; 6],
        _reserved: [0; 12],
        record_crc32: 77,
        sender_signature: [5; 64],
    };
    let b = bytemuck::bytes_of(&rec);
    assert_eq!(l.record_size as usize, b.len());
    assert_eq!(uint(b, l, "timestamp"), 12345);
    assert_eq!(uint(b, l, "transfer_mode"), 2);
    assert_eq!(uint(b, l, "record_crc32"), 77);
    assert_eq!(raw(b, l, "sender_signature"), &[5u8; 64]);
    assert_eq!(raw(b, l, "to_pubkey"), &[3u8; 32]);
}

fn vector(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance")
        .join(s3a_spec_vectors::conformance::SUITE_VERSION)
        .join("files")
        .join(name)
}

#[test]
fn every_record_in_the_vault_vectors_resolves_to_a_layout() {
    let reg = s3a_vault::display_registry();
    let mut seen = 0;
    for f in [
        "vault_transfer_chain.s3a",
        "vault_delegation_governance.s3a",
        "vault_key_revocation.s3a",
        "vault_vc_import.s3a",
    ] {
        let r = MmapReader::open(vector(f)).unwrap();
        for i in 0..r.tile_count() as usize {
            let (h, p) = r.get_tile(i).unwrap();
            if h.tile_type != TileType::RECORD || h.record_count == 0 {
                continue;
            }
            let size = (h.payload_bytes / h.record_count as u32) as u32;
            let l = reg
                .find(64, None, Some(size))
                .unwrap_or_else(|| panic!("{f} tile {i}: no layout for record size {size}"));
            // The record's own stored CRC field must read back as a plausible non-zero value.
            let rec = &p[..size as usize];
            if l.name != "delegation_grant" {
                assert_ne!(uint(rec, l, "record_crc32"), 0, "{f} tile {i} {}", l.name);
            }
            seen += 1;
        }
    }
    assert!(seen >= 8, "saw only {seen} vault record tiles");
}

#[test]
fn every_tile_of_the_edge_vectors_with_a_known_type_resolves() {
    let reg = edge_display_registry();
    let mut seen = 0;
    for f in ["micro_valid_4kb.s3a", "micro_signed_seal_chain.s3a"] {
        let r = MmapReader::open(vector(f)).unwrap();
        for i in 0..r.tile_count() as usize {
            let Ok((h, p)) = r.try_get_edge_tile(i) else {
                continue;
            };
            if h.record_count == 0 {
                continue;
            }
            let size = (p.len() / h.record_count as usize) as u32;
            let l = reg.find(h.type_id as u32, None, Some(size));
            assert!(l.is_some(), "{f} tile {i}: type {} size {size}", h.type_id);
            seen += 1;
        }
    }
    assert!(seen >= 3);
}

#[test]
fn display_layouts_do_not_leak_into_the_sealing_schema() {
    // The sealing path (`standard_schema_for_type`) writes into files; it must keep emitting
    // the same fallback for these types, i.e. no layout added by this metadata registry.
    for t in [
        TileType::STATEMENT_INDEX,
        TileType::RECORD_ROOT,
        TileType::RECORD,
        12,
        14,
        15,
    ] {
        let d = s3a_engine::standard_schema_for_type(t);
        // Fallback is one opaque `record_data` field (type 64 has none); never a named layout.
        assert!(d.fields.iter().all(|f| f.name == "record_data"), "type {t}");
        assert!(d.fields.len() <= 1, "type {t}");
    }
}

#[test]
fn validation_rejects_bad_layouts() {
    let mut good = engine_display_layouts().remove(0);
    assert!(validate_layout(&good).is_ok());
    let base = good.clone();

    good.record_size = 0;
    assert_eq!(
        validate_layout(&good),
        Err(DisplayLayoutError::ZeroRecordSize)
    );
    let mut l = base.clone();
    l.fields.clear();
    assert_eq!(validate_layout(&l), Err(DisplayLayoutError::NoFields));
    let mut l = base.clone();
    l.fields[1].byte_offset = l.record_size; // runs past the end
    assert!(matches!(
        validate_layout(&l),
        Err(DisplayLayoutError::FieldOutOfBounds(_))
    ));
    let mut l = base.clone();
    l.fields[1].byte_offset = l.fields[0].byte_offset; // overlap
    assert!(matches!(
        validate_layout(&l),
        Err(DisplayLayoutError::FieldsOverlap(..))
    ));
    let mut l = base.clone();
    l.fields[1].field_type = 99;
    assert!(matches!(
        validate_layout(&l),
        Err(DisplayLayoutError::BadFieldType(_))
    ));
    let mut l = base.clone();
    l.fields[1].field_type = ft::U32; // wrong width for an 8-byte field
    assert!(matches!(
        validate_layout(&l),
        Err(DisplayLayoutError::BadFieldLength(_))
    ));
    let mut l = base.clone();
    l.fields[0].byte_offset = u32::MAX; // offset + len overflows
    assert!(matches!(
        validate_layout(&l),
        Err(DisplayLayoutError::FieldOutOfBounds(_))
    ));

    let mut reg = DisplayLayoutRegistry::new();
    reg.register(base.clone()).unwrap();
    assert!(matches!(
        reg.register(base),
        Err(DisplayLayoutError::Duplicate { .. })
    ));
}
