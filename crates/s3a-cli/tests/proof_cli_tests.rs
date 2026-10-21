//! Per-record Merkle proofs against the committed legacy v1.0 fixture (old-file compatibility).
use s3a_cli::proof_cli::{prove, tile_record_root, verify};
use std::process::Command;

const LEGACY: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../s3a-engine/tests/fixtures/legacy_v1_0_telemetry.s3a"
);

#[test]
fn legacy_file_records_prove_and_verify() {
    let (root, n) = tile_record_root(LEGACY, 0).expect("legacy tile 0 root");
    assert!(n > 0);
    for r in [0, (n / 2) as usize, (n - 1) as usize] {
        let p = prove(LEGACY, 0, r).unwrap();
        assert!(verify(&p, Some(&root)).unwrap());
        let mut bad = p.clone();
        let mut rec = hex::decode(&bad.record_hex).unwrap();
        rec[0] ^= 1;
        bad.record_hex = hex::encode(rec);
        assert!(!verify(&bad, Some(&root)).unwrap());
        if let Some(first) = p.path_hex.first() {
            let mut bad = p.clone();
            let mut b = hex::decode(first).unwrap();
            b[31] ^= 1;
            bad.path_hex[0] = hex::encode(b);
            assert!(!verify(&bad, None).unwrap());
        }
        assert!(!verify(&p, Some(&[0u8; 32])).unwrap());
    }
    assert!(prove(LEGACY, 0, n as usize).is_err());
}

#[test]
fn cli_prove_then_verify_proof() {
    let bin = env!("CARGO_BIN_EXE_s3a-cli");
    let out = Command::new(bin)
        .args(["prove", LEGACY, "0", "1"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut path = std::env::temp_dir();
    path.push(format!("s3a_proof_{}.json", std::process::id()));
    std::fs::write(&path, &out.stdout).unwrap();
    let v = Command::new(bin)
        .args(["verify-proof", path.to_str().unwrap(), LEGACY])
        .output()
        .unwrap();
    let _ = std::fs::remove_file(&path);
    assert!(v.status.success());
    assert!(String::from_utf8_lossy(&v.stdout).contains("PROOF OK"));
}
