//! Step 1 acceptance tests for the `s3a-pack-to` crate.
//!
//! Fixture: `tests/fixtures/synthetic-pump-manual.txt`, a synthetic
//! demonstration manual written for these tests. It is not a Technical
//! Order, not OEM content, and not ETIMS data.
//!
//! Tile allocation (spec amendment to `docs/spec/registry.md` §1):
//! tiles 41..=46 in the standard domain-pack band 32–63. IDs 34–35 are
//! free but 36–40 are already used in-tree by `s3a-vault`, so this pack
//! starts at the next free ID after that cluster.
//!
//! Addon bit: `ADDON_TO = 1 << 8` (`0x0100`), the next unallocated bit
//! after `ADDON_SPATIAL`.
//!
//! Bounding boxes are in page thousandths: x and y in `0..=1000`,
//! w and h in `1..=1000`, and `x + w` and `y + h` must be `<= 1000`.
//! That scale is stated here in the tests (AT-TO-04).
//!
//! Do NOT edit these tests to make them pass.

use bytemuck::{bytes_of, pod_read_unaligned};
use s3a_core::{
    registry::TileTypeRegistry, HyperTileHeader, S3AAddon, S3ACoordinate, ADDON_TO,
    FILE_HEADER_SIZE, HYPER_TILE_HEADER_SIZE,
};
use s3a_engine::{compute_tile_header_crc32, MmapReader, S3ACrudEngine, S3AError};
use s3a_pack_to::{
    decode_to_records, encode_to_fault_edge, encode_to_fault_node, encode_to_figure_anchor,
    encode_to_section, encode_to_spec_cell, encode_to_wcn, register_pack_types, ToCrudExt,
    ToDecoded, ToError, ToFaultEdgeRecord, ToFaultNodeRecord, ToFigureAnchorRecord, ToPack,
    ToSectionRecord, ToSpecCellRecord, ToTileType, ToWcnRecord, NODE_KIND_ACTION,
    NODE_KIND_CONCLUSION, NODE_KIND_TEST, WCN_KIND_CAUTION, WCN_KIND_NOTE, WCN_KIND_WARNING,
};
use sha2::{Digest, Sha256};
use std::io;

/// Synthetic fixture, embedded verbatim from the crate testdata.
const FIXTURE: &str = include_str!("fixtures/synthetic-pump-manual.txt");

/// SHA-256 of the fixture file (verified out-of-band before writing the tests).
const FIXTURE_SHA256_HEX: &str = "f2f25814e2061ecdb9d400dfae18c237c1b7d352f1115bef38dcedadddd85a2b";

const TITLE: &[u8] = b"Demonstration Pump Assembly";
const TITLE_SHA256_HEX: &str = "f79859d2f745df14b28a9bb0abae80fd18a3c56ffd2fd417ac43be4dce1fa923";

const SCOPE: &[u8] =
    b"This synthetic manual describes a fictional demonstration pump used only to exercise Technical Order record layouts.";
const SCOPE_SHA256_HEX: &str = "4390dff56b22186f11cfa4e1e78461de5d0cc8793c30e7412f1a775292e00651";

const WARNING_TEXT: &[u8] =
    b"Do not operate the demonstration pump without the safety cap installed.";
const WARNING_SHA256_HEX: &str = "8282a066268d4dd9d343d021be56872915deb07c037559290ac33313485860c8";

const CAUTION_TEXT: &[u8] = b"Do not exceed the demonstration discharge pressure shown in Table 1.";
const CAUTION_SHA256_HEX: &str = "854d8c78dd9b454f9c0ce2fd45b835666af2a692615703e15481d58bb3bf35ef";

const NOTE_TEXT: &[u8] =
    b"The demonstration pump is a bench model and is not installed on any aircraft.";
const NOTE_SHA256_HEX: &str = "392a84fb45e85e9d1ef071f3b48d7fb8fb643ee730b238ef4762c3dda2b4537e";

