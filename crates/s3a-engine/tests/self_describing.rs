//! Item 6: self-describing archives (about entry) and manifest fixes.

use s3a_engine::{about_report, seal_buffer_v2, verify_sealed_archive, MmapReader, SealOptionsV2};

const LEGACY: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/legacy_v1_0_telemetry.s3a"
);

fn seal(name: &str, o: &SealOptionsV2) -> std::path::PathBuf {
    let out = seal_buffer_v2(&std::fs::read(LEGACY).unwrap(), o).unwrap();
    let p = std::env::temp_dir().join(format!("s3a_about_{name}_{}.s3a", std::process::id()));
    std::fs::write(&p, out).unwrap();
    p
}

#[test]
fn about_entry_names_spec_and_matches_embedded_spec() {
    let p = seal(
        "full",
        &SealOptionsV2::default()
            .with_manifest()
            .with_embed_spec()
            .with_record_roots()
            .with_about(),
    );
    let r = MmapReader::open(&p).unwrap();
    let ab = about_report(&r);
    let a = ab.about.expect("about entry");
    assert_eq!(a.spec_url, s3a_engine::SPEC_URL);
    assert_eq!(a.spec_version, s3a_engine::spec_version());
    assert_eq!(a.spec_sha256, s3a_engine::spec_sha256_hex());
    assert!(a.writer.starts_with("s3a-engine "));
    assert_eq!(ab.embedded_spec, "match");
    assert!(!ab.corrupt);
    // The catalog still decodes with the trailer present.
    assert!(r.catalog().is_ok());
    drop(r);
    let rep = verify_sealed_archive(&p, None).unwrap();
    assert!(rep.is_sealed);
    assert!(
        !rep.out_of_order_detected,
        "manifest reorder must restamp tile_index"
    );
}

#[test]
fn about_is_opt_in() {
    let p = seal("plain", &SealOptionsV2::default().with_manifest());
    let r = MmapReader::open(&p).unwrap();
    assert_eq!(about_report(&r).about, None);
}

#[test]
fn manifest_lists_record_root_and_never_fakes_created() {
    let p = seal(
        "rr",
        &SealOptionsV2::default().with_manifest().with_record_roots(),
    );
    let r = MmapReader::open(&p).unwrap();
    let m = r.get_manifest().unwrap();
    assert!(m.contains("83       record_root"), "{m}");
    let floors = m
        .lines()
        .find_map(|l| l.trim().strip_prefix("Total Floors: "))
        .unwrap();
    assert_eq!(floors.parse::<u32>().unwrap(), r.tile_count());
    assert!(!m.contains("Genesis (Unset)"));
    assert!(m.contains("SHA-256 of spec text: "));
}

#[test]
fn manifest_without_any_timestamp_says_not_recorded() {
    let t = s3a_engine::generate_plain_text_manifest(
        None,
        None,
        [1, 2],
        0,
        1_700_000_000,
        &[],
        (0, 0, 0),
        1,
        &[0; 32],
        false,
    );
    assert!(t.contains("Created:          Not recorded"));
    assert!(!t.contains("Genesis"));
}

// ---- about entry survives rewrites (compaction rebuilds the Schema tile; fusion copies it) ----

fn with_about_input(name: &str) -> (std::path::PathBuf, s3a_core::SchemaAbout) {
    use s3a_core::{
        HyperTileHeader, TileType, FILE_HEADER_SIZE, HYPER_TILE_HEADER_SIZE, HYPER_TILE_SIZE,
    };
    let src = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../conformance/vectors-1.1/files/valid_multi_tile.s3a"
    );
    let dir = std::env::temp_dir();
    let base = dir.join(format!(
        "s3a_about_rw_base_{name}_{}.s3a",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&base);
    s3a_engine::Compactor::compact(&[std::path::PathBuf::from(src)], base.clone()).unwrap();
    let about = s3a_core::SchemaAbout {
        spec_version: "test".into(),
        spec_url: "https://example.invalid/spec".into(),
        spec_sha256: "2".repeat(64),
        writer: "self_describing.rs".into(),
        ..Default::default()
    };
    let trailer = s3a_core::encode_schema_about(&about);
    let mut b = std::fs::read(&base).unwrap();
    let mut off = FILE_HEADER_SIZE;
    loop {
        let mut h: HyperTileHeader =
            bytemuck::pod_read_unaligned(&b[off..off + HYPER_TILE_HEADER_SIZE]);
        if h.tile_type == TileType::SCHEMA {
            let p0 = off + HYPER_TILE_HEADER_SIZE;
            let end = p0 + h.payload_bytes as usize;
            b[end..end + trailer.len()].copy_from_slice(&trailer);
            h.payload_bytes += trailer.len() as u32;
            h.data_crc32 = crc32c::crc32c(&b[p0..p0 + h.payload_bytes as usize]);
            h.header_crc32 = s3a_engine::compute_tile_header_crc32(&h);
            b[off..off + HYPER_TILE_HEADER_SIZE].copy_from_slice(bytemuck::bytes_of(&h));
            break;
        }
        off += HYPER_TILE_SIZE;
    }
    let p = dir.join(format!("s3a_about_rw_in_{name}_{}.s3a", std::process::id()));
    std::fs::write(&p, b).unwrap();
    assert_eq!(
        about_report(&MmapReader::open(&p).unwrap()).about,
        Some(about.clone())
    );
    (p, about)
}

fn out_path(name: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "s3a_about_rw_out_{name}_{}.s3a",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&p);
    p
}

#[test]
fn compaction_carries_about_through() {
    let (input, about) = with_about_input("compact");
    let out = out_path("compact");
    s3a_engine::Compactor::compact(&[input], out.clone()).unwrap();
    let r = MmapReader::open(&out).unwrap();
    assert_eq!(about_report(&r).about, Some(about));
    assert!(r.catalog().is_ok());
}

#[test]
fn fusion_carries_about_through() {
    let (input, about) = with_about_input("fuse");
    let out = out_path("fuse");
    s3a_engine::TileFusionEngine::fuse_telemetry_tiles(&[&input], &out).unwrap();
    assert_eq!(
        about_report(&MmapReader::open(&out).unwrap()).about,
        Some(about)
    );
}

#[test]
fn reseal_and_repair_carry_about_through() {
    let (input, about) = with_about_input("seal");
    // Sealing without --about keeps the existing entry; parity repair restores tiles verbatim.
    let mut sealed = seal_buffer_v2(
        &std::fs::read(&input).unwrap(),
        &SealOptionsV2 {
            enable_parity: true,
            ..SealOptionsV2::default()
        },
    )
    .unwrap();
    let schema_off = (0..)
        .map(|i| s3a_core::FILE_HEADER_SIZE + i * s3a_core::HYPER_TILE_SIZE)
        .find(|&o| {
            let h: s3a_core::HyperTileHeader =
                bytemuck::pod_read_unaligned(&sealed[o..o + s3a_core::HYPER_TILE_HEADER_SIZE]);
            h.tile_type == s3a_core::TileType::SCHEMA
        })
        .unwrap();
    sealed[schema_off + s3a_core::HYPER_TILE_HEADER_SIZE + 1] ^= 0xFF; // damage the Schema tile
    let p = out_path("sealed_damaged");
    std::fs::write(&p, &sealed).unwrap();
    let fixed = out_path("repaired");
    let rep = verify_sealed_archive(&p, Some(&fixed)).unwrap();
    assert_eq!(rep.repaired_tiles.len(), 1);
    assert_eq!(
        about_report(&MmapReader::open(&fixed).unwrap()).about,
        Some(about)
    );
}
