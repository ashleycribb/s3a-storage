use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use s3a_harness::{
    run_doctor, AgentConfig, AgentRuntime, DecisionThresholds, LocalStore, MockModel, Model,
    ModelError, RunLimits, RunState, RunStatus, SafeDefaults, Sandbox, Store,
};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(100);

fn create_temp_dir(prefix: &str) -> PathBuf {
    let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut path = std::env::temp_dir();
    path.push(format!(
        "s3a_operability_test_{}_{}_{}",
        prefix,
        std::process::id(),
        id
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn valid_test_config() -> AgentConfig {
    AgentConfig {
        id: "scholar_reviewer".to_string(),
        name: "Scholar Reviewer Agent".to_string(),
        system_prompt: "You are an academic reviewer.".to_string(),
        model: "mock-model".to_string(),
        escalation_model: None,
        limits: RunLimits {
            max_turns: 5,
            max_tool_calls: 5,
            timeout_secs: 10,
            token_budget: 10_000,
        },
        thresholds: DecisionThresholds {
            tool_selection: 0.70,
            trimming: 0.80,
            stuck_detection: 0.85,
            escalation: 0.90,
        },
        safe_defaults: SafeDefaults {
            tool_selection: "preflight_check".to_string(),
            trimming: "head_tail_trim".to_string(),
            stuck_detection: true,
            escalation: false,
        },
        allowed_tools: vec![
            "preflight_check".to_string(),
            "s3a_query".to_string(),
            "ask_human".to_string(),
        ],
    }
}

struct FailingModel;

impl Model for FailingModel {
    fn model_id(&self) -> &str {
        "broken-model"
    }

    fn complete(
        &self,
        _request: &s3a_harness::CompletionRequest,
    ) -> Result<s3a_harness::CompletionResponse, ModelError> {
        Err(ModelError::HttpError(
            "Backend service unavailable (HTTP 503)".to_string(),
        ))
    }
}

#[test]
fn test_doctor_diagnostics_all_healthy() {
    let root = create_temp_dir("doctor_healthy");
    let sandbox = Sandbox::new(&root, false).unwrap();
    let db_path = sandbox.root().join("test.s3a");
    let store: Arc<dyn Store> = Arc::new(LocalStore::new(&db_path, false));
    let model: Arc<dyn Model> =
        Arc::new(MockModel::new("mock-model").with_default_response("pong"));

    // Write a valid config file
    let cfg_path = root.join("valid_agent.toml");
    let cfg_toml = r#"
[agent]
id = "scholar_reviewer"
name = "Scholar Reviewer"
system_prompt = "Review"
model = "mock"

[limits]
max_turns = 5
max_tool_calls = 5
timeout_secs = 30
token_budget = 5000

[decision_thresholds]
tool_selection = 0.70
trimming = 0.80
stuck_detection = 0.85
escalation = 0.90

[safe_defaults]
tool_selection = "preflight_check"
trimming = "head_tail_trim"
stuck_detection = true
escalation = false

[tools]
allowed = ["preflight_check", "s3a_query"]
"#;
    fs::write(&cfg_path, cfg_toml).unwrap();

    let report = run_doctor(Some(&cfg_path), model.as_ref(), store.as_ref(), &sandbox);

    assert!(report.all_healthy, "Expected all diagnostic checks to pass");
    assert_eq!(report.checks.len(), 4);
    for check in &report.checks {
        assert!(
            check.passed,
            "Check '{}' failed: {}",
            check.name, check.message
        );
        assert!(check.remediation.is_none());
    }

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_doctor_detects_invalid_config() {
    let root = create_temp_dir("doctor_invalid_cfg");
    let sandbox = Sandbox::new(&root, false).unwrap();
    let db_path = sandbox.root().join("test.s3a");
    let store: Arc<dyn Store> = Arc::new(LocalStore::new(&db_path, false));
    let model: Arc<dyn Model> =
        Arc::new(MockModel::new("mock-model").with_default_response("pong"));

    // Write an invalid config file (max_turns = 0 violates validation)
    let bad_cfg_path = root.join("bad_agent.toml");
    let bad_toml = r#"
[agent]
id = "scholar_reviewer"
name = "Scholar Reviewer"
system_prompt = "Review"
model = "mock"

[limits]
max_turns = 0
max_tool_calls = 5
timeout_secs = 30
token_budget = 5000

[decision_thresholds]
tool_selection = 0.70
trimming = 0.80
stuck_detection = 0.85
escalation = 0.90

[safe_defaults]
tool_selection = "preflight_check"
trimming = "head_tail_trim"
stuck_detection = true
escalation = false

[tools]
allowed = ["preflight_check"]
"#;
    fs::write(&bad_cfg_path, bad_toml).unwrap();

    let report = run_doctor(
        Some(&bad_cfg_path),
        model.as_ref(),
        store.as_ref(),
        &sandbox,
    );

    assert!(
        !report.all_healthy,
        "Expected overall diagnostic status to be unhealthy"
    );
    let cfg_check = report
        .checks
        .iter()
        .find(|c| c.name == "Configuration Validation")
        .unwrap();
    assert!(!cfg_check.passed);
    assert!(cfg_check.remediation.is_some());

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_doctor_detects_unreachable_model() {
    let root = create_temp_dir("doctor_broken_model");
    let sandbox = Sandbox::new(&root, false).unwrap();
    let db_path = sandbox.root().join("test.s3a");
    let store: Arc<dyn Store> = Arc::new(LocalStore::new(&db_path, false));
    let model = FailingModel;

    let report = run_doctor(None, &model, store.as_ref(), &sandbox);

    assert!(!report.all_healthy);
    let model_check = report
        .checks
        .iter()
        .find(|c| c.name == "Model Reachability")
        .unwrap();
    assert!(!model_check.passed);
    assert!(model_check.message.contains("unreachable"));
    assert!(model_check.remediation.is_some());

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_run_snapshot_branch_forking() {
    let root = create_temp_dir("snapshot_branch");
    let sandbox = Sandbox::new(&root, false).unwrap();
    let db_path = sandbox.root().join("test.s3a");
    let store: Arc<dyn Store> = Arc::new(LocalStore::new(&db_path, false));
    let model: Arc<dyn Model> = Arc::new(
        MockModel::new("mock-model")
            .with_default_response("{\"answer\": \"preflight_check\", \"confidence\": 0.95, \"rationale\": \"Safe preflight check\"}")
    );

    let config = valid_test_config();
    let runtime = AgentRuntime::new(config, model, None, store, sandbox.clone());

    let initial_base_snapshot = "snap_sealed_root_genesis_42".to_string();
    let state = RunState::with_base_snapshot(
        "scholar_reviewer",
        "Perform preflight verification",
        [1, 2],
        [3, 4],
        Some(initial_base_snapshot.clone()),
    );

    let completed_state = runtime.run(state).unwrap();

    assert_eq!(
        completed_state.base_snapshot_id.as_deref(),
        Some("snap_sealed_root_genesis_42")
    );
    assert!(
        completed_state.forked_snapshot_id.is_some(),
        "Expected run to produce forked snapshot branch"
    );
    let branch_id = completed_state.forked_snapshot_id.as_ref().unwrap();
    assert!(branch_id.starts_with("snap_branch_"));

    // Verify saved run on disk preserves both snapshot branch IDs
    let loaded = runtime.load_run(&completed_state.run_id).unwrap();
    assert_eq!(loaded.base_snapshot_id, Some(initial_base_snapshot));
    assert_eq!(
        loaded.forked_snapshot_id,
        completed_state.forked_snapshot_id
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_diff_runs_identical_and_divergent() {
    let mut run_a = RunState::with_base_snapshot(
        "agent_a",
        "Task A",
        [1, 1],
        [2, 2],
        Some("snap_base_001".to_string()),
    );
    run_a.executed_tools = vec!["preflight_check".to_string(), "s3a_query".to_string()];
    run_a.turn_count = 2;
    run_a.tool_call_count = 2;
    run_a.tokens_used = 240;
    run_a.status = RunStatus::Completed {
        turns: 2,
        tool_calls: 2,
    };

    let mut run_b_identical = RunState::with_base_snapshot(
        "agent_b",
        "Task B",
        [1, 1],
        [2, 2],
        Some("snap_base_001".to_string()),
    );
    run_b_identical.executed_tools = vec!["preflight_check".to_string(), "s3a_query".to_string()];
    run_b_identical.turn_count = 2;
    run_b_identical.tool_call_count = 2;
    run_b_identical.tokens_used = 240;
    run_b_identical.status = RunStatus::Completed {
        turns: 2,
        tool_calls: 2,
    };

    // 1. Identical comparison
    let diff_identical = AgentRuntime::diff_runs(&run_a, &run_b_identical);
    assert!(diff_identical.base_snapshot_match);
    assert_eq!(diff_identical.tool_sequence_divergence_turn, None);
    assert_eq!(diff_identical.turns_a, diff_identical.turns_b);

    // 2. Divergent comparison
    let mut run_c_divergent = RunState::with_base_snapshot(
        "agent_c",
        "Task C",
        [1, 1],
        [2, 2],
        Some("snap_base_diff_002".to_string()),
    );
    run_c_divergent.executed_tools = vec!["preflight_check".to_string(), "ask_human".to_string()];
    run_c_divergent.turn_count = 2;
    run_c_divergent.status = RunStatus::PausedWaitingHuman {
        gate_id: "gate_1".to_string(),
        reason: "Review required".to_string(),
    };

    let diff_divergent = AgentRuntime::diff_runs(&run_a, &run_c_divergent);
    assert!(!diff_divergent.base_snapshot_match);
    assert_eq!(diff_divergent.tool_sequence_divergence_turn, Some(2));
    assert_ne!(diff_divergent.status_a, diff_divergent.status_b);
}

#[test]
fn test_diff_saved_runs_from_sandbox() {
    let root = create_temp_dir("diff_saved_runs");
    let sandbox = Sandbox::new(&root, false).unwrap();
    let db_path = sandbox.root().join("test.s3a");
    let store: Arc<dyn Store> = Arc::new(LocalStore::new(&db_path, false));
    let model: Arc<dyn Model> = Arc::new(MockModel::new("mock-model"));

    let config = valid_test_config();
    let runtime = AgentRuntime::new(config, model, None, store, sandbox.clone());

    let mut run_1 = RunState::with_base_snapshot(
        "agent_1",
        "Goal 1",
        [1, 2],
        [3, 4],
        Some("snap_base_x".to_string()),
    );
    run_1.executed_tools = vec!["preflight_check".to_string()];
    run_1.status = RunStatus::Completed {
        turns: 1,
        tool_calls: 1,
    };
    runtime.persist_run(&run_1).unwrap();

    let mut run_2 = RunState::with_base_snapshot(
        "agent_2",
        "Goal 2",
        [1, 2],
        [3, 4],
        Some("snap_base_x".to_string()),
    );
    run_2.executed_tools = vec!["s3a_query".to_string()];
    run_2.status = RunStatus::Completed {
        turns: 1,
        tool_calls: 1,
    };
    runtime.persist_run(&run_2).unwrap();

    let diff = AgentRuntime::diff_saved_runs(&sandbox, &run_1.run_id, &run_2.run_id).unwrap();
    assert!(diff.base_snapshot_match);
    assert_eq!(diff.tool_sequence_divergence_turn, Some(1));
    assert_eq!(diff.tools_a, vec!["preflight_check"]);
    assert_eq!(diff.tools_b, vec!["s3a_query"]);

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_init_logging_idempotence() {
    s3a_harness::init_logging(false);
    s3a_harness::init_logging(true);
}