const NODE1_TEXT: &[u8] = b"Is the inlet valve open?";
const NODE1_SHA256_HEX: &str = "3359e2ccf800fc336eb8a9a3bef77081acab433263086ff6ae095d4df4ab242f";
const NODE2_TEXT: &[u8] = b"Is discharge pressure within Table 1 limits?";
const NODE2_SHA256_HEX: &str = "d18bc66c507c4fb1331fff49c3f0117bef09b2dc79f257fa80df43fc53846db5";
const NODE3_TEXT: &[u8] = b"Open the inlet valve.";
const NODE3_SHA256_HEX: &str = "7f17b081316d26e9b561aae3b1db8bc2ff5faee975da5b455b97bb97ca86edea";
const NODE4_TEXT: &[u8] = b"Demonstration system is normal.";
const NODE4_SHA256_HEX: &str = "7eaca435f2c9eeed4e27fbf06fac2fcd9fab793d383ddda2b15e7c9b3ee3a4cb";
const NODE5_TEXT: &[u8] = b"Replace the demonstration pressure regulator.";
const NODE5_SHA256_HEX: &str = "2f50ec56a49dce1bc18ab71727434ed719b315becf9bef9f150f064b81153aa0";

const YES: &[u8] = b"YES";
const YES_SHA256_HEX: &str = "f9171164593756e56fb197327b529a4955590566560dbe62d586bff41be9d297";
const NO: &[u8] = b"NO";
const NO_SHA256_HEX: &str = "23794d91c53ae875c8e247d72561e35d9d06ee07c70c9e0dbcc977a6d161504a";

const CELL_NAME: &[u8] = b"Discharge pressure";
const CELL_NAME_SHA256_HEX: &str =
    "bc2c6b5855ac061a524855af6c7b850776ee87a955828213046706701f539d7f";
const CELL_VALUE: &[u8] = b"90";
const CELL_VALUE_SHA256_HEX: &str =
    "69f59c273b6e669ac32a6dd5e1b2cb63333d8b004f9696447aee2d422ce63763";
const UNIT_KPA: &[u8] = b"kPa";
const UNIT_KPA_SHA256_HEX: &str =
    "21bc9ccd4597c4c12c0c98eb0ab915f9e201de89234a48c2ba34be046a8718bc";

const FIG_QUOTE: &[u8] = b"demonstration pump assembly";
const FIG_QUOTE_SHA256_HEX: &str =
    "2bff3c100354134be5c6b74f3878568a99dfb211776e5e50d0a48f277d394835";

/// First 16 bytes of SHA-256(`SYN-DEMO-0001`), used as `doc_id`.
const DOC_ID: [u8; 16] = [
    0x39, 0xb4, 0xb8, 0x85, 0x39, 0x8b, 0x30, 0x90, 0x42, 0xd4, 0xa3, 0xaa, 0xad, 0xfd, 0xd5, 0x36,
];

/// Fixed non-zero origin pointer (opaque 16-byte pointer to the original file).
const ORIGIN_PTR: [u8; 16] = [
    0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32, 0x10, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32, 0x10,
];

/// Page-thousandths scale stated in this test file (AT-TO-04).
const PAGE_THOUSANDTHS_MAX: i32 = 1000;

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

fn sha256_hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn hex_to_32(hex: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        out[i] = u8::from_str_radix(std::str::from_utf8(chunk).unwrap(), 16).unwrap();
    }
    out
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

fn section_record() -> ToSectionRecord {
    encode_to_section(SCOPE, source_id(1), ORIGIN_PTR, DOC_ID, 1, 1, 1, TITLE)
        .expect("scope section must encode")
}

fn warning_record() -> ToWcnRecord {
    encode_to_wcn(
        WARNING_TEXT,
        source_id(2),
        ORIGIN_PTR,
        DOC_ID,
        1,
        1,
        1,
        WCN_KIND_WARNING,
    )
    .expect("warning must encode")
}

