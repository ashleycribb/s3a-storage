//! Tests for general S3A MCP App Record Viewer, multi-pack views, and UI SDK.
//! Covers:
//! 1. MCP Protocol handling: `resources/list`, `resources/read`, and tool calling for viewer tools.
//! 2. Multi-pack view detection and extraction: Scholar Research, Technical Data (TDP), Aura Backups, Fallback Manifest.
//! 3. Non-developer accessibility guarantee: no CLI, tile IDs, or QL visible in human summaries; technical details strictly isolated behind "Advanced".
//! 4. Vercel UI web components adapter & keyboard accessibility tokens.

use std::path::Path;
use std::sync::Arc;
use tempfile::tempdir;

use s3a_core::{HyperTileHeader, S3ACoordinate, TileType};
use s3a_engine::TileWriter;
use s3a_harness::mcp::S3AMcpDispatcher;
use s3a_harness::viewer::{generate_viewer_html, ViewerRegistry};
use s3a_harness::{LocalStore, Sandbox, Store};
use s3a_pack_scholar::{AcademicPaperRecord, HumanLrsRecord, ScholarTileType};
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

    // 1. Academic paper record: "Attention Is All You Need" (paper_id: 170603762, year: 2017, citations: 120000, open_access: 1, venue: 42)
    let paper = AcademicPaperRecord::new(
        170603762, 1497264000, 100, 0.1, 0.2, 0.3, 120000, 2017, 42, 1, 0, 0x1706,
    );
    let paper_bytes = bytemuck::bytes_of(&paper);
    let mut hdr = HyperTileHeader::new(0, ScholarTileType::ACADEMIC_PAPERS);
    hdr.record_count = 1;
    hdr.payload_bytes = paper_bytes.len() as u32;
    hdr.data_crc32 = crc32c::crc32c(paper_bytes);
    writer
        .write_raw_hyper_tile(&hdr, paper_bytes)
        .expect("write paper tile");

    // 2. Human LRS record (actor: 0xBEEF)
    let human = HumanLrsRecord::new(
        [1, 2],
        1700000000,
        0xBEEF,
        1,
        0.95,
        S3ACoordinate::default(),
        0xCAFE,
    );
    let human_bytes = bytemuck::bytes_of(&human);
    let mut hdr_human = HyperTileHeader::new(1, ScholarTileType::HUMAN_LRS);
    hdr_human.record_count = 1;
    hdr_human.payload_bytes = human_bytes.len() as u32;
    hdr_human.data_crc32 = crc32c::crc32c(human_bytes);
    writer
        .write_raw_hyper_tile(&hdr_human, human_bytes)
        .expect("write human tile");

    // 3. Raw JSON metadata tile (title and abstract)
    let payload = b"{\"title\":\"Epistemic Verification Architecture\",\"abstract\":\"Autonomous research provenance.\"}";
    let mut hdr_json = HyperTileHeader::new(2, ScholarTileType::SRS_WORKSPACE);
    hdr_json.record_count = 1;
    hdr_json.payload_bytes = payload.len() as u32;
    hdr_json.data_crc32 = crc32c::crc32c(payload);
    writer
        .write_raw_hyper_tile(&hdr_json, payload)
        .expect("write scholar json tile");
}

