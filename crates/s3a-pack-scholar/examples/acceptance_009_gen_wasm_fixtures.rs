//! One-time generator for crates/s3a-wasm-snapshot/tests/fixtures/acceptance_009/.
//! Writes only missing files; never overwrites. Run from the repo root:
//!   cargo run -p s3a-pack-scholar --example acceptance_009_gen_wasm_fixtures [-- --base-only]
use s3a_pack_scholar::{ScholarGraphEdge, ScholarPaperRecord};
use sha2::Digest;
use std::path::Path;

const DIR: &str = "crates/s3a-wasm-snapshot/tests/fixtures/acceptance_009";

pub fn base_archive() -> Vec<u8> {
    let mut fh = s3a_core::FileHeader::new(0, 1_763_164_800);
    fh.addon_mask = s3a_core::ADDON_EDU;
    fh.file_uuid = [0x0009_0009_0009_0009, 0x5EED_5EED_5EED_5EED];
    fh.generation = 1;
    fh.header_crc32 = s3a_engine::compute_header_crc32(&fh);
    let mut b = vec![0u8; s3a_core::FILE_HEADER_SIZE];
    b[..s3a_core::FILE_HEADER_SLOT_SIZE].copy_from_slice(bytemuck::bytes_of(&fh));
    b
}

fn write_new(name: &str, bytes: &[u8]) {
    let p = Path::new(DIR).join(name);
    if p.exists() {
        let old = std::fs::read(&p).unwrap();
        assert_eq!(
            old, bytes,
            "{name} exists and differs; refusing to overwrite a frozen fixture"
        );
        println!("unchanged {name}");
        return;
    }
    std::fs::write(&p, bytes).unwrap();
    println!("wrote {name} sha256={:x}", sha2::Sha256::digest(bytes));
}

fn main() {
    let base = base_archive();
    write_new("base.s3a", &base);
    if std::env::args().any(|a| a == "--base-only") {
        return;
    }
    let paper: ScholarPaperRecord =
        serde_json::from_str(&std::fs::read_to_string(Path::new(DIR).join("paper.json")).unwrap())
            .unwrap();
    let edge: ScholarGraphEdge =
        serde_json::from_str(&std::fs::read_to_string(Path::new(DIR).join("edge.json")).unwrap())
            .unwrap();
    let p = paper.put_to_bytes(&base).expect("put paper");
    write_new("expected_put_paper.s3a", &p);
    write_new(
        "expected_add_edge.s3a",
        &edge.add_to_bytes(&p).expect("add edge"),
    );
}