/// AT-TO-01: pack metadata, tile IDs 41..=46, addon bit 0x0100, and record sizes.
#[test]
fn at_to_01_pack_metadata_and_record_sizes() {
    assert_eq!(ADDON_TO, 0x0100);
    assert_eq!(ToTileType::TO_SECTION, 41);
    assert_eq!(ToTileType::TO_WCN, 42);
    assert_eq!(ToTileType::TO_FAULT_NODE, 43);
    assert_eq!(ToTileType::TO_FAULT_EDGE, 44);
    assert_eq!(ToTileType::TO_SPEC_CELL, 45);
    assert_eq!(ToTileType::TO_FIGURE_ANCHOR, 46);

    assert_eq!(std::mem::size_of::<ToSectionRecord>(), 128);
    assert_eq!(std::mem::align_of::<ToSectionRecord>(), 8);
    assert_eq!(ToSectionRecord::SIZE, 128);

    assert_eq!(std::mem::size_of::<ToWcnRecord>(), 128);
    assert_eq!(std::mem::align_of::<ToWcnRecord>(), 8);
    assert_eq!(ToWcnRecord::SIZE, 128);

    assert_eq!(std::mem::size_of::<ToFaultNodeRecord>(), 128);
    assert_eq!(std::mem::align_of::<ToFaultNodeRecord>(), 8);
    assert_eq!(ToFaultNodeRecord::SIZE, 128);

    assert_eq!(std::mem::size_of::<ToFaultEdgeRecord>(), 128);
    assert_eq!(std::mem::align_of::<ToFaultEdgeRecord>(), 8);
    assert_eq!(ToFaultEdgeRecord::SIZE, 128);

    assert_eq!(std::mem::size_of::<ToSpecCellRecord>(), 160);
    assert_eq!(std::mem::align_of::<ToSpecCellRecord>(), 8);
    assert_eq!(ToSpecCellRecord::SIZE, 160);

    assert_eq!(std::mem::size_of::<ToFigureAnchorRecord>(), 160);
    assert_eq!(std::mem::align_of::<ToFigureAnchorRecord>(), 8);
    assert_eq!(ToFigureAnchorRecord::SIZE, 160);

    assert_eq!(WCN_KIND_WARNING, 1);
    assert_eq!(WCN_KIND_CAUTION, 2);
    assert_eq!(WCN_KIND_NOTE, 3);
    assert_eq!(NODE_KIND_TEST, 1);
    assert_eq!(NODE_KIND_ACTION, 2);
    assert_eq!(NODE_KIND_CONCLUSION, 3);

    let pack = ToPack;
    assert_eq!(pack.addon_id(), ADDON_TO);
    assert_eq!(pack.name(), "s3a-pack-to");
    assert_eq!(pack.tile_range(), 41..=46);
    assert_eq!(pack.descriptors().len(), 6);

    let mut reg = TileTypeRegistry::new();
    register_pack_types(&mut reg).expect("TO pack types must register");
    for id in 41u32..=46 {
        let desc = reg.get(id).expect("tile type must be registered");
        assert_eq!(desc.required_addon, Some(ADDON_TO));
        assert_eq!(desc.current_layout_version, 1);
        assert_eq!(desc.min_supported_layout_version, 1);
        assert_eq!(desc.owner_pack, "Technical Order Pack");
    }
    assert_eq!(reg.get(41).unwrap().name, "to_section");
    assert_eq!(reg.get(41).unwrap().record_size, 128);
    assert_eq!(reg.get(42).unwrap().name, "to_wcn");
    assert_eq!(reg.get(42).unwrap().record_size, 128);
    assert_eq!(reg.get(43).unwrap().name, "to_fault_node");
    assert_eq!(reg.get(43).unwrap().record_size, 128);
    assert_eq!(reg.get(44).unwrap().name, "to_fault_edge");
    assert_eq!(reg.get(44).unwrap().record_size, 128);
    assert_eq!(reg.get(45).unwrap().name, "to_spec_cell");
    assert_eq!(reg.get(45).unwrap().record_size, 160);
    assert_eq!(reg.get(46).unwrap().name, "to_figure_anchor");
    assert_eq!(reg.get(46).unwrap().record_size, 160);
}

