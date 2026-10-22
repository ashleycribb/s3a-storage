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

fn json(o: &Output) -> serde_json::Value {
    let s = String::from_utf8_lossy(&o.stdout);
    serde_json::from_str(s.trim()).unwrap_or_else(|e| panic!("one JSON doc expected: {e}: {s}"))
}

#[test]
fn verify_json_record_roots_and_anchor_exit_4() {
    let (raw, plain, rr, anc, bad) = (
        tmp("j_raw"),
        tmp("j_plain"),
        tmp("j_rr"),
        tmp("j_anc"),
        tmp("j_bad"),
    );
    let _g = G(vec![
        raw.clone(),
        plain.clone(),
        rr.clone(),
        anc.clone(),
        bad.clone(),
    ]);
    let p = |x: &PathBuf| x.to_str().unwrap().to_string();
    assert!(run(&["insert", &p(&raw), "100", "1", "1", "5.0"])
        .status
        .success());
    assert!(run(&["seal", &p(&raw), &p(&plain)]).status.success());
    assert!(run(&["seal", &p(&raw), &p(&rr), "--record-roots"])
        .status
        .success());

    let o = run(&["verify", "--json", &p(&plain)]);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(json(&o)["record_roots"]["state"], "absent");
    let o = run(&["verify", "--json", &p(&rr)]);
    assert_eq!(o.status.code(), Some(0));
    let j = json(&o);
    assert_eq!(j["record_roots"]["state"], "ok");
    assert_eq!(j["ok"], true);

    // Good anchor: exit 0, anchors_ok true.
    std::fs::copy(&plain, &anc).unwrap();
    assert!(run(&["anchor", &p(&anc), "--mock"]).status.success());
    let o = run(&["verify", "--anchor", "--json", &p(&anc)]);
    assert_eq!(o.status.code(), Some(0));
    let j = json(&o);
    assert_eq!(j["anchors_ok"], true);
    assert!(j["anchors"]
        .as_array()
        .unwrap()
        .iter()
        .all(|a| a["match"] == "yes"));
    assert_eq!(
        run(&["verify", "--anchor", &p(&anc)]).status.code(),
        Some(0)
    );

    // Tampered anchor fingerprint: exit 4, text and JSON.
    let fp = j["fingerprint"].as_str().unwrap().as_bytes().to_vec();
    let mut b = std::fs::read(&anc).unwrap();
    let i = b
        .windows(fp.len())
        .position(|w| w == fp.as_slice())
        .expect("fingerprint in file");
    b[i] = if b[i] == b'0' { b'1' } else { b'0' };
    std::fs::write(&bad, &b).unwrap();
    let o = run(&["verify", "--anchor", "--json", &p(&bad)]);
    assert_eq!(o.status.code(), Some(4));
    let j = json(&o);
    assert_eq!(j["ok"], false);
    assert_eq!(j["anchors_ok"], false);
    let o = run(&["verify", "--anchor", &p(&bad)]);
    assert_eq!(o.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&o.stdout).contains("Anchors: FAILED"));
}
