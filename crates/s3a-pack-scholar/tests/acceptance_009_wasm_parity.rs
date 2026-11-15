//! Acceptance 009 follow-up: WASM parity, native side. Written before the fix. Do not edit.
//!
//! Native output must equal the committed byte fixtures that the Node smoke test also compares
//! the built WASM against (crates/s3a-wasm-snapshot/tests/fixtures/acceptance_009/README.md).
//! The two `expected_*` fixtures are generated once from native (see README); until then this
//! test fails with "fixture missing".

use s3a_pack_scholar::{ScholarGraphEdge, ScholarPaperRecord};
use std::path::PathBuf;

fn fx(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../s3a-wasm-snapshot/tests/fixtures/acceptance_009")
        .join(name)
}

fn read_fixture(name: &str) -> Vec<u8> {
    std::fs::read(fx(name)).unwrap_or_else(|_| {
        panic!(
            "fixture missing: {name}. Generate once from native after the paper/edge features are final: \
             `cargo run -p s3a-pack-scholar --example acceptance_009_gen_wasm_fixtures`, commit, never edit"
        )
    })
}

fn inputs() -> (Vec<u8>, String, String) {
    (
        read_fixture("base.s3a"),
        String::from_utf8(read_fixture("paper.json")).unwrap(),
        String::from_utf8(read_fixture("edge.json")).unwrap(),
    )
}

#[test]
fn base_fixture_is_a_valid_empty_archive() {
    let base = read_fixture("base.s3a");
    let r = s3a_engine::MmapReader::from_bytes(&base).expect("base.s3a opens with the real reader");
    assert_eq!(r.tile_count(), 0);
    assert_eq!(
        r.file_uuid(),
        [0x0009_0009_0009_0009, 0x5EED_5EED_5EED_5EED]
    );
}

#[test]
fn native_put_paper_and_add_edge_are_deterministic() {
    let (base, pj, ej) = inputs();
    let paper: ScholarPaperRecord = serde_json::from_str(&pj).unwrap();
    let a = paper.put_to_bytes(&base).unwrap();
    assert_eq!(
        a,
        paper.put_to_bytes(&base).unwrap(),
        "same input, same bytes"
    );
    let edge: ScholarGraphEdge = serde_json::from_str(&ej).unwrap();
    assert_eq!(
        edge.add_to_bytes(&a).unwrap(),
        edge.add_to_bytes(&a).unwrap()
    );
}

#[test]
fn native_put_paper_matches_committed_fixture() {
    let (base, pj, _) = inputs();
    let paper: ScholarPaperRecord = serde_json::from_str(&pj).unwrap();
    let native = paper.put_to_bytes(&base).unwrap();
    assert!(
        native == read_fixture("expected_put_paper.s3a"),
        "native put_paper bytes differ from the frozen fixture"
    );
    let back = ScholarPaperRecord::get_from_bytes(&native, "arxiv:1706.03762")
        .unwrap()
        .expect("paper readable");
    assert_eq!(
        serde_json::to_value(&back).unwrap()["published_date"],
        "2017-06-12T00:00:00Z"
    );
}

#[test]
fn native_add_edge_matches_committed_fixture() {
    let (_, _, ej) = inputs();
    let with_paper = read_fixture("expected_put_paper.s3a");
    let edge: ScholarGraphEdge = serde_json::from_str(&ej).unwrap();
    assert!(
        edge.add_to_bytes(&with_paper).unwrap() == read_fixture("expected_add_edge.s3a"),
        "native add_edge bytes differ from the frozen fixture"
    );
}