/// AT-TO-02: encode and decode every record type from the synthetic fixture
/// with exact field values and an exact byte round-trip.
#[test]
fn at_to_02_encode_decode_fixture_records() {
    // The fixture spans used as encode inputs must appear verbatim.
    let fixture = FIXTURE.as_bytes();
    for span in [
        TITLE,
        SCOPE,
        WARNING_TEXT,
        CAUTION_TEXT,
        NOTE_TEXT,
        NODE1_TEXT,
        NODE2_TEXT,
        NODE3_TEXT,
        NODE4_TEXT,
        NODE5_TEXT,
        YES,
        NO,
        CELL_NAME,
        CELL_VALUE,
        UNIT_KPA,
        FIG_QUOTE,
    ] {
        assert!(
            fixture.windows(span.len()).any(|w| w == span),
            "fixture is missing span: {:?}",
            std::str::from_utf8(span)
        );
    }

    let section = section_record();
    assert_eq!(section.doc_id, DOC_ID);
    assert_eq!(section.chapter, 1);
    assert_eq!(section.section, 1);
    assert_eq!(section.page, 1);
    assert_eq!(section.title_hash, hex_to_32(TITLE_SHA256_HEX));
    assert_eq!(section.content_hash, hex_to_32(SCOPE_SHA256_HEX));
    assert_eq!(section._pad, [0u8; 4]);
    let back: ToSectionRecord = pod_read_unaligned(bytes_of(&section));
    assert_eq!(back, section);

    let warning = warning_record();
    assert_eq!(warning.kind, WCN_KIND_WARNING);
    assert_eq!(warning.text_hash, hex_to_32(WARNING_SHA256_HEX));
    assert_eq!(warning.content_hash, hex_to_32(WARNING_SHA256_HEX));
    assert_eq!(warning._pad, [0u8; 3]);

    let caution = encode_to_wcn(
        CAUTION_TEXT,
        source_id(3),
        ORIGIN_PTR,
        DOC_ID,
        1,
        1,
        1,
        WCN_KIND_CAUTION,
    )
    .unwrap();
    assert_eq!(caution.kind, WCN_KIND_CAUTION);
    assert_eq!(caution.text_hash, hex_to_32(CAUTION_SHA256_HEX));

    let note = encode_to_wcn(
        NOTE_TEXT,
        source_id(4),
        ORIGIN_PTR,
        DOC_ID,
        1,
        1,
        1,
        WCN_KIND_NOTE,
    )
    .unwrap();
    assert_eq!(note.kind, WCN_KIND_NOTE);
    assert_eq!(note.text_hash, hex_to_32(NOTE_SHA256_HEX));

    let node1 = encode_to_fault_node(
        NODE1_TEXT,
        source_id(5),
        ORIGIN_PTR,
        DOC_ID,
        1,
        NODE_KIND_TEST,
    )
    .unwrap();
    assert_eq!(node1.node_id, 1);
    assert_eq!(node1.kind, NODE_KIND_TEST);
    assert_eq!(node1.label_hash, hex_to_32(NODE1_SHA256_HEX));
    assert_eq!(node1._pad, [0u8; 2]);
    assert_eq!(node1._pad2, [0u8; 4]);

    let node2 = encode_to_fault_node(
        NODE2_TEXT,
        source_id(6),
        ORIGIN_PTR,
        DOC_ID,
        2,
        NODE_KIND_TEST,
    )
    .unwrap();
    let node3 = encode_to_fault_node(
        NODE3_TEXT,
        source_id(7),
        ORIGIN_PTR,
        DOC_ID,
        3,
        NODE_KIND_ACTION,
    )
    .unwrap();
    let node4 = encode_to_fault_node(
        NODE4_TEXT,
        source_id(8),
        ORIGIN_PTR,
        DOC_ID,
        4,
        NODE_KIND_CONCLUSION,
    )
    .unwrap();
    let node5 = encode_to_fault_node(
        NODE5_TEXT,
        source_id(9),
        ORIGIN_PTR,
        DOC_ID,
        5,
        NODE_KIND_ACTION,
    )
    .unwrap();
    assert_eq!(node2.label_hash, hex_to_32(NODE2_SHA256_HEX));
    assert_eq!(node3.kind, NODE_KIND_ACTION);
    assert_eq!(node3.label_hash, hex_to_32(NODE3_SHA256_HEX));
    assert_eq!(node4.kind, NODE_KIND_CONCLUSION);
    assert_eq!(node4.label_hash, hex_to_32(NODE4_SHA256_HEX));
    assert_eq!(node5.label_hash, hex_to_32(NODE5_SHA256_HEX));

    let yes_12 = encode_to_fault_edge(YES, source_id(10), ORIGIN_PTR, DOC_ID, 1, 2).unwrap();
    assert_eq!(yes_12.from_node_id, 1);
    assert_eq!(yes_12.to_node_id, 2);
    assert_eq!(yes_12.outcome_hash, hex_to_32(YES_SHA256_HEX));

    let no_13 = encode_to_fault_edge(NO, source_id(11), ORIGIN_PTR, DOC_ID, 1, 3).unwrap();
    assert_eq!(no_13.outcome_hash, hex_to_32(NO_SHA256_HEX));

    let cell_value =
        encode_to_spec_cell(CELL_VALUE, source_id(12), ORIGIN_PTR, 1, 1, 2, UNIT_KPA).unwrap();
    assert_eq!(cell_value.table_id, 1);
    assert_eq!(cell_value.row, 1);
    assert_eq!(cell_value.col, 2);
    assert_eq!(cell_value.value_hash, hex_to_32(CELL_VALUE_SHA256_HEX));
    assert_eq!(cell_value.unit_hash, hex_to_32(UNIT_KPA_SHA256_HEX));
    assert_eq!(cell_value._pad, [0u8; 16]);

    let cell_name =
        encode_to_spec_cell(CELL_NAME, source_id(13), ORIGIN_PTR, 1, 1, 1, b"").unwrap();
    assert_eq!(cell_name.value_hash, hex_to_32(CELL_NAME_SHA256_HEX));
    assert_eq!(cell_name.unit_hash, sha256(b""));

    let figure = encode_to_figure_anchor(
        FIG_QUOTE,
        source_id(14),
        ORIGIN_PTR,
        DOC_ID,
        4,
        1,
        4,
        1,
        100,
        150,
        400,
        300,
    )
    .unwrap();
    assert_eq!(figure.figure_id, 1);
    assert_eq!(figure.bbox_x, 100);
    assert_eq!(figure.bbox_y, 150);
    assert_eq!(figure.bbox_w, 400);
    assert_eq!(figure.bbox_h, 300);
    assert_eq!(figure.quote_hash, hex_to_32(FIG_QUOTE_SHA256_HEX));
    assert_eq!(figure._pad, [0u8; 16]);
    let back: ToFigureAnchorRecord = pod_read_unaligned(bytes_of(&figure));
    assert_eq!(back, figure);
}

