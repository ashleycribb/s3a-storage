//! `tour --html`: the page shows the archive's tiles, stays offline, and cannot be turned
//! into script by archive content.
use s3a_cli::tour_html::{build_data, escape_json_for_html, render_html};
use serde_json::{json, Value};

fn vector(name: &str) -> String {
    format!(
        "{}/../../conformance/vectors-1.5/files/{name}.s3a",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn embedded(html: &str) -> Value {
    let start = html
        .find(r#"<script type="application/json" id="s3a-data">"#)
        .unwrap();
    let rest = &html[start..];
    let body = &rest[rest.find('>').unwrap() + 1..rest.find("</script>").unwrap()];
    serde_json::from_str(body).unwrap()
}

#[test]
fn sealed_file_with_record_roots_lists_every_tile() {
    let data = build_data(&vector("record_roots_ok")).unwrap();
    let html = render_html(&data);
    let d = embedded(&html);
    let floors = d["floors"].as_array().unwrap();
    let types: Vec<u64> = floors
        .iter()
        .map(|f| f["type_id"].as_u64().unwrap())
        .collect();
    assert!(types.contains(&83), "record-root tile missing: {types:?}");
    assert_eq!(d["record_roots"]["status"], "ok");
    assert_eq!(d["lobby"]["seal_status"]["is_sealed"], true);
    assert_eq!(d["details"].as_array().unwrap().len(), floors.len());
    assert_eq!(d["header"]["slots"].as_array().unwrap().len(), 2);
}

#[test]
fn unsealed_multi_tile_has_decoded_records() {
    let d = embedded(&render_html(
        &build_data(&vector("valid_multi_tile")).unwrap(),
    ));
    assert_eq!(d["floors"].as_array().unwrap().len(), 3);
    assert_eq!(d["record_roots"]["status"], "absent");
    assert!(d["details"][0]["records"]
        .as_array()
        .is_some_and(|r| !r.is_empty()));
}

#[test]
fn page_is_self_contained() {
    let html = render_html(&build_data(&vector("valid_multi_tile")).unwrap());
    for bad in [
        "<link",
        " src=",
        " href=",
        "@import",
        "fetch(",
        "XMLHttpRequest",
        "url(",
    ] {
        assert!(
            !html.contains(bad),
            "page references external content: {bad}"
        );
    }
    assert!(html.contains("Content-Security-Policy"));
    assert!(
        !html.contains("innerHTML"),
        "page must render with textContent only"
    );
}

#[test]
fn archive_content_cannot_break_out_of_the_data_block() {
    let evil = "</script><img src=x onerror=alert(1)><!-- & \u{2028}";
    let data = json!({
        "file": evil,
        "file_bytes": 1,
        "lobby": {"manifest": evil, "seal_status": {}, "footer_version": evil},
        "floors": [{"index": 0, "type_id": 1, "type_name": evil, "version": 1, "record_count": 1,
                    "size_bytes": 1, "min_key": 0, "max_key": 0, "payload_hash_short": evil}],
        "details": [{"records": [{"field": evil}]}],
        "header": {"slots": [], "active_slot": 0},
        "record_roots": {"status": "absent", "checked": 0},
    });
    let html = render_html(&data);
    // Only the two script blocks of the template close, and no archive text appears raw.
    assert_eq!(html.matches("</script>").count(), 2);
    assert!(!html.contains("<img"));
    assert!(!html.contains("<!--"));
    assert!(html.contains("<title>S3A tour · &lt;/script&gt;&lt;img"));
    // The embedded JSON decodes back to exactly the original strings.
    assert_eq!(embedded(&html), data);
    assert_eq!(escape_json_for_html("<>&"), "\\u003c\\u003e\\u0026");
}
