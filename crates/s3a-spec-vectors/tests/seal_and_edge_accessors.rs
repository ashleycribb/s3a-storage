//! Read-only seal-footer probe and edge accessors, checked against the committed vectors.

use std::path::PathBuf;

use s3a_engine::{probe_seal_footer, verify_sealed_archive, MmapReader};
use serde_json::Value;

fn suite() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance")
        .join(s3a_spec_vectors::conformance::SUITE_VERSION)
}

fn archives() -> Vec<(String, PathBuf, Vec<u8>)> {
    let dir = suite();
    let m: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap();
    m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["kind"] == "archive")
        .map(|c| {
            let path = dir.join(c["file"].as_str().unwrap());
            (
                c["id"].as_str().unwrap().to_string(),
                path.clone(),
                std::fs::read(path).unwrap(),
            )
        })
        .collect()
}

#[test]
fn probe_agrees_with_full_verification_on_every_vector() {
    let mut sealed_ok = 0;
    for (id, path, bytes) in archives() {
        let p = probe_seal_footer(&bytes); // must never panic
        if let Ok(r) = verify_sealed_archive(&path, None) {
            if r.is_sealed && !r.rebuilt_directory {
                sealed_ok += 1;
                assert!(p.has_endcap_magic, "{id}");
                assert_eq!(p.footer_version, r.footer_version, "{id}");
                assert!(p.footer_intact(), "{id}: {p:?}");
                assert_eq!(p.tile_count, r.tile_count, "{id}");
                assert_eq!((p.k, p.m), (r.k, r.m), "{id}");
                if r.footer_version == 2 {
                    assert_eq!(p.seal_hash, r.seal_hash, "{id}");
                    assert_eq!(p.prev_seal_hash, r.prev_seal_hash, "{id}");
                    assert_eq!(p.seal_generation, r.seal_generation, "{id}");
                }
            } else if r.warnings.iter().any(|w| w.starts_with("endcap_corrupt")) {
                assert!(
                    p.has_endcap_magic && !p.endcap_crc_ok && !p.footer_intact(),
                    "{id}"
                );
            } else if !r.is_sealed && r.warnings.iter().any(|w| w.starts_with("footer_missing")) {
                assert!(!p.footer_intact(), "{id}");
            }
        }
    }
    assert!(sealed_ok >= 2, "expected sealed vectors, saw {sealed_ok}");
}

#[test]
fn probe_never_panics_on_truncated_or_flipped_footers() {
    let (_, _, sealed) = archives()
        .into_iter()
        .find(|(id, _, _)| id == "sealed_ok")
        .unwrap();
    for cut in 0..sealed.len().min(2048) {
        let _ = probe_seal_footer(&sealed[..cut]);
    }
    for i in sealed.len().saturating_sub(300)..sealed.len() {
        let mut b = sealed.clone();
        b[i] ^= 0xFF;
        let p = probe_seal_footer(&b);
        assert!(!p.footer_intact(), "flip at {i} was not noticed");
    }
    // Hostile offsets must not overflow.
    let mut b = sealed.clone();
    let n = b.len();
    b[n - 56..n - 48].copy_from_slice(&u64::MAX.to_le_bytes());
    let _ = probe_seal_footer(&b);
}

#[test]
fn edge_unverified_matches_verified_on_good_tiles_and_shows_damaged_ones() {
    let mut edge_files = 0;
    for (id, path, _) in archives() {
        let Ok(r) = MmapReader::open(&path) else {
            continue;
        };
        if !r.is_edge_archive() {
            continue;
        }
        edge_files += 1;
        for i in 0..r.tile_count() as usize {
            let u = r.try_get_edge_tile_unverified(i);
            match r.try_get_edge_tile(i) {
                Ok((h, p)) => {
                    let (uh, up) = u.unwrap_or_else(|e| panic!("{id}/{i}: {e:?}"));
                    assert_eq!(bytemuck::bytes_of(h), bytemuck::bytes_of(uh), "{id}/{i}");
                    assert_eq!(p, up, "{id}/{i}");
                }
                // A CRC failure is exactly where the unverified accessor must still return data.
                Err(c)
                    if matches!(
                        c.reason,
                        s3a_engine::CorruptReason::HeaderCrcMismatch { .. }
                            | s3a_engine::CorruptReason::DataCrcMismatch { .. }
                    ) =>
                {
                    assert!(
                        u.is_ok(),
                        "{id}/{i}: unverified must still return a CRC-damaged tile"
                    );
                }
                // Structural damage fails identically on both.
                Err(c) => assert_eq!(
                    format!("{:?}", u.unwrap_err()),
                    format!("{c:?}"),
                    "{id}/{i}"
                ),
            }
        }
        // Out-of-range never panics.
        assert!(r
            .try_get_edge_tile_unverified(r.tile_count() as usize + 7)
            .is_err());
    }
    assert!(edge_files >= 1);
}

