//! `s3a-cli seal <in> <out>` must never write its input.

use std::path::PathBuf;
use std::process::Command;

const LEGACY: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../s3a-engine/tests/fixtures/legacy_v1_0_telemetry.s3a"
);

fn tmp(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!("s3a_seal_ro_{}_{}.s3a", label, std::process::id()))
}
fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_s3a-cli"))
        .args(args)
        .output()
        .unwrap()
}

fn seal_leaves_input_untouched(input: &PathBuf, label: &str) {
    let out = tmp(&format!("{label}_out"));
    let before = std::fs::read(input).unwrap();
    for extra in [&[][..], &["--record-roots"][..], &["--parity"][..]] {
        let _ = std::fs::remove_file(&out);
        let mut a = vec!["seal", input.to_str().unwrap(), out.to_str().unwrap()];
        a.extend_from_slice(extra);
        let o = run(&a);
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        assert!(String::from_utf8_lossy(&o.stdout).contains("created successfully"));
        assert_eq!(
            std::fs::read(input).unwrap(),
            before,
            "{label} input changed by seal {extra:?}"
        );
        assert_eq!(
            run(&["verify", out.to_str().unwrap()]).status.code(),
            Some(0)
        );
        let scratch = format!("{}.seal-input.tmp.s3a", out.display());
        assert!(
            !std::path::Path::new(&scratch).exists(),
            "scratch file left behind"
        );
    }
    let _ = std::fs::remove_file(&out);
}

#[test]
fn seal_does_not_modify_legacy_v1_0_input() {
    let input = tmp("legacy_in");
    std::fs::copy(LEGACY, &input).unwrap();
    seal_leaves_input_untouched(&input, "legacy");
    let _ = std::fs::remove_file(&input);
}

#[test]
fn seal_does_not_modify_current_format_input() {
    let input = tmp("cur_in");
    let _ = std::fs::remove_file(&input);
    assert!(
        run(&["insert", input.to_str().unwrap(), "100", "1", "1", "5.0"])
            .status
            .success()
    );
    seal_leaves_input_untouched(&input, "current");
    let _ = std::fs::remove_file(&input);
}