/// AT-TO-03: content_hash is SHA-256 of the provided source bytes; source_id
/// and origin_ptr are present and non-zero. Missing any one fails.
#[test]
fn at_to_03_content_hash_and_provenance() {
    assert_eq!(sha256_hex(&sha256(FIXTURE.as_bytes())), FIXTURE_SHA256_HEX);
    assert_eq!(sha256_hex(&sha256(TITLE)), TITLE_SHA256_HEX);
    assert_eq!(&sha256(b"SYN-DEMO-0001")[..16], &DOC_ID);

    let section = section_record();
    assert_eq!(section.content_hash, sha256(SCOPE));
    assert_eq!(sha256_hex(&section.content_hash), SCOPE_SHA256_HEX);
    assert_eq!(section.source_id, source_id(1));
    assert_ne!(section.source_id, [0u8; 16]);
    assert_eq!(section.origin_ptr, ORIGIN_PTR);
    assert_ne!(section.origin_ptr, [0u8; 16]);

    let err = encode_to_section(SCOPE, [0u8; 16], ORIGIN_PTR, DOC_ID, 1, 1, 1, TITLE)
        .expect_err("all-zero source_id must be rejected");
    assert_eq!(err, ToError::MissingSourceId);

    let err = encode_to_section(SCOPE, source_id(1), [0u8; 16], DOC_ID, 1, 1, 1, TITLE)
        .expect_err("all-zero origin_ptr must be rejected");
    assert_eq!(err, ToError::MissingOriginPtr);

    let err = encode_to_section(SCOPE, source_id(1), ORIGIN_PTR, [0u8; 16], 1, 1, 1, TITLE)
        .expect_err("all-zero doc_id must be rejected");
    assert_eq!(err, ToError::MissingDocId);

    let err = encode_to_wcn(
        WARNING_TEXT,
        [0u8; 16],
        ORIGIN_PTR,
        DOC_ID,
        1,
        1,
        1,
        WCN_KIND_WARNING,
    )
    .expect_err("all-zero source_id must be rejected on WCN");
    assert_eq!(err, ToError::MissingSourceId);
}

