//! The algorithm registry has four copies: Rust, the Python and TypeScript readers, and
//! docs/spec/registry.md §6.
//! They must agree.
use s3a_core::algorithms::{AlgKind, ALGORITHMS};

fn root(p: &str) -> String {
    std::fs::read_to_string(format!("{}/../../{p}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[test]
fn python_reader_and_docs_match_rust_registry() {
    let py = root("readers/python/s3a_reader/reader.py");
    let md = root("docs/spec/registry.md");
    let js = root("readers/js/src/reader.ts");
    let py_rows = py.matches("): \"").count();
    assert_eq!(py_rows, ALGORITHMS.len(), "python registry size");
    for e in ALGORITHMS {
        let (kind, key) = match e.kind {
            AlgKind::SealHash => ("seal_hash", "seal_hash_alg"),
            AlgKind::Signature => ("signature", "sig_alg"),
        };
        let row = format!("(\"{kind}\", \"{}\"): \"{}\"", e.id, e.status.as_str());
        assert!(py.contains(&row), "python reader lacks {row}");
        let js_row = format!("\"{kind}/{}\": \"{}\"", e.id, e.status.as_str());
        assert!(js.contains(&js_row), "TypeScript reader lacks {js_row}");
        let md_row = format!("| `{}` | `{key}` | {} |", e.id, e.status.as_str());
        assert!(md.contains(&md_row), "registry.md lacks {md_row}");
    }
}

#[test]
fn seal_about_declares_sha256_and_checks_pass() {
    let a = s3a_engine::build_about();
    assert_eq!(a.seal_hash_alg, "sha256");
    assert!(s3a_engine::check_about_algorithms(&a).is_ok());
    let mut bad = a.clone();
    bad.sig_alg = "ml-dsa-87".into();
    assert_eq!(
        s3a_engine::check_about_algorithms(&bad).unwrap_err().0,
        "UnsupportedAlgorithm"
    );
    bad.sig_alg = "dsa".into();
    assert_eq!(
        s3a_engine::check_about_algorithms(&bad).unwrap_err().0,
        "UnknownAlgorithm"
    );
}

#[test]
fn about_without_algorithm_keys_keeps_its_bytes() {
    // Pre-v1.5 entries must encode exactly as before: no new keys appear when unset.
    let a = s3a_core::SchemaAbout {
        spec_version: "v".into(),
        spec_url: "u".into(),
        spec_sha256: "s".into(),
        writer: "w".into(),
        ..Default::default()
    };
    let b = s3a_core::encode_schema_about(&a);
    assert_eq!(
        &b[16..],
        b"spec_version=v\nspec_url=u\nspec_sha256=s\nwriter=w\n"
    );
}
