//! CI gate: our reader must pass the committed language-neutral suite, and
//! regenerating the suite must give the same cases and expectations.

use std::path::PathBuf;

use serde_json::Value;

fn suite() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../conformance/vectors-1.0")
}

fn manifest(dir: &std::path::Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap()
}

#[test]
fn reference_reader_passes_committed_suite() {
    let (pass, fails) = s3a_spec_vectors::conformance::run_suite(&suite()).unwrap();
    assert!(fails.is_empty(), "{fails:#?}");
    let m = manifest(&suite());
    assert_eq!(m["version"], s3a_spec_vectors::conformance::SUITE_VERSION);
    assert_eq!(pass as u64, m["case_count"].as_u64().unwrap());
    assert!(pass >= 50);
}

#[test]
fn regenerated_suite_matches_committed_expectations() {
    let tmp = std::env::temp_dir().join(format!("s3a_conf_regen_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let st = std::process::Command::new(env!("CARGO_BIN_EXE_s3a-conformance"))
        .args(["gen", tmp.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(st.success());
    let key = |m: &Value| -> Vec<(String, String)> {
        m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                (
                    c["id"].as_str().unwrap().to_string(),
                    c["expect"].to_string(),
                )
            })
            .collect()
    };
    assert_eq!(
        key(&manifest(&tmp)),
        key(&manifest(&suite())),
        "re-run `s3a-conformance gen conformance/vectors-1.0`"
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn corrupted_vector_file_is_detected() {
    let tmp = std::env::temp_dir().join(format!("s3a_conf_corrupt_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("files")).unwrap();
    let m = manifest(&suite());
    std::fs::copy(suite().join("manifest.json"), tmp.join("manifest.json")).unwrap();
    for c in m["cases"].as_array().unwrap() {
        if let Some(f) = c["file"].as_str() {
            std::fs::copy(suite().join(f), tmp.join(f)).unwrap();
        }
    }
    let f = tmp.join("files/valid_multi_tile.s3a");
    let mut b = std::fs::read(&f).unwrap();
    b[600] ^= 1;
    std::fs::write(&f, b).unwrap();
    let (_, fails) = s3a_spec_vectors::conformance::run_suite(&tmp).unwrap();
    assert_eq!(fails.len(), 1);
    assert!(fails[0].contains("valid_multi_tile") && fails[0].contains("sha256"));
    let _ = std::fs::remove_dir_all(&tmp);
}
