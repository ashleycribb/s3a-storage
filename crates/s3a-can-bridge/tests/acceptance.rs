//! Acceptance tests for the `s3a-can-bridge` crate.
//!
//! Fixtures in `tests/fixtures/` are synthetic demonstration data written
//! for these tests. They are not vehicle captures, not Technical Orders,
//! not OEM content, and not ETIMS data.
//!
//! Tile allocation (spec amendment to `docs/spec/registry.md` §1):
//! tiles 47..=48, the next free IDs after the Technical Order pack
//! (tiles 41..=46).
//!
//! Addon bit: `ADDON_CAN = 1 << 9` (`0x0200`), the next unallocated bit
//! after `ADDON_TO`. The bit is required because this crate writes its
//! own pack tiles; core must never depend on this crate.
//!
//! Record layouts, layout version 1, padding bytes must be zero:
//!
//! `CanDtcRecord` (tile 47, `can_dtc`, 128 B):
//! | Off | Len | Field |
//! |---:|---:|---|
//! | 0 | 16 | `source_id` |
//! | 16 | 32 | `content_hash` (SHA-256 of the 4-byte J1939-73 DTC slot, or of the 2-byte OBD DTC) |
//! | 48 | 16 | `origin_ptr` |
//! | 64 | 32 | `log_hash` (SHA-256 of the complete source log bytes) |
//! | 96 | 8 | `frame_offset` (0-based index in the parsed frame list; for BAM, the TP.CM frame) |
//! | 104 | 4 | `can_id` |
//! | 108 | 4 | `pgn` (0 for OBD-II) |
//! | 112 | 4 | `spn` (0 for OBD-II) |
//! | 116 | 1 | `fmi` |
//! | 117 | 1 | `occurrence_count` |
//! | 118 | 1 | `conversion_method` |
//! | 119 | 1 | `kind` (1=J1939 DM1, 2=OBD-II Mode 03) |
//! | 120 | 1 | `source_address` (0 for OBD-II) |
//! | 121 | 1 | `_pad0` (must be zero) |
//! | 122 | 2 | `obd_raw` (0 for J1939; SAE J2012 16-bit encoding for OBD) |
//! | 124 | 4 | `_pad` (must be zero) |
//!
//! `CanTriggerRecord` (tile 48, `can_trigger`, 128 B):
//! | Off | Len | Field |
//! |---:|---:|---|
//! | 0 | 16 | `source_id` |
//! | 16 | 32 | `content_hash` (SHA-256 of the canonical key: `j1939:{spn}:{fmi}`, `obd:{P|C|B|U}{dddd}`, or `param:{spn}`) |
//! | 48 | 16 | `origin_ptr` |
//! | 64 | 32 | `log_hash` |
//! | 96 | 8 | `frame_offset` |
//! | 104 | 8 | `fault_node_id` (s3a-pack-to node id, must be non-zero) |
//! | 112 | 4 | `spn` (0 for OBD) |
//! | 116 | 4 | `observed_raw` (kind=param only; 0 otherwise) |
//! | 120 | 2 | `fmi` (0 when not a J1939 FMI mapping) |
//! | 122 | 2 | `obd_raw` (0 when not an OBD mapping) |
//! | 124 | 1 | `kind` (1=J1939 SPN/FMI, 2=OBD DTC, 3=out-of-range parameter) |
//! | 125 | 3 | `_pad` (must be zero) |
//!
//! Timestamps in candump and CSV logs are Unix seconds with a decimal
//! fraction. They are parsed as decimal text, not IEEE floats: the
//! integer part is multiplied by 1_000_000_000 and the fractional digits
//! are padded or accepted up to 9 digits as nanoseconds. More than 9
//! fractional digits is rejected. Vector ASC timestamps use the same
//! decimal rule on the leading numeric field and are not converted
//! through the date line (no timezone is invented).
//!
//! candump ID width: 1..=3 hex digits is an 11-bit standard ID; 8 hex
//! digits is a 29-bit extended ID. Any other width is rejected.
//! The same width rule applies to the CSV `can_id` column (no `0x`
//! prefix). In Vector ASC, a trailing `x` marks an extended 29-bit ID.
//!
//! .blf logs are out of scope.
//!
//! Do NOT edit these tests to make them pass.

use bytemuck::{bytes_of, pod_read_unaligned};
use s3a_can_bridge::{
    decode_can_records, decode_dm1, decode_j1939_dtcs, encode_can_dtc, encode_can_trigger,
    format_obd_dtc, ingest_candump, parse_can_csv, parse_candump, parse_j1939_id, parse_obd_mode03,
    parse_vector_asc, register_pack_types, CanBridgePack, CanCrudExt, CanDecoded, CanDtcRecord,
    CanError, CanTileType, CanTriggerRecord, MappingTable, ThresholdTable, TriggerResult,
    DTC_KIND_J1939_DM1, DTC_KIND_OBD_MODE03, SOCKETCAN_ENABLED, TRIGGER_KIND_J1939,
    TRIGGER_KIND_OBD, TRIGGER_KIND_PARAM,
};
use s3a_core::{
    registry::TileTypeRegistry, required_addon_for_tile, HyperTileHeader, S3AAddon, S3ACoordinate,
    ADDON_CAN, ADDON_TO, FILE_HEADER_SIZE, HYPER_TILE_HEADER_SIZE,
};
use s3a_engine::{
    compute_tile_header_crc32, BackpressurePolicy, L0RingBuffer, MmapReader, S3ACrudEngine,
    S3AError,
};
use s3a_pack_to::{encode_to_fault_node, ToCrudExt, NODE_KIND_ACTION, NODE_KIND_TEST};
use sha2::{Digest, Sha256};
use std::io;

