//! Per-record Merkle proofs against the committed legacy v1.0 fixture after `migrate`
//! (old-file compatibility). Raw v1.0 files fail closed on read until migrated.
use s3a_cli::proof_cli::{prove, tile_record_root, verify};
use std::process::Command;
use std::sync::OnceLock;

const LEGACY: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../s3a-engine/tests/fixtures/legacy_v1_0_telemetry.s3a"
);

/// The v1.0 fixture migrated to the current format once per test process.
fn migrated() -> &'static str {
    static P: OnceLock<String> = OnceLock::new();
    P.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("s3a_proof_mig_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("migrated.s3a");
        s3a_engine::migrate::migrate_file(std::path::Path::new(LEGACY), &out, false).unwrap();
        out.to_str().unwrap().to_string()
    })
}

#[test]
fn migrated_legacy_file_records_prove_and_verify() {
    let mig = migrated();
    let (root, n) = tile_record_root(mig, 0).expect("legacy tile 0 root");
    assert!(n > 0);
    for r in [0, (n / 2) as usize, (n - 1) as usize] {
        let p = prove(mig, 0, r).unwrap();
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
    assert!(prove(mig, 0, n as usize).is_err());
}

#[test]
fn cli_prove_then_verify_proof() {
    let mig = migrated();
    let bin = env!("CARGO_BIN_EXE_s3a-cli");
    let out = Command::new(bin)
        .args(["prove", mig, "0", "1"])
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
        .args(["verify-proof", path.to_str().unwrap(), mig])
        .output()
        .unwrap();
    let _ = std::fs::remove_file(&path);
    assert!(v.status.success());
    assert!(String::from_utf8_lossy(&v.stdout).contains("PROOF OK"));
}

#[test]
fn cli_seal_record_roots_then_verify_proof_against_seal() {
    let bin = env!("CARGO_BIN_EXE_s3a-cli");
    let dir = std::env::temp_dir();
    let id = std::process::id();
    let inp = dir.join(format!("s3a_rr_in_{}.s3a", id));
    let sealed = dir.join(format!("s3a_rr_sealed_{}.s3a", id));
    let proof = dir.join(format!("s3a_rr_proof_{}.json", id));
    std::fs::copy(LEGACY, &inp).unwrap();
    let s = Command::new(bin)
        .args([
            "seal",
            inp.to_str().unwrap(),
            sealed.to_str().unwrap(),
            "--record-roots",
        ])
        .output()
        .unwrap();
    assert!(s.status.success(), "{}", String::from_utf8_lossy(&s.stdout));
    let v = Command::new(bin)
        .args(["verify", sealed.to_str().unwrap()])
        .output()
        .unwrap();
    let vo = String::from_utf8_lossy(&v.stdout);
    assert!(vo.contains("Record roots: OK"), "{}", vo);
    // find the telemetry tile (manifest/schema tiles shift indices)
    let r = s3a_engine::MmapReader::open(&sealed).unwrap();
    let t = (0..r.tile_count() as usize)
        .find(|&i| r.get_tile(i).unwrap().0.tile_type == s3a_core::TileType::TELEMETRY)
        .unwrap()
        .to_string();
    let p = Command::new(bin)
        .args(["prove", sealed.to_str().unwrap(), &t, "1"])
        .output()
        .unwrap();
    assert!(p.status.success());
    std::fs::write(&proof, &p.stdout).unwrap();
    let ok = Command::new(bin)
        .args([
            "verify-proof",
            proof.to_str().unwrap(),
            sealed.to_str().unwrap(),
            "--against-seal",
        ])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&ok.stdout).contains("PROOF OK"));
    // legacy file has no record-root tile: --against-seal must refuse, not pass silently
    let no = Command::new(bin)
        .args([
            "verify-proof",
            proof.to_str().unwrap(),
            LEGACY,
            "--against-seal",
        ])
        .output()
        .unwrap();
    assert!(!no.status.success());
    for f in [&inp, &sealed, &proof] {
        let _ = std::fs::remove_file(f);
    }
}

#[test]
fn raw_v1_0_file_fails_closed_on_prove() {
    assert!(tile_record_root(LEGACY, 0).is_err());
}
