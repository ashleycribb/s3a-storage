//! A1: recomputes spec-vectors/hashes/*.json from the reference code.
use serde_json::Value;
use sha2::{Digest, Sha256};

fn v(name: &str) -> Value {
    let p = format!(
        "{}/../../spec-vectors/hashes/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}
fn hx(j: &Value, k: &str) -> Vec<u8> {
    hex::decode(j[k].as_str().unwrap()).unwrap()
}

#[test]
fn payload_crc32c() {
    let j = v("payload_crc32c");
    assert_eq!(
        crc32c::crc32c(&hx(&j, "input_hex")) as u64,
        j["expected_u32"].as_u64().unwrap()
    );
}

#[test]
fn tile_header_crc32c() {
    let j = v("tile_header_crc32c");
    let bytes = hx(&j, "input_hex");
    let h: s3a_engine::HyperTileHeader = bytemuck::pod_read_unaligned(&bytes);
    assert_eq!(
        s3a_engine::compute_tile_header_crc32(&h) as u64,
        j["expected_u32"].as_u64().unwrap()
    );
}

#[test]
fn provenance_root() {
    // Mirrors s3a-engine lib.rs (~L566-572); the writer computes it inline.
    let j = v("provenance_root");
    let p = hx(&j, "input_hex");
    let mut root = [0u8; 32];
    root[0..4].copy_from_slice(&crc32c::crc32c(&p).to_le_bytes());
    root[4..8].copy_from_slice(&(p.len() as u32).to_le_bytes());
    assert_eq!(root.to_vec(), hx(&j, "expected_hex"));
}

#[test]
fn seal_hash_v2() {
    let j = v("seal_hash_v2");
    let prev: [u8; 32] = hx(&j, "prev_seal_hash_hex").try_into().unwrap();
    let u = j["file_uuid"].as_array().unwrap();
    let got = s3a_engine::compute_seal_hash_v2(
        &prev,
        j["seal_generation"].as_u64().unwrap(),
        j["tile_count"].as_u64().unwrap() as u32,
        [u[0].as_u64().unwrap(), u[1].as_u64().unwrap()],
        &[],
    );
    assert_eq!(got.to_vec(), hx(&j, "expected_hex"));
}

#[test]
fn gate_address() {
    let j = v("gate_address");
    let pk: [u8; 32] = hx(&j, "input_hex").try_into().unwrap();
    assert_eq!(
        s3a_vault::GateAddress::from_public_key(&pk).to_address_string(),
        j["expected"].as_str().unwrap()
    );
}

#[test]
fn sync_inventory() {
    // Mirrors s3a-vcs sync.rs (~L62): Sha256::digest(payload).
    let j = v("sync_inventory");
    assert_eq!(
        Sha256::digest(hx(&j, "input_hex")).to_vec(),
        hx(&j, "expected_hex")
    );
}