/// AT-TO-04: decoding and encoding fail closed on unknown kinds, truncated
/// records, empty text, zero table/node/figure ids, bad bounding boxes, and
/// self-loop edges.
#[test]
fn at_to_04_fail_closed_on_invalid_records() {
    let err = encode_to_wcn(WARNING_TEXT, source_id(2), ORIGIN_PTR, DOC_ID, 1, 1, 1, 0)
        .expect_err("kind 0 must be rejected");
    assert_eq!(err, ToError::UnknownWcnKind { kind: 0 });

    let err = encode_to_wcn(WARNING_TEXT, source_id(2), ORIGIN_PTR, DOC_ID, 1, 1, 1, 4)
        .expect_err("kind 4 must be rejected");
    assert_eq!(err, ToError::UnknownWcnKind { kind: 4 });

    let err = encode_to_wcn(
        b"",
        source_id(2),
        ORIGIN_PTR,
        DOC_ID,
        1,
        1,
        1,
        WCN_KIND_WARNING,
    )
    .expect_err("empty WCN text must be rejected");
    assert_eq!(err, ToError::EmptyText);

    let err = encode_to_section(SCOPE, source_id(1), ORIGIN_PTR, DOC_ID, 1, 1, 1, b"")
        .expect_err("empty title must be rejected");
    assert_eq!(err, ToError::EmptyText);

    let err = encode_to_fault_node(
        NODE1_TEXT,
        source_id(5),
        ORIGIN_PTR,
        DOC_ID,
        0,
        NODE_KIND_TEST,
    )
    .expect_err("node_id 0 must be rejected");
    assert_eq!(err, ToError::ZeroNodeId);

    let err = encode_to_fault_node(NODE1_TEXT, source_id(5), ORIGIN_PTR, DOC_ID, 1, 0)
        .expect_err("node kind 0 must be rejected");
    assert_eq!(err, ToError::UnknownNodeKind { kind: 0 });

    let err = encode_to_fault_edge(YES, source_id(10), ORIGIN_PTR, DOC_ID, 1, 1)
        .expect_err("self-loop edge must be rejected");
    assert_eq!(
        err,
        ToError::SelfLoopEdge {
            from_node_id: 1,
            to_node_id: 1
        }
    );

    let err = encode_to_fault_edge(YES, source_id(10), ORIGIN_PTR, DOC_ID, 0, 2)
        .expect_err("from_node_id 0 must be rejected");
    assert_eq!(err, ToError::ZeroNodeId);

    let err = encode_to_spec_cell(CELL_VALUE, source_id(12), ORIGIN_PTR, 0, 1, 2, UNIT_KPA)
        .expect_err("table_id 0 must be rejected");
    assert_eq!(err, ToError::ZeroTableId);

    let err = encode_to_spec_cell(CELL_VALUE, source_id(12), ORIGIN_PTR, 1, 0, 2, UNIT_KPA)
        .expect_err("row 0 must be rejected");
    assert_eq!(err, ToError::ZeroTableIndex);

    let err = encode_to_spec_cell(CELL_VALUE, source_id(12), ORIGIN_PTR, 1, 1, 0, UNIT_KPA)
        .expect_err("col 0 must be rejected");
    assert_eq!(err, ToError::ZeroTableIndex);

    let err = encode_to_figure_anchor(
        FIG_QUOTE,
        source_id(14),
        ORIGIN_PTR,
        DOC_ID,
        4,
        1,
        4,
        0,
        100,
        150,
        400,
        300,
    )
    .expect_err("figure_id 0 must be rejected");
    assert_eq!(err, ToError::ZeroFigureId);

    let err = encode_to_figure_anchor(
        FIG_QUOTE,
        source_id(14),
        ORIGIN_PTR,
        DOC_ID,
        4,
        1,
        4,
        1,
        100,
        150,
        0,
        300,
    )
    .expect_err("non-positive width must be rejected");
    assert_eq!(err, ToError::NonPositiveBBox);

    let err = encode_to_figure_anchor(
        FIG_QUOTE,
        source_id(14),
        ORIGIN_PTR,
        DOC_ID,
        4,
        1,
        4,
        1,
        800,
        150,
        400,
        300,
    )
    .expect_err("box past the page edge must be rejected");
    assert_eq!(
        err,
        ToError::BBoxOutOfPage {
            max: PAGE_THOUSANDTHS_MAX
        }
    );

    let err = encode_to_figure_anchor(
        FIG_QUOTE,
        source_id(14),
        ORIGIN_PTR,
        DOC_ID,
        4,
        1,
        4,
        1,
        -1,
        150,
        400,
        300,
    )
    .expect_err("negative origin must be rejected");
    assert_eq!(
        err,
        ToError::BBoxOutOfPage {
            max: PAGE_THOUSANDTHS_MAX
        }
    );

    let section = section_record();
    let bytes = bytes_of(&section);
    assert_eq!(bytes.len(), 128);
    let err = decode_to_records(ToTileType::TO_SECTION, 1, &bytes[..127])
        .expect_err("truncated section payload must be rejected");
    match err {
        ToError::TruncatedRecord { expected, actual } => {
            assert_eq!(expected, 128);
            assert_eq!(actual, 127);
        }
        other => panic!("wrong error for truncated record: {:?}", other),
    }

    let cell =
        encode_to_spec_cell(CELL_VALUE, source_id(12), ORIGIN_PTR, 1, 1, 2, UNIT_KPA).unwrap();
    let err = decode_to_records(ToTileType::TO_SPEC_CELL, 1, &bytes_of(&cell)[..159])
        .expect_err("truncated spec-cell payload must be rejected");
    match err {
        ToError::TruncatedRecord { expected, actual } => {
            assert_eq!(expected, 160);
            assert_eq!(actual, 159);
        }
        other => panic!("wrong error for truncated spec cell: {:?}", other),
    }
}

