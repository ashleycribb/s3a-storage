//! S3A Standalone Research Agent Harness & Autonomous Execution Engine.
//! Replaces OpenHarness, Gno Vault, LightRAG, SQL LRS, and LearnMCP-xAPI into a unified,
//! zero-overhead Rust engine operating over S3A Hyper-Tiles and SRS Snapshots.

pub mod agent;
pub mod config;
pub mod decision;
pub mod doctor;
pub mod eval;
pub mod mcp;
pub mod model;
pub mod runtime;
pub mod sandbox;
pub mod srs;
pub mod store;
pub mod tools;
pub mod trajectory;
pub mod vcs;

pub use vcs::harness_merge;

pub use agent::{
    EpistemicConfidenceTier, GateRejectionLog, GateStatus, HallucinationRisk, HumanGateRequest,
    ResearchAction, ResearchLoopEngine, TaGuidance, TA_VERB_APPROVED_DRAFT, TA_VERB_ESCALATED,
    TA_VERB_PROVIDED_ANALOGY, TA_VERB_REDIRECTED,
};
pub use config::{AgentConfig, DecisionThresholds, RunLimits, SafeDefaults};
pub use decision::{
    is_claim_judgment_call, DecisionEngine, DecisionKind, DecisionRequest, DecisionResult,
    DecisionValue, OperationalDecisionType, ShadowEvaluation,
};
pub use doctor::{run_doctor, CheckResult, DoctorReport};
pub use eval::{run_eval_set, EvalCase, EvalCaseResult, EvalResultSet};
pub use mcp::S3AMcpDispatcher;
pub use model::{
    ChatMessage, CompletionRequest, CompletionResponse, MockModel, Model, ModelError,
    OpenAiHttpAdapter,
};
pub use runtime::{AgentRuntime, RunDiffSummary, RunState, RunStatus, ShadowReportSummary};
pub use sandbox::{Sandbox, SandboxError};
pub use srs::SrsContainerManager;
pub use store::{LocalStore, LrsStore, Store, StoreError};
pub use tools::{
    compute_sha256, CrossrefLookupTool, McpExternalTool, OpenAlexLookupTool,
    RetractionWatchCheckTool, S3aQueryTool, S3aWriteTool, Tool, ToolCallResult, ToolError,
    ToolExecutionContext, ToolInfo, ToolRegistry,
};
pub use trajectory::{
    export_trajectories, is_consent_permitted, run_to_chatml, run_to_sharegpt,
    TrajectoryExportSummary, TrajectoryFormat,
};

/// Initializes structured logging via tracing-subscriber.
pub fn init_logging(verbose: bool) {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = if verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
    };
    let _ = fmt().with_env_filter(filter).with_target(false).try_init();
}

use uuid::Uuid;

/// Generates a real UUIDv7 conforming to RFC 9562.
pub fn generate_uuid_v7() -> Uuid {
    Uuid::now_v7()
}

/// Generates a real UUIDv7 formatted as a canonical hyphenated string.
pub fn generate_uuid_v7_string() -> String {
    Uuid::now_v7().to_string()
}

/// Generates a real UUIDv7 represented as a pair of big-endian u64 integers [high, low].
pub fn generate_uuid_v7_pair() -> [u64; 2] {
    let u = Uuid::now_v7();
    let bytes = u.as_bytes();
    let high = u64::from_be_bytes(bytes[0..8].try_into().unwrap());
    let low = u64::from_be_bytes(bytes[8..16].try_into().unwrap());
    [high, low]
}

/// Converts a 128-bit [u64; 2] pair back to a canonical hyphenated UUID string.
pub fn uuid_pair_to_string(pair: [u64; 2]) -> String {
    let mut bytes = [0u8; 16];
    bytes[0..8].copy_from_slice(&pair[0].to_be_bytes());
    bytes[8..16].copy_from_slice(&pair[1].to_be_bytes());
    Uuid::from_bytes(bytes).to_string()
}