/// Builds a small signed edge archive the way a device does: identity first, then data tiles,
/// each followed by a seal tile that carries the Ed25519 signature over the previous tile's
/// chain hash. (The committed `micro_signed_seal_chain` vector signs in memory but stores no
/// seal tiles, so it cannot exercise the stored records.)
fn signed_edge_archive() -> (Vec<u8>, [u8; 32], [u8; 16]) {
    use s3a_edge::{
        sign_seal_hash, CompactEdgeSensorRecord, DeviceIdentityRecord, DeviceSealRecord,
        EdgeWriter, RamFlash, SigningKey, TILE_TYPE_SEAL,
    };
    let device_id = [0x11u8; 16];
    let sk = SigningKey::from_bytes(&[0x42u8; 32]);
    let pk = sk.verifying_key().to_bytes();
    let flash = RamFlash::<65536>::new(256, 512);
    let mut w = EdgeWriter::<_, 4096>::open_or_format(
        flash,
        device_id,
        [0x1111_2222_3333_4444, 0x5555_6666_7777_8888],
        1_700_000_000,
    )
    .unwrap();
    w.append_device_identity(&DeviceIdentityRecord::new(
        device_id,
        "test-model",
        [1, 0, 0, 0],
        pk,
        1_700_000_000,
        1,
    ))
    .unwrap();
    let mut seal = w.seal_tile(1_700_000_001).unwrap(); // tile 0: identity
    let mut idx = 0u32; // index of the tile `seal` belongs to
    let mut prev = [0u8; 32];
    for k in 0..2u64 {
        // A seal tile (idx + 1) carries the signature for tile `idx`.
        let rec =
            DeviceSealRecord::new(idx, w.generation(), seal, prev, sign_seal_hash(&sk, &seal));
        w.append_record(TILE_TYPE_SEAL, bytemuck::bytes_of(&rec))
            .unwrap();
        let seal_tile_hash = w.seal_tile(1_700_000_010 + k).unwrap();
        prev = seal_tile_hash; // chain predecessor of the next data tile
                               // Then a data tile (idx + 2), which the next round signs.
        w.append_sensor_record(&CompactEdgeSensorRecord::new(k as u16, 1, 1, 200, 0))
            .unwrap();
        seal = w.seal_tile(1_700_000_020 + k).unwrap();
        idx += 2;
    }
    let n = 512 + w.tile_count() as usize * 4096;
    (w.into_storage().as_slice()[..n].to_vec(), pk, device_id)
}

#[test]
fn edge_signed_chain_identity_and_seal_records() {
    let (bytes, pk, device_id) = signed_edge_archive();
    let path = std::env::temp_dir().join(format!("s3a_edge_acc_{}.s3a", std::process::id()));
    std::fs::write(&path, &bytes).unwrap();
    let r = MmapReader::open(&path).unwrap();
    std::fs::remove_file(&path).unwrap();

    let id = r.edge_device_identity().expect("device identity");
    assert!(id.verify());
    assert_eq!(id.public_key, pk);
    assert_eq!(id.device_id, device_id);
    let seals = r.edge_seal_records();
    assert!(!seals.is_empty(), "archive carries stored seal records");
    let chain = r.edge_seal_chain(&device_id);
    assert_eq!(chain.len(), r.tile_count() as usize);
    let wrong = r.edge_seal_chain(&[0xAB; 16]);
    for e in &seals {
        let rec = &e.record;
        assert!(rec.verify());
        // The scheme (S3A-EDGE-PROFILE section 6): the record's seal_hash is the chain hash of
        // the tile it names, and the signature is Ed25519 over those 32 bytes under the device key.
        assert_eq!(
            chain.get(rec.tile_index as usize),
            Some(&rec.seal_hash),
            "tile {}",
            rec.tile_index
        );
        assert!(s3a_edge::verify_signature_raw(
            &id.public_key,
            &rec.seal_hash,
            &rec.signature
        ));
        assert_ne!(wrong.get(rec.tile_index as usize), Some(&rec.seal_hash));
    }
}

#[test]
fn edge_accessors_are_empty_on_hyper_tile_archives() {
    for (id, path, _) in archives() {
        let Ok(r) = MmapReader::open(&path) else {
            continue;
        };
        if r.is_edge_archive() {
            continue;
        }
        assert!(r.edge_device_identity().is_none(), "{id}");
        assert!(r.edge_seal_records().is_empty(), "{id}");
        assert!(r.edge_seal_chain(&[0; 16]).is_empty(), "{id}");
    }
}
