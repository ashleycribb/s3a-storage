//! ADR-0012 follow-up: record-root tile (type 83) covered by the unchanged v2 seal hash.

use s3a_core::{FileHeader, FILE_HEADER_SLOT_A_OFFSET, FILE_HEADER_SLOT_SIZE};
use s3a_engine::{compute_header_crc32, seal_buffer_v2, verify_sealed_archive, SealOptionsV2};

pub const LEGACY: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/legacy_v1_0_telemetry.s3a"
);

/// Legacy fixture with a fixed file_uuid so seals are deterministic.
pub fn legacy_fixed() -> Vec<u8> {
    let mut b = std::fs::read(LEGACY).unwrap();
    let h: &mut FileHeader = bytemuck::from_bytes_mut(
        &mut b[FILE_HEADER_SLOT_A_OFFSET..FILE_HEADER_SLOT_A_OFFSET + FILE_HEADER_SLOT_SIZE],
    );
    h.file_uuid = [0x1111, 0x2222];
    h.header_crc32 = compute_header_crc32(h);
    b
}

pub fn tmp(name: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("s3a_rr_{}_{}.s3a", name, std::process::id()));
    let _ = std::fs::remove_file(&p);
    p
}

pub fn seal_to(name: &str, input: &[u8], o: &SealOptionsV2) -> (std::path::PathBuf, [u8; 32]) {
    let out = seal_buffer_v2(input, o).unwrap();
    let p = tmp(name);
    std::fs::write(&p, out).unwrap();
    let rep = verify_sealed_archive(&p, None).unwrap();
    assert!(rep.is_sealed);
    (p, rep.seal_hash)
}

use s3a_core::{HyperTileHeader, TileType, FILE_HEADER_SIZE, HYPER_TILE_HEADER_SIZE};
use s3a_edge::merkle;
use s3a_engine::record_roots::{self, tile_root};
use s3a_engine::{Compactor, MmapReader, TileFusionEngine, TileWriter};

/// Seal hash of the fixed-uuid legacy fixture sealed with default options, computed on `main`
/// before this change (a0e3037). Files without the tile must seal exactly as before.
const MAIN_SEAL_HASH: &str = "2eb4622fb94f097d125d26a32d6741a351500e981041cc3bb82b66e3f8ccf896";

fn rr_opts() -> SealOptionsV2 {
    SealOptionsV2::default().with_record_roots()
}

fn count_rr(p: &std::path::Path) -> usize {
    let r = MmapReader::open(p).unwrap();
    (0..r.tile_count() as usize)
        .filter_map(|i| r.get_tile(i))
        .filter(|(h, _)| h.tile_type == TileType::RECORD_ROOT)
        .count()
}

#[test]
fn seal_hash_without_tile_unchanged_vs_main() {
    let (p, h) = seal_to("nohash", &legacy_fixed(), &SealOptionsV2::default());
    assert_eq!(hex::encode(h), MAIN_SEAL_HASH);
    assert_eq!(count_rr(&p), 0);
    assert!(!record_roots::check(&MmapReader::open(&p).unwrap()).present);
}

#[test]
fn legacy_v1_0_file_unaffected() {
    let r = MmapReader::open(LEGACY).unwrap();
    let rep = record_roots::check(&r);
    assert!(!rep.present && rep.ok());
}

#[test]
fn seal_with_tile_then_prove_against_it() {
    let (p, _) = seal_to("with", &legacy_fixed(), &rr_opts());
    assert_eq!(count_rr(&p), 1);
    let r = MmapReader::open(&p).unwrap();
    let rep = record_roots::check(&r);
    assert!(rep.present && rep.ok(), "{:?}", rep);
    let (h, payload) = r.get_tile(0).unwrap();
    let sealed = record_roots::sealed_root_for(&r, 0).expect("tile 0 root sealed");
    assert_eq!(sealed.record_count, h.record_count as u64);
    assert_eq!(Some(sealed.root), tile_root(h, payload));
    // inclusion proof for one record verifies against the sealed root
    let n = h.record_count as usize;
    let rs = h.payload_bytes as usize / n;
    let mut buf = [[0u8; 32]; merkle::MAX_PROOF_LEN];
    let k = n / 2;
    let len = merkle::inclusion_proof_fixed(&payload[..n * rs], rs, k, &mut buf).unwrap();
    assert!(merkle::verify_inclusion(
        &payload[k * rs..(k + 1) * rs],
        k as u64,
        n as u64,
        &buf[..len],
        &sealed.root
    ));
}

#[test]
fn tampered_record_detected() {
    let out = seal_buffer_v2(&legacy_fixed(), &rr_opts()).unwrap();
    let mut bad = out.clone();
    bad[FILE_HEADER_SIZE + HYPER_TILE_HEADER_SIZE + 3] ^= 0xFF; // tile 0, record 0
    let p = tmp("tamper");
    std::fs::write(&p, &bad).unwrap();
    let rep = record_roots::check(&MmapReader::open(&p).unwrap());
    assert!(!rep.ok());
    assert!(
        rep.mismatches.iter().any(|m| m.contains("tile 0")),
        "{:?}",
        rep
    );
}

#[test]
fn stale_root_never_survives_compaction_fusion_or_reseal() {
    let (p, _) = seal_to("stale", &legacy_fixed(), &rr_opts());
    let c = tmp("stale_c");
    Compactor::compact(&[&p], &c).unwrap();
    assert_eq!(count_rr(&c), 0, "compactor must drop the derived tile");
    let f = tmp("stale_f");
    TileFusionEngine::fuse_telemetry_tiles(&[&p], &f).unwrap();
    assert_eq!(count_rr(&f), 0, "fusion must drop the derived tile");
    // re-sealing a file carrying a record-root tile regenerates it (with flag) or drops it.
    let sealed = std::fs::read(&p).unwrap();
    let (p2, _) = seal_to("reseal_on", &sealed, &rr_opts());
    assert_eq!(count_rr(&p2), 1);
    assert!(record_roots::check(&MmapReader::open(&p2).unwrap()).ok());
    let (p3, h3) = seal_to("reseal_off", &sealed, &SealOptionsV2::default());
    assert_eq!(count_rr(&p3), 0);
    assert_eq!(hex::encode(h3), MAIN_SEAL_HASH);
}

#[test]
fn unknown_tile_pass_through_by_type_agnostic_copier() {
    // An older tool that does not know type 83 copies every tile through byte-for-byte.
    let (p, _) = seal_to("pt", &legacy_fixed(), &rr_opts());
    let r = MmapReader::open(&p).unwrap();
    let o = tmp("pt_out");
    let mut w = TileWriter::create(&o).unwrap();
    for i in 0..r.tile_count() as usize {
        let (h, payload) = r.get_tile(i).unwrap();
        let mut nh = HyperTileHeader::new(h.tile_id, h.tile_type);
        nh.record_count = h.record_count;
        nh.record_layout_version = h.record_layout_version;
        w.write_raw_hyper_tile(&nh, &payload[..h.payload_bytes as usize])
            .unwrap();
    }
    drop(w);
    let r2 = MmapReader::open(&o).unwrap();
    r2.verify_checksums().unwrap();
    assert_eq!(count_rr(&o), 1);
    assert_eq!(
        record_roots::read_entries(&r2),
        record_roots::read_entries(&r)
    );
    assert!(record_roots::check(&r2).ok());
}
