//! Acceptance tests for s3a-pack-to tile 52 (`to_scan_agreement`).
//!
//! Tile 52 is a new adjunct record that links to an existing section/cell
//! (or other TO tile in 41..=46 or 49..=51) by `target_source_id` and
//! `target_tile_type`. Existing tiles are not resized, so archives that
//! lack tile 52 stay readable. Tiles 47..=48 remain `s3a-can-bridge`.
//!
//! All sample strings below are synthetic demonstration text. They are
//! not Technical Order or OEM content.
//!
//! Do NOT edit these tests to make them pass.

use bytemuck::bytes_of;
use s3a_core::{registry::TileTypeRegistry, required_addon_for_tile, S3AAddon, ADDON_TO};
use s3a_engine::{MmapReader, S3ACrudEngine};
use s3a_pack_to::{
    decode_to_records, encode_to_scan_agreement, encode_to_section, register_pack_types,
    scan_status_label, ToCrudExt, ToDecoded, ToError, ToPack, ToScanAgreementRecord, ToTileType,
    SCAN_FLAG_CER_PRESENT, SCAN_STATUS_HELD, SCAN_STATUS_HUMAN_VERIFIED,
    SCAN_STATUS_SCAN_AGREEMENT, SCAN_STATUS_VERIFIED,
};
use sha2::{Digest, Sha256};
use std::io;

const ENGINE_A: &[u8] = b"tesseract";
const ENGINE_A_VER: &[u8] = b"5.5.0";
const ENGINE_B: &[u8] = b"rapidocr";
const ENGINE_B_VER: &[u8] = b"1.2.3";
const OCR_TEXT: &[u8] = b"The fixture code is ALPHA-937.";
const TITLE: &[u8] = b"Demonstration Scan Section";
const REVIEWER: &[u8] = b"reviewer-fixture";
const REVIEWED_AT: u64 = 1_700_000_000;
const MAX_CER: f64 = 0.15;
const SECTION_SCOPE: &[u8] = b"SYN-TO-00-00-001 1.1";

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

fn pad32(bytes: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[..bytes.len()].copy_from_slice(bytes);
    out
}

fn pad16(bytes: &[u8]) -> [u8; 16] {
    let mut out = [0u8; 16];
    out[..bytes.len()].copy_from_slice(bytes);
    out
}

fn scan_content_hash(
    engine_a_name: &[u8],
    engine_a_version: &[u8],
    engine_b_name: &[u8],
    engine_b_version: &[u8],
    ocr_a: [u8; 32],
    ocr_b: [u8; 32],
    crop: [u8; 32],
) -> [u8; 32] {
    let mut src = Vec::new();
    src.extend_from_slice(engine_a_name);
    src.push(0x1f);
    src.extend_from_slice(engine_a_version);
    src.push(0x1f);
    src.extend_from_slice(engine_b_name);
    src.push(0x1f);
    src.extend_from_slice(engine_b_version);
    src.push(0x1f);
    src.extend_from_slice(&ocr_a);
    src.extend_from_slice(&ocr_b);
    src.extend_from_slice(&crop);
    sha256(&src)
}

fn crop_hash() -> [u8; 32] {
    sha256(b"synthetic-crop-bytes")
}

fn encode_agree(cer: f64) -> ToScanAgreementRecord {
    encode_to_scan_agreement(
        source_id(30),
        ORIGIN_PTR,
        DOC_ID,
        source_id(1),
        ToTileType::TO_SECTION,
        SCAN_STATUS_SCAN_AGREEMENT,
        ENGINE_A,
        ENGINE_A_VER,
        ENGINE_B,
        ENGINE_B_VER,
        sha256(OCR_TEXT),
        sha256(OCR_TEXT),
        crop_hash(),
        Some(cer),
        MAX_CER,
        None,
        None,
    )
    .expect("synthetic scan_agreement record must encode")
}

