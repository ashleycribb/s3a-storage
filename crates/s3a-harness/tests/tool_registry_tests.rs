use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;

use s3a_harness::config::{AgentConfig, DecisionThresholds, RunLimits, SafeDefaults};
use s3a_harness::model::MockModel;
use s3a_harness::runtime::{AgentRuntime, RunState, RunStatus};
use s3a_harness::sandbox::Sandbox;
use s3a_harness::store::LocalStore;
use s3a_harness::tools::{
    CrossrefLookupTool, OpenAlexLookupTool, RetractionWatchCheckTool, S3aQueryTool, S3aWriteTool,
    Tool, ToolError, ToolExecutionContext, ToolRegistry,
};

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
        id: "scholar_reviewer".to_string(),
        name: "Scholar Literature Reviewer".to_string(),
        system_prompt: "You are a test reviewer agent.".to_string(),
        model: "mock-3b".to_string(),
        escalation_model: None,
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
            "s3a_query".to_string(),
            "s3a_write".to_string(),
            "openalex_lookup".to_string(),
            "crossref_lookup".to_string(),
            "retraction_watch_check".to_string(),
            "preflight_check".to_string(),
        ],
        require_grant: false,
    }
}

fn create_test_context() -> (ToolExecutionContext, TempDirGuard) {
    let tmp = TempDirGuard::new("s3a_tool_test");
    let sandbox = Sandbox::new(&tmp.path, false).expect("sandbox");
    let store = Arc::new(LocalStore::new(sandbox.default_db(), false));
    let ctx = ToolExecutionContext {
        agent_id: "scholar_reviewer".to_string(),
        session_uuid: [0x1111_2222_3333_4444, 0x5555_6666_7777_8888],
        sandbox,
        store,
        grant_record: None,
    };
    (ctx, tmp)
}

#[test]
fn test_tool_registry_list_tools_and_schemas() {
    let registry = ToolRegistry::new_default();
    let tools = registry.list_tools(None);

    let names: Vec<String> = tools.iter().map(|t| t.name.clone()).collect();
    assert!(names.contains(&"s3a_query".to_string()));
    assert!(names.contains(&"s3a_write".to_string()));
    assert!(names.contains(&"openalex_lookup".to_string()));
    assert!(names.contains(&"crossref_lookup".to_string()));
    assert!(names.contains(&"retraction_watch_check".to_string()));
    assert!(names.contains(&"preflight_check".to_string()));

    // Verify all tools provide valid JSON Schema definitions
    for tool in tools {
        assert_eq!(tool.input_schema["type"], "object");
        assert!(tool.input_schema.get("properties").is_some());
    }
}

#[test]
fn test_tool_registry_per_agent_allowlist_enforcement() {
    let registry = ToolRegistry::new_default();
    let (ctx, _tmp) = create_test_context();

    let mut config = create_test_config();
    config.id = "restricted_agent".to_string();
    config.allowed_tools = vec!["s3a_query".to_string()];

    // 1. Allowed tool execution succeeds
    let query_args = json!({ "query": "AlphaFold", "limit": 5 });
    let res = registry.call_tool(&config, "s3a_query", &query_args, &ctx);
    assert!(res.is_ok(), "Allowed tool call must succeed");

    // 2. Disallowed tool execution rejected with DisallowedTool error
    let write_args = json!({ "target": "note", "content": "Unauthorized note" });
    let err = registry
        .call_tool(&config, "s3a_write", &write_args, &ctx)
        .expect_err("Disallowed tool must fail");
    assert!(
        matches!(err, ToolError::DisallowedTool { ref tool, ref agent } if tool == "s3a_write" && agent == "restricted_agent"),
        "Expected DisallowedTool error, got {:?}",
        err
    );
}

#[test]
fn test_tool_call_logs_inputs_hash_and_result_hash_provenance() {
    let registry = ToolRegistry::new_default();
    let (ctx, _tmp) = create_test_context();

    let mut config = create_test_config();
    config.id = "scholar_reviewer".to_string();
    config.allowed_tools = vec!["openalex_lookup".to_string()];

    let args = json!({ "doi": "10.1038/s41586-020-2649-2" });
    let call_res = registry
        .call_tool(&config, "openalex_lookup", &args, &ctx)
        .expect("Tool call execution");

    // Assert cryptographic 64-character SHA-256 hexadecimal digests
    assert_eq!(call_res.inputs_hash.len(), 64);
    assert_eq!(call_res.result_hash.len(), 64);
    assert_ne!(call_res.inputs_hash, call_res.result_hash);
    assert_eq!(call_res.tool_name, "openalex_lookup");

    // Verify provenance statement was committed to Store
    let session_str = s3a_harness::uuid_pair_to_string(ctx.session_uuid);
    let timeline = ctx
        .store
        .get_researcher_timeline(None, Some(&session_str), Some(10))
        .expect("Get timeline");

    assert!(
        !timeline.is_empty(),
        "Provenance timeline must not be empty"
    );
    println!("timeline[0] = {}", timeline[0]);
    assert!(timeline[0].get("timestamp").is_some());
}