/// Parses a UUID string (hyphenated or 32-char hex) into a [u64; 2] pair.
pub fn string_to_uuid_pair(s: &str) -> Option<[u64; 2]> {
    let u = Uuid::parse_str(s).ok()?;
    let bytes = u.as_bytes();
    let high = u64::from_be_bytes(bytes[0..8].try_into().unwrap());
    let low = u64::from_be_bytes(bytes[8..16].try_into().unwrap());
    Some([high, low])
}

#[cfg(test)]
mod tests {
    use super::*;
    use s3a_core::ADDON_EDU;
    use s3a_engine::S3ACrudEngine;
    use s3a_pack_scholar::SrsWorkspaceRecord;

    struct TempArchive {
        path: std::path::PathBuf,
    }

    impl TempArchive {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let mut path = std::env::temp_dir();
            path.push(format!(
                "s3a_harness_test_{}_{}_{}.s3a",
                std::process::id(),
                ts,
                id
            ));
            Self { path }
        }
    }

    impl Drop for TempArchive {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    #[test]
    fn test_harness_autonomous_research_loop_and_gates() {
        let live_db = TempArchive::new();
        let engine = S3ACrudEngine::open_or_create(&live_db.path).unwrap();
        engine.enable_addon(ADDON_EDU).unwrap();

        let session_uuid = generate_uuid_v7_pair();
        let loop_engine = ResearchLoopEngine::new(session_uuid, 42);

        // 1. Ingest paper
        let paper_coord = loop_engine
            .register_paper(&engine, 0x99998888, [0.2, 0.4, 0.6], 100, 2025, 1, true)
            .unwrap();
        assert_eq!(paper_coord.level, 0);

        // 2. High confidence AI step (no gate)
        assert!(!loop_engine.requires_human_gate(0.85, false));
        let ai_coord = loop_engine
            .record_ai_step(&engine, 1, 0.85, s3a_core::S3ACoordinate::new(0, 0, 0), 555)
            .unwrap();

        // 3. Low confidence AI step (triggers gate)
        assert!(loop_engine.requires_human_gate(0.45, false));

        // 4. Human oversight decision
        let human_coord = loop_engine
            .record_human_decision(&engine, 0x05E8_0001, 1, 1.0, ai_coord, 0x99998888_u32)
            .unwrap();
        assert_eq!(human_coord.level, 0);

        // 5. Epistemic tier mapping
        let tier = EpistemicConfidenceTier::from_score(0.85);
        assert_eq!(tier, EpistemicConfidenceTier::High);
        assert!(tier.epistemic_phrase().contains("Substantial"));
    }

    #[test]
    fn test_harness_mcp_dispatcher_json_rpc() {
        let live_db = TempArchive::new();
        let path_str = live_db.path.to_str().unwrap();

        // 1. Initialize
        let init_req = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#;
        let init_resp = S3AMcpDispatcher::handle_request(init_req, path_str).unwrap();
        assert!(init_resp.contains("s3a-academic-harness"));

        // 2. Tools List
        let list_req = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
        let list_resp = S3AMcpDispatcher::handle_request(list_req, path_str).unwrap();
        assert!(list_resp.contains("s3a_search_library"));
        assert!(list_resp.contains("s3a_get_synthesis_matrix"));
        assert!(list_resp.contains("s3a_export_srs"));
    }

    #[test]
    fn test_harness_srs_packaging_and_mounting() {
        let live_db = TempArchive::new();
        let snap_file = TempArchive::new();

        let engine = S3ACrudEngine::open_or_create(&live_db.path).unwrap();
        engine.enable_addon(ADDON_EDU).unwrap();
        let session_uuid = generate_uuid_v7_pair();
        let loop_engine = ResearchLoopEngine::new(session_uuid, 100);
        let _ = loop_engine
            .register_paper(&engine, 0x12345, [0.1, 0.2, 0.3], 50, 2024, 1, true)
            .unwrap();

        let p_uuid = generate_uuid_v7_pair();
        let s_uuid = generate_uuid_v7_pair();
        let ws = SrsWorkspaceRecord::new(p_uuid, 888, 1, [0.1, 0.2, 0.3], 1);

        // Package SRS snapshot
        let packaged = SrsContainerManager::package_project(
            &live_db.path,
            &snap_file.path,
            p_uuid,
            s_uuid,
            [0, 0],
            Some(&ws),
        )
        .unwrap();
        assert_eq!(packaged.papers.len(), 1);
        assert!(packaged.workspace.is_some());

        // Zero-copy mount
        let mounted = SrsContainerManager::mount_snapshot(&snap_file.path).unwrap();
        assert_eq!(mounted.manifest.project_uuid, p_uuid);
        assert_eq!(mounted.papers.len(), 1);
        assert_eq!(mounted.papers[0].paper_id, 0x12345);
    }

    #[test]
    fn test_teacher_assistant_supervisory_gate() {
        let live_db = TempArchive::new();
        let engine = S3ACrudEngine::open_or_create(&live_db.path).unwrap();
        engine.enable_addon(ADDON_EDU).unwrap();
        engine.enable_addon(s3a_core::ADDON_ONTOLOGY).unwrap();

        let session_uuid = [0xAAAA_BBBB, 0xCCCC_DDDD];
        let loop_engine = ResearchLoopEngine::new(session_uuid, 77);

        // 1. Preflight check: low confidence triggers HallucinationRisk::LowConfidence
        let res_low = loop_engine.preflight_check(0.55, &[], true);
        assert!(matches!(
            res_low,
            Err(HallucinationRisk::LowConfidence { .. })
        ));

        // 2. Preflight check: unvetted analogy triggers HallucinationRisk::UnverifiedAnalogy
        let res_analogy = loop_engine.preflight_check(0.85, &[], false);
        assert!(matches!(
            res_analogy,
            Err(HallucinationRisk::UnverifiedAnalogy { .. })
        ));

        // 3. Preflight check: unbound entity triggers HallucinationRisk::UnboundEntity
        let res_entity = loop_engine.preflight_check(0.95, &[0xDEAD_CAFE], true);
        assert!(matches!(
            res_entity,
            Err(HallucinationRisk::UnboundEntity {
                entity_hash: 0xDEAD_CAFE
            })
        ));

        // 4. Preflight check passes on grounded, high-confidence draft
        assert!(loop_engine.preflight_check(0.92, &[], true).is_ok());

        // 5. Agent halts on unvetted analogy and "Raises Hand to TA"
        let risk = res_analogy.unwrap_err();
        let candidate_draft = "Air molecules freeze into a slush inside the tire.";
        let ta_guidance = loop_engine.raise_hand_to_ta(
            risk,
            "Why does tire pressure drop in cold weather?",
            candidate_draft,
            Some(TaGuidance::ProvideAlternativeAnalogy {
                approved_analogy:
                    "Think of air molecules like bouncing tennis balls losing kinetic energy."
                        .into(),
                pedagogical_edge: s3a_pack_ontology::PREDICATE_HAS_APPROVED_ANALOGY,
            }),
        );

        // 6. TA intervenes with approved analogy
        assert!(matches!(
            ta_guidance,
            TaGuidance::ProvideAlternativeAnalogy { .. }
        ));

        // 7. Atomic dual-actor log: record AI trace (halted) and link to TA decision
        let ai_coord = loop_engine
            .record_ai_step(
                &engine,
                2, // ReasoningStep / AnalogyAttempt
                0.60,
                s3a_core::S3ACoordinate::new(0, 0, 0),
                123,
            )
            .unwrap();

        let ta_id = 0x05E8_7777; // TA user ID
        let ta_coord = loop_engine
            .record_ta_decision(
                &engine,
                ta_id,
                TA_VERB_PROVIDED_ANALOGY,
                1.0, // Approved
                ai_coord,
                0x104, // Concept: IdealGasLaw
            )
            .unwrap();

        assert_eq!(ta_coord.level, 0);
    }

    #[test]
    fn test_agent_allowlist_and_preflight_supervisory_halt() {
        use s3a_pack_scholar::{
            seed_default_agents, AgentDenied, ScholarCrudExt, TOOL_CREDENTIAL_EXPORT,
            TOOL_OPENALEX_QUERY, TOOL_PUBLISH,
        };

        let live_db = TempArchive::new();
        let engine = S3ACrudEngine::open_or_create(&live_db.path).unwrap();
        engine.enable_addon(ADDON_EDU).unwrap();

        let session_uuid = [0x5555_6666_7777_8888, 0x1111_2222_3333_4444];
        let default_agents = seed_default_agents();
        let claim_checker = &default_agents[2]; // claim-checker
        let loop_engine = ResearchLoopEngine::new(session_uuid, claim_checker.agent_id);

        // 1. Agent allowlist enforcement: claim-checker cannot publish
        let allow_err = loop_engine.assert_agent_allows(claim_checker, None, Some(TOOL_PUBLISH));
        assert!(
            matches!(allow_err, Err(AgentDenied::ToolExplicitlyDenied { tool_bit }) if tool_bit == TOOL_PUBLISH)
        );

        let cred_err =
            loop_engine.assert_agent_allows(claim_checker, None, Some(TOOL_CREDENTIAL_EXPORT));
        assert!(
            matches!(cred_err, Err(AgentDenied::ToolExplicitlyDenied { tool_bit }) if tool_bit == TOOL_CREDENTIAL_EXPORT)
        );

        let unalloc_err =
            loop_engine.assert_agent_allows(claim_checker, None, Some(TOOL_OPENALEX_QUERY));
        assert!(
            matches!(unalloc_err, Err(AgentDenied::ToolNotAllowlisted { tool_bit }) if tool_bit == TOOL_OPENALEX_QUERY)
        );

        // Human gate requirement: claim-checker without clearance returns Err(AgentDenied::RequiresHumanGate)
        let gate_err =
            loop_engine.assert_agent_allows_gated(claim_checker, Some(1 << 5), None, false);
        assert!(matches!(gate_err, Err(AgentDenied::RequiresHumanGate)));

        let gate_ok =
            loop_engine.assert_agent_allows_gated(claim_checker, Some(1 << 5), None, true);
        assert!(gate_ok.is_ok());

        // 2. Preflight check fails on unvetted speculation (< 0.70 confidence)
        let preflight_res = loop_engine.preflight_check(0.40, &[], false);
        assert!(preflight_res.is_err());
        let risk = preflight_res.unwrap_err();

        // 3. Emission halted! Agent raises hand to TA
        let ta_guidance = loop_engine.raise_hand_to_ta(
            risk,
            "What causes gravitational time dilation?",
            "Time runs slower because gravity condenses clock gears.",
            Some(TaGuidance::RedirectToExcerpt {
                target_coord: s3a_core::S3ACoordinate::new(0, 12, 4),
            }),
        );
        assert!(matches!(ta_guidance, TaGuidance::RedirectToExcerpt { .. }));

        // 4. Log AI trace and TA decision with shared session_uuid
        let ai_coord = loop_engine
            .record_ai_step(
                &engine,
                0x0006, // VERB_ID_FLAGGED_HALLUCINATION
                0.40,
                s3a_core::S3ACoordinate::new(0, 0, 0),
                789,
            )
            .unwrap();

        let _ta_coord = loop_engine
            .record_ta_decision(
                &engine,
                0x05E8_9999, // TA actor id
                TA_VERB_REDIRECTED,
                1.0,
                ai_coord,
                1204, // object id
            )
            .unwrap();

        // 5. Query session bundle: dual-actor join HumanLrsRecord.session_uuid == AiTraceRecord.session_uuid
        let (humans, ais) = engine.query_session_bundle(session_uuid).unwrap();
        assert_eq!(humans.len(), 1);
        assert_eq!(ais.len(), 1);

        let (human_rec, h_coord) = &humans[0];
        let (ai_rec, a_coord) = &ais[0];

        assert_eq!(human_rec.session_uuid, session_uuid);
        assert_eq!(ai_rec.session_uuid, session_uuid);
        assert_eq!(human_rec.session_uuid, ai_rec.session_uuid);
        assert_eq!(human_rec.verb_id, TA_VERB_REDIRECTED);
        assert_eq!(human_rec.ai_trace_coord, ai_coord);
        assert_eq!(h_coord.level, 0);
        assert_eq!(a_coord.level, 0);
        assert_eq!(h_coord.record_offset, 0);
        assert_eq!(a_coord.record_offset, 0);
    }
}