fn create_tdp_technical_data_archive(path: &Path) {
    let mut writer = TileWriter::create(path).expect("create tdp archive");
    writer.enable_addon(0xFFFF_FFFF).expect("enable addons");
    drop(writer);

    let pdf_bytes = s3a_pack_tdp::pdf::create_test_pdf("Gearbox TDP Technical Drawing");
    let _ = s3a_pack_tdp::store::store_pdf(path, "drawings/gearbox.pdf", &pdf_bytes)
        .expect("store pdf in tdp");

    let step_bytes = include_bytes!("../../s3a-pack-tdp/tests/fixtures/sample.step");
    let _ = s3a_pack_tdp::store::store_step(path, "models/sample.step", step_bytes)
        .expect("store step in tdp");
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
        Some("Researcher Approved")
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
    let has_paper = tree.iter().any(|node| {
        node.get("id")
            .and_then(|id| id.as_str())
            .is_some_and(|id| id == "paper_170603762")
    });
    assert!(has_paper, "tree must contain paper_170603762 node");

    // 3. s3a_viewer_get_properties for paper
    let (prop_data, is_err) = call_tool(
        &dispatcher,
        "s3a_viewer_get_properties",
        json!({ "file_path": path_str, "node_id": "paper_170603762" }),
    );
    assert!(!is_err);
    let props = prop_data
        .get("properties")
        .and_then(|p| p.as_array())
        .expect("properties array");
    let year_prop = props
        .iter()
        .find(|item| item[0].as_str() == Some("Publication Year"));
    assert_eq!(year_prop.and_then(|p| p[1].as_str()), Some("2017"));
    let cit_prop = props
        .iter()
        .find(|item| item[0].as_str() == Some("Citation Count"));
    assert_eq!(cit_prop.and_then(|p| p[1].as_str()), Some("120000"));
    let oa_prop = props
        .iter()
        .find(|item| item[0].as_str() == Some("Open Access"));
    assert_eq!(oa_prop.and_then(|p| p[1].as_str()), Some("Yes"));

    // 4. s3a_viewer_get_preview for paper
    let (prev_data, is_err) = call_tool(
        &dispatcher,
        "s3a_viewer_get_preview",
        json!({ "file_path": path_str, "node_id": "paper_170603762" }),
    );
    assert!(!is_err);
    let content = prev_data.get("content").and_then(|c| c.as_str()).unwrap();
    assert!(content.contains("170603762") || content.contains("0x000000000a2b2572"));
    assert!(content.contains("2017"));

    // 5. s3a_viewer_get_history and s3a_viewer_get_approval_status
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
    let has_asm = tree
        .iter()
        .any(|node| node.get("id").and_then(|v| v.as_str()) == Some("asm_ASM-1000"));
    assert!(has_asm, "tree must contain asm_ASM-1000 root");

    // Check part children
    let asm_node = tree
        .iter()
        .find(|n| n.get("id").and_then(|v| v.as_str()) == Some("asm_ASM-1000"))
        .unwrap();
    let children = asm_node
        .get("children")
        .and_then(|c| c.as_array())
        .expect("children array");
    let has_shaft = children
        .iter()
        .any(|n| n.get("id").and_then(|v| v.as_str()) == Some("part_SHAFT-001"));
    assert!(has_shaft, "assembly must contain part_SHAFT-001");
    let has_bom = children
        .iter()
        .any(|n| n.get("id").and_then(|v| v.as_str()) == Some("bom_ASM-1000"));
    assert!(has_bom, "assembly must contain bom_ASM-1000");

    // 3. s3a_viewer_get_preview for asm_ASM-1000 (dynamic SVG)
    let (prev_data, is_err) = call_tool(
        &dispatcher,
        "s3a_viewer_get_preview",
        json!({ "file_path": path_str, "node_id": "asm_ASM-1000" }),
    );
    assert!(!is_err);
    assert_eq!(
        prev_data.get("mime_type").and_then(|v| v.as_str()),
        Some("image/svg+xml")
    );
    let svg = prev_data.get("content").and_then(|c| c.as_str()).unwrap();
    assert!(svg.contains("<svg"));
    assert!(svg.contains("ASM-1000"));
    assert!(svg.contains("Ashley C."));

    // Preview for part_SHAFT-001
    let (part_prev, is_err) = call_tool(
        &dispatcher,
        "s3a_viewer_get_preview",
        json!({ "file_path": path_str, "node_id": "part_SHAFT-001" }),
    );
    assert!(!is_err);
    let part_svg = part_prev.get("content").and_then(|c| c.as_str()).unwrap();
    assert!(part_svg.contains("SHAFT-001"));
    assert!(part_svg.contains("Input Drive Shaft"));

    // Preview for bom_ASM-1000 (CSV)
    let (bom_prev, is_err) = call_tool(
        &dispatcher,
        "s3a_viewer_get_preview",
        json!({ "file_path": path_str, "node_id": "bom_ASM-1000" }),
    );
    assert!(!is_err);
    assert_eq!(
        bom_prev.get("mime_type").and_then(|v| v.as_str()),
        Some("text/plain")
    );
    let bom_csv = bom_prev.get("content").and_then(|c| c.as_str()).unwrap();
    assert!(bom_csv.contains("SHAFT-001,Input Drive Shaft"));

    // 4. s3a_viewer_get_properties
    let (prop_data, is_err) = call_tool(
        &dispatcher,
        "s3a_viewer_get_properties",
        json!({ "file_path": path_str, "node_id": "part_SHAFT-001" }),
    );
    assert!(!is_err);
    assert_eq!(
        prop_data.get("label").and_then(|v| v.as_str()),
        Some("Component SHAFT-001")
    );
    let props = prop_data
        .get("properties")
        .and_then(|p| p.as_array())
        .expect("properties array");
    let pn_prop = props
        .iter()
        .find(|item| item[0].as_str() == Some("Part Number"));
    assert_eq!(pn_prop.and_then(|p| p[1].as_str()), Some("SHAFT-001"));
    let name_prop = props
        .iter()
        .find(|item| item[0].as_str() == Some("Part Name"));
    assert_eq!(
        name_prop.and_then(|p| p[1].as_str()),
        Some("Input Drive Shaft")
    );
    let adv_props = prop_data
        .get("advanced_properties")
        .and_then(|p| p.as_array())
        .expect("advanced_properties array");
    let author_prop = adv_props
        .iter()
        .find(|item| item[0].as_str() == Some("Author"));
    assert_eq!(author_prop.and_then(|p| p[1].as_str()), Some("Ashley C."));

    // Check assembly root properties for Author
    let (asm_prop_data, is_err) = call_tool(
        &dispatcher,
        "s3a_viewer_get_properties",
        json!({ "file_path": path_str, "node_id": "asm_ASM-1000" }),
    );
    assert!(!is_err);
    let asm_props = asm_prop_data
        .get("properties")
        .and_then(|p| p.as_array())
        .expect("asm properties array");
    let asm_author = asm_props
        .iter()
        .find(|item| item[0].as_str() == Some("Author"));
    assert_eq!(asm_author.and_then(|p| p[1].as_str()), Some("Ashley C."));
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

#[test]
fn test_anti_fabrication_no_hardcoded_demo_strings() {
    let viewer_source = include_str!("../src/viewer.rs");

    let forbidden_strings = [
        "Multidimensional Spatial Hyper-Tile Architecture",
        "Dr. A. Cribb",
        "part_piston",
        "ASM-704",
        "fig_1",
        "tab_1",
        "Forged Piston",
        "704-02-A",
        "704-01-A",
        "Cylinder Block",
        "aura-node-0x0042",
        "Aura Edge Sentinel",
        "Linux 6.6.21-aura-rt",
        "Dr. Jane Doe",
        "Chief Mechanical Engineer",
        "Aura Fleet Security Daemon",
    ];

    for s in forbidden_strings {
        assert!(
            !viewer_source.contains(s),
            "viewer.rs must NOT contain hardcoded demo string: '{}'",
            s
        );
    }
}

#[test]
fn test_anti_fabrication_distinct_vaults_produce_different_properties() {
    let tmp = tempdir().unwrap();
    let reg = ViewerRegistry::new();

    // Vault 1: Attention Is All You Need (Paper ID: 170603762, Year: 2017, Actor: 0xBEEF)
    let vault1_path = tmp.path().join("vault1_transformer.s3a");
    {
        let mut writer = TileWriter::create(&vault1_path).expect("create vault1");
        writer.enable_addon(0xFFFF_FFFF).expect("enable addons");

        let paper1 = AcademicPaperRecord::new(
            170603762, 1497264000, 100, 0.1, 0.2, 0.3, 120000, 2017, 42, 1, 0, 0x1706,
        );
        let paper_bytes = bytemuck::bytes_of(&paper1);
        let mut hdr = HyperTileHeader::new(0, ScholarTileType::ACADEMIC_PAPERS);
        hdr.record_count = 1;
        hdr.payload_bytes = paper_bytes.len() as u32;
        hdr.data_crc32 = crc32c::crc32c(paper_bytes);
        writer.write_raw_hyper_tile(&hdr, paper_bytes).unwrap();

        let human1 = HumanLrsRecord::new(
            [1, 1],
            1700000000,
            0xBEEF,
            1,
            0.99,
            S3ACoordinate::default(),
            0x1706,
        );
        let human_bytes = bytemuck::bytes_of(&human1);
        let mut hdr_h = HyperTileHeader::new(1, ScholarTileType::HUMAN_LRS);
        hdr_h.record_count = 1;
        hdr_h.payload_bytes = human_bytes.len() as u32;
        hdr_h.data_crc32 = crc32c::crc32c(human_bytes);
        writer.write_raw_hyper_tile(&hdr_h, human_bytes).unwrap();
    }

    // Vault 2: Deep Residual Learning (Paper ID: 151203385, Year: 2015, Actor: 0xCAFE)
    let vault2_path = tmp.path().join("vault2_resnet.s3a");
    {
        let mut writer = TileWriter::create(&vault2_path).expect("create vault2");
        writer.enable_addon(0xFFFF_FFFF).expect("enable addons");

        let paper2 = AcademicPaperRecord::new(
            151203385, 1450000000, 200, 0.4, 0.5, 0.6, 180000, 2015, 88, 1, 0, 0x1512,
        );
        let paper_bytes = bytemuck::bytes_of(&paper2);
        let mut hdr = HyperTileHeader::new(0, ScholarTileType::ACADEMIC_PAPERS);
        hdr.record_count = 1;
        hdr.payload_bytes = paper_bytes.len() as u32;
        hdr.data_crc32 = crc32c::crc32c(paper_bytes);
        writer.write_raw_hyper_tile(&hdr, paper_bytes).unwrap();

        let human2 = HumanLrsRecord::new(
            [2, 2],
            1710000000,
            0xCAFE,
            2,
            0.88,
            S3ACoordinate::default(),
            0x1512,
        );
        let human_bytes = bytemuck::bytes_of(&human2);
        let mut hdr_h = HyperTileHeader::new(1, ScholarTileType::HUMAN_LRS);
        hdr_h.record_count = 1;
        hdr_h.payload_bytes = human_bytes.len() as u32;
        hdr_h.data_crc32 = crc32c::crc32c(human_bytes);
        writer.write_raw_hyper_tile(&hdr_h, human_bytes).unwrap();
    }

    let state1 = reg.load_archive(&vault1_path).expect("load vault1");
    let state2 = reg.load_archive(&vault2_path).expect("load vault2");

    // 1. Anti-fabrication: Summaries must reflect distinct approval actors
    assert_ne!(state1.summary.human_approval, state2.summary.human_approval);
    assert!(state1
        .summary
        .human_approval
        .to_lowercase()
        .contains("beef"));
    assert!(state2
        .summary
        .human_approval
        .to_lowercase()
        .contains("cafe"));

    // 2. Anti-fabrication: Trees must contain distinct node IDs originating from data
    assert_eq!(state1.tree[0].id, "paper_170603762");
    assert_eq!(state2.tree[0].id, "paper_151203385");
    assert_ne!(state1.tree[0].id, state2.tree[0].id);

    // 3. Anti-fabrication: Properties must return exact distinct values from the file
    let props1 = reg
        .get_properties(&vault1_path, "paper_170603762")
        .expect("props1");
    let props2 = reg
        .get_properties(&vault2_path, "paper_151203385")
        .expect("props2");

    let year1 = props1
        .properties
        .iter()
        .find(|p| p.0 == "Publication Year")
        .unwrap();
    let year2 = props2
        .properties
        .iter()
        .find(|p| p.0 == "Publication Year")
        .unwrap();
    assert_eq!(year1.1, "2017");
    assert_eq!(year2.1, "2015");
    assert_ne!(year1.1, year2.1);

    let cit1 = props1
        .properties
        .iter()
        .find(|p| p.0 == "Citation Count")
        .unwrap();
    let cit2 = props2
        .properties
        .iter()
        .find(|p| p.0 == "Citation Count")
        .unwrap();
    assert_eq!(cit1.1, "120000");
    assert_eq!(cit2.1, "180000");
    assert_ne!(cit1.1, cit2.1);
}
