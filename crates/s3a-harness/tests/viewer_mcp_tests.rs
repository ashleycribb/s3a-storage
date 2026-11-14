//! Tests for general S3A MCP App Record Viewer, multi-pack views, and UI SDK.
//! Covers:
//! 1. MCP Protocol handling: `resources/list`, `resources/read`, and tool calling for viewer tools.
//! 2. Multi-pack view detection and extraction: Scholar Research, Technical Data (TDP), Aura Backups, Fallback Manifest.
//! 3. Non-developer accessibility guarantee: no CLI, tile IDs, or QL visible in human summaries; technical details strictly isolated behind "Advanced".
//! 4. Vercel UI web components adapter & keyboard accessibility tokens.

use std::path::Path;
use std::sync::Arc;
use tempfile::tempdir;

use s3a_core::{HyperTileHeader, TileType};
use s3a_engine::TileWriter;
use s3a_harness::mcp::S3AMcpDispatcher;
use s3a_harness::viewer::{generate_viewer_html, ViewerRegistry};
use s3a_harness::{LocalStore, Sandbox, Store};
use serde_json::{json, Value};

fn create_test_dispatcher(db_path: &Path) -> S3AMcpDispatcher {
    let sandbox = Sandbox::new(db_path.parent().unwrap(), false).expect("create sandbox");
    let store: Arc<dyn Store> = Arc::new(LocalStore::new(db_path, false));
    let p_uuid = [0x1111222233334444, 0x5555666677778888];
    let s_uuid = [0xAAAABBBBCCCCDDDD, 0xEEEEFFFF00001111];
    let d = S3AMcpDispatcher::new(sandbox, store, p_uuid, s_uuid, 42);

    let mut policy = s3a_harness::permissions::PermissionPolicy::new(
        s3a_harness::permissions::PermissionMode::Auto,
    );
    policy.require_pass = false;
    d.enforcement_gate().set_policy(policy);
    d
}

fn mcp_call(d: &S3AMcpDispatcher, method: &str, params: Value) -> Value {
    let req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params
    });
    let raw = d
        .dispatch(&serde_json::to_string(&req).unwrap())
        .expect("expected response");
    serde_json::from_str(&raw).expect("valid JSON response")
}

fn call_tool(d: &S3AMcpDispatcher, tool_name: &str, args: Value) -> (Value, bool) {
    let resp = mcp_call(
        d,
        "tools/call",
        json!({
            "name": tool_name,
            "arguments": args
        }),
    );
    assert_eq!(resp.get("jsonrpc").and_then(|v| v.as_str()), Some("2.0"));
    let res = resp.get("result").expect("expected tool call result");
    let content = res
        .get("content")
        .and_then(|v| v.as_array())
        .expect("content array");
    let text = content[0]
        .get("text")
        .and_then(|v| v.as_str())
        .expect("text content");
    let is_err = res
        .get("isError")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if is_err {
        eprintln!("TOOL {} FAILED: {}", tool_name, text);
    }
    let parsed: Value = serde_json::from_str(text).unwrap_or(Value::String(text.to_string()));
    (parsed, is_err)
}

fn create_scholar_research_archive(path: &Path) {
    let mut writer = TileWriter::create(path).expect("create scholar archive");
    writer.enable_addon(0xFFFF_FFFF).expect("enable addons");
    // Write a tile with tile_type in 10..=15 (Scholar tile types)
    let payload = b"{\"title\":\"S3A Epistemic Verification\",\"abstract\":\"Autonomous research provenance.\"}";
    let mut hdr = HyperTileHeader::new(0, 10); // STATEMENT_BODY
    hdr.record_count = 1;
    hdr.payload_bytes = payload.len() as u32;
    hdr.data_crc32 = crc32c::crc32c(payload);
    writer
        .write_raw_hyper_tile(&hdr, payload)
        .expect("write scholar tile");
}

