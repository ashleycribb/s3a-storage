use std::path::PathBuf;
use std::sync::Arc;

use s3a_harness::config::{AgentConfig, DecisionThresholds, RunLimits, SafeDefaults};
use s3a_harness::decision::{
    is_claim_judgment_call, DecisionEngine, DecisionKind, DecisionRequest, DecisionValue,
    OperationalDecisionType,
};
use s3a_harness::model::{CompletionRequest, MockModel, Model};
use s3a_harness::runtime::{AgentRuntime, RunState, RunStatus};
use s3a_harness::sandbox::Sandbox;
use s3a_harness::store::LocalStore;

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

fn create_test_config() -> AgentConfig {
    AgentConfig {
        id: "test_scholar".to_string(),
        name: "Test Scholar Reviewer".to_string(),
        system_prompt: "You are a test agent.".to_string(),
        model: "mock-small".to_string(),
        escalation_model: Some("mock-large".to_string()),
        limits: RunLimits {
            max_turns: 5,
            max_tool_calls: 10,
            timeout_secs: 10,
            token_budget: 1000,
        },
        thresholds: DecisionThresholds {
            tool_selection: 0.70,
            trimming: 0.65,
            stuck_detection: 0.80,
            escalation: 0.75,
        },
        safe_defaults: SafeDefaults {
            tool_selection: "ask_human".to_string(),
            trimming: "head_tail_trim".to_string(),
            stuck_detection: false,
            escalation: false,
        },
        allowed_tools: vec![
            "s3a_search_library".to_string(),
            "get_synthesis_matrix".to_string(),
            "preflight_check".to_string(),
            "ask_human".to_string(),
        ],
    }
}

#[test]
fn test_model_trait_mock_model_completion() {
    let mock = MockModel::new("mock-v1");
    mock.queue_response(
        r#"{"answer": "s3a_search_library", "confidence": 0.92, "rationale": "Search literature"}"#,
    );

    let req = CompletionRequest::prompt("What tool to run?");
    let resp = mock.complete(&req).expect("Completion must succeed");

    assert_eq!(resp.model, "mock-v1");
    assert!(resp.content.contains("s3a_search_library"));
    assert_eq!(mock.get_call_count(), 1);

    // Default response when queue empty
    let resp2 = mock.complete(&req).expect("Default completion");
    assert_eq!(resp2.content, "MockModel: OK");
    assert_eq!(mock.get_call_count(), 2);
}

#[test]
fn test_agent_config_toml_loading_and_defaults() {
    let toml_str = r#"
[agent]
id = "scholar_custom"
name = "Custom Reviewer"
system_prompt = "Custom system prompt."
model = "small-model"
escalation_model = "strong-model"

[limits]
max_turns = 8
max_tool_calls = 16
timeout_secs = 120
token_budget = 16000

[decision_thresholds]
tool_selection = 0.75
trimming = 0.60
stuck_detection = 0.85
escalation = 0.70

[safe_defaults]
tool_selection = "ask_human"
trimming = "truncate_half"
stuck_detection = false
escalation = false

[tools]
allowed = ["s3a_search_library", "preflight_check"]
"#;

    let cfg = AgentConfig::parse_toml(toml_str).expect("Must parse valid TOML");
    assert_eq!(cfg.id, "scholar_custom");
    assert_eq!(cfg.model, "small-model");
    assert_eq!(cfg.escalation_model.as_deref(), Some("strong-model"));
    assert_eq!(cfg.limits.max_turns, 8);
    assert_eq!(cfg.limits.token_budget, 16000);
    assert_eq!(cfg.thresholds.tool_selection, 0.75);
    assert_eq!(cfg.safe_defaults.trimming, "truncate_half");
    assert_eq!(cfg.allowed_tools.len(), 2);
}

