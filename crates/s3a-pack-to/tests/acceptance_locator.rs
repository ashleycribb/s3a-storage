//! Acceptance tests for s3a-pack-to tiles 49..=51 (F10).
//!
//! Tiles 47..=48 are reserved for `s3a-can-bridge` (PR #224) and must
//! not be claimed by this pack. The original band 41..=46 is unchanged.
//!
//! All sample strings below are synthetic demonstration text. They are
//! not Technical Order or OEM content.
//!
//! Do NOT edit these tests to make them pass.

use bytemuck::{bytes_of, pod_read_unaligned};
use s3a_core::{
    registry::TileTypeRegistry, required_addon_for_tile, S3AAddon, ADDON_CAN, ADDON_TO,
};
use s3a_engine::{MmapReader, S3ACrudEngine};
use s3a_pack_to::{
    decode_to_records, encode_to_locator, encode_to_procedure_step, encode_to_reference,
    register_pack_types, ToCrudExt, ToDecoded, ToError, ToLocatorRecord, ToPack,
    ToProcedureStepRecord, ToReferenceRecord, ToTileType,
};
use sha2::{Digest, Sha256};
use std::io;

const STEP_TEXT: &[u8] = b"Close the demonstration inlet valve.";
const PARA_PATH: &[u8] = b"4.2.3";
const PRINTED_PAGE: &[u8] = b"4-2";
const PUB_ID: &[u8] = b"SYN-TO-00-00-001";
const PUB_TITLE: &[u8] = b"Demonstration Related Publication";

const DOC_ID: [u8; 16] = [
    0x39, 0xb4, 0xb8, 0x85, 0x39, 0x8b, 0x30, 0x90, 0x42, 0xd4, 0xa3, 0xaa, 0xad, 0xfd, 0xd5, 0x36,
];
const ORIGIN_PTR: [u8; 16] = [
    0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32, 0x10, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32, 0x10,
];

