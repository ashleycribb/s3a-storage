//! Step 1 acceptance tests for the `s3a-spatial` pack (REVIEW_0_aq1b.md §7).
//!
//! Fixture: the OGC GeoPose 1.0 Basic-Quaternion example JSON, clause 9.2.3
//! (Figures 24/25), doc 21-056r11, fetched live 2026-11-19 from
//! https://docs.ogc.org/is/21-056r11/21-056r11.html. The Strict and Permissive
//! examples carry identical values; the fixture below is the compact
//! single-line serialization of exactly those values (158 bytes).
//!
//! Do NOT edit these tests to make them pass.

use bytemuck::{bytes_of, pod_read_unaligned};
use s3a_core::ADDON_SPATIAL;
use s3a_edge::{
    crc32c, EdgeWriter, FileHeader, RamFlash, COMMON_TILE_HEADER_SIZE, FILE_HEADER_SIZE,
    FLAG_PROFILE_EDGE,
};
use s3a_engine::{MmapReader, S3ACrudEngine};
use s3a_spatial::{
    decode_geopose_basic_quat, encode_geopose_basic_quat, GeoposeBasicQuatRecord, GeoposeError,
    SpatialCrudExt, SpatialTileType,
};
use sha2::{Digest, Sha256};
use std::io;

/// OGC GeoPose 1.0 Basic-Quaternion example (clause 9.2.3), values verbatim.
pub const OGC_BASIC_QUAT_JSON: &[u8] = b"{\"position\":{\"lat\":47.7,\"lon\":-122.3,\"h\":11.5},\"quaternion\":{\"x\":0.20054473382601948,\"y\":-0.08111675703887213,\"z\":0.3660908114262869,\"w\":-0.9050852994339209}}";

/// SHA-256 of `OGC_BASIC_QUAT_JSON` (verified out-of-band before writing the tests).
pub const OGC_BASIC_QUAT_SHA256_HEX: &str =
    "3d9756500a9ea0ca4576e322345baba778a4cf5457c84f9a353a2baf27c14592";

/// Quaternion unit-norm tolerance, STATED HERE IN THE TEST (AT-SP-03).
/// The OGC example deviates from unit norm-squared by 2.2e-16 (one f64 ULP),
/// so 1e-6 admits exact-decimal rounding while rejecting any quaternion off
/// by more than one part per million.
pub const QUAT_UNIT_TOL: f64 = 1e-6;

/// Fixed non-zero source id (a UUID-shaped 16-byte value).
pub const SOURCE_ID: [u8; 16] = [
    0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef,
];

/// Fixed non-zero origin pointer (opaque 16-byte pointer to the original file).
pub const ORIGIN_PTR: [u8; 16] = [
    0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32, 0x10, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32, 0x10,
];

fn ogc_record() -> GeoposeBasicQuatRecord {
    encode_geopose_basic_quat(OGC_BASIC_QUAT_JSON, SOURCE_ID, ORIGIN_PTR, QUAT_UNIT_TOL)
        .expect("OGC example must encode")
}

/// AT-SP-01: encode the OGC Basic-Quaternion example JSON into a 128 B record
/// and decode lat, lon, h, qx, qy, qz, qw with exact f64 values.
#[test]
fn at_sp_01_encode_decode_exact_f64() {
    assert_eq!(size_of::<GeoposeBasicQuatRecord>(), 128);
    assert_eq!(SpatialTileType::GEOPOSE_BASIC_QUAT, 33);

    let record = ogc_record();
    assert_eq!(record._pad, 0);

    let decoded = decode_geopose_basic_quat(&record).expect("record must decode");
    assert_eq!(decoded.lat, 47.7);
    assert_eq!(decoded.lon, -122.3);
    assert_eq!(decoded.h, 11.5);
    assert_eq!(decoded.qx, 0.20054473382601948);
    assert_eq!(decoded.qy, -0.08111675703887213);
    assert_eq!(decoded.qz, 0.3660908114262869);
    assert_eq!(decoded.qw, -0.9050852994339209);

    // The decoded values are the same f64s the record carries.
    assert_eq!(decoded.lat, record.lat);
    assert_eq!(decoded.qw, record.qw);

    // Round-trip through raw bytes is exact.
    let bytes = bytes_of(&record);
    assert_eq!(bytes.len(), 128);
    let back: GeoposeBasicQuatRecord = pod_read_unaligned(bytes);
    assert_eq!(back, record);
}