fn temp_archive(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "s3a_test_pack_to_{}_{}_{}.s3a",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

/// AT-TO-16: tile 52 is a new s3a-pack-to type; existing bands stay put.
#[test]
fn at_to_16_tile_52_is_scan_agreement_in_to_pack() {
    assert_eq!(ToTileType::TO_SCAN_AGREEMENT, 52);
    assert_eq!(std::mem::size_of::<ToScanAgreementRecord>(), 384);
    assert_eq!(std::mem::align_of::<ToScanAgreementRecord>(), 8);
    assert_eq!(ToScanAgreementRecord::SIZE, 384);
    assert_eq!(SCAN_STATUS_VERIFIED, 1);
    assert_eq!(SCAN_STATUS_HELD, 2);
    assert_eq!(SCAN_STATUS_SCAN_AGREEMENT, 3);
    assert_eq!(SCAN_STATUS_HUMAN_VERIFIED, 4);
    assert_eq!(SCAN_FLAG_CER_PRESENT, 1);

    let pack = ToPack;
    assert_eq!(pack.addon_id(), ADDON_TO);
    assert_eq!(pack.tile_range(), 41..=46);
    assert_eq!(ToPack::additional_tile_range(), 49..=51);
    assert_eq!(ToPack::scan_tile_range(), 52..=52);
    assert!(!pack.tile_range().contains(&52));
    assert!(!ToPack::additional_tile_range().contains(&52));
    assert!(!ToPack::scan_tile_range().contains(&47));
    assert!(!ToPack::scan_tile_range().contains(&48));
    assert!(!ToPack::scan_tile_range().contains(&51));

    let mut reg = TileTypeRegistry::new();
    register_pack_types(&mut reg).expect("TO pack types must register");
    let desc = reg.get(52).expect("tile 52 must be registered");
    assert_eq!(desc.name, "to_scan_agreement");
    assert_eq!(desc.record_size, 384);
    assert_eq!(desc.required_addon, Some(ADDON_TO));
    assert_eq!(desc.owner_pack, "s3a-pack-to");
    // Existing types are still present and unchanged.
    assert_eq!(reg.get(41).unwrap().name, "to_section");
    assert_eq!(reg.get(41).unwrap().record_size, 128);
    assert_eq!(reg.get(51).unwrap().name, "to_locator");
    assert_eq!(reg.get(51).unwrap().record_size, 160);

    let (name, mask) = required_addon_for_tile(52).expect("tile 52 must require ADDON_TO");
    assert_eq!(name, "s3a-pack-to");
    assert_eq!(mask, ADDON_TO);
}

/// AT-TO-17: a scan_agreement record round-trips through encode/decode and
/// carries status, both engine names and versions, both text hashes, the
/// crop hash, the CER, and the threshold. content_hash is defined, not invented.
#[test]
fn at_to_17_encode_decode_scan_agreement_roundtrip() {
    let rec = encode_agree(0.0);
    assert_eq!(rec.source_id, source_id(30));
    assert_eq!(rec.origin_ptr, ORIGIN_PTR);
    assert_eq!(rec.doc_id, DOC_ID);
    assert_eq!(rec.target_source_id, source_id(1));
    assert_eq!(rec.target_tile_type, ToTileType::TO_SECTION);
    assert_eq!(rec.status, SCAN_STATUS_SCAN_AGREEMENT);
    assert_eq!(rec.flags, SCAN_FLAG_CER_PRESENT);
    assert_eq!(rec.engine_a_name, pad32(ENGINE_A));
    assert_eq!(rec.engine_a_version, pad16(ENGINE_A_VER));
    assert_eq!(rec.engine_b_name, pad32(ENGINE_B));
    assert_eq!(rec.engine_b_version, pad16(ENGINE_B_VER));
    assert_eq!(rec.ocr_a_sha256, sha256(OCR_TEXT));
    assert_eq!(rec.ocr_b_sha256, sha256(OCR_TEXT));
    assert_eq!(rec.crop_sha256, crop_hash());
    assert_eq!(rec.cer_bits, 0.0f64.to_bits());
    assert_eq!(rec.max_cer_bits, MAX_CER.to_bits());
    assert_eq!(rec.reviewed_at, 0);
    assert_eq!(rec.reviewer, [0u8; 32]);
    assert_eq!(rec._pad0, [0u8; 2]);
    assert_eq!(rec._pad, [0u8; 32]);
    assert_eq!(
        rec.content_hash,
        scan_content_hash(
            ENGINE_A,
            ENGINE_A_VER,
            ENGINE_B,
            ENGINE_B_VER,
            sha256(OCR_TEXT),
            sha256(OCR_TEXT),
            crop_hash(),
        )
    );

    let decoded = decode_to_records(ToTileType::TO_SCAN_AGREEMENT, 1, bytes_of(&rec))
        .expect("scan_agreement payload must decode");
    match decoded {
        ToDecoded::ScanAgreement(recs) => assert_eq!(recs, vec![rec]),
        other => panic!("expected ScanAgreement, got {:?}", other),
    }
}

/// AT-TO-18: a record stored as scan_agreement never reads back as verified
/// and is never labelled "verified".
#[test]
fn at_to_18_scan_agreement_never_reads_back_as_verified() {
    let rec = encode_agree(0.0);
    assert_eq!(rec.status, SCAN_STATUS_SCAN_AGREEMENT);
    assert_ne!(rec.status, SCAN_STATUS_VERIFIED);
    assert_eq!(scan_status_label(rec.status), "scan_agreement");
    assert_ne!(scan_status_label(rec.status), "verified");

    let decoded = decode_to_records(ToTileType::TO_SCAN_AGREEMENT, 1, bytes_of(&rec))
        .expect("scan_agreement payload must decode");
    match decoded {
        ToDecoded::ScanAgreement(recs) => {
            assert_eq!(recs.len(), 1);
            assert_eq!(recs[0].status, SCAN_STATUS_SCAN_AGREEMENT);
            assert_ne!(recs[0].status, SCAN_STATUS_VERIFIED);
            assert_eq!(scan_status_label(recs[0].status), "scan_agreement");
            assert_ne!(scan_status_label(recs[0].status), "verified");
        }
        other => panic!("expected ScanAgreement, got {:?}", other),
    }

    assert_eq!(scan_status_label(SCAN_STATUS_VERIFIED), "verified");
    assert_eq!(scan_status_label(SCAN_STATUS_HELD), "held");
    assert_eq!(
        scan_status_label(SCAN_STATUS_HUMAN_VERIFIED),
        "human_verified"
    );
}

/// AT-TO-19: human_verified requires a reviewer, a timestamp, and the three
/// evidence hashes (both OCR text sha256s and the crop sha256).
#[test]
fn at_to_19_human_verified_requires_reviewer_timestamp_and_evidence_hashes() {
    let ocr = sha256(OCR_TEXT);
    let crop = crop_hash();

    assert_eq!(
        encode_to_scan_agreement(
            source_id(31),
            ORIGIN_PTR,
            DOC_ID,
            source_id(1),
            ToTileType::TO_SECTION,
            SCAN_STATUS_HUMAN_VERIFIED,
            ENGINE_A,
            ENGINE_A_VER,
            ENGINE_B,
            ENGINE_B_VER,
            ocr,
            ocr,
            crop,
            Some(0.0),
            MAX_CER,
            None,
            Some(REVIEWED_AT),
        )
        .unwrap_err(),
        ToError::MissingReviewer
    );

    assert_eq!(
        encode_to_scan_agreement(
            source_id(31),
            ORIGIN_PTR,
            DOC_ID,
            source_id(1),
            ToTileType::TO_SECTION,
            SCAN_STATUS_HUMAN_VERIFIED,
            ENGINE_A,
            ENGINE_A_VER,
            ENGINE_B,
            ENGINE_B_VER,
            ocr,
            ocr,
            crop,
            Some(0.0),
            MAX_CER,
            Some(REVIEWER),
            None,
        )
        .unwrap_err(),
        ToError::MissingReviewedAt
    );

    assert_eq!(
        encode_to_scan_agreement(
            source_id(31),
            ORIGIN_PTR,
            DOC_ID,
            source_id(1),
            ToTileType::TO_SECTION,
            SCAN_STATUS_HUMAN_VERIFIED,
            ENGINE_A,
            ENGINE_A_VER,
            ENGINE_B,
            ENGINE_B_VER,
            [0u8; 32],
            ocr,
            crop,
            Some(0.0),
            MAX_CER,
            Some(REVIEWER),
            Some(REVIEWED_AT),
        )
        .unwrap_err(),
        ToError::MissingOcrHash
    );

    assert_eq!(
        encode_to_scan_agreement(
            source_id(31),
            ORIGIN_PTR,
            DOC_ID,
            source_id(1),
            ToTileType::TO_SECTION,
            SCAN_STATUS_HUMAN_VERIFIED,
            ENGINE_A,
            ENGINE_A_VER,
            ENGINE_B,
            ENGINE_B_VER,
            ocr,
            [0u8; 32],
            crop,
            Some(0.0),
            MAX_CER,
            Some(REVIEWER),
            Some(REVIEWED_AT),
        )
        .unwrap_err(),
        ToError::MissingOcrHash
    );

    assert_eq!(
        encode_to_scan_agreement(
            source_id(31),
            ORIGIN_PTR,
            DOC_ID,
            source_id(1),
            ToTileType::TO_SECTION,
            SCAN_STATUS_HUMAN_VERIFIED,
            ENGINE_A,
            ENGINE_A_VER,
            ENGINE_B,
            ENGINE_B_VER,
            ocr,
            ocr,
            [0u8; 32],
            Some(0.0),
            MAX_CER,
            Some(REVIEWER),
            Some(REVIEWED_AT),
        )
        .unwrap_err(),
        ToError::MissingCropHash
    );

    let rec = encode_to_scan_agreement(
        source_id(31),
        ORIGIN_PTR,
        DOC_ID,
        source_id(1),
        ToTileType::TO_SECTION,
        SCAN_STATUS_HUMAN_VERIFIED,
        ENGINE_A,
        ENGINE_A_VER,
        ENGINE_B,
        ENGINE_B_VER,
        ocr,
        ocr,
        crop,
        Some(0.0),
        MAX_CER,
        Some(REVIEWER),
        Some(REVIEWED_AT),
    )
    .expect("complete human_verified evidence must encode");
    assert_eq!(rec.status, SCAN_STATUS_HUMAN_VERIFIED);
    assert_ne!(rec.status, SCAN_STATUS_VERIFIED);
    assert_ne!(rec.status, SCAN_STATUS_SCAN_AGREEMENT);
    assert_eq!(rec.reviewer, pad32(REVIEWER));
    assert_eq!(rec.reviewed_at, REVIEWED_AT);
    assert_eq!(rec.ocr_a_sha256, ocr);
    assert_eq!(rec.ocr_b_sha256, ocr);
    assert_eq!(rec.crop_sha256, crop);
    assert_eq!(scan_status_label(rec.status), "human_verified");

    let decoded = decode_to_records(ToTileType::TO_SCAN_AGREEMENT, 1, bytes_of(&rec))
        .expect("human_verified payload must decode");
    match decoded {
        ToDecoded::ScanAgreement(recs) => {
            assert_eq!(recs, vec![rec]);
            assert_eq!(recs[0].status, SCAN_STATUS_HUMAN_VERIFIED);
        }
        other => panic!("expected ScanAgreement, got {:?}", other),
    }
}

/// AT-TO-20: encode and decode fail closed on missing or invalid fields.
/// There is no silent default for the CER threshold.
#[test]
fn at_to_20_fail_closed_on_invalid_scan_records() {
    let ocr = sha256(OCR_TEXT);
    let crop = crop_hash();

    assert_eq!(
        encode_to_scan_agreement(
            [0u8; 16],
            ORIGIN_PTR,
            DOC_ID,
            source_id(1),
            ToTileType::TO_SECTION,
            SCAN_STATUS_SCAN_AGREEMENT,
            ENGINE_A,
            ENGINE_A_VER,
            ENGINE_B,
            ENGINE_B_VER,
            ocr,
            ocr,
            crop,
            Some(0.0),
            MAX_CER,
            None,
            None,
        )
        .unwrap_err(),
        ToError::MissingSourceId
    );

    assert_eq!(
        encode_to_scan_agreement(
            source_id(30),
            ORIGIN_PTR,
            DOC_ID,
            [0u8; 16],
            ToTileType::TO_SECTION,
            SCAN_STATUS_SCAN_AGREEMENT,
            ENGINE_A,
            ENGINE_A_VER,
            ENGINE_B,
            ENGINE_B_VER,
            ocr,
            ocr,
            crop,
            Some(0.0),
            MAX_CER,
            None,
            None,
        )
        .unwrap_err(),
        ToError::MissingTargetSourceId
    );

    assert_eq!(
        encode_to_scan_agreement(
            source_id(30),
            ORIGIN_PTR,
            DOC_ID,
            source_id(1),
            47,
            SCAN_STATUS_SCAN_AGREEMENT,
            ENGINE_A,
            ENGINE_A_VER,
            ENGINE_B,
            ENGINE_B_VER,
            ocr,
            ocr,
            crop,
            Some(0.0),
            MAX_CER,
            None,
            None,
        )
        .unwrap_err(),
        ToError::TargetTileNotTo { tile_type: 47 }
    );

    assert_eq!(
        encode_to_scan_agreement(
            source_id(30),
            ORIGIN_PTR,
            DOC_ID,
            source_id(1),
            52,
            SCAN_STATUS_SCAN_AGREEMENT,
            ENGINE_A,
            ENGINE_A_VER,
            ENGINE_B,
            ENGINE_B_VER,
            ocr,
            ocr,
            crop,
            Some(0.0),
            MAX_CER,
            None,
            None,
        )
        .unwrap_err(),
        ToError::TargetTileNotTo { tile_type: 52 }
    );

    assert_eq!(
        encode_to_scan_agreement(
            source_id(30),
            ORIGIN_PTR,
            DOC_ID,
            source_id(1),
            ToTileType::TO_SECTION,
            0,
            ENGINE_A,
            ENGINE_A_VER,
            ENGINE_B,
            ENGINE_B_VER,
            ocr,
            ocr,
            crop,
            Some(0.0),
            MAX_CER,
            None,
            None,
        )
        .unwrap_err(),
        ToError::UnknownScanStatus { status: 0 }
    );

    assert_eq!(
        encode_to_scan_agreement(
            source_id(30),
            ORIGIN_PTR,
            DOC_ID,
            source_id(1),
            ToTileType::TO_SECTION,
            SCAN_STATUS_SCAN_AGREEMENT,
            b"",
            ENGINE_A_VER,
            ENGINE_B,
            ENGINE_B_VER,
            ocr,
            ocr,
            crop,
            Some(0.0),
            MAX_CER,
            None,
            None,
        )
        .unwrap_err(),
        ToError::MissingEngineName
    );

    assert_eq!(
        encode_to_scan_agreement(
            source_id(30),
            ORIGIN_PTR,
            DOC_ID,
            source_id(1),
            ToTileType::TO_SECTION,
            SCAN_STATUS_SCAN_AGREEMENT,
            ENGINE_A,
            b"",
            ENGINE_B,
            ENGINE_B_VER,
            ocr,
            ocr,
            crop,
            Some(0.0),
            MAX_CER,
            None,
            None,
        )
        .unwrap_err(),
        ToError::MissingEngineVersion
    );

    let too_long_name = vec![b'x'; 33];
    assert_eq!(
        encode_to_scan_agreement(
            source_id(30),
            ORIGIN_PTR,
            DOC_ID,
            source_id(1),
            ToTileType::TO_SECTION,
            SCAN_STATUS_SCAN_AGREEMENT,
            &too_long_name,
            ENGINE_A_VER,
            ENGINE_B,
            ENGINE_B_VER,
            ocr,
            ocr,
            crop,
            Some(0.0),
            MAX_CER,
            None,
            None,
        )
        .unwrap_err(),
        ToError::EngineFieldTooLong
    );

    assert_eq!(
        encode_to_scan_agreement(
            source_id(30),
            ORIGIN_PTR,
            DOC_ID,
            source_id(1),
            ToTileType::TO_SECTION,
            SCAN_STATUS_SCAN_AGREEMENT,
            ENGINE_A,
            ENGINE_A_VER,
            ENGINE_B,
            ENGINE_B_VER,
            ocr,
            ocr,
            crop,
            Some(0.0),
            f64::NAN,
            None,
            None,
        )
        .unwrap_err(),
        ToError::InvalidThreshold
    );

    assert_eq!(
        encode_to_scan_agreement(
            source_id(30),
            ORIGIN_PTR,
            DOC_ID,
            source_id(1),
            ToTileType::TO_SECTION,
            SCAN_STATUS_SCAN_AGREEMENT,
            ENGINE_A,
            ENGINE_A_VER,
            ENGINE_B,
            ENGINE_B_VER,
            ocr,
            ocr,
            crop,
            Some(0.0),
            1.1,
            None,
            None,
        )
        .unwrap_err(),
        ToError::InvalidThreshold
    );

    assert_eq!(
        encode_to_scan_agreement(
            source_id(30),
            ORIGIN_PTR,
            DOC_ID,
            source_id(1),
            ToTileType::TO_SECTION,
            SCAN_STATUS_SCAN_AGREEMENT,
            ENGINE_A,
            ENGINE_A_VER,
            ENGINE_B,
            ENGINE_B_VER,
            ocr,
            ocr,
            crop,
            Some(0.0),
            -0.01,
            None,
            None,
        )
        .unwrap_err(),
        ToError::InvalidThreshold
    );

    assert_eq!(
        encode_to_scan_agreement(
            source_id(30),
            ORIGIN_PTR,
            DOC_ID,
            source_id(1),
            ToTileType::TO_SECTION,
            SCAN_STATUS_SCAN_AGREEMENT,
            ENGINE_A,
            ENGINE_A_VER,
            ENGINE_B,
            ENGINE_B_VER,
            ocr,
            ocr,
            crop,
            None,
            MAX_CER,
            None,
            None,
        )
        .unwrap_err(),
        ToError::MissingCer
    );

    assert_eq!(
        encode_to_scan_agreement(
            source_id(30),
            ORIGIN_PTR,
            DOC_ID,
            source_id(1),
            ToTileType::TO_SECTION,
            SCAN_STATUS_SCAN_AGREEMENT,
            ENGINE_A,
            ENGINE_A_VER,
            ENGINE_B,
            ENGINE_B_VER,
            ocr,
            ocr,
            crop,
            Some(0.2105),
            MAX_CER,
            None,
            None,
        )
        .unwrap_err(),
        ToError::CerExceedsThreshold
    );

    // Disagreement is stored as held, not as scan_agreement or verified.
    let held = encode_to_scan_agreement(
        source_id(32),
        ORIGIN_PTR,
        DOC_ID,
        source_id(1),
        ToTileType::TO_SPEC_CELL,
        SCAN_STATUS_HELD,
        ENGINE_A,
        ENGINE_A_VER,
        ENGINE_B,
        ENGINE_B_VER,
        ocr,
        sha256(b"different-ocr"),
        crop,
        Some(0.2105),
        MAX_CER,
        None,
        None,
    )
    .expect("held record with CER above threshold must encode");
    assert_eq!(held.status, SCAN_STATUS_HELD);
    assert_ne!(held.status, SCAN_STATUS_VERIFIED);
    assert_ne!(held.status, SCAN_STATUS_SCAN_AGREEMENT);
    assert_eq!(held.target_tile_type, ToTileType::TO_SPEC_CELL);
    assert_eq!(held.cer_bits, 0.2105f64.to_bits());
    assert_eq!(held.max_cer_bits, MAX_CER.to_bits());
    assert_eq!(scan_status_label(held.status), "held");

    // Empty OCR may be held with no CER; hashes of empty text are allowed.
    let empty = sha256(b"");
    let empty_held = encode_to_scan_agreement(
        source_id(33),
        ORIGIN_PTR,
        DOC_ID,
        source_id(1),
        ToTileType::TO_SECTION,
        SCAN_STATUS_HELD,
        ENGINE_A,
        ENGINE_A_VER,
        ENGINE_B,
        ENGINE_B_VER,
        empty,
        empty,
        crop,
        None,
        MAX_CER,
        None,
        None,
    )
    .expect("held record with empty OCR and no CER must encode");
    assert_eq!(empty_held.status, SCAN_STATUS_HELD);
    assert_eq!(empty_held.flags, 0);
    assert_eq!(empty_held.cer_bits, 0);
    assert_eq!(empty_held.max_cer_bits, MAX_CER.to_bits());

    assert_eq!(
        encode_to_scan_agreement(
            source_id(30),
            ORIGIN_PTR,
            DOC_ID,
            source_id(1),
            ToTileType::TO_SECTION,
            SCAN_STATUS_SCAN_AGREEMENT,
            ENGINE_A,
            ENGINE_A_VER,
            ENGINE_B,
            ENGINE_B_VER,
            ocr,
            ocr,
            crop,
            Some(0.0),
            MAX_CER,
            Some(REVIEWER),
            None,
        )
        .unwrap_err(),
        ToError::UnexpectedReviewEvidence
    );

    let mut bad = encode_agree(0.0);
    bad._pad[0] = 1;
    let err = decode_to_records(ToTileType::TO_SCAN_AGREEMENT, 1, bytes_of(&bad))
        .expect_err("nonzero pad must be rejected");
    assert_eq!(err, ToError::NonZeroPad);

    let err = decode_to_records(
        ToTileType::TO_SCAN_AGREEMENT,
        99,
        bytes_of(&encode_agree(0.0)),
    )
    .expect_err("layout version 99 must be rejected");
    assert_eq!(
        err,
        ToError::UnsupportedLayoutVersion {
            tile_type: 52,
            version: 99
        }
    );

    let record = encode_agree(0.0);
    let short = &bytes_of(&record)[..10];
    let err = decode_to_records(ToTileType::TO_SCAN_AGREEMENT, 1, short)
        .expect_err("truncated record must be rejected");
    match err {
        ToError::TruncatedRecord { expected, actual } => {
            assert_eq!(expected, 384);
            assert_eq!(actual, 10);
        }
        other => panic!("expected TruncatedRecord, got {:?}", other),
    }
}

/// AT-TO-21: append without ADDON_TO is denied; with the bit, a section and
/// its scan_agreement adjunct round-trip through an S3A archive.
#[test]
fn at_to_21_roundtrip_scan_agreement_tile_in_archive() {
    let temp = temp_archive("at21");
    let engine = S3ACrudEngine::open_or_create(&temp).unwrap();
    let rec = encode_agree(0.0);
    let err = engine
        .create_to_scan_agreement_records(&[rec])
        .expect_err("append without ADDON_TO must fail");
    assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);

    engine.enable_addon(ADDON_TO).unwrap();
    let section = encode_to_section(
        SECTION_SCOPE,
        source_id(1),
        ORIGIN_PTR,
        DOC_ID,
        1,
        1,
        1,
        TITLE,
    )
    .expect("section must encode");
    engine.create_to_section_records(&[section]).unwrap();
    engine.create_to_scan_agreement_records(&[rec]).unwrap();

    let reader = MmapReader::open(&temp).unwrap();
    assert_eq!(reader.tile_count(), 2);

    let (header, raw) = reader.get_tile(0).expect("section tile must be readable");
    assert_eq!(header.tile_type, ToTileType::TO_SECTION);
    let got = &raw[..header.payload_bytes as usize];
    match decode_to_records(header.tile_type, header.record_layout_version, got).unwrap() {
        ToDecoded::Section(recs) => assert_eq!(recs, vec![section]),
        other => panic!("expected Section, got {:?}", other),
    }

    let (header, raw) = reader.get_tile(1).expect("scan tile must be readable");
    assert_eq!(header.tile_type, ToTileType::TO_SCAN_AGREEMENT);
    assert_eq!(header.record_layout_version, 1);
    let got = &raw[..header.payload_bytes as usize];
    assert_eq!(got, bytes_of(&rec));
    match decode_to_records(header.tile_type, header.record_layout_version, got).unwrap() {
        ToDecoded::ScanAgreement(recs) => {
            assert_eq!(recs, vec![rec]);
            assert_eq!(recs[0].status, SCAN_STATUS_SCAN_AGREEMENT);
            assert_ne!(recs[0].status, SCAN_STATUS_VERIFIED);
            assert_eq!(scan_status_label(recs[0].status), "scan_agreement");
        }
        other => panic!("expected ScanAgreement, got {:?}", other),
    }

    let _ = std::fs::remove_file(&temp);
}