fn source_id(n: u8) -> [u8; 16] {
    let mut id = [0u8; 16];
    id[0] = 0xa1;
    id[1] = 0x70;
    id[15] = n;
    id
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn first16(hash: [u8; 32]) -> [u8; 16] {
    let mut out = [0u8; 16];
    out.copy_from_slice(&hash[..16]);
    out
}

fn temp_archive(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "s3a_test_pack_to_loc_{}_{}_{}.s3a",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn procedure_record() -> ToProcedureStepRecord {
    encode_to_procedure_step(
        STEP_TEXT,
        source_id(21),
        ORIGIN_PTR,
        DOC_ID,
        PARA_PATH,
        1,
        32,
    )
    .expect("synthetic procedure step must encode")
}

fn reference_record() -> ToReferenceRecord {
    encode_to_reference(PUB_TITLE, source_id(22), ORIGIN_PTR, DOC_ID, PUB_ID, 11)
        .expect("synthetic reference must encode")
}

fn locator_record() -> ToLocatorRecord {
    encode_to_locator(
        source_id(23),
        ORIGIN_PTR,
        DOC_ID,
        source_id(21),
        ToTileType::TO_PROCEDURE_STEP,
        32,
        PARA_PATH,
        PRINTED_PAGE,
    )
    .expect("synthetic locator must encode")
}

/// AT-TO-11: tiles 49..=51 belong to s3a-pack-to under ADDON_TO; 47..=48 do not.
#[test]
fn at_to_11_tiles_49_to_51_are_to_pack_not_can() {
    assert_eq!(ToTileType::TO_PROCEDURE_STEP, 49);
    assert_eq!(ToTileType::TO_REFERENCE, 50);
    assert_eq!(ToTileType::TO_LOCATOR, 51);

    assert_eq!(std::mem::size_of::<ToProcedureStepRecord>(), 160);
    assert_eq!(std::mem::align_of::<ToProcedureStepRecord>(), 8);
    assert_eq!(ToProcedureStepRecord::SIZE, 160);

    assert_eq!(std::mem::size_of::<ToReferenceRecord>(), 128);
    assert_eq!(std::mem::align_of::<ToReferenceRecord>(), 8);
    assert_eq!(ToReferenceRecord::SIZE, 128);

    assert_eq!(std::mem::size_of::<ToLocatorRecord>(), 160);
    assert_eq!(std::mem::align_of::<ToLocatorRecord>(), 8);
    assert_eq!(ToLocatorRecord::SIZE, 160);

    let pack = ToPack;
    assert_eq!(pack.addon_id(), ADDON_TO);
    assert_eq!(pack.tile_range(), 41..=46);
    assert_eq!(ToPack::additional_tile_range(), 49..=51);
    assert!(!ToPack::additional_tile_range().contains(&47));
    assert!(!ToPack::additional_tile_range().contains(&48));
    assert!(!pack.tile_range().contains(&47));
    assert!(!pack.tile_range().contains(&48));

    let mut reg = TileTypeRegistry::new();
    register_pack_types(&mut reg).expect("TO pack types must register");
    assert_eq!(reg.get(49).unwrap().name, "to_procedure_step");
    assert_eq!(reg.get(49).unwrap().record_size, 160);
    assert_eq!(reg.get(49).unwrap().required_addon, Some(ADDON_TO));
    assert_eq!(reg.get(49).unwrap().owner_pack, "Technical Order Pack");
    assert_eq!(reg.get(50).unwrap().name, "to_reference");
    assert_eq!(reg.get(50).unwrap().record_size, 128);
    assert_eq!(reg.get(50).unwrap().required_addon, Some(ADDON_TO));
    assert_eq!(reg.get(51).unwrap().name, "to_locator");
    assert_eq!(reg.get(51).unwrap().record_size, 160);
    assert_eq!(reg.get(51).unwrap().required_addon, Some(ADDON_TO));
    assert!(reg.get(47).is_none());
    assert!(reg.get(48).is_none());

    let (name, mask) = required_addon_for_tile(49).expect("tile 49 must require ADDON_TO");
    assert_eq!(name, "s3a-pack-to");
    assert_eq!(mask, ADDON_TO);
    let (name, mask) = required_addon_for_tile(50).expect("tile 50 must require ADDON_TO");
    assert_eq!(name, "s3a-pack-to");
    assert_eq!(mask, ADDON_TO);
    let (name, mask) = required_addon_for_tile(51).expect("tile 51 must require ADDON_TO");
    assert_eq!(name, "s3a-pack-to");
    assert_eq!(mask, ADDON_TO);

    let (name, mask) = required_addon_for_tile(47).expect("tile 47 remains CAN");
    assert_eq!(name, "s3a-can-bridge");
    assert_eq!(mask, ADDON_CAN);
    let (name, mask) = required_addon_for_tile(48).expect("tile 48 remains CAN");
    assert_eq!(name, "s3a-can-bridge");
    assert_eq!(mask, ADDON_CAN);
}

/// AT-TO-12: encode and decode synthetic procedure, reference, and locator records.
#[test]
fn at_to_12_encode_decode_procedure_reference_locator() {
    let step = procedure_record();
    assert_eq!(step.source_id, source_id(21));
    assert_eq!(step.origin_ptr, ORIGIN_PTR);
    assert_eq!(step.doc_id, DOC_ID);
    assert_eq!(step.content_hash, sha256(STEP_TEXT));
    assert_eq!(step.text_hash, sha256(STEP_TEXT));
    assert_eq!(step.para_path_hash, sha256(PARA_PATH));
    assert_eq!(step.step_no, 1);
    assert_eq!(step.page, 32);
    assert_eq!(step._pad, [0u8; 8]);

    let decoded = decode_to_records(
        ToTileType::TO_PROCEDURE_STEP,
        1,
        bytes_of(&step),
    )
    .expect("procedure payload must decode");
    match decoded {
        ToDecoded::ProcedureStep(recs) => assert_eq!(recs, vec![step]),
        other => panic!("expected ProcedureStep, got {:?}", other),
    }

    let reference = reference_record();
    assert_eq!(reference.content_hash, sha256(PUB_TITLE));
    assert_eq!(reference.pub_id_hash, sha256(PUB_ID));
    assert_eq!(reference.page, 11);
    assert_eq!(reference._pad, [0u8; 12]);
    let decoded = decode_to_records(ToTileType::TO_REFERENCE, 1, bytes_of(&reference))
        .expect("reference payload must decode");
    match decoded {
        ToDecoded::Reference(recs) => assert_eq!(recs, vec![reference]),
        other => panic!("expected Reference, got {:?}", other),
    }

    let locator = locator_record();
    let mut loc_src = PARA_PATH.to_vec();
    loc_src.push(0x1f);
    loc_src.extend_from_slice(PRINTED_PAGE);
    assert_eq!(locator.content_hash, sha256(&loc_src));
    assert_eq!(locator.target_source_id, source_id(21));
    assert_eq!(locator.target_tile_type, ToTileType::TO_PROCEDURE_STEP);
    assert_eq!(locator.page, 32);
    assert_eq!(locator.para_path_hash, sha256(PARA_PATH));
    assert_eq!(locator.printed_page_hash, first16(sha256(PRINTED_PAGE)));
    assert_eq!(locator._pad, [0u8; 8]);
    let decoded = decode_to_records(ToTileType::TO_LOCATOR, 1, bytes_of(&locator))
        .expect("locator payload must decode");
    match decoded {
        ToDecoded::Locator(recs) => assert_eq!(recs, vec![locator]),
        other => panic!("expected Locator, got {:?}", other),
    }
}

/// AT-TO-13: encode and decode fail closed on empty text, zero ids, bad target tiles, and nonzero pad.
#[test]
fn at_to_13_fail_closed_on_invalid_locator_records() {
    assert_eq!(
        encode_to_procedure_step(b"", source_id(21), ORIGIN_PTR, DOC_ID, PARA_PATH, 1, 32)
            .unwrap_err(),
        ToError::EmptyText
    );
    assert_eq!(
        encode_to_procedure_step(STEP_TEXT, source_id(21), ORIGIN_PTR, DOC_ID, b"", 1, 32)
            .unwrap_err(),
        ToError::EmptyText
    );
    assert_eq!(
        encode_to_procedure_step(STEP_TEXT, [0u8; 16], ORIGIN_PTR, DOC_ID, PARA_PATH, 1, 32)
            .unwrap_err(),
        ToError::MissingSourceId
    );
    assert_eq!(
        encode_to_procedure_step(STEP_TEXT, source_id(21), ORIGIN_PTR, DOC_ID, PARA_PATH, 0, 32)
            .unwrap_err(),
        ToError::ZeroStepNo
    );
    assert_eq!(
        encode_to_procedure_step(STEP_TEXT, source_id(21), ORIGIN_PTR, DOC_ID, PARA_PATH, 1, 0)
            .unwrap_err(),
        ToError::ZeroPage
    );

    assert_eq!(
        encode_to_reference(b"", source_id(22), ORIGIN_PTR, DOC_ID, PUB_ID, 11).unwrap_err(),
        ToError::EmptyText
    );
    assert_eq!(
        encode_to_reference(PUB_TITLE, source_id(22), ORIGIN_PTR, DOC_ID, b"", 11).unwrap_err(),
        ToError::EmptyText
    );
    assert_eq!(
        encode_to_reference(PUB_TITLE, source_id(22), ORIGIN_PTR, DOC_ID, PUB_ID, 0).unwrap_err(),
        ToError::ZeroPage
    );

    assert_eq!(
        encode_to_locator(
            source_id(23),
            ORIGIN_PTR,
            DOC_ID,
            [0u8; 16],
            ToTileType::TO_PROCEDURE_STEP,
            32,
            PARA_PATH,
            PRINTED_PAGE,
        )
        .unwrap_err(),
        ToError::MissingTargetSourceId
    );
    assert_eq!(
        encode_to_locator(
            source_id(23),
            ORIGIN_PTR,
            DOC_ID,
            source_id(21),
            0,
            32,
            PARA_PATH,
            PRINTED_PAGE,
        )
        .unwrap_err(),
        ToError::TargetTileNotTo { tile_type: 0 }
    );
    assert_eq!(
        encode_to_locator(
            source_id(23),
            ORIGIN_PTR,
            DOC_ID,
            source_id(21),
            47,
            32,
            PARA_PATH,
            PRINTED_PAGE,
        )
        .unwrap_err(),
        ToError::TargetTileNotTo { tile_type: 47 }
    );
    assert_eq!(
        encode_to_locator(
            source_id(23),
            ORIGIN_PTR,
            DOC_ID,
            source_id(21),
            48,
            32,
            PARA_PATH,
            PRINTED_PAGE,
        )
        .unwrap_err(),
        ToError::TargetTileNotTo { tile_type: 48 }
    );
    assert_eq!(
        encode_to_locator(
            source_id(23),
            ORIGIN_PTR,
            DOC_ID,
            source_id(21),
            ToTileType::TO_PROCEDURE_STEP,
            0,
            PARA_PATH,
            PRINTED_PAGE,
        )
        .unwrap_err(),
        ToError::ZeroPage
    );
    assert_eq!(
        encode_to_locator(
            source_id(23),
            ORIGIN_PTR,
            DOC_ID,
            source_id(21),
            ToTileType::TO_PROCEDURE_STEP,
            32,
            b"",
            PRINTED_PAGE,
        )
        .unwrap_err(),
        ToError::EmptyText
    );
    assert_eq!(
        encode_to_locator(
            source_id(23),
            ORIGIN_PTR,
            DOC_ID,
            source_id(21),
            ToTileType::TO_PROCEDURE_STEP,
            32,
            PARA_PATH,
            b"",
        )
        .unwrap_err(),
        ToError::EmptyText
    );

    let mut bad = procedure_record();
    bad._pad[0] = 1;
    let err = decode_to_records(ToTileType::TO_PROCEDURE_STEP, 1, bytes_of(&bad))
        .expect_err("nonzero pad must be rejected");
    assert_eq!(err, ToError::NonZeroPad);

    let err = decode_to_records(ToTileType::TO_PROCEDURE_STEP, 99, bytes_of(&procedure_record()))
        .expect_err("layout version 99 must be rejected");
    assert_eq!(
        err,
        ToError::UnsupportedLayoutVersion {
            tile_type: 49,
            version: 99
        }
    );

    let record = procedure_record();
    let short = &bytes_of(&record)[..10];
    let err = decode_to_records(ToTileType::TO_PROCEDURE_STEP, 1, short)
        .expect_err("truncated record must be rejected");
    match err {
        ToError::TruncatedRecord { expected, actual } => {
            assert_eq!(expected, 160);
            assert_eq!(actual, 10);
        }
        other => panic!("expected TruncatedRecord, got {:?}", other),
    }
}

/// AT-TO-14: append without ADDON_TO is denied; with the bit, records round-trip.
#[test]
fn at_to_14_roundtrip_procedure_reference_locator_tiles() {
    let temp = temp_archive("at14");
    let engine = S3ACrudEngine::open_or_create(&temp).unwrap();
    let step = procedure_record();
    let err = engine
        .create_to_procedure_step_records(&[step])
        .expect_err("append without ADDON_TO must fail");
    assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);

    engine.enable_addon(ADDON_TO).unwrap();
    let reference = reference_record();
    let locator = locator_record();
    engine.create_to_procedure_step_records(&[step]).unwrap();
    engine.create_to_reference_records(&[reference]).unwrap();
    engine.create_to_locator_records(&[locator]).unwrap();

    let reader = MmapReader::open(&temp).unwrap();
    assert_eq!(reader.tile_count(), 3);

    let expected = [
        (ToTileType::TO_PROCEDURE_STEP, bytes_of(&step).to_vec()),
        (ToTileType::TO_REFERENCE, bytes_of(&reference).to_vec()),
        (ToTileType::TO_LOCATOR, bytes_of(&locator).to_vec()),
    ];
    for (i, (tile_type, payload)) in expected.iter().enumerate() {
        let (header, raw) = reader.get_tile(i).expect("tile must be readable");
        assert_eq!(header.tile_type, *tile_type);
        assert_eq!(header.record_layout_version, 1);
        let got = &raw[..header.payload_bytes as usize];
        assert_eq!(got, payload.as_slice());
        let decoded = decode_to_records(header.tile_type, header.record_layout_version, got)
            .expect("written payload must decode");
        match (tile_type, decoded) {
            (t, ToDecoded::ProcedureStep(recs)) if *t == ToTileType::TO_PROCEDURE_STEP => {
                assert_eq!(recs, vec![step]);
            }
            (t, ToDecoded::Reference(recs)) if *t == ToTileType::TO_REFERENCE => {
                assert_eq!(recs, vec![reference]);
            }
            (t, ToDecoded::Locator(recs)) if *t == ToTileType::TO_LOCATOR => {
                assert_eq!(recs, vec![locator]);
            }
            other => panic!("unexpected decode for tile {}: {:?}", i, other),
        }
    }

    let _ = std::fs::remove_file(&temp);
}

/// AT-TO-15: a locator may target any TO tile in 41..=46 or 49..=51, and only those.
#[test]
fn at_to_15_locator_targets_only_to_tiles() {
    for tile in [41u32, 42, 43, 44, 45, 46, 49, 50, 51] {
        encode_to_locator(
            source_id(23),
            ORIGIN_PTR,
            DOC_ID,
            source_id(21),
            tile,
            32,
            PARA_PATH,
            PRINTED_PAGE,
        )
        .unwrap_or_else(|_| panic!("locator must accept TO tile {tile}"));
    }
    for tile in [1u32, 3, 33, 47, 48, 52, 96] {
        let err = encode_to_locator(
            source_id(23),
            ORIGIN_PTR,
            DOC_ID,
            source_id(21),
            tile,
            32,
            PARA_PATH,
            PRINTED_PAGE,
        )
        .expect_err("non-TO tile must be rejected");
        assert_eq!(err, ToError::TargetTileNotTo { tile_type: tile });
    }
}

/// AT-TO-16: these fixtures are synthetic demonstration text, not a real TO.
#[test]
fn at_to_16_locator_fixture_is_synthetic_only() {
    for sample in [STEP_TEXT, PARA_PATH, PRINTED_PAGE, PUB_ID, PUB_TITLE] {
        let text = std::str::from_utf8(sample).unwrap().to_lowercase();
        assert!(
            text.contains("demonstration")
                || text.contains("syn-to")
                || text == "4.2.3"
                || text == "4-2",
            "unexpected fixture text: {text}"
        );
        assert!(!text.contains("etims"));
    }
}

/// Keep a read of pod_read_unaligned so a future layout-shift test can use it.
#[test]
fn at_to_17_procedure_record_is_plain_pod() {
    let step = procedure_record();
    let bytes = bytes_of(&step);
    let copy: ToProcedureStepRecord = pod_read_unaligned(bytes);
    assert_eq!(copy, step);
}