#[test]
fn test_s3a_query_and_write_tools_in_sandbox() {
    let (ctx, _tmp) = create_test_context();

    // 1. Write note
    let write_tool = S3aWriteTool;
    let write_args = json!({
        "target": "claim",
        "content": "Empirical claim regarding structural convergence in AlphaFold",
        "archive_name": "evidence.s3a"
    });
    let write_res = write_tool
        .execute(&write_args, &ctx)
        .expect("Execute write");
    assert_eq!(write_res["status"], "committed");
    assert!(write_res["content_hash"].as_str().is_some());

    // Verify written file exists under <root>/exports/
    let written_file = ctx.sandbox.exports_dir().join("evidence.s3a");
    assert!(written_file.exists());

    // 2. Query
    let query_tool = S3aQueryTool;
    let query_args = json!({ "query": "AlphaFold", "min_year": 2020 });
    let query_res = query_tool
        .execute(&query_args, &ctx)
        .expect("Execute query");
    assert!(query_res["count"].as_u64().unwrap() >= 1);
}

#[test]
fn test_openalex_and_crossref_lookup_tools() {
    let (ctx, _tmp) = create_test_context();

    // OpenAlex lookup
    let openalex = OpenAlexLookupTool;
    let res_oa = openalex
        .execute(&json!({ "doi": "10.1038/s41586-020-2649-2" }), &ctx)
        .expect("OpenAlex execution");
    assert!(res_oa["title"].as_str().is_some());
    let title = res_oa["title"].as_str().unwrap();
    assert!(title.contains("NumPy") || title.contains("AlphaFold") || title.contains("Array"));
    assert_eq!(res_oa["is_retracted"], false);

    // Crossref lookup
    let crossref = CrossrefLookupTool;
    let res_cr = crossref
        .execute(&json!({ "doi": "10.1038/s41586-020-2649-2" }), &ctx)
        .expect("Crossref execution");
    assert!(res_cr["title"].as_str().is_some());
    assert!(res_cr["publisher"].as_str().is_some());
}

#[test]
fn test_retraction_watch_check_clean_and_retracted() {
    let (ctx, _tmp) = create_test_context();
    let rw = RetractionWatchCheckTool;

    // Clean DOI
    let clean = rw
        .execute(&json!({ "doi": "10.1038/s41586-020-2649-2" }), &ctx)
        .expect("Retraction check clean");
    assert_eq!(clean["is_retracted"], false);
    assert_eq!(clean["status"], "clean");

    // Retracted DOI
    let retracted = rw
        .execute(&json!({ "doi": "10.1016/j.cell.2020.01.001" }), &ctx)
        .expect("Retraction check retracted");
    assert_eq!(retracted["is_retracted"], true);
    assert_eq!(retracted["status"], "retracted");
    assert!(retracted["reason"].as_str().is_some());

    // Expression of concern
    let concern = rw
        .execute(&json!({ "doi": "10.1000/concern-study-2022" }), &ctx)
        .expect("Retraction check concern");
    assert_eq!(concern["status"], "expression_of_concern");
}

#[test]
fn test_mcp_client_external_tool_delegation() {
    let registry = ToolRegistry::new();
    let (ctx, _tmp) = create_test_context();

    // Register simulated external MCP tool
    registry.register_mcp_client(
        "external_biomedical_mcp",
        vec![(
            "predict_binding_affinity".to_string(),
            "Calculate binding affinity score".to_string(),
            json!({
                "type": "object",
                "properties": {
                    "ligand": { "type": "string" },
                    "target": { "type": "string" }
                }
            }),
        )],
        None, // In-process mock MCP
    );

    let mut config = create_test_config();
    config
        .allowed_tools
        .push("predict_binding_affinity".to_string());
    let args = json!({ "ligand": "CID_2244", "target": "P00533" });

    let res = registry
        .call_tool(&config, "predict_binding_affinity", &args, &ctx)
        .expect("Call external MCP tool");

    assert_eq!(res.tool_name, "predict_binding_affinity");
    assert_eq!(res.output["status"], "success");
    assert_eq!(res.output["mcp_server"], "external_biomedical_mcp");
    assert_eq!(res.inputs_hash.len(), 64);
    assert_eq!(res.result_hash.len(), 64);
}

#[test]
fn test_agent_runtime_executes_tools_and_populates_provenance() {
    let tmp = TempDirGuard::new("s3a_runtime_tool_test");
    let sandbox = Sandbox::new(&tmp.path, false).expect("sandbox");
    let store = Arc::new(LocalStore::new(sandbox.root(), false));

    let mut config = create_test_config();
    config.id = "scholar_reviewer".to_string();
    config.allowed_tools = vec!["s3a_query".to_string(), "preflight_check".to_string()];

    let mock = Arc::new(MockModel::new("mock-3b"));
    mock.queue_response(
        r#"{"answer": "s3a_query", "confidence": 0.95, "rationale": "Query literature"}"#,
    );
    mock.queue_response(
        r#"{"answer": "preflight_check", "confidence": 0.95, "rationale": "Verify existence"}"#,
    );

    let runtime = AgentRuntime::new(config, mock, None, store, sandbox);
    let state = RunState::new(
        "scholar_reviewer",
        "Protein folding synthesis",
        [1, 2],
        [3, 4],
    );

    let final_state = runtime.run(state).expect("Run runtime with tools");
    assert!(matches!(final_state.status, RunStatus::Completed { .. }));

    // Assert that tool provenance was populated during the run
    assert!(
        !final_state.tool_provenance.is_empty(),
        "Tool provenance must be captured"
    );
    for prov in &final_state.tool_provenance {
        assert_eq!(prov.inputs_hash.len(), 64);
        assert_eq!(prov.result_hash.len(), 64);
    }
}
