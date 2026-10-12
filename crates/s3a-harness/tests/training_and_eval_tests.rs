use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::json;

use s3a_harness::{
    export_trajectories, run_eval_set, AgentConfig, AgentRuntime, DecisionThresholds, LocalStore,
    MockModel, Model, RunLimits, RunState, RunStatus, SafeDefaults, Sandbox, Store, ToolCallResult,
    TrajectoryFormat,
};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(500);

fn create_temp_dir(prefix: &str) -> PathBuf {
    let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut path = std::env::temp_dir();
    path.push(format!(
        "s3a_train_test_{}_{}_{}",
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
        require_grant: false,
    }
}

#[test]
fn test_trajectory_export_chatml_and_sharegpt_formats() {
    let root = create_temp_dir("export_formats");
    let sandbox = Sandbox::new(&root, false).unwrap();
    let db_path = sandbox.root().join("test.s3a");
    let store: Arc<dyn Store> = Arc::new(LocalStore::new(&db_path, false));
    let model: Arc<dyn Model> = Arc::new(MockModel::new("mock-model"));
    let config = valid_test_config();
    let runtime = AgentRuntime::new(config, model, None, store, sandbox.clone());

    // Create a run with tool provenance and persist to sandbox
    let mut run = RunState::with_base_snapshot(
        "scholar_reviewer",
        "Verify citation DOI 10.1038/s41586-020-2649-2",
        [1, 1],
        [2, 2],
        Some("snap_base_01".to_string()),
    );
    run.turn_count = 2;
    run.tool_call_count = 1;
    run.executed_tools.push("preflight_check".to_string());
    run.tool_provenance.push(ToolCallResult {
        tool_name: "preflight_check".to_string(),
        inputs: json!({ "query": "10.1038/s41586-020-2649-2" }),
        output: json!({ "status": "verified", "passed": true }),
        inputs_hash: "a1b2c3d4e5f60718293a4b5c6d7e8f90123456789abcdef0123456789abcdef0".to_string(),
        result_hash: "0f9e8d7c6b5a4321fedcba9876543210fedcba9876543210fedcba9876543210".to_string(),
        execution_time_ms: 12,
    });
    run.status = RunStatus::Completed {
        turns: 2,
        tool_calls: 1,
    };
    runtime.persist_run(&run).unwrap();

    // 1. Export in ChatML format
    let chatml_out = root.join("trajectories_chatml.jsonl");
    let summary_chatml = export_trajectories(
        &sandbox,
        TrajectoryFormat::ChatMl,
        &chatml_out,
        "model_training",
    )
    .unwrap();
    assert_eq!(summary_chatml.exported_records, 1);
    assert!(chatml_out.exists());

    let content_chatml = fs::read_to_string(&chatml_out).unwrap();
    let val_chatml: serde_json::Value = serde_json::from_str(&content_chatml).unwrap();
    let messages = val_chatml["messages"].as_array().unwrap();
    assert!(messages.iter().any(|m| m["role"] == "system"));
    assert!(messages.iter().any(|m| m["role"] == "user"));
    assert!(messages
        .iter()
        .any(|m| m["role"] == "assistant" && m.get("tool_calls").is_some()));
    assert!(messages.iter().any(|m| m["role"] == "tool"));

    // 2. Export in ShareGPT format
    let sharegpt_out = root.join("trajectories_sharegpt.jsonl");
    let summary_sharegpt = export_trajectories(
        &sandbox,
        TrajectoryFormat::ShareGpt,
        &sharegpt_out,
        "model_training",
    )
    .unwrap();
    assert_eq!(summary_sharegpt.exported_records, 1);
    assert!(sharegpt_out.exists());

    let content_sharegpt = fs::read_to_string(&sharegpt_out).unwrap();
    let val_sharegpt: serde_json::Value = serde_json::from_str(&content_sharegpt).unwrap();
    let conversations = val_sharegpt["conversations"].as_array().unwrap();
    assert!(conversations.iter().any(|c| c["from"] == "system"));
    assert!(conversations.iter().any(|c| c["from"] == "human"));
    assert!(conversations.iter().any(|c| c["from"] == "gpt"));
    assert!(conversations.iter().any(|c| c["from"] == "tool"));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_trajectory_export_consent_filtering() {
    let root = create_temp_dir("consent_filter");
    let sandbox = Sandbox::new(&root, false).unwrap();
    let db_path = sandbox.root().join("test.s3a");
    let store: Arc<dyn Store> = Arc::new(LocalStore::new(&db_path, false));
    let model: Arc<dyn Model> = Arc::new(MockModel::new("mock-model"));
    let config = valid_test_config();
    let runtime = AgentRuntime::new(config, model, None, store, sandbox.clone());

    // Run 1: Permitted for model_training
    let mut run_consented =
        RunState::with_base_snapshot("agent_1", "Consented run", [1, 1], [2, 2], None)
            .with_consent_purposes(vec![
                "model_training".to_string(),
                "research_audit".to_string(),
            ]);
    run_consented.status = RunStatus::Completed {
        turns: 1,
        tool_calls: 0,
    };
    runtime.persist_run(&run_consented).unwrap();

    // Run 2: Restricted (only clinical_care, no model_training)
    let mut run_restricted =
        RunState::with_base_snapshot("agent_2", "Restricted run", [1, 1], [2, 2], None)
            .with_consent_purposes(vec!["clinical_care".to_string()]);
    run_restricted.status = RunStatus::Completed {
        turns: 1,
        tool_calls: 0,
    };
    runtime.persist_run(&run_restricted).unwrap();

    let out_file = root.join("consent_filtered.jsonl");
    let summary = export_trajectories(
        &sandbox,
        TrajectoryFormat::ChatMl,
        &out_file,
        "model_training",
    )
    .unwrap();

    assert_eq!(summary.total_runs_found, 2);
    assert_eq!(summary.consent_accepted, 1);
    assert_eq!(summary.consent_filtered, 1);
    assert_eq!(summary.exported_records, 1);

    let content = fs::read_to_string(&out_file).unwrap();
    assert!(content.contains(&run_consented.run_id));
    assert!(!content.contains(&run_restricted.run_id));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_eval_runner_passes_action_assertions_and_tracks_metrics() {
    let root = create_temp_dir("eval_suite");
    let sandbox = Sandbox::new(&root, false).unwrap();
    let db_path = sandbox.root().join("test.s3a");
    let store: Arc<dyn Store> = Arc::new(LocalStore::new(&db_path, false));
    let model: Arc<dyn Model> = Arc::new(
        MockModel::new("mock-model")
            .with_default_response("{\"answer\": \"preflight_check\", \"confidence\": 0.95, \"rationale\": \"Safe preflight check\"}")
    );
    let config = valid_test_config();
    let runtime = AgentRuntime::new(config, model, None, store, sandbox.clone());

    // Write a test eval suite with 2 passing cases and 1 failing expectation
    let eval_file = root.join("test_evals.jsonl");
    let eval_content = r#"
{"id": "case_pass_1", "goal": "Verify metadata for paper", "expected_tool": "preflight_check", "should_hit_gate": false}
{"id": "case_pass_2", "goal": "Run preflight check", "should_hit_gate": false, "should_ask_human": false}
{"id": "case_fail_expected_tool", "goal": "Verify metadata", "expected_tool": "s3a_write"}
"#;
    fs::write(&eval_file, eval_content).unwrap();

    let results = run_eval_set(&runtime, &eval_file).unwrap();

    assert_eq!(results.total_cases, 3);
    assert_eq!(results.passed_cases, 2);
    assert_eq!(results.failed_cases, 1);
    assert!((results.success_rate - 66.66).abs() < 1.0);

    let failed_case = results
        .cases
        .iter()
        .find(|c| c.id == "case_fail_expected_tool")
        .unwrap();
    assert!(!failed_case.passed);
    assert!(failed_case
        .failure_reason
        .as_ref()
        .unwrap()
        .contains("Expected tool 's3a_write' was not invoked"));

    let _ = fs::remove_dir_all(&root);
}