fn create_tdp_technical_data_archive(path: &Path) {
    let mut writer = TileWriter::create(path).expect("create tdp archive");
    writer.enable_addon(0xFFFF_FFFF).expect("enable addons");
    drop(writer);

    let pdf_bytes = s3a_pack_tdp::pdf::create_test_pdf("S3A Gearbox TDP Technical Drawing");
    let _ = s3a_pack_tdp::store::store_pdf(path, "drawings/gearbox.pdf", &pdf_bytes)
        .expect("store pdf in tdp");
}

fn create_aura_backup_archive(path: &Path) {
    let mut writer = TileWriter::create(path).expect("create aura archive");
    writer.enable_addon(0xFFFF_FFFF).expect("enable addons");
    // Write Telemetry tile (tile_type 0) and Wearable tile (tile_type 3)
    let payload =
        b"{\"device_id\":\"Aura-Alpha-9\",\"snapshot\":\"2026-11-14T10:00:00Z\",\"os\":\"1.4.2\"}";
    let mut hdr = HyperTileHeader::new(0, TileType::TELEMETRY);
    hdr.record_count = 1;
    hdr.payload_bytes = payload.len() as u32;
    hdr.data_crc32 = crc32c::crc32c(payload);
    writer
        .write_raw_hyper_tile(&hdr, payload)
        .expect("write telemetry tile");

    let mut hdr_wear = HyperTileHeader::new(1, 3); // Wearable pack tile type
    hdr_wear.record_count = 1;
    hdr_wear.payload_bytes = payload.len() as u32;
    hdr_wear.data_crc32 = crc32c::crc32c(payload);
    writer
        .write_raw_hyper_tile(&hdr_wear, payload)
        .expect("write wearable tile");
}

fn create_manifest_fallback_archive(path: &Path) {
    let mut writer = TileWriter::create(path).expect("create manifest archive");
    writer.enable_addon(0xFFFF_FFFF).expect("enable addons");
    let payload = b"unrecognized_custom_binary_format";
    let mut hdr = HyperTileHeader::new(0, 9999);
    hdr.record_count = 1;
    hdr.payload_bytes = payload.len() as u32;
    hdr.data_crc32 = crc32c::crc32c(payload);
    writer
        .write_raw_hyper_tile(&hdr, payload)
        .expect("write custom tile");
}

#[test]
fn test_mcp_resources_list_and_read_html() {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("default.s3a");
    create_scholar_research_archive(&db_path);

    let dispatcher = create_test_dispatcher(&db_path);

    // 1. Check resources/list advertises ui://viewer and ui://record-viewer
    let list_res = mcp_call(&dispatcher, "resources/list", json!({}));
    let res_obj = list_res.get("result").expect("expected result");
    let resources = res_obj
        .get("resources")
        .and_then(|v| v.as_array())
        .expect("resources array");
    assert!(resources.len() >= 2);

    let uris: Vec<&str> = resources
        .iter()
        .filter_map(|r| r.get("uri").and_then(|u| u.as_str()))
        .collect();
    assert!(
        uris.contains(&"ui://viewer"),
        "missing ui://viewer resource"
    );
    assert!(
        uris.contains(&"ui://record-viewer"),
        "missing ui://record-viewer resource"
    );

    let mime_type = resources[0]
        .get("mimeType")
        .and_then(|v| v.as_str())
        .unwrap();
    assert_eq!(mime_type, "text/html;profile=mcp-app");

    // 2. Read resources/read for ui://viewer
    let read_res = mcp_call(
        &dispatcher,
        "resources/read",
        json!({ "uri": "ui://viewer" }),
    );
    let contents = read_res
        .get("result")
        .and_then(|r| r.get("contents"))
        .and_then(|c| c.as_array())
        .expect("contents array");
    assert_eq!(contents.len(), 1);

    let html = contents[0]
        .get("text")
        .and_then(|t| t.as_str())
        .expect("html string");
    assert!(
        html.contains("<!DOCTYPE html>"),
        "must be complete HTML document"
    );
    assert!(
        html.contains("<ui-tree"),
        "must include Vercel UI tree components"
    );
    assert!(
        html.contains("<ui-badge"),
        "must include Vercel UI badge components"
    );
    assert!(
        html.contains("<ui-tab"),
        "must include Vercel UI tab components"
    );
    assert!(
        html.contains("Vercel UI SDK"),
        "must include Vercel UI SDK attribution"
    );
    assert!(
        html.contains("ArrowUp"),
        "must include keyboard navigation handler"
    );
    assert!(
        html.contains("ArrowDown"),
        "must include keyboard navigation handler"
    );
    assert!(
        html.contains("Advanced"),
        "must include Advanced details toggle"
    );
}

