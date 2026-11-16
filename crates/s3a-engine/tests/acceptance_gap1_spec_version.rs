//! Acceptance (gap 1): the spec version comes only from the spec's `**Document Version:**`
//! line. A spec without that line (or with an empty value) is an explicit error, never the
//! string "unknown", and the error reaches callers that build the about entry.
//!
//! Written before the fix. Do not edit.

use std::io::ErrorKind;

const NO_VERSION: &str = "# S3A Hyper-Tile Specification\n\nSome text, no version line.\n";
const EMPTY_VERSION: &str = "# S3A\n\n**Document Version:**   \n";

fn docs_spec_version() -> String {
    let text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/spec/S3A-TILE-SPEC.md"),
    )
    .unwrap();
    text.lines()
        .find_map(|l| l.strip_prefix("**Document Version:**"))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .expect("docs/spec/S3A-TILE-SPEC.md must carry a Document Version line")
}

#[test]
fn missing_version_line_is_an_error_not_unknown() {
    let e = s3a_engine::sealed::spec_version_from(NO_VERSION)
        .expect_err("a spec without a Document Version line must be an error");
    assert_eq!(e.kind(), ErrorKind::InvalidData);
    assert!(e.to_string().contains("Document Version"), "{e}");
}

#[test]
fn empty_version_value_is_an_error() {
    let e = s3a_engine::sealed::spec_version_from(EMPTY_VERSION)
        .expect_err("an empty Document Version value must be an error");
    assert_eq!(e.kind(), ErrorKind::InvalidData);
}

#[test]
fn embedded_spec_version_matches_docs() {
    let v = s3a_engine::sealed::spec_version().expect("the embedded spec has a version");
    assert_eq!(v, docs_spec_version());
    assert_ne!(v, "unknown");
}

#[test]
fn about_entry_propagates_the_error() {
    let e = s3a_engine::sealed::about_from_spec(NO_VERSION)
        .expect_err("building an about entry from a version-less spec must fail");
    assert_eq!(e.kind(), ErrorKind::InvalidData);
    let a = s3a_engine::build_about().expect("about entry from the embedded spec");
    assert_eq!(a.spec_version, docs_spec_version());
}
