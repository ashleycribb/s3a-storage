//! CI gate: our reader must pass the committed language-neutral suite, and
//! regenerating the suite must give the same cases and expectations.

use std::path::PathBuf;

use serde_json::Value;

fn suite_v(v: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance")
        .join(v)
}
fn suite() -> PathBuf {
    suite_v("vectors-1.0")
}
use s3a_spec_vectors::conformance::SUITE_VERSIONS;

fn manifest(dir: &std::path::Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap()
}

#[test]
fn reference_reader_passes_committed_suite() {
    for v in SUITE_VERSIONS {
        let (pass, fails) = s3a_spec_vectors::conformance::run_suite(&suite_v(v)).unwrap();
        assert!(fails.is_empty(), "{v}: {fails:#?}");
        let m = manifest(&suite_v(v));
        assert_eq!(m["version"], v);
        assert_eq!(pass as u64, m["case_count"].as_u64().unwrap());
        assert!(pass >= 50);
    }
    assert_eq!(
        *SUITE_VERSIONS.last().unwrap(),
        s3a_spec_vectors::conformance::SUITE_VERSION
    );
}

#[test]
fn newer_suite_is_superset_of_older() {
    let ids = |v: &str| -> Vec<(String, String)> {
        manifest(&suite_v(v))["cases"]
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
    let old = ids("vectors-1.0");
    let new = ids("vectors-1.1");
    for c in &old {
        assert!(
            new.contains(c),
            "vectors-1.1 changed or dropped 1.0 case {}",
            c.0
        );
    }
}

#[test]
fn regenerated_suite_matches_committed_expectations() {
    for v in SUITE_VERSIONS {
        let tmp = std::env::temp_dir()
            .join(format!("s3a_conf_regen_{}", std::process::id()))
            .join(v);
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
            key(&manifest(&suite_v(v))),
            "re-run `s3a-conformance gen conformance/{v}`"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }
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