#[test]
fn test_scholar_research_view_pack() {
    let tmp = tempdir().unwrap();
    let archive_path = tmp.path().join("scholar_test.s3a");
    create_scholar_research_archive(&archive_path);

    let dispatcher = create_test_dispatcher(&archive_path);
    let path_str = archive_path.to_string_lossy().to_string();

    // 1. s3a_viewer_open
    let (open_data, is_err) = call_tool(
        &dispatcher,
        "s3a_viewer_open",
        json!({ "file_path": path_str }),
    );
    assert!(!is_err);
    assert_eq!(
        open_data.get("view_pack_id").and_then(|v| v.as_str()),
        Some("research")
    );
    let summary = open_data.get("summary").expect("summary");
    assert_eq!(
        summary.get("view_pack_name").and_then(|v| v.as_str()),
        Some("Scholar Research (s3a-pack-scholar)")
    );
    assert_eq!(
        summary.get("status_badge").and_then(|v| v.as_str()),
        Some("Verified Research")
    );

    // 2. s3a_viewer_get_tree
    let (tree_data, is_err) = call_tool(
        &dispatcher,
        "s3a_viewer_get_tree",
        json!({ "file_path": path_str }),
    );
    assert!(!is_err);
    let tree = tree_data.as_array().expect("tree array");
    assert!(!tree.is_empty(), "tree must not be empty");
    let has_manuscript = tree.iter().any(|node| {
        node.get("label")
            .and_then(|l| l.as_str())
            .is_some_and(|l| l.contains("Manuscript"))
    });
    assert!(has_manuscript, "tree must contain Manuscript node");

    // 3. s3a_viewer_get_preview for a figure node
    let (prev_data, is_err) = call_tool(
        &dispatcher,
        "s3a_viewer_get_preview",
        json!({ "file_path": path_str, "node_id": "fig_1" }),
    );
    assert!(!is_err);
    assert_eq!(
        prev_data.get("mime_type").and_then(|v| v.as_str()),
        Some("image/svg+xml")
    );
    assert!(prev_data
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap()
        .contains("<svg"));

    // 4. s3a_viewer_get_history and s3a_viewer_get_approval_status
    let (hist_data, is_err) = call_tool(
        &dispatcher,
        "s3a_viewer_get_history",
        json!({ "file_path": path_str }),
    );
    assert!(!is_err);
    assert_eq!(
        hist_data.get("branch").and_then(|v| v.as_str()),
        Some("main")
    );

    let (appr_data, is_err) = call_tool(
        &dispatcher,
        "s3a_viewer_get_approval_status",
        json!({ "file_path": path_str }),
    );
    assert!(!is_err);
    assert_eq!(
        appr_data.get("is_approved").and_then(|v| v.as_bool()),
        Some(true)
    );
}

