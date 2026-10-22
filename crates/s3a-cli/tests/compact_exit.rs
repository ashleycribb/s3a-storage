//! `s3a-cli compact` must exit nonzero when compaction fails (scripts rely on it).

#[test]
fn compact_failure_exits_nonzero() {
    let dir = std::env::temp_dir();
    let out = dir.join(format!("s3a_cexit_out_{}.s3a", std::process::id()));
    let missing = dir.join(format!("s3a_cexit_missing_{}.s3a", std::process::id()));
    let st = std::process::Command::new(env!("CARGO_BIN_EXE_s3a-cli"))
        .args(["compact", out.to_str().unwrap(), missing.to_str().unwrap()])
        .status()
        .unwrap();
    assert_eq!(st.code(), Some(2));
    assert!(!out.exists());
}

#[test]
fn compact_success_exits_zero() {
    let src = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../conformance/vectors-1.1/files/valid_multi_tile.s3a"
    );
    let out = std::env::temp_dir().join(format!("s3a_cexit_ok_{}.s3a", std::process::id()));
    let _ = std::fs::remove_file(&out);
    let st = std::process::Command::new(env!("CARGO_BIN_EXE_s3a-cli"))
        .args(["compact", out.to_str().unwrap(), src])
        .status()
        .unwrap();
    assert_eq!(st.code(), Some(0));
}
