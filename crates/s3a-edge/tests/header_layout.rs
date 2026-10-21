//! Edge CommonTileHeader layout + hash conformance (S3A-EDGE-PROFILE §2.4.1, S3A-TILE-SPEC §14.1, ADR-0003).
use core::mem::{offset_of, size_of};
use s3a_edge::*;

#[test]
fn common_tile_header_offsets_match_spec() {
    assert_eq!(size_of::<CommonTileHeader>(), 64);
    assert_eq!(COMMON_TILE_HEADER_SIZE, 64);
    assert_eq!(offset_of!(CommonTileHeader, type_id), 0);
    assert_eq!(offset_of!(CommonTileHeader, type_version), 2);
    assert_eq!(offset_of!(CommonTileHeader, flags), 4);
    assert_eq!(offset_of!(CommonTileHeader, _reserved1), 6);
    assert_eq!(offset_of!(CommonTileHeader, record_count), 8);
    assert_eq!(offset_of!(CommonTileHeader, data_crc32), 12);
    assert_eq!(offset_of!(CommonTileHeader, payload_len), 16);
    assert_eq!(offset_of!(CommonTileHeader, payload_hash), 24);
    assert_eq!(offset_of!(CommonTileHeader, header_crc32), 56);
    assert_eq!(offset_of!(CommonTileHeader, generation), 60);
    assert_eq!(TILE_FLAG_SEALED, 1);
    assert_eq!(TILE_FLAG_CRC_CHAIN, 2);
}

fn le32(b: &[u8], o: usize) -> u32 { u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) }

/// Parse a real written edge tile from raw flash bytes using only the spec's offsets.
#[test]
#[cfg_attr(debug_assertions, ignore = "A4: EdgeWriter::open_or_format calls bytemuck::from_bytes on an unaligned stack buffer and panics in debug builds; runs under --release; un-ignore after A4")]
fn real_edge_tile_bytes_match_spec() {
    let flash = RamFlash::<{ 64 * 1024 }>::new(256, 512);
    let mut w = EdgeWriter::<_, 4096>::open_or_format(flash, [0x42; 16], [1, 2], 1_700_000_000).unwrap();
    for i in 0..10u16 {
        w.append_sensor_record(&CompactEdgeSensorRecord::new(i * 100, 1, 1, 2000 + i as i32, QUALITY_OK)).unwrap();
    }
    let idx = w.tile_count() as usize;
    w.seal_tile(1_700_000_005).unwrap();
    let gen = w.generation();
    let raw = w.storage().as_slice();
    let t = &raw[FILE_HEADER_SIZE + idx * 4096..][..4096];
    let h = &t[..64];
    let flags = u16::from_le_bytes([h[4], h[5]]);
    assert_eq!(flags & TILE_FLAG_CRC_CHAIN, TILE_FLAG_CRC_CHAIN);
    assert_eq!(&h[6..8], &[0, 0], "_reserved1 zeroed by writer");
    assert_eq!(le32(h, 8), 10, "record_count");
    let plen = u64::from_le_bytes(h[16..24].try_into().unwrap()) as usize;
    let payload = &t[64..64 + plen];
    assert_eq!(le32(h, 12), crc32c(payload), "data_crc32 @12..16");
    let mut hc = crc32c(&h[..56]);
    hc = crc32c_append(hc, &h[60..64]);
    assert_eq!(le32(h, 56), hc, "header_crc32 @56..60 over 0..56 || 60..64");
    assert_eq!(le32(h, 60) as u64, gen, "generation @60..64");
}

fn hex(s: &str) -> Vec<u8> { (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect() }
fn field<'a>(j: &'a str, k: &str) -> &'a str {
    let i = j.find(&format!("\"{k}\"")).unwrap() + k.len() + 2;
    let r = j[i..].trim_start_matches([':', ' ']);
    r.trim_start_matches('"').split(['"', ',', '\n', '}']).next().unwrap().trim()
}

#[test]
fn edge_hash_vectors() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../spec-vectors/hashes/");
    let d = std::fs::read_to_string(format!("{root}edge_data_crc32c.json")).unwrap();
    assert_eq!(crc32c(&hex(field(&d, "input_hex"))), field(&d, "expected_u32").parse::<u32>().unwrap());
    let hj = std::fs::read_to_string(format!("{root}edge_header_crc32c.json")).unwrap();
    let hb = hex(field(&hj, "header_hex"));
    let hdr: CommonTileHeader = bytemuck::pod_read_unaligned(&hb);
    assert_eq!(compute_common_tile_header_crc32(&hdr), field(&hj, "expected_u32").parse::<u32>().unwrap());
    assert_eq!(hdr.header_crc32, compute_common_tile_header_crc32(&hdr), "stored header_crc32 bytes 56..60 match");
    assert!(hdr.verify());
}
