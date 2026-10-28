//! `MmapReader::from_bytes` / `verify_sealed_bytes` must behave exactly like the path-based
//! calls on every committed conformance archive (valid and deliberately damaged alike).

use std::path::PathBuf;

use s3a_engine::{sealed::verify_sealed_archive, sealed::verify_sealed_bytes, MmapReader};
use serde_json::Value;

fn suite() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance")
        .join(s3a_spec_vectors::conformance::SUITE_VERSION)
}

/// Everything observable from a reader, as a string, so two readers can be compared.
fn fingerprint(r: &MmapReader) -> String {
    let mut s = format!(
        "gen={} slot={} tiles={} stride={} edge={} addon={} uuid={:?} hdr={:?} ext={} size_ok={:?} crc={:?}\n",
        r.generation(),
        r.active_slot(),
        r.tile_count(),
        r.tile_stride(),
        r.is_edge_archive(),
        r.addon_mask(),
        r.file_uuid(),
        bytemuck::bytes_of(r.file_header()),
        r.header_extent(),
        r.validate_file_size().map_err(|e| format!("{e:?}")),
        r.verify_checksums().map_err(|e| format!("{e:?}")),
    );
    for i in 0..(r.tile_count() as usize).min(64) {
        if r.is_edge_archive() {
            match r.try_get_edge_tile(i) {
                Ok((h, p)) => {
                    s.push_str(&format!("e{i} {:?} {}\n", bytemuck::bytes_of(h), p.len()))
                }
                Err(e) => s.push_str(&format!("e{i} err {e:?}\n")),
            }
        } else {
            match r.try_get_tile(i) {
                Ok((h, p)) => s.push_str(&format!(
                    "t{i} {:?} {:?}\n",
                    bytemuck::bytes_of(h),
                    p.iter().map(|b| *b as u32).sum::<u32>()
                )),
                Err(e) => s.push_str(&format!("t{i} err {e:?}\n")),
            }
        }
    }
    s
}

#[test]
fn from_bytes_matches_open_on_every_archive_vector() {
    let dir = suite();
    let m: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap();
    let mut n = 0;
    for c in m["cases"].as_array().unwrap() {
        if c["kind"] != "archive" {
            continue;
        }
        let path = dir.join(c["file"].as_str().unwrap());
        let bytes = std::fs::read(&path).unwrap();
        let a = MmapReader::open(&path);
        let b = MmapReader::from_bytes(&bytes);
        let id = c["id"].as_str().unwrap();
        match (a, b) {
            (Ok(a), Ok(b)) => {
                assert_eq!(fingerprint(&a), fingerprint(&b), "{id}");
                assert_eq!(a.as_bytes(), b.as_bytes(), "{id}");
                assert_eq!(b.as_bytes(), &bytes[..], "{id}");
            }
            (Err(a), Err(b)) => assert_eq!(a.to_string(), b.to_string(), "{id}"),
            (a, b) => panic!("{id}: open={:?} from_bytes={:?}", a.is_ok(), b.is_ok()),
        }
        // Sealed verification: same outcome from the path and from the bytes.
        let vp = verify_sealed_archive(&path, None);
        let vb = verify_sealed_bytes(&bytes);
        assert_eq!(format!("{vp:?}"), format!("{vb:?}"), "{id}: verify_sealed");
        n += 1;
    }
    assert!(n >= 50, "only {n} archive vectors compared");
}

#[test]
fn from_bytes_handles_empty_and_tiny_input_without_panicking() {
    for len in [0usize, 1, 7, 8, 63, 511] {
        let bytes = vec![0u8; len];
        assert!(MmapReader::from_bytes(&bytes).is_err(), "len {len}");
        let _ = verify_sealed_bytes(&bytes);
    }
}

#[test]
fn from_bytes_is_independent_of_the_source_buffer() {
    let dir = suite();
    let mut bytes = std::fs::read(dir.join("files/valid_empty.s3a")).unwrap();
    let r = MmapReader::from_bytes(&bytes).unwrap();
    let before = r.as_bytes().to_vec();
    bytes.fill(0xFF); // mutating the caller's buffer must not affect the reader
    assert_eq!(r.as_bytes(), &before[..]);
}