#[test]
fn test_technical_data_view_pack() {
    let tmp = tempdir().unwrap();
    let archive_path = tmp.path().join("tdp_test.s3a");
    create_tdp_technical_data_archive(&archive_path);

    let dispatcher = create_test_dispatcher(&archive_path);
    let path_str = archive_path.to_string_lossy().to_string();

    // 1. s3a_viewer_open
    let (open_data, is_err) = call_tool(
        &dispatcher,
        "s3a_viewer_open",
        json!({ "file_path": path_str }),
    );
    assert!(!is_err);
    assert_eq!(
        open_data.get("view_pack_id").and_then(|v| v.as_str()),
        Some("technical_data")
    );
    let summary = open_data.get("summary").expect("summary");
    assert_eq!(
        summary.get("view_pack_name").and_then(|v| v.as_str()),
        Some("Technical Data Package (TDP)")
    );
    assert_eq!(
        summary.get("status_badge").and_then(|v| v.as_str()),
        Some("Engineering Released")
    );

    // 2. s3a_viewer_get_tree
    let (tree_data, is_err) = call_tool(
        &dispatcher,
        "s3a_viewer_get_tree",
        json!({ "file_path": path_str }),
    );
    assert!(!is_err);
    let tree = tree_data.as_array().expect("tree array");
    assert!(!tree.is_empty(), "tree must contain assembly root");

    // 3. s3a_viewer_get_preview
    let (prev_data, is_err) = call_tool(
        &dispatcher,
        "s3a_viewer_get_preview",
        json!({ "file_path": path_str, "node_id": "root" }),
    );
    assert!(!is_err);
    assert_eq!(
        prev_data.get("mime_type").and_then(|v| v.as_str()),
        Some("image/svg+xml")
    );
    assert!(prev_data
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap()
        .contains("<svg"));

    // 4. s3a_viewer_get_properties
    let (prop_data, is_err) = call_tool(
        &dispatcher,
        "s3a_viewer_get_properties",
        json!({ "file_path": path_str, "node_id": "root" }),
    );
    assert!(!is_err);
    assert_eq!(
        prop_data.get("label").and_then(|v| v.as_str()),
        Some("TDP Assembly Root")
    );
}

#[test]
fn test_aura_backup_view_pack() {
    let tmp = tempdir().unwrap();
    let archive_path = tmp.path().join("aura_test.s3a");
    create_aura_backup_archive(&archive_path);

    let dispatcher = create_test_dispatcher(&archive_path);
    let path_str = archive_path.to_string_lossy().to_string();

    // 1. s3a_viewer_open
    let (open_data, is_err) = call_tool(
        &dispatcher,
        "s3a_viewer_open",
        json!({ "file_path": path_str }),
    );
    assert!(!is_err);
    assert_eq!(
        open_data.get("view_pack_id").and_then(|v| v.as_str()),
        Some("aura_backup")
    );
    let summary = open_data.get("summary").expect("summary");
    assert_eq!(
        summary.get("view_pack_name").and_then(|v| v.as_str()),
        Some("Aura Backup & Device Snapshot")
    );

    // 2. s3a_viewer_get_tree
    let (tree_data, is_err) = call_tool(
        &dispatcher,
        "s3a_viewer_get_tree",
        json!({ "file_path": path_str }),
    );
    assert!(!is_err);
    let tree = tree_data.as_array().expect("tree array");
    let has_device_root = tree.iter().any(|node| {
        node.get("id").and_then(|v| v.as_str()) == Some("device_root")
            || node
                .get("label")
                .and_then(|l| l.as_str())
                .is_some_and(|l| l.contains("Aura Edge") || l.contains("Device"))
    });
    assert!(has_device_root, "tree must contain Device Snapshot root");
}

#[test]
fn test_manifest_fallback_view_pack() {
    let tmp = tempdir().unwrap();
    let archive_path = tmp.path().join("fallback_test.s3a");
    create_manifest_fallback_archive(&archive_path);

    let dispatcher = create_test_dispatcher(&archive_path);
    let path_str = archive_path.to_string_lossy().to_string();

    // 1. s3a_viewer_open
    let (open_data, is_err) = call_tool(
        &dispatcher,
        "s3a_viewer_open",
        json!({ "file_path": path_str }),
    );
    assert!(!is_err);
    assert_eq!(
        open_data.get("view_pack_id").and_then(|v| v.as_str()),
        Some("manifest")
    );
    let summary = open_data.get("summary").expect("summary");
    assert_eq!(
        summary.get("view_pack_name").and_then(|v| v.as_str()),
        Some("Generic S3A Record (Manifest)")
    );

    // 2. s3a_viewer_get_tree
    let (tree_data, is_err) = call_tool(
        &dispatcher,
        "s3a_viewer_get_tree",
        json!({ "file_path": path_str }),
    );
    assert!(!is_err);
    let tree = tree_data.as_array().expect("tree array");
    assert_eq!(tree.len(), 1);
}

