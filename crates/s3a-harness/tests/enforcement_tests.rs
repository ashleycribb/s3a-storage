//! Runtime enforcement of the agent-permissions policy at the harness boundary.
use ed25519_dalek::SigningKey;
use s3a_harness::config::AgentConfig;
use s3a_harness::enforcement::{
    classify, ActionClass, AuditLog, EnforcementGate, GateDecision, SignOff, SIGNOFF_ARG,
};
use s3a_harness::mcp::S3AMcpDispatcher;
use s3a_harness::permissions::{PermissionMode, PermissionPolicy, ToolRule};
use s3a_harness::sandbox::Sandbox;
use s3a_harness::store::{LocalStore, Store};
use s3a_harness::tools::{ToolExecutionContext, ToolRegistry};
use serde_json::{json, Value};
use std::sync::Arc;

struct Tmp(std::path::PathBuf);
impl Tmp {
    fn new(p: &str) -> Self {
        let d = std::env::temp_dir().join(format!("{p}_{}_{}", std::process::id(), uuid_like()));
        std::fs::create_dir_all(&d).unwrap();
        Tmp(d)
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn uuid_like() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
fn human() -> SigningKey {
    SigningKey::from_bytes(&[9u8; 32])
}
fn open_policy() -> PermissionPolicy {
    let mut p = PermissionPolicy::new(PermissionMode::Auto);
    p.require_pass = false;
    p
}
fn cfg() -> AgentConfig {
    AgentConfig {
        id: "agent_a".into(),
        name: "A".into(),
        system_prompt: String::new(),
        model: "m".into(),
        escalation_model: None,
        limits: Default::default(),
        thresholds: Default::default(),
        safe_defaults: Default::default(),
        allowed_tools: vec![],
        require_grant: false,
    }
}
fn ctx(t: &Tmp) -> ToolExecutionContext {
    let sandbox = Sandbox::new(&t.0, false).unwrap();
    let store = Arc::new(LocalStore::new(sandbox.default_db(), false));
    ToolExecutionContext {
        agent_id: "agent_a".into(),
        session_uuid: [1, 2],
        sandbox,
        store,
        grant_record: None,
    }
}
fn write_args() -> Value {
    json!({ "target": "note", "content": "hello" })
}

#[test]
fn allowed_action_passes_and_is_audited() {
    let t = Tmp::new("enf_allow");
    let r = ToolRegistry::new_default();
    r.set_permission_policy(open_policy());
    assert!(r
        .call_tool(&cfg(), "s3a_write", &write_args(), &ctx(&t))
        .is_ok());
    let e = r.enforcement_gate().audit_log().entries();
    assert_eq!(e.len(), 1);
    assert_eq!(e[0].decision, GateDecision::Allow);
    assert_eq!(
        (
            e[0].agent_id.as_str(),
            e[0].action.as_str(),
            e[0].target.as_str()
        ),
        ("agent_a", "s3a_write", "note")
    );
    assert!(e[0].timestamp > 0 && !e[0].reason.is_empty());
}

#[test]
fn writes_denied_by_default_without_policy_reads_allowed() {
    let t = Tmp::new("enf_default");
    let r = ToolRegistry::new_default();
    let err = r
        .call_tool(&cfg(), "s3a_write", &write_args(), &ctx(&t))
        .unwrap_err();
    assert!(format!("{err:?}").contains("deny by default"));
    // existence check is fine
    assert!(r
        .call_tool(&cfg(), "s3a_query", &json!({ "query": "x" }), &ctx(&t))
        .is_ok());
    let e = r.enforcement_gate().audit_log().entries();
    assert_eq!(
        e.iter().map(|e| e.decision).collect::<Vec<_>>(),
        vec![GateDecision::Deny, GateDecision::Allow]
    );
}

#[test]
fn disallowed_by_rule_is_denied_and_audited() {
    let t = Tmp::new("enf_never");
    let r = ToolRegistry::new_default();
    r.set_permission_policy(open_policy().with_rule("s3a_write", ToolRule::Never));
    assert!(r
        .call_tool(&cfg(), "s3a_write", &write_args(), &ctx(&t))
        .is_err());
    let e = r.enforcement_gate().audit_log().entries();
    assert_eq!(e[0].decision, GateDecision::Deny);
    assert!(e[0].reason.contains("not allowed"));
}

#[test]
fn human_only_denied_without_signoff_allowed_with_it() {
    let g = EnforcementGate::new();
    g.set_policy(open_policy());
    g.register_human("ashley", human().verifying_key());
    let tool = "claim_support_judgment";
    assert_eq!(classify(tool, "claim-7"), ActionClass::HumanOnly);
    let base = json!({ "target": "claim-7" });
    assert!(g
        .check("agent_a", tool, &base, now())
        .unwrap_err()
        .contains("no human sign-off"));

    let so = SignOff::sign(
        &human(),
        "ashley",
        "agent_a",
        tool,
        "claim-7",
        now() + 60,
        "n1",
    );
    let mut ok = base.clone();
    ok[SIGNOFF_ARG] = so.to_value();
    let entry = g.check("agent_a", tool, &ok, now()).unwrap();
    assert_eq!(entry.signed_off_by.as_deref(), Some("ashley"));
    // single use
    assert!(g
        .check("agent_a", tool, &ok, now())
        .unwrap_err()
        .contains("already used"));
    // bound to target and agent
    let mut other = json!({ "target": "claim-8" });
    other[SIGNOFF_ARG] = SignOff::sign(
        &human(),
        "ashley",
        "agent_a",
        tool,
        "claim-7",
        now() + 60,
        "n2",
    )
    .to_value();
    assert!(g
        .check("agent_a", tool, &other, now())
        .unwrap_err()
        .contains("does not match"));
    // expired
    let mut exp = base.clone();
    exp[SIGNOFF_ARG] = SignOff::sign(
        &human(),
        "ashley",
        "agent_a",
        tool,
        "claim-7",
        now() - 1,
        "n3",
    )
    .to_value();
    assert!(g
        .check("agent_a", tool, &exp, now())
        .unwrap_err()
        .contains("expired"));
    // an agent signing for itself with an unregistered key fails
    let mut forged = base.clone();
    forged[SIGNOFF_ARG] = SignOff::sign(
        &SigningKey::from_bytes(&[1u8; 32]),
        "agent_a",
        "agent_a",
        tool,
        "claim-7",
        now() + 60,
        "n4",
    )
    .to_value();
    assert!(g
        .check("agent_a", tool, &forged, now())
        .unwrap_err()
        .contains("unregistered human"));
    // tampered signature
    let mut bad = base.clone();
    let mut s = SignOff::sign(
        &human(),
        "ashley",
        "agent_a",
        tool,
        "claim-7",
        now() + 60,
        "n5",
    );
    s.expires_at += 1;
    bad[SIGNOFF_ARG] = s.to_value();
    assert!(g
        .check("agent_a", tool, &bad, now())
        .unwrap_err()
        .contains("does not verify"));

    let e = g.audit_log().entries();
    assert_eq!(e.len(), 7, "every decision audited");
    assert_eq!(
        e.iter()
            .filter(|e| e.decision == GateDecision::Allow)
            .count(),
        1
    );
    assert!(AuditLog::verify_chain(&e).is_ok());
}

#[test]
fn human_only_fails_closed_even_in_auto_mode_through_registry() {
    let t = Tmp::new("enf_judg");
    let r = ToolRegistry::new_default();
    r.set_permission_policy(open_policy());
    let err = r
        .call_tool(&cfg(), "erasure_purge", &json!({}), &ctx(&t))
        .unwrap_err();
    assert!(format!("{err:?}").contains("judgment action"));
}

#[test]
fn core_write_by_agent_rejected_even_with_policy_and_signoff() {
    let g = EnforcementGate::new();
    g.set_policy(open_policy());
    g.register_human("ashley", human().verifying_key());
    for (tool, target) in [
        ("s3a_core_write", "tile-1"),
        ("s3a_write", "crates/s3a-core/src/lib.rs"),
        ("core_compact", ""),
    ] {
        let mut a = json!({ "target": target });
        a[SIGNOFF_ARG] = SignOff::sign(
            &human(),
            "ashley",
            "agent_a",
            tool,
            target,
            now() + 60,
            tool,
        )
        .to_value();
        let err = g.check("agent_a", tool, &a, now()).unwrap_err();
        assert!(err.contains("S3A core"), "{tool}: {err}");
    }
    let e = g.audit_log().entries();
    assert!(e
        .iter()
        .all(|e| e.decision == GateDecision::Deny && e.class == ActionClass::Core));
}

#[test]
fn file_audit_log_is_append_only_and_tamper_evident() {
    let t = Tmp::new("enf_audit");
    let path = t.0.join("audit.jsonl");
    let g = EnforcementGate::new();
    g.set_audit_log(Arc::new(AuditLog::file(&path).unwrap()));
    let _ = g.check("agent_a", "s3a_write", &write_args(), now());
    let _ = g.check("agent_a", "s3a_query", &json!({}), now());
    // a second handle continues the same chain
    let g2 = EnforcementGate::new();
    g2.set_audit_log(Arc::new(AuditLog::file(&path).unwrap()));
    let _ = g2.check("agent_b", "s3a_write", &write_args(), now());
    let e = AuditLog::file(&path).unwrap().entries();
    assert_eq!(e.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![1, 2, 3]);
    assert!(AuditLog::verify_chain(&e).is_ok());
    let mut forged = e.clone();
    forged[0].decision = GateDecision::Allow;
    assert_eq!(AuditLog::verify_chain(&forged), Err(1));
}

#[test]
fn audit_write_failure_denies_the_call() {
    let t = Tmp::new("enf_audit_fail");
    // The audit path is a directory, so it cannot be opened for append: the gate fails closed.
    let dir = t.0.join("not_a_file");
    std::fs::create_dir_all(&dir).unwrap();
    let g = EnforcementGate::new();
    g.set_audit_log(Arc::new(AuditLog::file(&dir).unwrap()));
    let err = g
        .check("agent_a", "s3a_query", &json!({}), now())
        .unwrap_err();
    assert!(err.contains("audit record could not be written"), "{err}");
}

fn dispatcher(t: &Tmp) -> S3AMcpDispatcher {
    let sandbox = Sandbox::new(&t.0, false).unwrap();
    let store: Arc<dyn Store> = Arc::new(LocalStore::new(sandbox.default_db(), false));
    S3AMcpDispatcher::new(sandbox, store, [1, 2], [3, 4], 42)
}
fn mcp_call(d: &S3AMcpDispatcher, name: &str, args: Value) -> Value {
    let req = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } });
    serde_json::from_str(&d.dispatch(&req.to_string()).unwrap()).unwrap()
}

