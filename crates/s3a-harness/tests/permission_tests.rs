use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};

use s3a_harness::config::{AgentConfig, DecisionThresholds, RunLimits, SafeDefaults};
use s3a_harness::permissions::{
    ApprovalRequest, Decision, HumanApprover, PermissionMode, PermissionPolicy, ToolRule,
};
use s3a_harness::sandbox::Sandbox;
use s3a_harness::store::LocalStore;
use s3a_harness::tools::{Tool, ToolError, ToolExecutionContext, ToolRegistry};
use s3a_vault::rotation::RevocationRegistry;
use s3a_vault::{
    issue_agent_pass, AgentPass, KeyStore, MemoryKeyStore, PassConditions, RevokedKeyRecord,
};

const OWNER: [u8; 16] = [1; 16];
const AGENT: [u8; 16] = [2; 16];
const AGENT_ID: &str = "pass_agent";

struct Echo(&'static str);
impl Tool for Echo {
    fn name(&self) -> &str {
        self.0
    }
    fn description(&self) -> &str {
        "echo"
    }
    fn input_schema(&self) -> Value {
        json!({"type": "object"})
    }
    fn execute(&self, args: &Value, _c: &ToolExecutionContext) -> Result<Value, ToolError> {
        Ok(json!({"echo": args}))
    }
}

struct Human {
    answer: bool,
    asked: AtomicUsize,
}
impl HumanApprover for Human {
    fn approve(&self, _r: &ApprovalRequest) -> bool {
        self.asked.fetch_add(1, Ordering::SeqCst);
        self.answer
    }
}

const TOOLS: [&str; 5] = [
    "s3a_query",
    "s3a_write",
    "vault_export",
    "erasure_execute",
    "other_tool",
];

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn config() -> AgentConfig {
    AgentConfig {
        id: AGENT_ID.into(),
        name: "t".into(),
        system_prompt: String::new(),
        model: "mock".into(),
        escalation_model: None,
        limits: RunLimits {
            max_turns: 5,
            max_tool_calls: 10,
            timeout_secs: 10,
            token_budget: 1000,
        },
        thresholds: DecisionThresholds {
            tool_selection: 0.7,
            trimming: 0.65,
            stuck_detection: 0.8,
            escalation: 0.75,
        },
        safe_defaults: SafeDefaults {
            tool_selection: "ask_human".into(),
            trimming: "head_tail_trim".into(),
            stuck_detection: false,
            escalation: false,
        },
        allowed_tools: vec![],
        require_grant: false,
    }
}

fn ctx() -> (ToolExecutionContext, tempfile::TempDir) {
    let d = tempfile::tempdir().unwrap();
    let sandbox = Sandbox::new(d.path(), false).unwrap();
    let store = Arc::new(LocalStore::new(sandbox.default_db(), false));
    (
        ToolExecutionContext {
            agent_id: AGENT_ID.into(),
            session_uuid: [1, 2],
            sandbox,
            store,
            grant_record: None,
        },
        d,
    )
}

fn keys() -> (MemoryKeyStore, String) {
    let mut ks = MemoryKeyStore::new();
    ks.generate_keypair(&OWNER).unwrap();
    let agent = hex::encode(ks.generate_keypair(&AGENT).unwrap());
    (ks, agent)
}

fn pass(ks: &MemoryKeyStore, agent: &str, kinds: &[&str], nb: u64, na: u64) -> AgentPass {
    issue_agent_pass(
        ks,
        &OWNER,
        PassConditions {
            pass_id: "p".into(),
            agent_pubkey: agent.into(),
            owner_pubkey: String::new(),
            tool_kinds: kinds.iter().map(|s| s.to_string()).collect(),
            scopes: vec!["lab".into()],
            not_before: nb,
            not_after: na,
        },
    )
    .unwrap()
}

fn registry(policy: PermissionPolicy) -> ToolRegistry {
    let r = ToolRegistry::new();
    for t in TOOLS {
        r.register(Arc::new(Echo(t)));
    }
    r.set_permission_policy(policy);
    r
}

fn call(r: &ToolRegistry, tool: &str, args: Value) -> Result<(), String> {
    let (c, _d) = ctx();
    r.call_tool(&config(), tool, &args, &c)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn good_policy(mode: PermissionMode) -> (PermissionPolicy, MemoryKeyStore, String) {
    let (ks, agent) = keys();
    let p = pass(
        &ks,
        &agent,
        &["s3a_query", "s3a_write", "vault_export", "other_tool"],
        0,
        now() + 3600,
    );
    (
        PermissionPolicy::new(mode).with_pass(AGENT_ID, &agent, p),
        ks,
        agent,
    )
}

#[test]
fn mode_table() {
    use Decision::*;
    let expect = [
        (PermissionMode::Chat, [Deny, Deny, Deny, Deny, Deny]),
        (PermissionMode::Approve, [Ask, Ask, Ask, Deny, Ask]),
        (PermissionMode::Smart, [Allow, Ask, Ask, Deny, Ask]),
        (PermissionMode::Auto, [Allow, Allow, Ask, Deny, Allow]),
    ];
    for (mode, want) in expect {
        let p = PermissionPolicy::new(mode);
        let got: Vec<_> = TOOLS.iter().map(|t| p.decide(t)).collect();
        assert_eq!(got, want, "{mode:?}");
    }
}

#[test]
fn per_tool_rules_and_pinned_sensitive_tools() {
    let p = PermissionPolicy::new(PermissionMode::Auto)
        .with_rule("s3a_write", ToolRule::Never)
        .with_rule("s3a_query", ToolRule::Ask)
        .with_rule("vault_export", ToolRule::Always)
        .with_rule("erasure_execute", ToolRule::Always);
    assert_eq!(p.decide("s3a_write"), Decision::Deny);
    assert_eq!(p.decide("s3a_query"), Decision::Ask);
    // Always on a vault or judgment tool is read as Ask, in every mode.
    assert_eq!(p.decide("vault_export"), Decision::Ask);
    assert_eq!(p.decide("erasure_execute"), Decision::Ask);
    let a = PermissionPolicy::new(PermissionMode::Approve).with_rule("s3a_query", ToolRule::Always);
    assert_eq!(
        a.decide("s3a_query"),
        Decision::Ask,
        "approve mode asks even for always rules"
    );
    let s =
        PermissionPolicy::new(PermissionMode::Smart).with_rule("erasure_execute", ToolRule::Ask);
    assert_eq!(
        s.decide("erasure_execute"),
        Decision::Ask,
        "smart never auto-approves judgment"
    );
}

#[test]
fn ask_without_human_handler_is_denied() {
    let (p, _ks, _a) = good_policy(PermissionMode::Approve);
    let e = call(&registry(p), "s3a_query", json!({})).unwrap_err();
    assert!(e.contains("no human handler"), "{e}");
}

#[test]
fn ask_with_human_handler_runs_or_is_declined() {
    for answer in [true, false] {
        let h = Arc::new(Human {
            answer,
            asked: AtomicUsize::new(0),
        });
        let (p, _ks, _a) = good_policy(PermissionMode::Approve);
        let r = registry(p.with_approver(h.clone()));
        assert_eq!(call(&r, "s3a_write", json!({})).is_ok(), answer);
        assert_eq!(h.asked.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn auto_mode_runs_with_valid_pass_and_checks_scope() {
    let (p, _ks, _a) = good_policy(PermissionMode::Auto);
    let r = registry(p);
    call(&r, "s3a_write", json!({"scope": "lab"})).unwrap();
    assert!(call(&r, "s3a_write", json!({"scope": "other-lab"})).is_err());
}

#[test]
fn missing_expired_and_tampered_pass_refused() {
    let r = registry(PermissionPolicy::new(PermissionMode::Auto));
    assert!(call(&r, "s3a_query", json!({}))
        .unwrap_err()
        .contains("no owner-signed pass"));

    let (ks, agent) = keys();
    let expired = pass(&ks, &agent, &["s3a_query"], 10, 20);
    let r =
        registry(PermissionPolicy::new(PermissionMode::Auto).with_pass(AGENT_ID, &agent, expired));
    assert!(call(&r, "s3a_query", json!({}))
        .unwrap_err()
        .contains("pass check failed"));

    let mut tampered = pass(&ks, &agent, &["s3a_query"], 0, now() + 3600);
    tampered.conditions.tool_kinds.push("s3a_write".into());
    let r =
        registry(PermissionPolicy::new(PermissionMode::Auto).with_pass(AGENT_ID, &agent, tampered));
    assert!(call(&r, "s3a_write", json!({}))
        .unwrap_err()
        .contains("signature"));
    // A tool the (untampered) pass doesn't list is refused even in auto mode.
    let narrow = pass(&ks, &agent, &["s3a_query"], 0, now() + 3600);
    let r =
        registry(PermissionPolicy::new(PermissionMode::Auto).with_pass(AGENT_ID, &agent, narrow));
    assert!(call(&r, "other_tool", json!({})).is_err());
}

#[test]
fn revoked_owner_blocks_every_call() {
    let (p, ks, _a) = good_policy(PermissionMode::Auto);
    let mut rec = RevokedKeyRecord {
        revocation_id: [9; 16],
        revoked_public_key: ks.get_public_key(&OWNER).unwrap(),
        revoked_at: (now() + 10_000).to_le_bytes(),
        reason_code: 1,
        record_crc32: 0,
    };
    rec.record_crc32 = rec.compute_crc();
    let mut reg = RevocationRegistry::new();
    reg.register_revocation(&rec).unwrap();
    let r = registry(p.with_revocations(reg));
    for t in ["s3a_query", "s3a_write", "other_tool"] {
        assert!(
            call(&r, t, json!({})).unwrap_err().contains("revoked"),
            "{t}"
        );
    }
}

#[test]
fn judgment_refused_unless_a_human_approves() {
    // Default: judgment tools are never allowed, whatever the mode.
    let (p, _ks, _a) = good_policy(PermissionMode::Auto);
    let h = Arc::new(Human {
        answer: true,
        asked: AtomicUsize::new(0),
    });
    let r = registry(p.clone().with_approver(h.clone()));
    assert!(call(&r, "erasure_execute", json!({})).is_err());
    assert_eq!(h.asked.load(Ordering::SeqCst), 0);
    // A pass can never list a judgment kind.
    let (ks, agent) = keys();
    assert!(issue_agent_pass(
        &ks,
        &OWNER,
        PassConditions {
            pass_id: "j".into(),
            agent_pubkey: agent,
            owner_pubkey: String::new(),
            tool_kinds: vec!["erasure_execute".into()],
            scopes: vec![],
            not_before: 0,
            not_after: now() + 10,
        }
    )
    .is_err());
    // Rule set to ask: smart mode still routes to a human; no handler means deny.
    let r = registry(p.clone().with_rule("erasure_execute", ToolRule::Ask));
    assert!(call(&r, "erasure_execute", json!({}))
        .unwrap_err()
        .contains("no human handler"));
    let r = registry(
        p.with_rule("erasure_execute", ToolRule::Ask)
            .with_approver(h.clone()),
    );
    call(&r, "erasure_execute", json!({})).unwrap();
    assert_eq!(h.asked.load(Ordering::SeqCst), 1);
}

#[test]
fn legacy_registry_without_policy_unchanged() {
    let r = ToolRegistry::new();
    r.register(Arc::new(Echo("s3a_query")));
    r.register(Arc::new(Echo("erasure_execute")));
    let (c, _d) = ctx();
    r.call_tool(&config(), "s3a_query", &json!({}), &c).unwrap();
    assert!(r
        .call_tool(&config(), "erasure_execute", &json!({}), &c)
        .is_err());
}