const CANDUMP: &str = include_str!("fixtures/synthetic-pump-candump.log");
const CSV: &str = include_str!("fixtures/synthetic-pump-can.csv");
const ASC: &str = include_str!("fixtures/synthetic-pump-can.asc");
const MAPPING: &str = include_str!("fixtures/mapping.csv");
const THRESHOLDS: &str = include_str!("fixtures/thresholds.csv");
const MANUAL: &str = include_str!("fixtures/synthetic-pump-fault-isolation.txt");

const CANDUMP_SHA256_HEX: &str = "dcbdfa147368b9a995f1c03c6f316291979d816bb7d4fa31267eb37c36421a3c";
const CSV_SHA256_HEX: &str = "81d795a2a08ae9d264b7899dc772206e8134c52dc6e892ed3b54a2476312b555";
const ASC_SHA256_HEX: &str = "989c3edc1b61233a0310f479ba0751756956736f462f31671c1e8e1f3c4901eb";
const MAPPING_SHA256_HEX: &str = "53b9d7a48716984507df38867b4848114962d04065df2aef29eb5c8d744e7051";
const THRESHOLDS_SHA256_HEX: &str =
    "31facc018477430439193cbc5adeac841d147ac31c24a49c777d5fe222945762";
const MANUAL_SHA256_HEX: &str = "e248b01e5f7f34ebe26a0629ee2a70ce915129f533b79b781aa3e6d4a966276e";

const DTC_110_16_SLOT: [u8; 4] = [0x6E, 0x00, 0x10, 0x03];
const DTC_94_1_SLOT: [u8; 4] = [0x5E, 0x00, 0x01, 0x01];
const DTC_190_3_SLOT: [u8; 4] = [0xBE, 0x00, 0x03, 0x05];
const OBD_P0133_RAW: u16 = 0x0133;
const OBD_P0133_BYTES: [u8; 2] = [0x01, 0x33];

const DTC_110_16_SHA256_HEX: &str =
    "9bb5e3d247e02c1bc5a2ff16dfd90cbbf76ea3d98e42d0fe1350b56f261405dc";
const DTC_94_1_SHA256_HEX: &str =
    "cf7516b9863b44fa6e46fcf3271f4584527025e5e0d88917af4af40261ae0741";
const DTC_190_3_SHA256_HEX: &str =
    "35eee23397aeb21a59f7224a20ed6819407eabf20070ce3f8eae4b9ba196f6a4";
const OBD_P0133_SHA256_HEX: &str =
    "4efb97e7cc4520b9d437ce251ecdd94e243437933cdfa44ec76ec52e5a9a9b04";
const TRIG_J1939_110_16_SHA256_HEX: &str =
    "e75d044142811371022270a8e9d692244482ba8f540481190d4ffe4b665e0133";
const TRIG_OBD_P0133_SHA256_HEX: &str =
    "322ffc2eca18556b8d34583142284384ac5a8628b4305d3bf8263fc8056f4a3d";
const TRIG_PARAM_110_SHA256_HEX: &str =
    "c6280e594efde57a1854f0d4c6d6b9920e053d6e9f33d0eeb4f4cbbecd9d677a";

const TS0: u64 = 1_609_430_400_000_000_000;
const TS1: u64 = 1_609_430_400_010_000_000;
const TS2: u64 = 1_609_430_400_020_000_000;
const TS3: u64 = 1_609_430_400_030_000_000;
const TS4: u64 = 1_609_430_400_040_000_000;
const TS5: u64 = 1_609_430_400_041_000_000;
const TS6: u64 = 1_609_430_400_042_000_000;

const ORIGIN_PTR: [u8; 16] = [
    0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32, 0x10, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32, 0x10,
];