/// AT-SP-02: content_hash is SHA-256 of the original JSON bytes; source_id and
/// origin_ptr are present and non-zero. Missing any one fails.
#[test]
fn at_sp_02_content_hash_and_provenance() {
    let record = ogc_record();

    let expected: [u8; 32] = Sha256::digest(OGC_BASIC_QUAT_JSON).into();
    assert_eq!(record.content_hash, expected);

    // The hash is the precomputed constant, not a hash of something else.
    let mut hex = String::new();
    for b in record.content_hash {
        hex.push_str(&format!("{:02x}", b));
    }
    assert_eq!(hex, OGC_BASIC_QUAT_SHA256_HEX);

    assert_ne!(record.source_id, [0u8; 16]);
    assert_eq!(record.source_id, SOURCE_ID);
    assert_ne!(record.origin_ptr, [0u8; 16]);
    assert_eq!(record.origin_ptr, ORIGIN_PTR);

    // Missing source id fails.
    let err = encode_geopose_basic_quat(OGC_BASIC_QUAT_JSON, [0u8; 16], ORIGIN_PTR, QUAT_UNIT_TOL)
        .expect_err("all-zero source_id must be rejected");
    assert_eq!(err, GeoposeError::MissingSourceId);

    // Missing origin pointer fails.
    let err = encode_geopose_basic_quat(OGC_BASIC_QUAT_JSON, SOURCE_ID, [0u8; 16], QUAT_UNIT_TOL)
        .expect_err("all-zero origin_ptr must be rejected");
    assert_eq!(err, GeoposeError::MissingOriginPtr);
}

/// AT-SP-03: a quaternion whose x²+y²+z²+w² is not 1 within the documented
/// f64 tolerance is rejected. The tolerance is stated in this test (QUAT_UNIT_TOL).
#[test]
fn at_sp_03_non_unit_quaternion_rejected() {
    // The OGC example itself is unit to one ULP, so it passes at the stated tolerance.
    let _ = ogc_record();

    // Same pose, quaternion scaled by 2: norm-squared is 4, deviation 3.0.
    let doubled = b"{\"position\":{\"lat\":47.7,\"lon\":-122.3,\"h\":11.5},\"quaternion\":{\"x\":0.40108946765203896,\"y\":-0.16223351407774426,\"z\":0.7321816228525738,\"w\":-1.8101705988678418}}";
    let err = encode_geopose_basic_quat(doubled, SOURCE_ID, ORIGIN_PTR, QUAT_UNIT_TOL)
        .expect_err("norm-squared 4 must be rejected");
    match err {
        GeoposeError::NonUnitQuaternion { norm_sq, tolerance } => {
            assert!((norm_sq - 4.0).abs() < 1e-9);
            assert_eq!(tolerance, QUAT_UNIT_TOL);
        }
        other => panic!("wrong error for non-unit quaternion: {:?}", other),
    }

    // Zero quaternion is rejected too.
    let zero = b"{\"position\":{\"lat\":47.7,\"lon\":-122.3,\"h\":11.5},\"quaternion\":{\"x\":0.0,\"y\":0.0,\"z\":0.0,\"w\":0.0}}";
    let err = encode_geopose_basic_quat(zero, SOURCE_ID, ORIGIN_PTR, QUAT_UNIT_TOL)
        .expect_err("zero quaternion must be rejected");
    assert!(matches!(err, GeoposeError::NonUnitQuaternion { .. }));

    // Malformed JSON and missing fields are rejected as well.
    let bad = b"{\"position\":{\"lat\":47.7}}";
    assert!(encode_geopose_basic_quat(bad, SOURCE_ID, ORIGIN_PTR, QUAT_UNIT_TOL).is_err());
    let not_json = b"not json at all";
    assert!(encode_geopose_basic_quat(not_json, SOURCE_ID, ORIGIN_PTR, QUAT_UNIT_TOL).is_err());
}

