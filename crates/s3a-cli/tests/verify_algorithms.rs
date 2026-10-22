//! `s3a-cli verify` rejects sealed archives that declare an unknown or reserved algorithm ID
//! (TILE-SPEC §14.5.4, registry.md §6) and accepts registered ones.
use std::process::Command;

fn seal_and_verify(case: &str) -> (Option<i32>, String) {
    let src = format!(
        "{}/../../conformance/vectors-1.5/files/{case}.s3a",
        env!("CARGO_MANIFEST_DIR")
    );
    let dir = std::env::temp_dir();
    let input = dir.join(format!("s3a_alg_{case}_{}.s3a", std::process::id()));
    let sealed = dir.join(format!("s3a_alg_{case}_{}.sealed.s3a", std::process::id()));
    std::fs::copy(&src, &input).unwrap();
    let bin = env!("CARGO_BIN_EXE_s3a-cli");
    // Sealing without --about copies the Schema tile, so the declared IDs carry through.
    let st = Command::new(bin)
        .args(["seal", input.to_str().unwrap(), sealed.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(st.success(), "seal {case}");
    let out = Command::new(bin)
        .args(["verify", sealed.to_str().unwrap()])
        .output()
        .unwrap();
    let _ = std::fs::remove_file(&input);
    let _ = std::fs::remove_file(&sealed);
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn reserved_ml_dsa_is_rejected() {
    let (code, err) = seal_and_verify("alg_reserved_ml_dsa");
    assert_eq!(code, Some(2));
    assert!(
        err.contains("UnsupportedAlgorithm") && err.contains("ml-dsa-65"),
        "{err}"
    );
}

#[test]
fn unknown_seal_hash_is_rejected() {
    let (code, err) = seal_and_verify("alg_unknown_seal_hash");
    assert_eq!(code, Some(2));
    assert!(
        err.contains("UnknownAlgorithm") && err.contains("sha3-512"),
        "{err}"
    );
}

#[test]
fn registered_ids_verify() {
    assert_eq!(seal_and_verify("alg_declared_recommended").0, Some(0));
}