/// AT-TO-22: an archive that has no tile 52 still reads after the new type
/// is registered. Existing section bytes are unchanged.
#[test]
fn at_to_22_old_archives_without_tile_52_still_read() {
    let temp = temp_archive("at22");
    let engine = S3ACrudEngine::open_or_create(&temp).unwrap();
    engine.enable_addon(ADDON_TO).unwrap();
    let section = encode_to_section(
        SECTION_SCOPE,
        source_id(1),
        ORIGIN_PTR,
        DOC_ID,
        1,
        1,
        1,
        TITLE,
    )
    .expect("section must encode");
    engine.create_to_section_records(&[section]).unwrap();

    let reader = MmapReader::open(&temp).unwrap();
    assert_eq!(reader.tile_count(), 1);
    let (header, raw) = reader.get_tile(0).expect("section tile must be readable");
    assert_eq!(header.tile_type, ToTileType::TO_SECTION);
    assert_ne!(header.tile_type, ToTileType::TO_SCAN_AGREEMENT);
    let got = &raw[..header.payload_bytes as usize];
    match decode_to_records(header.tile_type, header.record_layout_version, got).unwrap() {
        ToDecoded::Section(recs) => assert_eq!(recs, vec![section]),
        other => panic!("expected Section, got {:?}", other),
    }

    let _ = std::fs::remove_file(&temp);
}