#[test]
fn mcp_writes_and_human_decisions_are_gated() {
    let t = Tmp::new("enf_mcp");
    let d = dispatcher(&t);
    let v = mcp_call(
        &d,
        "record_ai_step",
        json!({ "step_type": 1, "confidence": 0.9 }),
    );
    assert_eq!(v["result"]["isError"], true);
    assert!(v["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("deny by default"));
    d.enforcement_gate().set_policy(open_policy());
    let v = mcp_call(
        &d,
        "record_human_decision",
        json!({ "actor_id": 1, "verb_id": 3, "decision_score": 1.0 }),
    );
    assert!(v["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("no human sign-off"));
    d.enforcement_gate()
        .register_human("ashley", human().verifying_key());
    let so = SignOff::sign(
        &human(),
        "ashley",
        "mcp-agent-42",
        "record_human_decision",
        "",
        now() + 60,
        "m1",
    );
    let v = mcp_call(
        &d,
        "record_human_decision",
        json!({ "actor_id": 1, "verb_id": 3, "decision_score": 1.0, "human_signoff": so.to_value() }),
    );
    assert!(!v["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("permission denied"));
    let e = d.enforcement_gate().audit_log().entries();
    assert_eq!(e.len(), 3);
    assert_eq!(e[2].signed_off_by.as_deref(), Some("ashley"));
}
