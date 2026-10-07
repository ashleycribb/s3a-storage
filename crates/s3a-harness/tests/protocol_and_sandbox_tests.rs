use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

use s3a_harness::{LocalStore, S3AMcpDispatcher, Sandbox, SandboxError, Store};

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("{}_{}_{}_{}", prefix, std::process::id(), ts, id));
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn create_test_dispatcher(sandbox: Sandbox, read_only: bool) -> S3AMcpDispatcher {
    let db_path = sandbox.default_db().to_path_buf();
    let store: Arc<dyn Store> = Arc::new(LocalStore::new(&db_path, read_only));
    let p_uuid = [0x1111222233334444, 0x5555666677778888];
    let s_uuid = [0xAAAABBBBCCCCDDDD, 0xEEEEFFFF00001111];
    S3AMcpDispatcher::new(sandbox, store, p_uuid, s_uuid, 42)
}

#[test]
fn test_protocol_malformed_json_returns_32700() {
    let guard = TempDirGuard::new("s3a_proto_test");
    let sandbox = Sandbox::new(&guard.path, false).unwrap();
    let dispatcher = create_test_dispatcher(sandbox, false);

    let malformed = "{not a valid json string";
    let resp = dispatcher
        .dispatch(malformed)
        .expect("Must return response for malformed JSON");
    let parsed: serde_json::Value = serde_json::from_str(&resp).unwrap();

    assert_eq!(parsed["jsonrpc"], "2.0");
    assert!(parsed["id"].is_null());
    assert_eq!(parsed["error"]["code"], -32700);
    assert!(parsed["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Parse error"));
}

#[test]
fn test_protocol_invalid_request_returns_32600() {
    let guard = TempDirGuard::new("s3a_proto_test");
    let sandbox = Sandbox::new(&guard.path, false).unwrap();
    let dispatcher = create_test_dispatcher(sandbox, false);

    // 1. Missing jsonrpc field
    let no_jsonrpc = r#"{"id": 1, "method": "ping"}"#;
    let resp1 = dispatcher.dispatch(no_jsonrpc).unwrap();
    let v1: serde_json::Value = serde_json::from_str(&resp1).unwrap();
    assert_eq!(v1["error"]["code"], -32600);

    // 2. Wrong jsonrpc version
    let bad_version = r#"{"jsonrpc": "1.0", "id": 1, "method": "ping"}"#;
    let resp2 = dispatcher.dispatch(bad_version).unwrap();
    let v2: serde_json::Value = serde_json::from_str(&resp2).unwrap();
    assert_eq!(v2["error"]["code"], -32600);

    // 3. Not an object
    let not_obj = r#"["jsonrpc", "2.0"]"#;
    let resp3 = dispatcher.dispatch(not_obj).unwrap();
    let v3: serde_json::Value = serde_json::from_str(&resp3).unwrap();
    assert_eq!(v3["error"]["code"], -32600);
}

#[test]
fn test_protocol_notifications_never_replied() {
    let guard = TempDirGuard::new("s3a_proto_test");
    let sandbox = Sandbox::new(&guard.path, false).unwrap();
    let dispatcher = create_test_dispatcher(sandbox, false);

    // 1. Standard MCP notification: notifications/initialized (no id)
    let notif1 = r#"{"jsonrpc": "2.0", "method": "notifications/initialized"}"#;
    assert!(
        dispatcher.dispatch(notif1).is_none(),
        "Must not reply to notification"
    );

    // 2. Notification with null id
    let notif2 = r#"{"jsonrpc": "2.0", "id": null, "method": "custom_notify"}"#;
    assert!(
        dispatcher.dispatch(notif2).is_none(),
        "Must not reply to notification with null id"
    );

    // 3. Ping without id is also a notification
    let notif3 = r#"{"jsonrpc": "2.0", "method": "ping"}"#;
    assert!(
        dispatcher.dispatch(notif3).is_none(),
        "Must not reply to ping without id"
    );
}