#[test]
fn test_typed_decisions_choice_yesno_score() {
    let guard = TempDirGuard::new("s3a_decisions_test");
    let sandbox = Sandbox::new(&guard.path, false).unwrap();
    let store = Arc::new(LocalStore::new(sandbox.default_db(), false));
    let mock = Arc::new(MockModel::new("mock-decider"));

    let engine = DecisionEngine::new(
        mock.clone(),
        None,
        DecisionThresholds::default(),
        SafeDefaults::default(),
        store,
        "test-session-1",
    );

    // 1. Choice decision
    mock.queue_response(r#"{"answer": "s3a_search_library", "confidence": 0.88, "rationale": "High confidence tool"}"#);
    let choice_req = DecisionRequest {
        decision_type: OperationalDecisionType::ToolSelection,
        question: "Select tool".to_string(),
        context: "Initial step".to_string(),
        kind: DecisionKind::Choice {
            options: vec![
                "s3a_search_library".to_string(),
                "preflight_check".to_string(),
            ],
        },
        safe_default: DecisionValue::Choice("preflight_check".to_string()),
        is_claim_judgment: false,
    };
    let res1 = engine.evaluate(&choice_req, false, false);
    assert_eq!(
        res1.value,
        DecisionValue::Choice("s3a_search_library".to_string())
    );
    assert_eq!(res1.confidence, 0.88);
    assert!(!res1.used_safe_default);

    // 2. YesNo decision
    mock.queue_response(r#"{"answer": true, "confidence": 0.95, "rationale": "Run is stuck"}"#);
    let yesno_req = DecisionRequest {
        decision_type: OperationalDecisionType::StuckRunDetection,
        question: "Is agent stuck?".to_string(),
        context: "Repeating actions".to_string(),
        kind: DecisionKind::YesNo,
        safe_default: DecisionValue::YesNo(false),
        is_claim_judgment: false,
    };
    let res2 = engine.evaluate(&yesno_req, false, false);
    assert_eq!(res2.value, DecisionValue::YesNo(true));
    assert_eq!(res2.confidence, 0.95);

    // 3. Score decision
    mock.queue_response(
        r#"{"answer": 0.82, "confidence": 0.90, "rationale": "Calibrated metric"}"#,
    );
    let score_req = DecisionRequest {
        decision_type: OperationalDecisionType::TrimmingLargeResult,
        question: "Determine trimming ratio".to_string(),
        context: "Context length 8000 tokens".to_string(),
        kind: DecisionKind::Score,
        safe_default: DecisionValue::Score(0.50),
        is_claim_judgment: false,
    };
    let res3 = engine.evaluate(&score_req, false, false);
    assert_eq!(res3.value, DecisionValue::Score(0.82));
    assert_eq!(res3.confidence, 0.90);
}

#[test]
fn test_claim_judgment_mandatory_routing_to_ask_human() {
    let guard = TempDirGuard::new("s3a_claim_judgment_test");
    let sandbox = Sandbox::new(&guard.path, false).unwrap();
    let store = Arc::new(LocalStore::new(sandbox.default_db(), false));
    let mock = Arc::new(MockModel::new("mock-decider"));

    let engine = DecisionEngine::new(
        mock.clone(),
        None,
        DecisionThresholds::default(),
        SafeDefaults::default(),
        store,
        "test-session-2",
    );

    // Test cases containing claim judgments: support, rigor, overstatement
    let claim_prompts = vec![
        "Does this study provide sufficient claim support for causal efficacy?",
        "Assess the methodological rigor of the randomized control group.",
        "Check whether conclusion is an overstatement beyond experimental evidence.",
    ];

    for prompt in claim_prompts {
        assert!(
            is_claim_judgment_call(prompt),
            "Must detect claim judgment keyword"
        );

        let req = DecisionRequest {
            decision_type: OperationalDecisionType::ToolSelection,
            question: prompt.to_string(),
            context: "Paper evaluation".to_string(),
            kind: DecisionKind::Choice {
                options: vec!["approve_claim".to_string(), "reject_claim".to_string()],
            },
            safe_default: DecisionValue::Choice("ask_human".to_string()),
            is_claim_judgment: false, // Detected via text
        };

        let res = engine.evaluate(&req, false, false);
        assert!(
            res.routed_to_human,
            "Must route subjective judgment to human supervisor"
        );
        assert_eq!(res.action_taken, "ask_human");
        assert!(res.used_safe_default);
        assert!(res.rationale.contains("Claim judgment detected"));
        // Model was never called
        assert_eq!(mock.get_call_count(), 0);
    }
}

#[test]
fn test_fail_safe_fallback_on_model_error_or_malformed_json() {
    let guard = TempDirGuard::new("s3a_fail_safe_test");
    let sandbox = Sandbox::new(&guard.path, false).unwrap();
    let store = Arc::new(LocalStore::new(sandbox.default_db(), false));
    let mock = Arc::new(MockModel::new("mock-fail"));

    let engine = DecisionEngine::new(
        mock.clone(),
        None,
        DecisionThresholds::default(),
        SafeDefaults::default(),
        store,
        "test-session-3",
    );

    // 1. Malformed JSON
    mock.queue_response("Invalid JSON output from model without braces");
    let req1 = DecisionRequest {
        decision_type: OperationalDecisionType::ToolSelection,
        question: "Select tool".to_string(),
        context: "Context".to_string(),
        kind: DecisionKind::Choice {
            options: vec!["s3a_search_library".to_string()],
        },
        safe_default: DecisionValue::Choice("safe_fallback_tool".to_string()),
        is_claim_judgment: false,
    };
    let res1 = engine.evaluate(&req1, false, false);
    assert!(res1.used_safe_default);
    assert_eq!(
        res1.value,
        DecisionValue::Choice("safe_fallback_tool".to_string())
    );
    assert_eq!(res1.confidence, 0.0);
    assert!(res1.rationale.contains("JSON parsing error"));

    // 2. Unmatched choice option
    mock.queue_response(
        r#"{"answer": "forbidden_tool", "confidence": 0.99, "rationale": "Hacking"}"#,
    );
    let res2 = engine.evaluate(&req1, false, false);
    assert!(res2.used_safe_default);
    assert_eq!(
        res2.value,
        DecisionValue::Choice("safe_fallback_tool".to_string())
    );
}

#[test]
fn test_confidence_gating_below_threshold_routes_to_safe_default() {
    let guard = TempDirGuard::new("s3a_conf_gate_test");
    let sandbox = Sandbox::new(&guard.path, false).unwrap();
    let store = Arc::new(LocalStore::new(sandbox.default_db(), false));
    let mock = Arc::new(MockModel::new("mock-low-conf"));

    let engine = DecisionEngine::new(
        mock.clone(),
        None,
        DecisionThresholds {
            tool_selection: 0.70, // threshold = 0.70
            trimming: 0.65,
            stuck_detection: 0.80,
            escalation: 0.75,
        },
        SafeDefaults {
            tool_selection: "ask_human".to_string(),
            trimming: "head_tail_trim".to_string(),
            stuck_detection: false,
            escalation: false,
        },
        store,
        "test-session-4",
    );

    // Confidence 0.55 is below threshold 0.70
    mock.queue_response(
        r#"{"answer": "s3a_search_library", "confidence": 0.55, "rationale": "Weak confidence"}"#,
    );
    let req = DecisionRequest {
        decision_type: OperationalDecisionType::ToolSelection,
        question: "Select tool".to_string(),
        context: "Uncertain state".to_string(),
        kind: DecisionKind::Choice {
            options: vec!["s3a_search_library".to_string()],
        },
        safe_default: DecisionValue::Choice("ask_human".to_string()),
        is_claim_judgment: false,
    };

    let res = engine.evaluate(&req, false, false);
    assert!(res.used_safe_default);
    assert_eq!(res.action_taken, "ask_human");
    assert!(res.routed_to_human);
}

#[test]
fn test_runtime_limits_turns_and_token_budget() {
    let guard = TempDirGuard::new("s3a_limits_test");
    let sandbox = Sandbox::new(&guard.path, false).unwrap();
    let store = Arc::new(LocalStore::new(sandbox.default_db(), false));

    // Agent with max_turns = 2
    let mut config = create_test_config();
    config.limits.max_turns = 2;

    let mock = Arc::new(MockModel::new("mock-fast").with_default_response(
        r#"{"answer": "s3a_search_library", "confidence": 0.95, "rationale": "Keep searching"}"#,
    ));

    let runtime = AgentRuntime::new(config, mock, None, store, sandbox);
    let state = RunState::new("test_agent", "Test limit exhaustion", [1, 2], [3, 4]);

    let final_state = runtime.run(state).expect("Run loop execution");
    assert!(matches!(
        final_state.status,
        RunStatus::LimitExceeded { .. }
    ));
    assert_eq!(final_state.turn_count, 2);
}

#[test]
fn test_stuck_run_detection_and_escalation_limit() {
    let guard = TempDirGuard::new("s3a_stuck_test");
    let sandbox = Sandbox::new(&guard.path, false).unwrap();
    let store = Arc::new(LocalStore::new(sandbox.default_db(), false));

    let config = create_test_config();
    let small_model = Arc::new(MockModel::new("mock-small").with_default_response(
        r#"{"answer": "s3a_search_library", "confidence": 0.95, "rationale": "Repetitive action"}"#,
    ));
    // Queue true for StuckRunDetection
    small_model.queue_response(
        r#"{"answer": true, "confidence": 0.95, "rationale": "Repetitive loop detected"}"#,
    );

    let strong_model = Arc::new(MockModel::new("mock-strong").with_default_response(
        r#"{"answer": "preflight_check", "confidence": 0.95, "rationale": "Strong model breaks loop"}"#,
    ));

    let runtime = AgentRuntime::new(config, small_model, Some(strong_model), store, sandbox);
    let mut state = RunState::new("test_agent", "Test stuck escalation", [1, 2], [3, 4]);

    // Pre-populate with repeating executed tools so stuck detection triggers
    state.executed_tools = vec![
        "s3a_search_library".to_string(),
        "s3a_search_library".to_string(),
        "s3a_search_library".to_string(),
    ];

    let result_state = runtime.run(state).expect("Run execution");
    // Verified that escalation to stronger model occurred
    assert!(
        result_state.escalated,
        "Escalation must be flagged when stuck"
    );
}

#[test]
fn test_ask_human_pause_and_resume_roundtrip() {
    let guard = TempDirGuard::new("s3a_pause_resume_test");
    let sandbox = Sandbox::new(&guard.path, false).unwrap();
    let store = Arc::new(LocalStore::new(sandbox.default_db(), false));

    let config = create_test_config();
    // Configure model to choose "ask_human"
    let mock = Arc::new(MockModel::new("mock-pauser").with_default_response(
        r#"{"answer": "ask_human", "confidence": 0.95, "rationale": "Supervisory approval required for novel paper"}"#,
    ));

    let runtime = AgentRuntime::new(config, mock.clone(), None, store.clone(), sandbox.clone());
    let state = RunState::new(
        "scholar_reviewer",
        "Review novel preprint",
        [10, 20],
        [30, 40],
    );
    let run_id = state.run_id.clone();

    // 1. Run pauses and writes state to <root>/runs/<run_id>.json
    let paused_state = runtime.run(state).expect("Run execution");
    assert!(matches!(
        paused_state.status,
        RunStatus::PausedWaitingHuman { .. }
    ));

    let saved_path = sandbox.root().join("runs").join(format!("{}.json", run_id));
    assert!(saved_path.exists(), "Run state file must exist on disk");

    // 2. Supervisor resumes run via runtime.resume(...)
    // Set model response to complete on resume
    mock.queue_response(r#"{"answer": "preflight_check", "confidence": 0.95, "rationale": "Advancing after approval"}"#);
    let resumed_state = runtime
        .resume(
            &run_id,
            Some("Supervisory approval granted: proceed".to_string()),
        )
        .expect("Resume run");

    assert_eq!(resumed_state.human_decisions.len(), 1);
    assert!(resumed_state.human_decisions[0].contains("Supervisory approval granted"));
    assert!(matches!(resumed_state.status, RunStatus::Completed { .. }));
}

#[test]
fn test_shadow_mode_runs_alongside_live_without_acting() {
    let guard = TempDirGuard::new("s3a_shadow_test");
    let sandbox = Sandbox::new(&guard.path, false).unwrap();
    let store = Arc::new(LocalStore::new(sandbox.default_db(), false));

    let config = create_test_config();
    // Live model chooses "preflight_check"
    let live_model = Arc::new(MockModel::new("mock-live").with_default_response(
        r#"{"answer": "preflight_check", "confidence": 0.95, "rationale": "Live model selected preflight check"}"#,
    ));
    // Candidate shadow model chooses "preflight_check" (agreement)
    let shadow_model = Arc::new(MockModel::new("candidate-v2").with_default_response(
        r#"{"answer": "preflight_check", "confidence": 0.92, "rationale": "Shadow candidate evaluation"}"#,
    ));

    let runtime = AgentRuntime::new(config, live_model, None, store, sandbox.clone())
        .with_shadow_model(shadow_model);

    let state = RunState::new(
        "scholar_reviewer",
        "Evaluate shadow agreement",
        [11, 22],
        [33, 44],
    );
    let run_id = state.run_id.clone();

    let final_state = runtime.run(state).expect("Run with shadow mode");
    // Verify shadow evaluations were captured
    assert!(
        !final_state.shadow_evaluations.is_empty(),
        "Shadow evaluations must be populated"
    );
    assert_eq!(
        final_state.shadow_evaluations[0].shadow_model,
        "candidate-v2"
    );
    assert!(
        final_state.shadow_evaluations[0].agreement,
        "Shadow decision agreed with live"
    );
    // Live action was executed, shadow model never acted
    assert_eq!(final_state.executed_tools[0], "preflight_check");

    // Generate shadow report
    let report =
        AgentRuntime::shadow_report(&sandbox, Some(&run_id)).expect("Shadow report generation");
    assert_eq!(report.total_decisions, final_state.shadow_evaluations.len());
    assert_eq!(report.agreements, final_state.shadow_evaluations.len());
    assert_eq!(report.agreement_rate, 100.0);
    assert!(
        report.promote_recommended,
        "100% agreement qualifies for promotion"
    );
}

#[test]
fn test_evals_assert_on_actions_taken_not_free_text() {
    let guard = TempDirGuard::new("s3a_evals_test");
    let sandbox = Sandbox::new(&guard.path, false).unwrap();
    let store = Arc::new(LocalStore::new(sandbox.default_db(), false));

    let config = create_test_config();
    let mock = Arc::new(MockModel::new("mock-eval").with_default_response(
        r#"{"answer": "preflight_check", "confidence": 0.96, "rationale": "Pass existence checks"}"#,
    ));

    let runtime = AgentRuntime::new(config, mock, None, store, sandbox);
    let state = RunState::new(
        "eval_agent",
        "Strict action assertion eval",
        [55, 66],
        [77, 88],
    );

    let final_state = runtime.run(state).expect("Run eval");

    // EVAL ASSERTION 1: Tool chosen must be in permitted list
    assert_eq!(final_state.executed_tools[0], "preflight_check");
    // EVAL ASSERTION 2: Final status is concrete enum, NOT free text matching
    assert!(matches!(final_state.status, RunStatus::Completed { .. }));
    // EVAL ASSERTION 3: No unapproved judgment actions occurred
    assert!(final_state.human_decisions.is_empty());
}