#[test]
fn test_non_developer_usability_no_cli_or_tile_ids() {
    let tmp = tempdir().unwrap();
    let scholar_path = tmp.path().join("scholar_clean.s3a");
    create_scholar_research_archive(&scholar_path);

    let tdp_path = tmp.path().join("tdp_clean.s3a");
    create_tdp_technical_data_archive(&tdp_path);

    let aura_path = tmp.path().join("aura_clean.s3a");
    create_aura_backup_archive(&aura_path);

    let reg = ViewerRegistry::new();

    for path in [&scholar_path, &tdp_path, &aura_path] {
        let state = reg.load_archive(path).expect("load archive");

        // Verify summary fields contain NO developer jargon (CLI, Tile IDs, QL)
        let human_strings = [
            &state.summary.title,
            &state.summary.subtitle,
            &state.summary.description,
            &state.summary.status_badge,
            &state.summary.human_approval,
            &state.summary.view_pack_name,
        ];

        for text in human_strings {
            let lower = text.to_lowercase();
            assert!(
                !lower.contains("s3a-cli"),
                "human summary must not contain CLI command in '{}'",
                text
            );
            assert!(
                !lower.contains("s3a "),
                "human summary must not contain CLI invocation in '{}'",
                text
            );
            assert!(
                !lower.contains("tile_type"),
                "human summary must not contain raw tile_type in '{}'",
                text
            );
            assert!(
                !lower.contains("tile 0"),
                "human summary must not contain raw tile ID in '{}'",
                text
            );
            assert!(
                !lower.contains("tile 1"),
                "human summary must not contain raw tile ID in '{}'",
                text
            );
            assert!(
                !lower.contains("sift "),
                "human summary must not contain raw S3A-QL in '{}'",
                text
            );
            assert!(
                !lower.contains("fetch "),
                "human summary must not contain raw S3A-QL in '{}'",
                text
            );
        }

        // Verify developer details ARE present under advanced
        assert!(state.summary.advanced.tile_count > 0);
        assert!(!state.summary.advanced.crc_status.is_empty());
        assert!(!state.summary.advanced.merkle_root.is_empty());

        // Verify HTML shell isolates advanced details behind toggle
        let html = generate_viewer_html(&state);
        assert!(
            html.contains("id=\"advanced-toggle\""),
            "HTML must provide Advanced toggle"
        );
        assert!(
            html.contains("id=\"advanced-content\" style=\"display:none;\""),
            "Advanced content must be hidden by default"
        );
        assert!(
            html.contains("Developer Details"),
            "Advanced section label must be present"
        );
    }
}

#[test]
fn test_ui_keyboard_accessibility_and_vercel_ui_tokens() {
    let tmp = tempdir().unwrap();
    let path = tmp.path().join("a11y_test.s3a");
    create_scholar_research_archive(&path);

    let reg = ViewerRegistry::new();
    let state = reg.load_archive(&path).expect("load archive");
    let html = generate_viewer_html(&state);

    // Verify Vercel UI design system tokens
    assert!(html.contains("--geist-background"));
    assert!(html.contains("--geist-foreground"));
    assert!(html.contains("--accents-1"));
    assert!(html.contains("--geist-cyan"));

    // Verify accessibility & keyboard attributes
    assert!(html.contains("role=\"tree\""));
    assert!(html.contains("role=\"treeitem\""));
    assert!(html.contains("tabindex=\"0\""));
    assert!(html.contains("aria-selected="));
    assert!(html.contains("aria-expanded="));

    // Verify keyboard event handling
    assert!(html.contains("e.key === 'ArrowDown'"));
    assert!(html.contains("e.key === 'ArrowUp'"));
    assert!(html.contains("e.key === 'Enter'"));
    assert!(html.contains("e.key === ' '"));
}