/// AT-TO-05: append without the addon_mask bit 0x0100 returns permission denied.
#[test]
fn at_to_05_append_without_addon_bit_is_permission_denied() {
    assert_eq!(ADDON_TO, 0x0100);

    let temp = temp_archive("at05");
    let engine = S3ACrudEngine::open_or_create(&temp).unwrap();
    let record = section_record();

    let err = engine
        .create_to_section_records(&[record])
        .expect_err("append without ADDON_TO must fail");
    assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);

    let reader = MmapReader::open(&temp).unwrap();
    assert_eq!(reader.tile_count(), 0);

    engine.enable_addon(ADDON_TO).unwrap();
    let tile_id = engine.create_to_section_records(&[record]).unwrap();
    assert_eq!(tile_id, 1);

    let reader = MmapReader::open(&temp).unwrap();
    assert_eq!(reader.tile_count(), 1);

    let _ = std::fs::remove_file(&temp);
}

/// AT-TO-06: every fixture record type round-trips through an S3A hyper-tile
/// and reads back as the same Pod bytes.
#[test]
fn at_to_06_roundtrip_fixture_through_s3a_tiles() {
    let temp = temp_archive("at06");
    let engine = S3ACrudEngine::open_or_create(&temp).unwrap();
    engine.enable_addon(ADDON_TO).unwrap();

    let section = section_record();
    let warning = warning_record();
    let node1 = encode_to_fault_node(
        NODE1_TEXT,
        source_id(5),
        ORIGIN_PTR,
        DOC_ID,
        1,
        NODE_KIND_TEST,
    )
    .unwrap();
    let node3 = encode_to_fault_node(
        NODE3_TEXT,
        source_id(7),
        ORIGIN_PTR,
        DOC_ID,
        3,
        NODE_KIND_ACTION,
    )
    .unwrap();
    let edge = encode_to_fault_edge(NO, source_id(11), ORIGIN_PTR, DOC_ID, 1, 3).unwrap();
    let cell =
        encode_to_spec_cell(CELL_VALUE, source_id(12), ORIGIN_PTR, 1, 1, 2, UNIT_KPA).unwrap();
    let figure = encode_to_figure_anchor(
        FIG_QUOTE,
        source_id(14),
        ORIGIN_PTR,
        DOC_ID,
        4,
        1,
        4,
        1,
        100,
        150,
        400,
        300,
    )
    .unwrap();

    engine.create_to_section_records(&[section]).unwrap();
    engine.create_to_wcn_records(&[warning]).unwrap();
    engine
        .create_to_fault_node_records(&[node1, node3])
        .unwrap();
    engine.create_to_fault_edge_records(&[edge]).unwrap();
    engine.create_to_spec_cell_records(&[cell]).unwrap();
    engine.create_to_figure_anchor_records(&[figure]).unwrap();

    let reader = MmapReader::open(&temp).unwrap();
    assert_eq!(reader.tile_count(), 6);

    let expected = [
        (ToTileType::TO_SECTION, bytes_of(&section).to_vec()),
        (ToTileType::TO_WCN, bytes_of(&warning).to_vec()),
        (
            ToTileType::TO_FAULT_NODE,
            [bytes_of(&node1), bytes_of(&node3)].concat(),
        ),
        (ToTileType::TO_FAULT_EDGE, bytes_of(&edge).to_vec()),
        (ToTileType::TO_SPEC_CELL, bytes_of(&cell).to_vec()),
        (ToTileType::TO_FIGURE_ANCHOR, bytes_of(&figure).to_vec()),
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
            (t, ToDecoded::Section(recs)) if *t == ToTileType::TO_SECTION => {
                assert_eq!(recs, vec![section]);
            }
            (t, ToDecoded::Wcn(recs)) if *t == ToTileType::TO_WCN => {
                assert_eq!(recs, vec![warning]);
            }
            (t, ToDecoded::FaultNode(recs)) if *t == ToTileType::TO_FAULT_NODE => {
                assert_eq!(recs, vec![node1, node3]);
            }
            (t, ToDecoded::FaultEdge(recs)) if *t == ToTileType::TO_FAULT_EDGE => {
                assert_eq!(recs, vec![edge]);
            }
            (t, ToDecoded::SpecCell(recs)) if *t == ToTileType::TO_SPEC_CELL => {
                assert_eq!(recs, vec![cell]);
            }
            (t, ToDecoded::FigureAnchor(recs)) if *t == ToTileType::TO_FIGURE_ANCHOR => {
                assert_eq!(recs, vec![figure]);
            }
            other => panic!("unexpected decode for tile {}: {:?}", i, other),
        }
    }

    let _ = std::fs::remove_file(&temp);
}

