//! One-time generator for crates/s3a-wasm-snapshot/tests/fixtures/sealed_with_schema.s3a,
//! the sealed archive smoke.mjs uses to check `read_embedded_registry` in WASM.
//! Built from the frozen acceptance_009 inputs and sealed by the native engine with an
//! embedded Schema tile (type 77) and about entry. Never overwrites an existing file.
//! Run from the repo root:
//!   cargo run -p s3a-pack-scholar --example gen_wasm_sealed_schema_fixture
use s3a_pack_scholar::{ScholarGraphEdge, ScholarPaperRecord};
use sha2::Digest;
use std::path::Path;

const SRC: &str = "crates/s3a-wasm-snapshot/tests/fixtures/acceptance_009";
const OUT: &str = "crates/s3a-wasm-snapshot/tests/fixtures/sealed_with_schema.s3a";

fn main() {
    let out = Path::new(OUT);
    if out.exists() {
        println!("{OUT} exists; refusing to overwrite a frozen fixture");
        return;
    }
    let src = Path::new(SRC);
    let base = std::fs::read(src.join("base.s3a")).expect("base.s3a");
    let paper: ScholarPaperRecord =
        serde_json::from_str(&std::fs::read_to_string(src.join("paper.json")).unwrap()).unwrap();
    let edge: ScholarGraphEdge =
        serde_json::from_str(&std::fs::read_to_string(src.join("edge.json")).unwrap()).unwrap();
    let bytes = paper.put_to_bytes(&base).expect("put paper");
    let bytes = edge.add_to_bytes(&bytes).expect("add edge");
    let opts = s3a_engine::SealOptionsV2 {
        about: true,
        write_manifest: true,
        ..Default::default()
    };
    let sealed = s3a_engine::seal_buffer_v2(&bytes, &opts).expect("seal");
    let info = s3a_engine::read_embedded_registry_from_bytes(&sealed).expect("embedded registry");
    assert!(
        info.spec_version.is_some(),
        "sealed fixture must carry an about entry"
    );
    assert!(
        info.types.iter().any(|t| t.type_id == 12) && info.types.iter().any(|t| t.type_id == 13)
    );
    std::fs::write(out, &sealed).unwrap();
    println!("wrote {OUT} sha256={:x}", sha2::Sha256::digest(&sealed));
}
