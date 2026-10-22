//! `s3a-cli verify` exit codes: 0 ok (warnings allowed), 2 failed, 3 record-root mismatch.

use std::path::PathBuf;
use std::process::{Command, Output};

fn tmp(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "s3a_verify_exit_{}_{}.s3a",
        label,
        std::process::id()
    ))
}
struct G(Vec<PathBuf>);
impl Drop for G {
    fn drop(&mut self) {
        for p in &self.0 {
            let _ = std::fs::remove_file(p);
        }
    }
}
fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_s3a-cli"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn verify_exit_codes() {
    let (raw, sealed, bad) = (tmp("raw"), tmp("sealed"), tmp("bad"));
    let _g = G(vec![raw.clone(), sealed.clone(), bad.clone()]);
    let (r, s, b) = (
        raw.to_str().unwrap(),
        sealed.to_str().unwrap(),
        bad.to_str().unwrap(),
    );
    assert!(run(&["insert", r, "100", "1", "1", "5.0"]).status.success());

    // Warning-only (unsealed, footer_missing): exit 0.
    let o = run(&["verify", r]);
    assert_eq!(o.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&o.stdout).contains("Warning:"));

    // Clean sealed archive: exit 0.
    assert!(run(&["seal", r, s]).status.success());
    assert_eq!(run(&["verify", s]).status.code(), Some(0));

    // Corrupt every tile (beyond Reed-Solomon repair): exit 2, text and JSON.
    let mut bytes = std::fs::read(&sealed).unwrap();
    for x in bytes.iter_mut().skip(512).filter(|x| **x != 0) {
        *x ^= 0x5a;
    }
    std::fs::write(&bad, &bytes).unwrap();
    let o = run(&["verify", b]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("FAILED"));
    let o = run(&["verify", "--json", b]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stdout).contains("\"ok\":false"));

    // Missing file: exit 2.
    assert_eq!(
        run(&["verify", "/nonexistent/x.s3a"]).status.code(),
        Some(2)
    );
}