/// AT-TO-07: a tile whose `record_layout_version` is newer than 1 is
/// rejected fail-closed by the pack decoder and by the engine reader.
#[test]
fn at_to_07_unsupported_layout_version_fails_closed() {
    let record = section_record();
    let err = decode_to_records(ToTileType::TO_SECTION, 99, bytes_of(&record))
        .expect_err("layout version 99 must be rejected");
    match err {
        ToError::UnsupportedLayoutVersion { tile_type, version } => {
            assert_eq!(tile_type, 41);
            assert_eq!(version, 99);
        }
        other => panic!("wrong error for new layout version: {:?}", other),
    }

    // Version 0 is the pre-Phase-2 alias for version 1 and must still decode.
    let decoded = decode_to_records(ToTileType::TO_SECTION, 0, bytes_of(&record))
        .expect("layout version 0 aliases to 1");
    match decoded {
        ToDecoded::Section(recs) => assert_eq!(recs, vec![record]),
        other => panic!("unexpected decode: {:?}", other),
    }

    let temp = temp_archive("at07");
    let engine = S3ACrudEngine::open_or_create(&temp).unwrap();
    engine.enable_addon(ADDON_TO).unwrap();
    engine.create_to_section_records(&[record]).unwrap();
    drop(engine);

    {
        use std::io::{Read, Seek, SeekFrom, Write};
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&temp)
            .unwrap();
        file.seek(SeekFrom::Start(FILE_HEADER_SIZE as u64)).unwrap();
        let mut header_bytes = vec![0u8; HYPER_TILE_HEADER_SIZE];
        file.read_exact(&mut header_bytes).unwrap();
        let mut header: HyperTileHeader = pod_read_unaligned(&header_bytes);
        header.record_layout_version = 99;
        header.header_crc32 = compute_tile_header_crc32(&header);
        file.seek(SeekFrom::Start(FILE_HEADER_SIZE as u64)).unwrap();
        file.write_all(bytes_of(&header)).unwrap();
        file.flush().unwrap();
    }

    let reader = MmapReader::open(&temp).unwrap();
    let (header, _) = reader
        .get_tile(0)
        .expect("patched tile must still be listed");
    assert_eq!(header.record_layout_version, 99);
    // read_record_by_coordinate treats the coordinate tile_id as the slot index.
    let res: Result<ToSectionRecord, S3AError> =
        reader.read_record_by_coordinate(&S3ACoordinate::new(0, 0, 0));
    match res {
        Err(S3AError::UnsupportedLayoutVersion(v)) => assert_eq!(v, 99),
        other => panic!(
            "Expected UnsupportedLayoutVersion(99) from engine, got {:?}",
            other
        ),
    }

    let _ = std::fs::remove_file(&temp);
}

/// AT-TO-08: the pack decoder rejects an unknown tile type fail-closed.
#[test]
fn at_to_08_unknown_tile_type_fails_closed() {
    let record = section_record();
    let err =
        decode_to_records(99, 1, bytes_of(&record)).expect_err("tile type 99 must be rejected");
    assert_eq!(err, ToError::UnknownTileType(99));

    // A known non-TO type (telemetry = 0) is also rejected by this pack.
    let err =
        decode_to_records(0, 1, bytes_of(&record)).expect_err("telemetry tiles are not TO records");
    assert_eq!(err, ToError::UnknownTileType(0));
}

/// AT-TO-09: the fixture is synthetic and is not ETIMS or official TO text.
#[test]
fn at_to_09_fixture_is_synthetic_only() {
    let fixture = FIXTURE;
    assert!(fixture.contains("SYNTHETIC MAINTENANCE MANUAL"));
    assert!(fixture.contains("NOT A TECHNICAL ORDER"));
    assert!(fixture.contains("no ETIMS content"));
    assert!(fixture.contains("SYN-DEMO-0001"));
    assert!(!fixture.contains("T.O."));
    assert_eq!(sha256_hex(&sha256(fixture.as_bytes())), FIXTURE_SHA256_HEX);
}
