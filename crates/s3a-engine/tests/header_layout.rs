//! A1: pins HyperTileHeader on-disk layout to docs/spec/S3A-TILE-SPEC.md §3.1/§3.2.
//! If this fails, either the spec or the struct drifted. Bytes must never change silently.
use core::mem::{offset_of, size_of};
use s3a_engine::*;

macro_rules! at { ($t:ty, $f:ident, $off:expr, $sz:expr) => {{
    assert_eq!(offset_of!($t, $f), $off, concat!(stringify!($t), ".", stringify!($f), " offset"));
    let v: $t = bytemuck::Zeroable::zeroed();
    assert_eq!(core::mem::size_of_val(&v.$f), $sz, concat!(stringify!($t), ".", stringify!($f), " size"));
}}; }

#[test]
fn hyper_tile_header_matches_spec_3_1() {
    assert_eq!(size_of::<HyperTileHeader>(), 512);
    at!(HyperTileHeader, tile_id, 0, 8);
    at!(HyperTileHeader, tile_type, 8, 4);
    at!(HyperTileHeader, record_count, 12, 4);
    at!(HyperTileHeader, min_timestamp, 16, 8);
    at!(HyperTileHeader, max_timestamp, 24, 8);
    at!(HyperTileHeader, data_crc32, 32, 4);
    at!(HyperTileHeader, header_crc32, 36, 4);
    at!(HyperTileHeader, payload_bytes, 40, 4);
    at!(HyperTileHeader, record_layout_version, 44, 4);
    at!(HyperTileHeader, hull, 48, 208);
    at!(HyperTileHeader, filter, 256, 128);
    at!(HyperTileHeader, provenance, 384, 32);
    at!(HyperTileHeader, lifecycle, 416, 24);
    at!(HyperTileHeader, security, 440, 16);
    at!(HyperTileHeader, encoding, 456, 16);
    at!(HyperTileHeader, learned_index, 472, 8);
    at!(HyperTileHeader, generation, 480, 8);
    at!(HyperTileHeader, tile_index, 488, 4);
    at!(HyperTileHeader, _reserved_addr, 492, 4);
    at!(HyperTileHeader, file_uuid, 496, 16);
}

#[test]
fn sub_structures_match_spec_3_2() {
    // Offsets relative to the sub-structure; spec §3.2 lists absolute = base + relative.
    at!(SimplexHull, dim, 0, 4); at!(SimplexHull, radius, 4, 4);
    at!(SimplexHull, min_bounds, 8, 64); at!(SimplexHull, max_bounds, 72, 64);
    at!(SimplexHull, centroid, 136, 64); at!(SimplexHull, _reserved, 200, 8);
    at!(TileFilterMetadata, filter_type, 0, 2); at!(TileFilterMetadata, hash_seed, 2, 2);
    at!(TileFilterMetadata, distinct_sensors, 4, 4); at!(TileFilterMetadata, bits, 8, 120);
    at!(TileProvenanceMetadata, merkle_root, 0, 32);
    at!(TileLifecycleMetadata, tombstone_count, 0, 4); at!(TileLifecycleMetadata, stratum_tier, 4, 1);
    at!(TileLifecycleMetadata, compaction_epoch, 5, 1); at!(TileLifecycleMetadata, _reserved_tier, 6, 2);
    at!(TileLifecycleMetadata, min_expiry_timestamp, 8, 8); at!(TileLifecycleMetadata, max_expiry_timestamp, 16, 8);
    at!(TileSecurityMetadata, tenant_id, 0, 8); at!(TileSecurityMetadata, security_tag_mask, 8, 4);
    at!(TileSecurityMetadata, min_clearance_level, 12, 1); at!(TileSecurityMetadata, _reserved, 13, 3);
    at!(TileEncodingMetadata, value_base, 0, 8); at!(TileEncodingMetadata, value_scale, 8, 4);
    at!(TileEncodingMetadata, encoding_mode, 12, 2); at!(TileEncodingMetadata, _reserved, 14, 2);
    at!(TileLearnedIndex, slope, 0, 4); at!(TileLearnedIndex, intercept, 4, 4);
}

#[test]
fn header_is_little_endian_on_disk() {
    let mut h: HyperTileHeader = bytemuck::Zeroable::zeroed();
    h.tile_id = 0x0102030405060708;
    h.filter.hash_seed = 0x533A;
    let b = bytemuck::bytes_of(&h);
    assert_eq!(&b[0..8], &[8, 7, 6, 5, 4, 3, 2, 1]);
    assert_eq!(&b[258..260], &[0x3A, 0x53]); // filter at 256, hash_seed at +2
}