#[test]
fn test_protocol_unknown_method_returns_32601() {
    let guard = TempDirGuard::new("s3a_proto_test");
    let sandbox = Sandbox::new(&guard.path, false).unwrap();
    let dispatcher = create_test_dispatcher(sandbox, false);

    let req = r#"{"jsonrpc": "2.0", "id": 100, "method": "unknown_future_method"}"#;
    let resp = dispatcher.dispatch(req).unwrap();
    let v: serde_json::Value = serde_json::from_str(&resp).unwrap();

    assert_eq!(v["id"], 100);
    assert_eq!(v["error"]["code"], -32601);
    assert!(v["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Method not found"));
}

#[test]
fn test_protocol_bad_params_returns_32602() {
    let guard = TempDirGuard::new("s3a_proto_test");
    let sandbox = Sandbox::new(&guard.path, false).unwrap();
    let dispatcher = create_test_dispatcher(sandbox, false);

    // 1. tools/call without name
    let req1 = r#"{"jsonrpc": "2.0", "id": 101, "method": "tools/call", "params": {}}"#;
    let resp1 = dispatcher.dispatch(req1).unwrap();
    let v1: serde_json::Value = serde_json::from_str(&resp1).unwrap();
    assert_eq!(v1["error"]["code"], -32602);

    // 2. tools/call with wrong type for parameters
    let req2 = r#"{"jsonrpc": "2.0", "id": 102, "method": "tools/call", "params": {"name": "s3a_search_library", "arguments": {"min_year": "invalid_string_not_int"}}}"#;
    let resp2 = dispatcher.dispatch(req2).unwrap();
    let v2: serde_json::Value = serde_json::from_str(&resp2).unwrap();
    assert_eq!(v2["error"]["code"], -32602);
}

#[test]
fn test_protocol_escaping_and_special_characters() {
    let guard = TempDirGuard::new("s3a_proto_test");
    let sandbox = Sandbox::new(&guard.path, false).unwrap();
    let dispatcher = create_test_dispatcher(sandbox, false);

    // Request with quotes, backslashes, newlines, and unicode in arguments
    let req = json!({
        "jsonrpc": "2.0",
        "id": 200,
        "method": "tools/call",
        "params": {
            "name": "request_human_gate",
            "arguments": {
                "action_type": "Hypothesis Synthesis: \"E = mc²\"\nSection \\1.2",
                "reason": "Special symbols test: 🔬 ∀x ∈ ℝ, \\path\\to\\somewhere\tnewline\n"
            }
        }
    })
    .to_string();

    let resp = dispatcher.dispatch(&req).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&resp).unwrap();

    assert_eq!(parsed["id"], 200);
    assert!(parsed.get("result").is_some());
    let text = parsed["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Hypothesis Synthesis"));
    assert!(text.contains("E = mc²"));
    assert!(text.contains("pending"));
}

#[test]
fn test_protocol_version_negotiation() {
    let guard = TempDirGuard::new("s3a_proto_test");
    let sandbox = Sandbox::new(&guard.path, false).unwrap();
    let dispatcher = create_test_dispatcher(sandbox, false);

    let req = r#"{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2024-11-05"}}"#;
    let resp = dispatcher.dispatch(req).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&resp).unwrap();

    assert_eq!(parsed["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(
        parsed["result"]["serverInfo"]["name"],
        "s3a-academic-harness"
    );
}

#[test]
fn test_sandbox_path_traversal_rejection() {
    let guard = TempDirGuard::new("s3a_sandbox_test");
    let sandbox = Sandbox::new(&guard.path, false).unwrap();

    // 1. Parent traversal with ".."
    assert!(matches!(
        sandbox.resolve_export_snapshot_path(Some("../evil.snapshot.s3a")),
        Err(SandboxError::PathTraversalDetected(_))
    ));

    // 2. Windows-style backslash traversal
    assert!(matches!(
        sandbox.resolve_export_snapshot_path(Some("..\\evil.snapshot.s3a")),
        Err(SandboxError::PathTraversalDetected(_))
    ));

    // 3. Absolute path
    assert!(matches!(
        sandbox.resolve_export_snapshot_path(Some("C:\\Windows\\evil.snapshot.s3a")),
        Err(SandboxError::PathTraversalDetected(_))
    ));

    // 4. Subdirectory separator attempt inside filename
    assert!(matches!(
        sandbox.resolve_export_snapshot_path(Some("sub/evil.snapshot.s3a")),
        Err(SandboxError::PathTraversalDetected(_))
    ));

    // 5. Valid filename confined to exports
    let resolved = sandbox
        .resolve_export_snapshot_path(Some("safe_project.snapshot.s3a"))
        .unwrap();
    assert!(resolved.starts_with(sandbox.exports_dir()));
    assert_eq!(resolved.file_name().unwrap(), "safe_project.snapshot.s3a");
}