/// AT-SP-04: append without the addon_mask bit 0x0080 returns permission denied.
#[test]
fn at_sp_04_append_without_addon_bit_is_permission_denied() {
    assert_eq!(ADDON_SPATIAL, 0x0080);

    let temp = std::env::temp_dir().join(format!(
        "s3a_test_spatial_at04_{}.s3a",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));

    let engine = S3ACrudEngine::open_or_create(&temp).unwrap();
    let record = ogc_record();

    // No addon enabled: the write gate refuses with PermissionDenied.
    let err = engine
        .create_geopose_records(&[record])
        .expect_err("append without ADDON_SPATIAL must fail");
    assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);

    // Fail-closed: nothing was written.
    let reader = MmapReader::open(&temp).unwrap();
    assert_eq!(reader.tile_count(), 0);

    // Control case: with the bit enabled, the same append succeeds.
    engine.enable_addon(ADDON_SPATIAL).unwrap();
    let tile_id = engine.create_geopose_records(&[record]).unwrap();
    assert_eq!(tile_id, 1);

    let reader = MmapReader::open(&temp).unwrap();
    assert_eq!(reader.tile_count(), 1);

    let _ = std::fs::remove_file(&temp);
}

/// AT-SP-05: one sealed 128 B record fits in a 4 KiB edge micro-tile
/// (tile_stride=4096) and is readable back.
#[test]
fn at_sp_05_sealed_record_in_4k_edge_tile() {
    let record = ogc_record();
    let record_bytes = bytes_of(&record);
    assert_eq!(record_bytes.len(), 128);

    // 128 B record + 64 B tile header fits a 4096 B micro-tile with room for 30 more.
    assert!(COMMON_TILE_HEADER_SIZE + 128 <= 4096);
    assert_eq!((4096 - COMMON_TILE_HEADER_SIZE) / 128, 31);

    let flash = RamFlash::<{ 64 * 1024 }>::new(256, 512);
    let device_id = [0x42u8; 16];
    let file_uuid = [0x11223344, 0x55667788];
    let now = 1700000000u64;

    let mut writer = EdgeWriter::<_, 4096>::open_or_format(flash, device_id, file_uuid, now)
        .expect("format should succeed");
    let tiles_before = writer.tile_count();

    writer
        .append_record(SpatialTileType::GEOPOSE_BASIC_QUAT as u16, record_bytes)
        .expect("128 B record must fit the 4 KiB staging buffer");
    let seal = writer.seal_tile(now + 5).expect("tile must seal");
    assert_ne!(seal, [0u8; 32]);
    assert_eq!(writer.tile_count(), tiles_before + 1);

    // Read back from the sealed tile bytes.
    let flash_bytes = writer.storage().as_slice();
    let tile_offset = FILE_HEADER_SIZE + (tiles_before as usize) * 4096;

    // The FileHeader slot names the stride: this is a genuine 4 KiB edge archive.
    let header: FileHeader = pod_read_unaligned(&flash_bytes[..64]);
    assert!(header.verify());
    assert_eq!(header.tile_stride, 4096);
    assert_ne!(header.flags & FLAG_PROFILE_EDGE, 0);

    // The tile header names type 33, one record, 128 payload bytes.
    let tile_header: s3a_edge::CommonTileHeader =
        pod_read_unaligned(&flash_bytes[tile_offset..tile_offset + COMMON_TILE_HEADER_SIZE]);
    assert_eq!(tile_header.type_id, 33);
    assert_eq!(tile_header.record_count, 1);
    assert_eq!(tile_header.payload_len, 128);

    // The payload is the record, byte for byte, and the seal covers it.
    let payload = &flash_bytes[tile_offset + COMMON_TILE_HEADER_SIZE
        ..tile_offset + COMMON_TILE_HEADER_SIZE + 128];
    assert_eq!(payload, record_bytes);
    assert_eq!(tile_header.data_crc32, crc32c(payload));
    let payload_hash: [u8; 32] = Sha256::digest(payload).into();
    assert_eq!(tile_header.payload_hash, payload_hash);

    // Decoding the read-back bytes gives the OGC pose exactly.
    let back: GeoposeBasicQuatRecord = pod_read_unaligned(payload);
    let decoded = decode_geopose_basic_quat(&back).expect("read-back record must decode");
    assert_eq!(decoded.lat, 47.7);
    assert_eq!(decoded.qw, -0.9050852994339209);
}