fn source_id(n: u8) -> [u8; 16] {
    let mut id = [0u8; 16];
    id[0] = 0xca;
    id[1] = 0xb1;
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
        "s3a_test_can_bridge_{}_{}_{}.s3a",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn expected_candump_ids_and_data() -> [(u64, u32, bool, [u8; 8]); 7] {
    [
        (
            TS0,
            0x18FECA00,
            true,
            [0x04, 0x00, 0x6E, 0x00, 0x10, 0x03, 0xFF, 0xFF],
        ),
        (
            TS1,
            0x18FEEE00,
            true,
            [0x50, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        ),
        (
            TS2,
            0x18FEEE00,
            true,
            [0xC8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        ),
        (
            TS3,
            0x7E8,
            false,
            [0x03, 0x43, 0x01, 0x33, 0x00, 0x00, 0x00, 0x00],
        ),
        (
            TS4,
            0x1CECFF21,
            true,
            [0x20, 0x0E, 0x00, 0x02, 0xFF, 0xCA, 0xFE, 0x00],
        ),
        (
            TS5,
            0x1CEBFF21,
            true,
            [0x01, 0x04, 0x00, 0x6E, 0x00, 0x10, 0x03, 0x5E],
        ),
        (
            TS6,
            0x1CEBFF21,
            true,
            [0x02, 0x00, 0x01, 0x01, 0xBE, 0x00, 0x03, 0x05],
        ),
    ]
}

/// AT-CAN-01: pack metadata, tile IDs 47..=48, addon bit 0x0200, and record sizes.
#[test]
fn at_can_01_pack_metadata_and_record_sizes() {
    assert_eq!(ADDON_CAN, 0x0200);
    assert_eq!(CanTileType::CAN_DTC, 47);
    assert_eq!(CanTileType::CAN_TRIGGER, 48);

    assert_eq!(std::mem::size_of::<CanDtcRecord>(), 128);
    assert_eq!(std::mem::align_of::<CanDtcRecord>(), 8);
    assert_eq!(CanDtcRecord::SIZE, 128);

    assert_eq!(std::mem::size_of::<CanTriggerRecord>(), 128);
    assert_eq!(std::mem::align_of::<CanTriggerRecord>(), 8);
    assert_eq!(CanTriggerRecord::SIZE, 128);

    assert_eq!(DTC_KIND_J1939_DM1, 1);
    assert_eq!(DTC_KIND_OBD_MODE03, 2);
    assert_eq!(TRIGGER_KIND_J1939, 1);
    assert_eq!(TRIGGER_KIND_OBD, 2);
    assert_eq!(TRIGGER_KIND_PARAM, 3);

    let pack = CanBridgePack;
    assert_eq!(pack.addon_id(), ADDON_CAN);
    assert_eq!(pack.name(), "s3a-can-bridge");
    assert_eq!(pack.tile_range(), 47..=48);
    assert_eq!(pack.descriptors().len(), 2);

    let mut reg = TileTypeRegistry::new();
    register_pack_types(&mut reg).expect("CAN bridge types must register");
    for id in 47u32..=48 {
        let desc = reg.get(id).expect("tile type must be registered");
        assert_eq!(desc.required_addon, Some(ADDON_CAN));
        assert_eq!(desc.current_layout_version, 1);
        assert_eq!(desc.min_supported_layout_version, 1);
        assert_eq!(desc.owner_pack, "CAN Bridge");
        assert_eq!(desc.record_size, 128);
    }
    assert_eq!(reg.get(47).unwrap().name, "can_dtc");
    assert_eq!(reg.get(48).unwrap().name, "can_trigger");

    let (name, mask) = required_addon_for_tile(47).expect("tile 47 must require an addon");
    assert_eq!(name, "s3a-can-bridge");
    assert_eq!(mask, ADDON_CAN);
    let (name, mask) = required_addon_for_tile(48).expect("tile 48 must require an addon");
    assert_eq!(name, "s3a-can-bridge");
    assert_eq!(mask, ADDON_CAN);
    assert!(required_addon_for_tile(0).is_none());
}

/// AT-CAN-02: parse 29-bit J1939 IDs (PDU1 and PDU2) and decode a single-frame DM1.
#[test]
fn at_can_02_j1939_id_and_single_frame_dm1() {
    let id = parse_j1939_id(0x18FECA00).expect("DM1 ID must parse");
    assert_eq!(id.priority, 6);
    assert_eq!(id.reserved, 0);
    assert_eq!(id.data_page, 0);
    assert_eq!(id.pdu_format, 0xFE);
    assert_eq!(id.pdu_specific, 0xCA);
    assert_eq!(id.source_address, 0x00);
    assert_eq!(id.pgn, 65226);

    let id = parse_j1939_id(0x1CECFF21).expect("TP.CM ID must parse");
    assert_eq!(id.priority, 7);
    assert_eq!(id.reserved, 0);
    assert_eq!(id.data_page, 0);
    assert_eq!(id.pdu_format, 0xEC);
    assert_eq!(id.pdu_specific, 0xFF);
    assert_eq!(id.source_address, 0x21);
    assert_eq!(id.pgn, 60416);

    let err = parse_j1939_id(0x7E8).expect_err("11-bit IDs are not J1939 29-bit IDs");
    assert_eq!(err, CanError::NotJ1939Id { can_id: 0x7E8 });

    let err = parse_j1939_id(0x20000000).expect_err("bits above 28 must be rejected");
    assert_eq!(err, CanError::NotJ1939Id { can_id: 0x20000000 });

    let dtcs = decode_dm1(&[0x04, 0x00, 0x6E, 0x00, 0x10, 0x03, 0xFF, 0xFF])
        .expect("single-frame DM1 must decode");
    assert_eq!(dtcs.len(), 1);
    assert_eq!(dtcs[0].spn, 110);
    assert_eq!(dtcs[0].fmi, 16);
    assert_eq!(dtcs[0].occurrence_count, 3);
    assert_eq!(dtcs[0].conversion_method, 0);
    assert_eq!(dtcs[0].slot, DTC_110_16_SLOT);

    let none = decode_dm1(&[0x04, 0x00]).expect("lamp-only DM1 has zero DTCs");
    assert!(none.is_empty());

    let none = decode_dm1(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF])
        .expect("all-zero and 0xFFFF slots are not DTCs");
    assert!(none.is_empty());

    let err = decode_dm1(&[0x04, 0x00, 0x6E]).expect_err("truncated DTC slot must fail");
    assert_eq!(err, CanError::ShortPayload);
}

/// AT-CAN-03: BAM reassembly of a 3-DTC DM1; incomplete, out-of-order, and
/// orphan transport frames fail closed.
#[test]
fn at_can_03_bam_dm1_and_fail_closed_transport() {
    let frames = parse_candump(CANDUMP).expect("fixture candump must parse");
    let bam = &frames[4..7];
    let dtcs = decode_j1939_dtcs(bam).expect("BAM DM1 must reassemble");
    assert_eq!(dtcs.len(), 3);

    assert_eq!(dtcs[0].frame_offset, 0);
    assert_eq!(dtcs[0].can_id, 0x1CECFF21);
    assert_eq!(dtcs[0].source_address, 0x21);
    assert_eq!(dtcs[0].pgn, 65226);
    assert_eq!(dtcs[0].spn, 110);
    assert_eq!(dtcs[0].fmi, 16);
    assert_eq!(dtcs[0].occurrence_count, 3);
    assert_eq!(dtcs[0].conversion_method, 0);
    assert_eq!(dtcs[0].slot, DTC_110_16_SLOT);

    assert_eq!(dtcs[1].spn, 94);
    assert_eq!(dtcs[1].fmi, 1);
    assert_eq!(dtcs[1].occurrence_count, 1);
    assert_eq!(dtcs[1].slot, DTC_94_1_SLOT);

    assert_eq!(dtcs[2].spn, 190);
    assert_eq!(dtcs[2].fmi, 3);
    assert_eq!(dtcs[2].occurrence_count, 5);
    assert_eq!(dtcs[2].slot, DTC_190_3_SLOT);

    let incomplete = decode_j1939_dtcs(&bam[..2]).expect_err("missing TP.DT must fail");
    assert_eq!(incomplete, CanError::IncompleteBam);

    let out_of_order = decode_j1939_dtcs(&[bam[0].clone(), bam[2].clone(), bam[1].clone()])
        .expect_err("out-of-order TP.DT must fail");
    assert_eq!(out_of_order, CanError::BamSequence);

    let orphan = decode_j1939_dtcs(&bam[1..2]).expect_err("TP.DT without TP.CM must fail");
    assert_eq!(orphan, CanError::UnexpectedTpDt);

    let all = decode_j1939_dtcs(&frames).expect("full log must decode J1939 DTCs");
    // Frame 0 single-frame DM1 (1 DTC) + BAM (3 DTCs). Temperature frames are not DTCs.
    assert_eq!(all.len(), 4);
    assert_eq!(all[0].frame_offset, 0);
    assert_eq!(all[0].can_id, 0x18FECA00);
    assert_eq!(all[0].source_address, 0);
    assert_eq!(all[1].frame_offset, 4);
    assert_eq!(all[1].spn, 110);
    assert_eq!(all[3].frame_offset, 4);
    assert_eq!(all[3].spn, 190);
}

/// AT-CAN-04: OBD-II Mode 03 P/C/B/U codes; padding 0x0000 is ignored;
/// invalid responses fail closed.
#[test]
fn at_can_04_obd_mode03() {
    assert_eq!(format_obd_dtc(0x0133), *b"P0133");
    assert_eq!(format_obd_dtc(0x0420), *b"P0420");
    assert_eq!(format_obd_dtc(0x4133), *b"C0133");
    assert_eq!(format_obd_dtc(0x8133), *b"B0133");
    assert_eq!(format_obd_dtc(0xC133), *b"U0133");
    assert_eq!(format_obd_dtc(0xC101), *b"U0101");

    let one = parse_obd_mode03(&[0x03, 0x43, 0x01, 0x33, 0x00, 0x00, 0x00, 0x00])
        .expect("single-frame Mode 03 must parse");
    assert_eq!(one, vec![OBD_P0133_RAW]);

    let two = parse_obd_mode03(&[0x05, 0x43, 0x01, 0x33, 0x04, 0x20, 0x00, 0x00])
        .expect("two Mode 03 DTCs must parse");
    assert_eq!(two, vec![0x0133, 0x0420]);

    let padded = parse_obd_mode03(&[0x07, 0x43, 0x01, 0x33, 0x00, 0x00, 0x00, 0x00])
        .expect("0x0000 padding is not a DTC");
    assert_eq!(padded, vec![OBD_P0133_RAW]);

    let err = parse_obd_mode03(&[0x03, 0x41, 0x01, 0x33, 0x00, 0x00, 0x00, 0x00])
        .expect_err("SID other than 0x43 must fail");
    assert_eq!(err, CanError::InvalidObdResponse);

    let err = parse_obd_mode03(&[0x02, 0x43, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00])
        .expect_err("odd DTC payload must fail");
    assert_eq!(err, CanError::InvalidObdResponse);

    let err = parse_obd_mode03(&[0x08, 0x43, 0x01, 0x33])
        .expect_err("PCI length past the buffer must fail");
    assert_eq!(err, CanError::InvalidObdResponse);

    let err = parse_obd_mode03(&[0x10, 0x08, 0x43, 0x01, 0x33, 0x04, 0x20, 0xC1])
        .expect_err("ISO-TP first-frame Mode 03 is not supported and must fail closed");
    assert_eq!(err, CanError::InvalidObdResponse);
}

/// AT-CAN-05: parse the candump fixture exactly; malformed lines fail closed.
#[test]
fn at_can_05_parse_candump() {
    assert_eq!(sha256_hex(&sha256(CANDUMP.as_bytes())), CANDUMP_SHA256_HEX);
    let frames = parse_candump(CANDUMP).expect("fixture candump must parse");
    assert_eq!(frames.len(), 7);
    for (i, (ts, id, ext, data)) in expected_candump_ids_and_data().into_iter().enumerate() {
        assert_eq!(frames[i].timestamp_ns, ts, "frame {i} timestamp");
        assert_eq!(frames[i].interface, "vcan0");
        assert_eq!(frames[i].id, id);
        assert_eq!(frames[i].extended, ext);
        assert_eq!(frames[i].dlc, 8);
        assert_eq!(frames[i].data, data);
    }

    let empty = parse_candump("# only a comment\n\n").expect("comments and blanks are skipped");
    assert!(empty.is_empty());

    let err = parse_candump("# c\n\nbad line\n").expect_err("malformed candump line");
    assert_eq!(err, CanError::MalformedLog { line: 3 });

    let err = parse_candump("(1.0) vcan0 7E88#00\n").expect_err("ID width 4..=7 is rejected");
    assert_eq!(err, CanError::MalformedLog { line: 1 });

    let err = parse_candump("(1609430400.0000000000) vcan0 7E8#00\n")
        .expect_err("more than 9 fractional digits must fail");
    assert_eq!(err, CanError::MalformedLog { line: 1 });
}

/// AT-CAN-06: parse the CSV fixture equivalently; bad headers and DLC mismatches fail closed.
#[test]
fn at_can_06_parse_csv() {
    assert_eq!(sha256_hex(&sha256(CSV.as_bytes())), CSV_SHA256_HEX);
    let frames = parse_can_csv(CSV).expect("fixture CSV must parse");
    assert_eq!(frames.len(), 7);
    for (i, (ts, id, ext, data)) in expected_candump_ids_and_data().into_iter().enumerate() {
        assert_eq!(frames[i].timestamp_ns, ts, "frame {i} timestamp");
        assert_eq!(frames[i].interface, "vcan0");
        assert_eq!(frames[i].id, id);
        assert_eq!(frames[i].extended, ext);
        assert_eq!(frames[i].dlc, 8);
        assert_eq!(frames[i].data, data);
    }

    let err = parse_can_csv("foo,bar\n").expect_err("unknown header must fail");
    assert_eq!(err, CanError::MalformedLog { line: 1 });

    let err = parse_can_csv(
        "timestamp_s,interface,can_id,dlc,data_hex\n0,vcan0,7E8,7,0343013300000000\n",
    )
    .expect_err("DLC must match data_hex length");
    assert_eq!(err, CanError::MalformedLog { line: 2 });

    let err = parse_can_csv(
        "timestamp_s,interface,can_id,dlc,data_hex\n0,vcan0,0x7E8,8,0343013300000000\n",
    )
    .expect_err("0x prefix is not part of the CSV format");
    assert_eq!(err, CanError::MalformedLog { line: 2 });
}

/// AT-CAN-07: parse the Vector ASC fixture; timestamps are the leading
/// numeric field in nanoseconds; a base other than hex, or a missing base
/// line, fails closed. The date line is not converted to a Unix epoch.
#[test]
fn at_can_07_parse_vector_asc() {
    assert_eq!(sha256_hex(&sha256(ASC.as_bytes())), ASC_SHA256_HEX);
    let frames = parse_vector_asc(ASC).expect("fixture ASC must parse");
    assert_eq!(frames.len(), 7);
    let expected_ts = [
        0, 10_000_000, 20_000_000, 30_000_000, 40_000_000, 41_000_000, 42_000_000,
    ];
    for (i, (_unix_ts, id, ext, data)) in expected_candump_ids_and_data().into_iter().enumerate() {
        assert_eq!(
            frames[i].timestamp_ns, expected_ts[i],
            "frame {i} asc timestamp"
        );
        assert_eq!(frames[i].id, id);
        assert_eq!(frames[i].extended, ext);
        assert_eq!(frames[i].dlc, 8);
        assert_eq!(frames[i].data, data);
    }

    let err = parse_vector_asc(
        "date Mon Nov 23 08:00:00.000 am 2026\nbase dec  timestamps absolute\n0.000000 1 18FECA00x Rx d 8 04 00 6E 00 10 03 FF FF\n",
    )
    .expect_err("non-hex base must fail");
    assert_eq!(err, CanError::MalformedLog { line: 2 });

    let err = parse_vector_asc("0.000000 1 18FECA00x Rx d 8 04 00 6E 00 10 03 FF FF\n")
        .expect_err("missing base hex line must fail");
    assert_eq!(err, CanError::MalformedLog { line: 1 });
}

/// AT-CAN-08: checked-in mapping and threshold tables; unmapped values
/// return NoMatch and are never guessed; malformed tables fail closed.
#[test]
fn at_can_08_mapping_and_thresholds_fail_closed() {
    assert_eq!(sha256_hex(&sha256(MAPPING.as_bytes())), MAPPING_SHA256_HEX);
    assert_eq!(
        sha256_hex(&sha256(THRESHOLDS.as_bytes())),
        THRESHOLDS_SHA256_HEX
    );

    let mapping = MappingTable::from_csv(MAPPING).expect("mapping must load");
    let thresholds = ThresholdTable::from_csv(THRESHOLDS).expect("thresholds must load");

    assert_eq!(
        mapping.lookup_j1939(110, 16),
        TriggerResult::Match { fault_node_id: 5 }
    );
    assert_eq!(mapping.lookup_j1939(94, 1), TriggerResult::NoMatch);
    assert_eq!(mapping.lookup_j1939(190, 3), TriggerResult::NoMatch);
    assert_eq!(mapping.lookup_j1939(110, 0), TriggerResult::NoMatch);

    assert_eq!(
        mapping.lookup_obd(OBD_P0133_RAW),
        TriggerResult::Match { fault_node_id: 2 }
    );
    assert_eq!(mapping.lookup_obd(0x0420), TriggerResult::NoMatch);

    assert_eq!(
        mapping.lookup_param(110),
        TriggerResult::Match { fault_node_id: 5 }
    );
    assert_eq!(mapping.lookup_param(94), TriggerResult::NoMatch);

    let none = thresholds
        .evaluate(65226, &[0x04, 0x00, 0x6E, 0x00, 0x10, 0x03, 0xFF, 0xFF])
        .expect("PGNs not in the table yield no evaluations");
    assert!(none.is_empty());

    let in_range = thresholds
        .evaluate(65262, &[0x50, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])
        .expect("in-range coolant raw=80");
    assert_eq!(in_range.len(), 1);
    assert_eq!(in_range[0].spn, 110);
    assert_eq!(in_range[0].raw, 80);
    assert!(!in_range[0].out_of_range);

    let oor = thresholds
        .evaluate(65262, &[0xC8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])
        .expect("out-of-range coolant raw=200");
    assert_eq!(oor.len(), 1);
    assert_eq!(oor[0].spn, 110);
    assert_eq!(oor[0].raw, 200);
    assert!(oor[0].out_of_range);

    let err = thresholds
        .evaluate(65262, &[0xC8])
        .expect_err("payload shorter than the signal must fail");
    assert_eq!(err, CanError::ShortPayload);

    let err = MappingTable::from_csv("j1939,110:16,5\nj1939,110:16,2\n")
        .expect_err("duplicate mapping keys must fail");
    assert_eq!(err, CanError::DuplicateMappingKey);

    let err = MappingTable::from_csv("foo,110,5\n").expect_err("unknown kind must fail");
    assert_eq!(err, CanError::UnknownMappingKind);

    let err = MappingTable::from_csv("j1939,110:16,0\n").expect_err("node 0 must fail");
    assert_eq!(err, CanError::ZeroFaultNodeId);

    let err = MappingTable::from_csv("obd,P133,2\n").expect_err("short OBD code must fail");
    assert_eq!(err, CanError::MalformedMapping { line: 1 });

    let err =
        ThresholdTable::from_csv("110,65262,0,0,16,145\n").expect_err("bit_length 0 must fail");
    assert_eq!(err, CanError::MalformedThreshold { line: 1 });

    let err = ThresholdTable::from_csv("110,65262,0,8,200,16\n")
        .expect_err("min_raw > max_raw must fail");
    assert_eq!(err, CanError::MalformedThreshold { line: 1 });
}

/// AT-CAN-09: ingest the candump fixture into an L0 ring buffer and write
/// tiles with log_hash and frame_offset provenance. Appending without
/// ADDON_CAN is PermissionDenied. Triggers reference s3a-pack-to node ids.
#[test]
fn at_can_09_ingest_ring_and_tiles_with_provenance() {
    let mapping = MappingTable::from_csv(MAPPING).unwrap();
    let thresholds = ThresholdTable::from_csv(THRESHOLDS).unwrap();
    let report = ingest_candump(CANDUMP, &mapping, &thresholds, source_id(1), ORIGIN_PTR)
        .expect("candump ingest must succeed");

    assert_eq!(report.dtcs.len(), 5);
    assert_eq!(report.triggers.len(), 4);
    assert_eq!(report.no_match_count, 2);

    let log_hash = hex_to_32(CANDUMP_SHA256_HEX);
    assert_eq!(sha256(CANDUMP.as_bytes()), log_hash);

    // DTC 0: single-frame DM1 SPN 110 FMI 16 at frame 0
    let d0 = &report.dtcs[0];
    assert_eq!(d0.source_id, source_id(1));
    assert_eq!(d0.origin_ptr, ORIGIN_PTR);
    assert_eq!(d0.log_hash, log_hash);
    assert_eq!(d0.frame_offset, 0);
    assert_eq!(d0.can_id, 0x18FECA00);
    assert_eq!(d0.pgn, 65226);
    assert_eq!(d0.spn, 110);
    assert_eq!(d0.fmi, 16);
    assert_eq!(d0.occurrence_count, 3);
    assert_eq!(d0.conversion_method, 0);
    assert_eq!(d0.kind, DTC_KIND_J1939_DM1);
    assert_eq!(d0.source_address, 0);
    assert_eq!(d0.obd_raw, 0);
    assert_eq!(d0._pad0, 0);
    assert_eq!(d0._pad, [0u8; 4]);
    assert_eq!(d0.content_hash, hex_to_32(DTC_110_16_SHA256_HEX));
    assert_eq!(d0.content_hash, sha256(&DTC_110_16_SLOT));

    // DTC 1: OBD P0133 at frame 3
    let d1 = &report.dtcs[1];
    assert_eq!(d1.frame_offset, 3);
    assert_eq!(d1.can_id, 0x7E8);
    assert_eq!(d1.pgn, 0);
    assert_eq!(d1.spn, 0);
    assert_eq!(d1.fmi, 0);
    assert_eq!(d1.occurrence_count, 0);
    assert_eq!(d1.conversion_method, 0);
    assert_eq!(d1.kind, DTC_KIND_OBD_MODE03);
    assert_eq!(d1.source_address, 0);
    assert_eq!(d1.obd_raw, OBD_P0133_RAW);
    assert_eq!(d1.content_hash, hex_to_32(OBD_P0133_SHA256_HEX));
    assert_eq!(d1.content_hash, sha256(&OBD_P0133_BYTES));

    // DTCs 2..=4: BAM, frame_offset is the TP.CM index (4)
    assert_eq!(report.dtcs[2].frame_offset, 4);
    assert_eq!(report.dtcs[2].can_id, 0x1CECFF21);
    assert_eq!(report.dtcs[2].source_address, 0x21);
    assert_eq!(report.dtcs[2].spn, 110);
    assert_eq!(report.dtcs[2].content_hash, sha256(&DTC_110_16_SLOT));
    assert_eq!(report.dtcs[3].spn, 94);
    assert_eq!(report.dtcs[3].content_hash, hex_to_32(DTC_94_1_SHA256_HEX));
    assert_eq!(report.dtcs[4].spn, 190);
    assert_eq!(report.dtcs[4].content_hash, hex_to_32(DTC_190_3_SHA256_HEX));

    // Triggers: frame 0 j1939 110:16 → 5; frame 2 param 110 raw=200 → 5;
    // frame 3 obd P0133 → 2; frame 4 BAM 110:16 → 5. Unmapped 94:1 and 190:3
    // increment no_match_count only.
    let t0 = &report.triggers[0];
    assert_eq!(t0.frame_offset, 0);
    assert_eq!(t0.kind, TRIGGER_KIND_J1939);
    assert_eq!(t0.fault_node_id, 5);
    assert_eq!(t0.spn, 110);
    assert_eq!(t0.fmi, 16);
    assert_eq!(t0.observed_raw, 0);
    assert_eq!(t0.obd_raw, 0);
    assert_eq!(t0.log_hash, log_hash);
    assert_eq!(t0.content_hash, hex_to_32(TRIG_J1939_110_16_SHA256_HEX));
    assert_eq!(t0.content_hash, sha256(b"j1939:110:16"));
    assert_eq!(t0._pad, [0u8; 3]);

    let t1 = &report.triggers[1];
    assert_eq!(t1.frame_offset, 2);
    assert_eq!(t1.kind, TRIGGER_KIND_PARAM);
    assert_eq!(t1.fault_node_id, 5);
    assert_eq!(t1.spn, 110);
    assert_eq!(t1.fmi, 0);
    assert_eq!(t1.observed_raw, 200);
    assert_eq!(t1.content_hash, hex_to_32(TRIG_PARAM_110_SHA256_HEX));
    assert_eq!(t1.content_hash, sha256(b"param:110"));

    let t2 = &report.triggers[2];
    assert_eq!(t2.frame_offset, 3);
    assert_eq!(t2.kind, TRIGGER_KIND_OBD);
    assert_eq!(t2.fault_node_id, 2);
    assert_eq!(t2.spn, 0);
    assert_eq!(t2.obd_raw, OBD_P0133_RAW);
    assert_eq!(t2.content_hash, hex_to_32(TRIG_OBD_P0133_SHA256_HEX));
    assert_eq!(t2.content_hash, sha256(b"obd:P0133"));

    let t3 = &report.triggers[3];
    assert_eq!(t3.frame_offset, 4);
    assert_eq!(t3.kind, TRIGGER_KIND_J1939);
    assert_eq!(t3.fault_node_id, 5);
    assert_eq!(t3.spn, 110);
    assert_eq!(t3.fmi, 16);

    let dtc_ring = L0RingBuffer::<CanDtcRecord>::new(16, BackpressurePolicy::Error);
    for rec in &report.dtcs {
        dtc_ring.push(*rec).expect("ring must accept DTCs");
    }
    let drained_dtcs = dtc_ring.drain_all();
    assert_eq!(drained_dtcs, report.dtcs);

    let trig_ring = L0RingBuffer::<CanTriggerRecord>::new(16, BackpressurePolicy::Error);
    for rec in &report.triggers {
        trig_ring.push(*rec).expect("ring must accept triggers");
    }
    let drained_trigs = trig_ring.drain_all();
    assert_eq!(drained_trigs, report.triggers);

    let temp = temp_archive("at09");
    let engine = S3ACrudEngine::open_or_create(&temp).unwrap();
    let err = engine
        .create_can_dtc_records(&drained_dtcs)
        .expect_err("append without ADDON_CAN must fail");
    assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);

    engine.enable_addon(ADDON_CAN).unwrap();
    engine.enable_addon(ADDON_TO).unwrap();
    let node5 = encode_to_fault_node(
        b"Replace the demonstration pressure regulator.",
        source_id(9),
        ORIGIN_PTR,
        {
            let mut doc = [0u8; 16];
            doc.copy_from_slice(&sha256(b"SYN-DEMO-0001")[..16]);
            doc
        },
        5,
        NODE_KIND_ACTION,
    )
    .unwrap();
    let node2 = encode_to_fault_node(
        b"Is discharge pressure within Table 1 limits?",
        source_id(6),
        ORIGIN_PTR,
        {
            let mut doc = [0u8; 16];
            doc.copy_from_slice(&sha256(b"SYN-DEMO-0001")[..16]);
            doc
        },
        2,
        NODE_KIND_TEST,
    )
    .unwrap();
    engine
        .create_to_fault_node_records(&[node2, node5])
        .unwrap();

    engine.create_can_dtc_records(&drained_dtcs).unwrap();
    engine.create_can_trigger_records(&drained_trigs).unwrap();

    let reader = MmapReader::open(&temp).unwrap();
    assert_eq!(reader.tile_count(), 3);

    let (header, raw) = reader.get_tile(1).expect("DTC tile");
    assert_eq!(header.tile_type, CanTileType::CAN_DTC);
    assert_eq!(header.record_layout_version, 1);
    let got = &raw[..header.payload_bytes as usize];
    let expected: Vec<u8> = drained_dtcs
        .iter()
        .flat_map(|r| bytes_of(r).to_vec())
        .collect();
    assert_eq!(got, expected.as_slice());
    match decode_can_records(header.tile_type, header.record_layout_version, got).unwrap() {
        CanDecoded::Dtc(recs) => assert_eq!(recs, drained_dtcs),
        other => panic!("expected DTC decode, got {other:?}"),
    }

    let (header, raw) = reader.get_tile(2).expect("trigger tile");
    assert_eq!(header.tile_type, CanTileType::CAN_TRIGGER);
    let got = &raw[..header.payload_bytes as usize];
    match decode_can_records(header.tile_type, header.record_layout_version, got).unwrap() {
        CanDecoded::Trigger(recs) => {
            assert_eq!(recs, drained_trigs);
            assert!(recs
                .iter()
                .all(|t| t.fault_node_id == 2 || t.fault_node_id == 5));
        }
        other => panic!("expected trigger decode, got {other:?}"),
    }

    let _ = std::fs::remove_file(&temp);
}

/// AT-CAN-10: encode/decode fail closed on missing provenance, zero node
/// ids, truncated payloads, unknown tiles, and unsupported layout versions.
/// Layout version 0 aliases to 1.
#[test]
fn at_can_10_encode_decode_fail_closed() {
    let log_hash = sha256(CANDUMP.as_bytes());
    let dtc = encode_can_dtc(
        &DTC_110_16_SLOT,
        source_id(1),
        ORIGIN_PTR,
        log_hash,
        0,
        0x18FECA00,
        65226,
        110,
        16,
        3,
        0,
        DTC_KIND_J1939_DM1,
        0,
        0,
    )
    .expect("valid DTC must encode");
    assert_eq!(dtc.content_hash, sha256(&DTC_110_16_SLOT));
    let back: CanDtcRecord = pod_read_unaligned(bytes_of(&dtc));
    assert_eq!(back, dtc);

    let err = encode_can_dtc(
        &DTC_110_16_SLOT,
        [0u8; 16],
        ORIGIN_PTR,
        log_hash,
        0,
        0x18FECA00,
        65226,
        110,
        16,
        3,
        0,
        DTC_KIND_J1939_DM1,
        0,
        0,
    )
    .expect_err("zero source_id must fail");
    assert_eq!(err, CanError::MissingSourceId);

    let err = encode_can_dtc(
        &DTC_110_16_SLOT,
        source_id(1),
        [0u8; 16],
        log_hash,
        0,
        0x18FECA00,
        65226,
        110,
        16,
        3,
        0,
        DTC_KIND_J1939_DM1,
        0,
        0,
    )
    .expect_err("zero origin_ptr must fail");
    assert_eq!(err, CanError::MissingOriginPtr);

    let err = encode_can_trigger(
        b"j1939:110:16",
        source_id(1),
        ORIGIN_PTR,
        log_hash,
        0,
        0,
        110,
        0,
        16,
        0,
        TRIGGER_KIND_J1939,
    )
    .expect_err("zero fault_node_id must fail");
    assert_eq!(err, CanError::ZeroFaultNodeId);

    let err = decode_can_records(CanTileType::CAN_DTC, 1, &bytes_of(&dtc)[..127])
        .expect_err("truncated DTC payload must fail");
    match err {
        CanError::TruncatedRecord { expected, actual } => {
            assert_eq!(expected, 128);
            assert_eq!(actual, 127);
        }
        other => panic!("wrong error for truncated record: {other:?}"),
    }

    let err = decode_can_records(CanTileType::CAN_DTC, 99, bytes_of(&dtc))
        .expect_err("layout 99 must fail");
    match err {
        CanError::UnsupportedLayoutVersion { tile_type, version } => {
            assert_eq!(tile_type, 47);
            assert_eq!(version, 99);
        }
        other => panic!("wrong error for new layout version: {other:?}"),
    }

    let decoded = decode_can_records(CanTileType::CAN_DTC, 0, bytes_of(&dtc))
        .expect("layout version 0 aliases to 1");
    match decoded {
        CanDecoded::Dtc(recs) => assert_eq!(recs, vec![dtc]),
        other => panic!("unexpected decode: {other:?}"),
    }

    let err = decode_can_records(99, 1, bytes_of(&dtc)).expect_err("unknown tile must fail");
    assert_eq!(err, CanError::UnknownTileType(99));
    let err = decode_can_records(0, 1, bytes_of(&dtc)).expect_err("telemetry is not a CAN tile");
    assert_eq!(err, CanError::UnknownTileType(0));

    let temp = temp_archive("at10");
    let engine = S3ACrudEngine::open_or_create(&temp).unwrap();
    engine.enable_addon(ADDON_CAN).unwrap();
    engine.create_can_dtc_records(&[dtc]).unwrap();
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
    let res: Result<CanDtcRecord, S3AError> =
        reader.read_record_by_coordinate(&S3ACoordinate::new(0, 0, 0));
    match res {
        Err(S3AError::UnsupportedLayoutVersion(v)) => assert_eq!(v, 99),
        other => panic!("Expected UnsupportedLayoutVersion(99), got {other:?}"),
    }
    let _ = std::fs::remove_file(&temp);
}

/// AT-CAN-11: every fixture is synthetic and is not a real manual or vehicle log.
#[test]
fn at_can_11_fixtures_are_synthetic_only() {
    for (name, text, hex) in [
        ("candump", CANDUMP, CANDUMP_SHA256_HEX),
        ("csv", CSV, CSV_SHA256_HEX),
        ("asc", ASC, ASC_SHA256_HEX),
        ("mapping", MAPPING, MAPPING_SHA256_HEX),
        ("thresholds", THRESHOLDS, THRESHOLDS_SHA256_HEX),
        ("manual", MANUAL, MANUAL_SHA256_HEX),
    ] {
        assert!(
            text.contains("SYNTHETIC") || text.contains("synthetic"),
            "{name} must be marked synthetic"
        );
        assert_eq!(sha256_hex(&sha256(text.as_bytes())), hex, "{name} hash");
        assert!(!text.contains("T.O."), "{name} must not contain T.O.");
        assert!(
            !text.contains("ETIMS")
                || text.to_lowercase().contains("no etims")
                || text.contains("NOT OEM or ETIMS")
                || text.contains("no ETIMS"),
            "{name}"
        );
    }
    assert!(MANUAL.contains("SYNTHETIC MAINTENANCE MANUAL"));
    assert!(MANUAL.contains("NOT A TECHNICAL ORDER"));
    assert!(MANUAL.contains("no ETIMS content"));
    assert!(MANUAL.contains("SYN-DEMO-0001"));
    assert!(MANUAL.contains("Replace the demonstration pressure regulator."));
    assert!(CANDUMP.contains("NOT a vehicle capture"));
    assert!(MAPPING.contains("SYNTHETIC"));
    assert!(THRESHOLDS.contains("SYNTHETIC"));
}

/// AT-CAN-12: live SocketCAN is feature-gated. The default build does not
/// enable it, and with the feature a missing interface fails closed. The
/// shipped crate must not fall back to a mock reader.
#[test]
fn at_can_12_socketcan_is_feature_gated() {
    #[cfg(not(feature = "socketcan"))]
    {
        assert!(!SOCKETCAN_ENABLED);
    }
    #[cfg(feature = "socketcan")]
    {
        assert!(SOCKETCAN_ENABLED);
        let err = s3a_can_bridge::SocketCanReader::open("s3a_missing_vcan_99")
            .expect_err("missing SocketCAN interface must fail closed");
        assert_eq!(err, CanError::SocketCanOpenFailed);
    }
}