#[test]
fn test_sandbox_read_only_mode_enforcement() {
    let guard = TempDirGuard::new("s3a_ro_test");
    let sandbox = Sandbox::new(&guard.path, true).unwrap(); // read_only = true
    let dispatcher = create_test_dispatcher(sandbox, true);

    // 1. Export SRS must be rejected in read-only mode
    let export_req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "s3a_export_srs",
            "arguments": { "filename": "test.snapshot.s3a" }
        }
    })
    .to_string();

    let export_resp = dispatcher.dispatch(&export_req).unwrap();
    let parsed1: serde_json::Value = serde_json::from_str(&export_resp).unwrap();
    assert_eq!(parsed1["result"]["isError"], true);
    assert!(parsed1["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("read-only mode"));

    // 2. record_human_decision must be rejected in read-only mode
    let decision_req = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "record_human_decision",
            "arguments": {
                "actor_id": 100,
                "verb_id": 3,
                "decision_score": 1.0
            }
        }
    })
    .to_string();

    let decision_resp = dispatcher.dispatch(&decision_req).unwrap();
    let parsed2: serde_json::Value = serde_json::from_str(&decision_resp).unwrap();
    assert_eq!(parsed2["result"]["isError"], true);
    assert!(parsed2["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("read-only mode"));

    // 3. record_ai_step must be rejected in read-only mode
    let ai_req = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "record_ai_step",
            "arguments": {
                "step_type": 1,
                "confidence": 0.95
            }
        }
    })
    .to_string();

    let ai_resp = dispatcher.dispatch(&ai_req).unwrap();
    let parsed3: serde_json::Value = serde_json::from_str(&ai_resp).unwrap();
    assert_eq!(parsed3["result"]["isError"], true);
    assert!(parsed3["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("read-only mode"));
}

#[test]
fn test_agent_gates_tools_and_rejection_logging() {
    let guard = TempDirGuard::new("s3a_gates_test");
    let sandbox = Sandbox::new(&guard.path, false).unwrap();
    let dispatcher = create_test_dispatcher(sandbox, false);

    // 1. Preflight check: low confidence (< 0.70) fails
    let pf_fail = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "preflight_check",
            "arguments": {
                "candidate_confidence": 0.50,
                "unbound_entities": [],
                "has_vetted_analogy": true
            }
        }
    })
    .to_string();
    let pf_resp1 = dispatcher.dispatch(&pf_fail).unwrap();
    let v_pf1: serde_json::Value = serde_json::from_str(&pf_resp1).unwrap();
    assert_eq!(v_pf1["result"]["isError"], true);
    assert!(v_pf1["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("LowConfidence"));

    // 2. Preflight check: high confidence passes
    let pf_pass = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "preflight_check",
            "arguments": {
                "candidate_confidence": 0.88,
                "unbound_entities": [],
                "has_vetted_analogy": true
            }
        }
    })
    .to_string();
    let pf_resp2 = dispatcher.dispatch(&pf_pass).unwrap();
    let v_pf2: serde_json::Value = serde_json::from_str(&pf_resp2).unwrap();
    assert_eq!(v_pf2["result"]["isError"], false);
    assert!(v_pf2["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("passed"));

    // 3. Request human gate -> returns status pending
    let gate_req = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "request_human_gate",
            "arguments": {
                "action_type": "Speculative Claim Insertion",
                "reason": "Empirical contradiction found in literature"
            }
        }
    })
    .to_string();
    let gate_resp = dispatcher.dispatch(&gate_req).unwrap();
    let v_gate: serde_json::Value = serde_json::from_str(&gate_resp).unwrap();
    assert_eq!(v_gate["result"]["isError"], false);
    let gate_text = v_gate["result"]["content"][0]["text"].as_str().unwrap();
    let gate_obj: serde_json::Value = serde_json::from_str(gate_text).unwrap();
    let gate_id = gate_obj["gate_id"].as_str().unwrap().to_string();
    assert_eq!(gate_obj["status"], "pending");

    // 4. Low confidence AI step WITHOUT human decision approval MUST BE REJECTED AND LOGGED
    let unapproved_ai = json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "tools/call",
        "params": {
            "name": "record_ai_step",
            "arguments": {
                "step_type": 1,
                "confidence": 0.45,
                "gate_id": gate_id
            }
        }
    })
    .to_string();
    let unapp_resp = dispatcher.dispatch(&unapproved_ai).unwrap();
    let v_unapp: serde_json::Value = serde_json::from_str(&unapp_resp).unwrap();
    assert_eq!(v_unapp["result"]["isError"], true);
    assert!(v_unapp["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("rejected"));

    // 5. Human supervisor approves the gate
    let human_dec = json!({
        "jsonrpc": "2.0",
        "id": 5,
        "method": "tools/call",
        "params": {
            "name": "record_human_decision",
            "arguments": {
                "actor_id": 0x05E8_1111,
                "verb_id": 3, // approved
                "decision_score": 1.0,
                "gate_id": gate_id
            }
        }
    })
    .to_string();
    let dec_resp = dispatcher.dispatch(&human_dec).unwrap();
    let v_dec: serde_json::Value = serde_json::from_str(&dec_resp).unwrap();
    assert_eq!(v_dec["result"]["isError"], false);

    // 6. Now AI step with approved gate succeeds
    let approved_ai = json!({
        "jsonrpc": "2.0",
        "id": 6,
        "method": "tools/call",
        "params": {
            "name": "record_ai_step",
            "arguments": {
                "step_type": 1,
                "confidence": 0.45,
                "gate_id": gate_id
            }
        }
    })
    .to_string();
    let app_resp = dispatcher.dispatch(&approved_ai).unwrap();
    let v_app: serde_json::Value = serde_json::from_str(&app_resp).unwrap();
    assert_eq!(v_app["result"]["isError"], false);
    assert!(v_app["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("recorded"));
}

#[test]
fn test_learnmcp_tools_with_local_store() {
    let guard = TempDirGuard::new("s3a_learnmcp_test");
    let sandbox = Sandbox::new(&guard.path, false).unwrap();
    let db_path = sandbox.default_db().to_path_buf();
    let local_store = LocalStore::new(&db_path, false);

    // 1. List academic verbs
    let verbs = local_store.list_academic_verbs().unwrap();
    assert!(!verbs.is_empty());
    assert!(verbs.iter().any(|v| v["label"] == "reviewed"));
    assert!(verbs.iter().any(|v| v["label"] == "ta-approved-draft"));

    // 2. Record xAPI statement compliant with research_traceability profile
    let session_uuid_str = "11111111-2222-3333-4444-555555555555";
    let stmt = json!({
        "id": "99999999-8888-7777-6666-555555555555",
        "actor": {
            "mbox": "mailto:dr.scholar@univ.edu",
            "name": "Dr. Scholar"
        },
        "verb": {
            "id": "https://ashecribb.dev/xapi/verbs/reviewed"
        },
        "object": {
            "id": "https://doi.org/10.1000/182",
            "objectType": "Activity"
        },
        "context": {
            "registration": session_uuid_str
        }
    });

    let stmt_id = local_store.record_xapi_statement(&stmt).unwrap();
    assert_eq!(stmt_id, "99999999-8888-7777-6666-555555555555");

    // 3. Get session context
    let ctx = local_store.get_session_context(session_uuid_str).unwrap();
    assert_eq!(ctx["human_decisions_count"], 1);
    assert_eq!(ctx["total_events"], 1);

    // 4. Get researcher timeline
    let timeline = local_store
        .get_researcher_timeline(None, Some(session_uuid_str), None)
        .unwrap();
    assert_eq!(timeline.len(), 1);
    assert_eq!(timeline[0]["actor_type"], "human");
}
