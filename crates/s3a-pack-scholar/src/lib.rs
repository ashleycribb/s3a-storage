//! # Scholar Explorer Domain Pack (`s3a-pack-scholar`)
//!
//! Provides educational, academic literature, cognitive provenance, and SRS/LRS semantics
//! as an independent domain pack for S3A Database.
//!
//! Owns Hyper-Tiles 10..=18:
//! - Tile 10: Human Researcher Decision Ledger (`HumanLrsRecord`)
//! - Tile 11: AI Agent Step Trace Log (`AiTraceRecord`)
//! - Tile 12: Academic Paper Corpus (`AcademicPaperRecord`)
//! - Tile 13: Research Knowledge Graph (`ResearchGraphEdgeRecord`)
//! - Tile 14: Scholar Research Workspace Context (`SrsWorkspaceRecord`)
//! - Tile 15: Scholar Research Snapshot Manifest (`SrsManifestHeader`)
//! - Tile 18: Task Agent Registry (`AgentRegistryRecord`)

pub mod committee;
pub mod display_layouts;
pub mod evidence;
pub mod ext;
pub mod ingest;
pub mod pack;
#[cfg(not(target_arch = "wasm32"))]
pub mod recheck;
pub mod records;
pub mod snapshot;
pub mod sync;
pub mod xapi;

pub use committee::{
    committee_comment_hash, committee_reviewer_hash, committee_submission_hash,
    verify_committee_chain, verify_committee_log, CommitteeDecisionRecord, CommitteeError,
    CommitteeVerdict, COMMITTEE_COMMENT_MAX_BYTES, COMMITTEE_SIGNING_DOMAIN,
};
pub use display_layouts::{display_layouts, display_registry};
pub use evidence::{
    build_claim_proof, export_bibliography_with_proof, BibliographyCompanionProof,
    BibliographyProofItem, ClaimProofChain,
};
pub use ext::{ScholarCrudExt, ScholarQueryExt};
pub use ingest::{ingest_scholar_intermediate_dir, ScholarIngestStats};
pub use pack::{register_pack_types, ScholarPack, ScholarTileType};
#[cfg(not(target_arch = "wasm32"))]
pub use recheck::{
    recheck_archive, MockScholarServer, RecheckItem, RecheckOptions, RecheckReport, RecheckStatus,
};
pub use records::{
    compute_keyword_bloom, matches_keyword_bloom, seed_default_agents, AcademicPaperRecord,
    AgentDenied, AgentRegistryRecord, AiTraceRecord, CitationFingerprintRecord, HumanLrsRecord,
    ResearchGraphEdgeRecord, ScholarNoteClaimRecord, SrsManifestHeader, SrsWorkspaceRecord,
    AGENT_FLAG_ACTIVE, AGENT_FLAG_DISABLED, AGENT_FLAG_REQUIRES_HUMAN_GATE,
    AGENT_KIND_CITATION_GENERATOR, AGENT_KIND_CUSTODIAN, AGENT_KIND_ORCHESTRATOR,
    AGENT_KIND_REVIEW_COORDINATOR, AGENT_KIND_VERIFIER, CLAIM_KIND_EMPIRICAL,
    CLAIM_KIND_HYPOTHESIS, CLAIM_KIND_NOTE, CLAIM_KIND_THEORETICAL, CONTENT_KIND_ABSTRACT,
    CONTENT_KIND_HTML, CONTENT_KIND_PDF, SCHOLAR_LAYOUT_VERSION_V1, STEP_CLAIM, STEP_HALLUCINATION,
    STEP_NOTE, STEP_REASONING, STEP_TOOL_CALL, TOOL_CREDENTIAL_EXPORT, TOOL_CROSSREF_QUERY,
    TOOL_EXPORT_SNAPSHOT, TOOL_EXTRACT_CLAIMS, TOOL_MUTATE_GRAPH, TOOL_OPENALEX_QUERY,
    TOOL_PUBLISH, TOOL_READ_FILE, TOOL_SYNTHESIZE_EVIDENCE, TOOL_TA_SUPERVISOR_CALL,
};
pub use snapshot::{
    export_srs_snapshot, intake_srs_snapshot, intake_srs_snapshot_into_live, SrsSnapshotBundle,
};
pub use sync::{SrsDeltaEntry, SrsSyncDeltaHeader, SrsSyncDeltaPacket, SyncError};

/// Registers S3A-QL handlers for the Scholar Explorer domain pack.
pub fn register_ql_handlers() {
    // SIFT HUMAN_LRS and SIFT LEARNING_ACTIVITIES (Tile 10)
    let human_lrs_handler = std::sync::Arc::new(|sift: &s3a_engine::ql::ast::SiftStatement| {
        let reader =
            s3a_engine::MmapReader::open(&sift.from_path).map_err(s3a_engine::ql::QlError::Io)?;

        if reader.addon_mask() & s3a_core::ADDON_EDU == 0 {
            return Err(s3a_engine::ql::QlError::AddonRequired {
                addon_mask: s3a_core::ADDON_EDU,
                name: "scholar / edu",
            });
        }

        let mut min_ts = 0u64;
        let mut max_ts = u64::MAX;
        let mut actor_hash = None;

        for cond in &sift.conditions {
            match cond {
                s3a_engine::ql::ast::Condition::TimeBetween(min, max) => {
                    min_ts = *min;
                    max_ts = *max;
                }
                s3a_engine::ql::ast::Condition::ActorId(a) => {
                    actor_hash = Some(*a);
                }
                _ => {}
            }
        }

        let sieve = s3a_engine::QuerySieve::new(&reader);
        let mut results = sieve.query_human_lrs(None, actor_hash, min_ts, max_ts);
        if let Some(lim) = sift.limit {
            results.truncate(lim);
        }
        let count = results.len();
        Ok(s3a_engine::ql::QueryResult::PackData {
            domain: sift.domain,
            count,
            summary: format!(
                "Found {} human decision/LRS records in '{}'",
                count, sift.from_path
            ),
            json_payload: None,
        })
    });

    s3a_engine::register_pack_sift_handler(
        s3a_engine::ql::ast::SiftDomain::HumanLrs,
        human_lrs_handler.clone(),
    );
    s3a_engine::register_pack_sift_handler(
        s3a_engine::ql::ast::SiftDomain::LearningActivities,
        human_lrs_handler,
    );

    // SIFT AI_TRACES (Tile 11)
    s3a_engine::register_pack_sift_handler(
        s3a_engine::ql::ast::SiftDomain::AiTraces,
        std::sync::Arc::new(|sift| {
            let reader = s3a_engine::MmapReader::open(&sift.from_path)
                .map_err(s3a_engine::ql::QlError::Io)?;

            if reader.addon_mask() & s3a_core::ADDON_EDU == 0 {
                return Err(s3a_engine::ql::QlError::AddonRequired {
                    addon_mask: s3a_core::ADDON_EDU,
                    name: "scholar / edu",
                });
            }

            let mut min_ts = 0u64;
            let mut max_ts = u64::MAX;
            let mut agent_id = None;

            for cond in &sift.conditions {
                match cond {
                    s3a_engine::ql::ast::Condition::TimeBetween(min, max) => {
                        min_ts = *min;
                        max_ts = *max;
                    }
                    s3a_engine::ql::ast::Condition::ActorId(a) => {
                        agent_id = Some(*a);
                    }
                    _ => {}
                }
            }

            let sieve = s3a_engine::QuerySieve::new(&reader);
            let mut results = sieve.query_ai_traces(None, agent_id, min_ts, max_ts);
            if let Some(lim) = sift.limit {
                results.truncate(lim);
            }
            let count = results.len();
            Ok(s3a_engine::ql::QueryResult::PackData {
                domain: s3a_engine::ql::ast::SiftDomain::AiTraces,
                count,
                summary: format!("Found {} AI trace records in '{}'", count, sift.from_path),
                json_payload: None,
            })
        }),
    );

    // SIFT ACADEMIC_PAPERS (Tile 12)
    s3a_engine::register_pack_sift_handler(
        s3a_engine::ql::ast::SiftDomain::AcademicPapers,
        std::sync::Arc::new(|sift| {
            let reader = s3a_engine::MmapReader::open(&sift.from_path)
                .map_err(s3a_engine::ql::QlError::Io)?;

            if reader.addon_mask() & s3a_core::ADDON_EDU == 0 {
                return Err(s3a_engine::ql::QlError::AddonRequired {
                    addon_mask: s3a_core::ADDON_EDU,
                    name: "scholar / edu",
                });
            }

            let sieve = s3a_engine::QuerySieve::new(&reader);
            let mut results = sieve.query_academic_papers_3d(
                [-f32::INFINITY; 3],
                [f32::INFINITY; 3],
                0,
                u16::MAX,
            );
            if let Some(lim) = sift.limit {
                results.truncate(lim);
            }
            let count = results.len();
            Ok(s3a_engine::ql::QueryResult::PackData {
                domain: s3a_engine::ql::ast::SiftDomain::AcademicPapers,
                count,
                summary: format!(
                    "Found {} academic paper records in '{}'",
                    count, sift.from_path
                ),
                json_payload: None,
            })
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use s3a_core::{S3ACoordinate, ADDON_EDU};
    use s3a_engine::{MmapReader, QuerySieve, S3ACrudEngine};

    struct TestTempFile {
        path: std::path::PathBuf,
    }

    impl TestTempFile {
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
                "s3a_pack_scholar_test_{}_{}_{}.s3a",
                std::process::id(),
                ts,
                id
            ));
            Self { path }
        }

        fn path(&self) -> &std::path::Path {
            &self.path
        }
    }

    impl Drop for TestTempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    #[test]
    fn test_dual_trace_human_and_ai_cross_reference() {
        let temp_file = TestTempFile::new();
        let path = temp_file.path();
        let engine = S3ACrudEngine::open_or_create(path).unwrap();

        // Enable Scholar Pack addon mask
        engine.enable_addon(ADDON_EDU).unwrap();

        let session_uuid = [0xAAAA_BBBB_CCCC_DDDD, 0x1111_2222_3333_4444];
        let human_coord = S3ACoordinate::new(0, 0, 0);
        let ai_coord = S3ACoordinate::new(0, 1, 0);

        let human_rec = HumanLrsRecord::new(
            session_uuid,
            1600000000,
            999, // researcher hash
            1,   // 'accepted'
            1.0,
            ai_coord, // pointer to AI trace at tile 1, offset 0
            42,
        );

        let ai_rec = AiTraceRecord::new(
            session_uuid,
            1600000001,
            888, // agent id
            2,   // reasoning step
            0.98,
            human_coord, // pointer to human feedback at tile 0, offset 0
            1234,
        );

        engine.create_human_lrs(&[human_rec]).unwrap();
        engine.create_ai_traces(&[ai_rec]).unwrap();

        // 1. Test bidirectional O(1) dereferencing
        let resolved_ai = engine.trace_ai_from_human(&human_coord).unwrap();
        assert_eq!(resolved_ai.session_uuid, session_uuid);
        assert_eq!(resolved_ai.agent_id, 888);
        assert_eq!(resolved_ai.confidence, 0.98);

        let resolved_human = engine.trace_human_from_ai(&ai_coord).unwrap();
        assert_eq!(resolved_human.session_uuid, session_uuid);
        assert_eq!(resolved_human.actor_hash, 999);
        assert_eq!(resolved_human.decision_score, 1.0);

        // 2. Test session correlation bundle query
        let (humans, ais) = engine.query_session_bundle(session_uuid).unwrap();
        assert_eq!(humans.len(), 1);
        assert_eq!(ais.len(), 1);
        assert_eq!(humans[0].0.actor_hash, 999);
        assert_eq!(ais[0].0.agent_id, 888);
    }

    #[test]
    fn test_srs_snapshot_export_and_intake_roundtrip() {
        let live_db = TestTempFile::new();
        let snapshot_file = TestTempFile::new();

        let engine = S3ACrudEngine::open_or_create(live_db.path()).unwrap();
        engine.enable_addon(ADDON_EDU).unwrap();

        let project_uuid = [0x1122334455667788, 0x99AABBCCDDEEFF00];
        let snapshot_uuid = [0x5555444433332222, 0x11110000FFFFEEEE];
        let session_uuid = [0xCAFEBABE00001111, 0xDEADBEEF22223333];

        // 1. Create Workspace
        let ws = SrsWorkspaceRecord::new(project_uuid, 123456, 0b11, [0.1, 0.2, 0.3], 1);
        engine.create_srs_workspace(&ws).unwrap();

        // 2. Create Paper
        let paper = AcademicPaperRecord::new(
            1001, 1600000000, 5000, 0.1, 0.2, 0.3, 25, 2024, 1, 1, 0, 999,
        );
        engine.create_academic_papers(&[paper]).unwrap();

        // 3. Create Edge
        let edge = ResearchGraphEdgeRecord::new(
            1001, 2002, 1600000001, 5001, 1, // SUPPORTS
            0.95,
        );
        engine.create_research_graph_edges(&[edge]).unwrap();

        // 4. Create Human Decision & AI Trace & Agent Registry
        let human = HumanLrsRecord::new(
            session_uuid,
            1600000002,
            777,
            1,
            1.0,
            S3ACoordinate::new(0, 4, 0),
            1001_u32,
        );
        let ai = AiTraceRecord::new(
            session_uuid,
            1600000003,
            888,
            2,
            0.99,
            S3ACoordinate::new(0, 3, 0),
            5000,
        );
        let default_agents = seed_default_agents();
        let agent = default_agents[0];
        engine.create_human_lrs(&[human]).unwrap();
        engine.create_ai_traces(&[ai]).unwrap();
        engine.create_agent_registry(&[agent]).unwrap();

        // 5. Export Snapshot
        let manifest = SrsManifestHeader::new(
            project_uuid,
            snapshot_uuid,
            [0, 0],
            session_uuid,
            1600000005,
            0,
            0,
        );

        let exported_bundle = engine
            .export_srs_snapshot(
                snapshot_file.path(),
                &manifest,
                Some(&ws),
                Some(project_uuid),
            )
            .unwrap();

        assert_eq!(exported_bundle.papers.len(), 1);
        assert_eq!(exported_bundle.evidence_edges.len(), 1);
        assert_eq!(exported_bundle.human_decisions.len(), 1);
        assert_eq!(exported_bundle.ai_traces.len(), 1);
        assert_eq!(exported_bundle.agents.len(), 1);
        assert_eq!(exported_bundle.agents[0].agent_id, agent.agent_id);
        // 6 payload tiles (14, 12, 13, 10, 11, 18) + 1 manifest tile (15) = 7 tiles
        assert_eq!(exported_bundle.manifest.tile_count, 7);
        assert_eq!(exported_bundle.manifest.total_records, 6);

        // 6. Intake Snapshot and verify zero-copy mmap roundtrip with authoritative Tile 15
        let intaked_bundle = intake_srs_snapshot(snapshot_file.path()).unwrap();
        assert_eq!(intaked_bundle.manifest.magic, SrsManifestHeader::MAGIC);
        assert_eq!(
            intaked_bundle.manifest.schema_version,
            SCHOLAR_LAYOUT_VERSION_V1
        );
        assert!(intaked_bundle.manifest.verify_checksum());
        assert_eq!(intaked_bundle.manifest.project_uuid, project_uuid);
        assert_eq!(intaked_bundle.manifest.snapshot_uuid, snapshot_uuid);
        assert_eq!(intaked_bundle.manifest.tile_count, 7);
        assert_eq!(intaked_bundle.manifest.total_records, 6);

        assert_eq!(intaked_bundle.papers.len(), 1);
        assert_eq!(intaked_bundle.papers[0].paper_id, 1001);
        assert_eq!(intaked_bundle.evidence_edges.len(), 1);
        assert_eq!(intaked_bundle.human_decisions.len(), 1);
        assert_eq!(intaked_bundle.ai_traces.len(), 1);
        assert_eq!(intaked_bundle.ai_traces[0].agent_id, 888);
        assert_eq!(intaked_bundle.agents.len(), 1);
        assert_eq!(intaked_bundle.agents[0].agent_id, agent.agent_id);

        // 7. Verify intake into live database
        let second_live_db = TestTempFile::new();
        let mounted_bundle =
            intake_srs_snapshot_into_live(snapshot_file.path(), second_live_db.path()).unwrap();
        assert_eq!(mounted_bundle.papers.len(), 1);
        assert_eq!(mounted_bundle.agents.len(), 1);

        let reader = MmapReader::open(second_live_db.path()).unwrap();
        assert_eq!(reader.file_header().addon_mask & ADDON_EDU, ADDON_EDU);
        let sieve = QuerySieve::new(&reader);
        let live_papers =
            sieve.query_academic_papers_3d([-f32::INFINITY; 3], [f32::INFINITY; 3], 0, u16::MAX);
        assert_eq!(live_papers.len(), 1);
        assert_eq!(live_papers[0].0.paper_id, 1001);
        let live_agents = sieve.query_agent_registry(None);
        assert_eq!(live_agents.len(), 1);
        assert_eq!(live_agents[0].0.agent_id, agent.agent_id);
    }

    #[test]
    fn test_srs_snapshot_intake_fails_on_corrupted_checksum() {
        let live_db = TestTempFile::new();
        let snapshot_file = TestTempFile::new();

        let engine = S3ACrudEngine::open_or_create(live_db.path()).unwrap();
        engine.enable_addon(ADDON_EDU).unwrap();

        let paper = AcademicPaperRecord::new(99, 100, 101, 1.0, 2.0, 3.0, 10, 2024, 1, 1, 0, 77);
        engine.create_academic_papers(&[paper]).unwrap();

        let manifest = SrsManifestHeader::new([1, 1], [2, 2], [0, 0], [3, 3], 100, 0, 0);
        engine
            .export_srs_snapshot(snapshot_file.path(), &manifest, None, None)
            .unwrap();

        // Mutate a byte in the snapshot payload to invalidate CRC32C
        {
            use std::io::{Seek, Write};
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .open(snapshot_file.path())
                .unwrap();
            // Payload of Tile 0 starts at offset 512 (DualFileHeader) + 512 (tile header) = 1024
            f.seek(std::io::SeekFrom::Start(1024 + 10)).unwrap();
            f.write_all(&[0xFF]).unwrap();
        }

        let res = intake_srs_snapshot(snapshot_file.path());
        assert!(
            res.is_err(),
            "Intake must fail when hardware CRC32C checksum is corrupted"
        );
        assert_eq!(res.unwrap_err().kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn test_srs_snapshot_intake_fails_without_addon_edu() {
        let temp_file = TestTempFile::new();
        // Create standard database without ADDON_EDU
        let _engine = S3ACrudEngine::open_or_create(temp_file.path()).unwrap();

        let res = intake_srs_snapshot(temp_file.path());
        assert!(
            res.is_err(),
            "Intake must fail when ADDON_EDU capability is absent"
        );
        assert_eq!(
            res.unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
    }

    #[test]
    fn test_scholar_ingest_v1_fixture() {
        let fixture_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("testdata")
            .join("scholar_ingest_v1");

        let out_db = TestTempFile::new();
        let stats = ingest_scholar_intermediate_dir(&fixture_dir, out_db.path(), true).unwrap();

        assert_eq!(stats.papers_count, 3);
        assert_eq!(stats.edges_count, 3);
        assert_eq!(stats.human_count, 3);
        assert_eq!(stats.ai_count, 3);
        assert_eq!(stats.workspace_count, 1);
        assert!(stats.total_tiles >= 5);

        // Verify loaded data in S3A
        let reader = MmapReader::open(out_db.path()).unwrap();
        assert_eq!(reader.file_header().addon_mask & ADDON_EDU, ADDON_EDU);

        let sieve = QuerySieve::new(&reader);
        let papers =
            sieve.query_academic_papers_3d([-f32::INFINITY; 3], [f32::INFINITY; 3], 0, u16::MAX);
        assert_eq!(papers.len(), 3);
        assert_eq!(papers[0].0.paper_id, 10001);

        let edges = sieve.query_research_graph(None, None);
        assert_eq!(edges.len(), 3);

        let human = sieve.query_human_lrs(None, None, 0, u64::MAX);
        assert_eq!(human.len(), 3);

        let ai = sieve.query_ai_traces(None, None, 0, u64::MAX);
        assert_eq!(ai.len(), 3);

        let ws = sieve.query_srs_workspace(None);
        assert_eq!(ws.len(), 1);
        assert_eq!(ws[0].0.question_hash, 884422);
    }

    #[test]
    fn test_agent_registry_strict_allow_and_deny_boundaries() {
        let default_agents = seed_default_agents();
        assert_eq!(default_agents.len(), 7);

        // Lab assistant: allowed TOOL_OPENALEX_QUERY, denied TOOL_PUBLISH
        let lab_asst = &default_agents[0];
        assert!(lab_asst.is_active());
        assert!(!lab_asst.requires_human_gate());

        // Allowed action
        assert!(lab_asst
            .assert_allows(Some(1 << 5), Some(TOOL_OPENALEX_QUERY))
            .is_ok());

        // Hard deny: explicit denied tool (TOOL_PUBLISH) returns Err(AgentDenied::ToolExplicitlyDenied)
        let res_denied = lab_asst.assert_allows(None, Some(TOOL_PUBLISH));
        assert!(
            matches!(res_denied, Err(AgentDenied::ToolExplicitlyDenied { tool_bit }) if tool_bit == TOOL_PUBLISH)
        );

        // Hard deny: tool not allowlisted (TOOL_MUTATE_GRAPH) returns Err(AgentDenied::ToolNotAllowlisted)
        let res_unallow = lab_asst.assert_allows(None, Some(TOOL_MUTATE_GRAPH));
        assert!(
            matches!(res_unallow, Err(AgentDenied::ToolNotAllowlisted { tool_bit }) if tool_bit == TOOL_MUTATE_GRAPH)
        );

        // Hard deny: verb not allowlisted
        let res_bad_verb = lab_asst.assert_allows(Some(1 << 20), None);
        assert!(
            matches!(res_bad_verb, Err(AgentDenied::VerbNotAllowlisted { verb_bit }) if verb_bit == (1 << 20))
        );

        // Disabled agent
        let mut disabled_agent = *lab_asst;
        disabled_agent.flags |= AGENT_FLAG_DISABLED;
        disabled_agent.sync_inverted_fields();
        assert!(!disabled_agent.is_active());
        assert!(matches!(
            disabled_agent.assert_allows(None, None),
            Err(AgentDenied::AgentDisabled)
        ));

        // Unsynced mutation triggers CorruptionDetected
        let mut corrupted_agent = *lab_asst;
        corrupted_agent.flags |= AGENT_FLAG_DISABLED; // not synced
        assert_eq!(
            corrupted_agent.assert_allows(None, None),
            Err(AgentDenied::CorruptionDetected)
        );

        // Claim-checker requires human gate
        let claim_checker = &default_agents[2];
        assert!(claim_checker.requires_human_gate());

        // assert_allows_gated: without human clearance, returns Err(AgentDenied::RequiresHumanGate)
        let unapproved = claim_checker.assert_allows_gated(Some(1 << 5), None, false);
        assert!(matches!(unapproved, Err(AgentDenied::RequiresHumanGate)));

        // assert_allows_gated: with human clearance, succeeds if allowlisted
        let approved = claim_checker.assert_allows_gated(Some(1 << 5), None, true);
        assert!(approved.is_ok());
    }

    #[test]
    fn test_xapi_profile_and_verb_mapping() {
        use crate::xapi::*;

        // TA Verbs
        let v_preflight = resolve_verb_by_id(VERB_ID_TA_APPROVED_DRAFT).unwrap();
        assert_eq!(v_preflight.label, "ta-approved-draft");
        assert_eq!(v_preflight.verb_bit, 1 << 0);
        assert_eq!(
            resolve_verb_by_iri("https://ashecribb.dev/xapi/verbs/ta-approved-draft")
                .unwrap()
                .verb_id,
            VERB_ID_TA_APPROVED_DRAFT
        );
        assert_eq!(
            resolve_verb_by_label("ta-approved-draft").unwrap().verb_id,
            VERB_ID_TA_APPROVED_DRAFT
        );

        let v_analogy = resolve_verb_by_id(VERB_ID_TA_PROVIDED_ANALOGY).unwrap();
        assert_eq!(v_analogy.label, "ta-provided-analogy");
        assert_eq!(v_analogy.verb_bit, 1 << 1);

        // Research Verbs
        let v_exp = resolve_verb_by_id(VERB_ID_EXPERIENCED).unwrap();
        assert_eq!(v_exp.label, "experienced");
        assert_eq!(v_exp.verb_bit, 1 << 4);

        let v_flag = resolve_verb_by_id(VERB_ID_FLAGGED_HALLUCINATION).unwrap();
        assert_eq!(v_flag.label, "flagged-hallucination");
        assert_eq!(v_flag.verb_bit, 1 << 9);

        // Hashing
        let h1 = hash_iri(SCHOLAR_XAPI_PROFILE_IRI);
        let h2 = hash_iri(SCHOLAR_XAPI_PROFILE_IRI);
        assert_eq!(h1, h2);
        assert_ne!(h1, 0);
    }

    #[test]
    fn test_marks_snapshot_export_and_query_hide_default() {
        use s3a_core::MarkRecord;

        let live_db = TestTempFile::new();
        let snapshot_file = TestTempFile::new();

        let engine = S3ACrudEngine::open_or_create(live_db.path()).unwrap();
        engine.enable_addon(ADDON_EDU).unwrap();

        let paper1 = AcademicPaperRecord::new(1001, 1000, 10, 1.0, 1.0, 1.0, 0, 2024, 0, 0, 0, 1);
        let paper2 = AcademicPaperRecord::new(1002, 1001, 20, 2.0, 2.0, 2.0, 0, 2024, 0, 0, 0, 2);
        let paper_tile_id = engine.create_academic_papers(&[paper1, paper2]).unwrap();

        let human1 = HumanLrsRecord::new(
            [1, 1],
            1002,
            1,
            2,
            1.0,
            s3a_core::S3ACoordinate::new(0, 0, 0),
            0,
        );
        let human_tile_id = engine.create_human_lrs(&[human1]).unwrap();

        // Initially without marks, all records returned
        let reader0 = MmapReader::open(live_db.path()).unwrap();
        let sieve0 = QuerySieve::new(&reader0);
        assert_eq!(
            sieve0
                .query_academic_papers_3d([-f32::INFINITY; 3], [f32::INFINITY; 3], 0, u16::MAX)
                .len(),
            2
        );
        assert_eq!(sieve0.query_human_lrs(None, None, 0, u64::MAX).len(), 1);
        drop(reader0);

        // Assert marks:
        // 1. Policy Mark: CONSENT_WITHDRAWN on paper2 (index 1)
        let m_consent = MarkRecord::new(
            ScholarTileType::ACADEMIC_PAPERS,
            paper_tile_id,
            1, // record_index 1
            1, // generation
            s3a_core::MARK_CONSENT_WITHDRAWN,
            s3a_core::MARK_SET,
            1005,
            0x1234,
        );
        // 2. Policy Mark: QUARANTINE on human1 (index 0)
        let m_quarantine = MarkRecord::new(
            ScholarTileType::HUMAN_LRS,
            human_tile_id,
            0,
            1,
            s3a_core::MARK_QUARANTINE,
            s3a_core::MARK_SET,
            1006,
            0x1234,
        );
        // 3. Ephemeral Mark: ACCESS_HEAT on paper1 (index 0)
        let m_heat = MarkRecord::new(
            ScholarTileType::ACADEMIC_PAPERS,
            paper_tile_id,
            0,
            1,
            s3a_core::MARK_ACCESS_HEAT,
            s3a_core::MARK_SET,
            1007,
            0x1234,
        );
        engine
            .create_marks(&[m_consent, m_quarantine, m_heat])
            .unwrap();

        // Verify query on live DB hides CONSENT_WITHDRAWN and QUARANTINE records by default
        let live_reader = MmapReader::open(live_db.path()).unwrap();
        let mut live_sieve = QuerySieve::new(&live_reader);
        let default_papers = live_sieve.query_academic_papers_3d(
            [-f32::INFINITY; 3],
            [f32::INFINITY; 3],
            0,
            u16::MAX,
        );
        assert_eq!(default_papers.len(), 1);
        assert_eq!(default_papers[0].0.paper_id, 1001); // paper2 is hidden!

        let default_humans = live_sieve.query_human_lrs(None, None, 0, u64::MAX);
        assert_eq!(default_humans.len(), 0); // human1 is hidden under quarantine!

        // Opt-in reveals them:
        live_sieve.set_include_hidden(true);
        let all_papers = live_sieve.query_academic_papers_3d(
            [-f32::INFINITY; 3],
            [f32::INFINITY; 3],
            0,
            u16::MAX,
        );
        assert_eq!(all_papers.len(), 2);
        let all_humans = live_sieve.query_human_lrs(None, None, 0, u64::MAX);
        assert_eq!(all_humans.len(), 1);
        drop(live_reader);

        // Export SRS snapshot
        let manifest = SrsManifestHeader::new([1, 1], [2, 2], [0, 0], [3, 3], 100, 0, 0);
        let exported = engine
            .export_srs_snapshot(snapshot_file.path(), &manifest, None, None)
            .unwrap();

        // Snapshot export keeps policy marks (CONSENT_WITHDRAWN, QUARANTINE) and drops ACCESS_HEAT
        assert_eq!(exported.policy_marks.len(), 2);
        assert!(exported
            .policy_marks
            .iter()
            .any(|m| m.mark_kind == s3a_core::MARK_CONSENT_WITHDRAWN));
        assert!(exported
            .policy_marks
            .iter()
            .any(|m| m.mark_kind == s3a_core::MARK_QUARANTINE));
        assert!(!exported
            .policy_marks
            .iter()
            .any(|m| m.mark_kind == s3a_core::MARK_ACCESS_HEAT));

        // Intake SRS snapshot
        let intaked = intake_srs_snapshot(snapshot_file.path()).unwrap();
        assert_eq!(intaked.policy_marks.len(), 2);

        // Verify queries on snapshot container also hide marked records by default
        let snap_reader = MmapReader::open(snapshot_file.path()).unwrap();
        let mut snap_sieve = QuerySieve::new(&snap_reader);
        assert_eq!(
            snap_sieve
                .query_academic_papers_3d([-f32::INFINITY; 3], [f32::INFINITY; 3], 0, u16::MAX)
                .len(),
            1
        );
        assert_eq!(snap_sieve.query_human_lrs(None, None, 0, u64::MAX).len(), 0);

        snap_sieve.set_include_hidden(true);
        assert_eq!(
            snap_sieve
                .query_academic_papers_3d([-f32::INFINITY; 3], [f32::INFINITY; 3], 0, u16::MAX)
                .len(),
            2
        );
        assert_eq!(snap_sieve.query_human_lrs(None, None, 0, u64::MAX).len(), 1);
    }

    #[test]
    fn test_pre_change_tile18_fixture_allows_permitted_actions() {
        use s3a_engine::TileWriter;

        let fixture_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("testdata")
            .join("pre_change_tile18_fixture.s3a");

        // Ensure fixture exists on disk with an unstamped legacy v1 Tile 18 record
        if !fixture_path.exists() {
            let legacy_agent = AgentRegistryRecord::new_legacy_unstamped(
                0xFEED_C0DE_1234,
                0x1111_2222,
                0x3333_4444,
                0xFFFF,
                TOOL_READ_FILE | TOOL_OPENALEX_QUERY | TOOL_EXTRACT_CLAIMS,
                TOOL_PUBLISH | TOOL_CREDENTIAL_EXPORT,
                AGENT_KIND_VERIFIER,
                AGENT_FLAG_ACTIVE,
                1,
            );
            assert_eq!(legacy_agent._reserved, [0u8; 16]);

            let mut writer = TileWriter::create(&fixture_path).unwrap();
            writer.enable_addon(ADDON_EDU).unwrap();
            writer
                .write_hyper_tile(ScholarTileType::AGENT_REGISTRY, &[legacy_agent], None, None)
                .unwrap();
            drop(writer);
        }

        // 1. Load directly from the pre-change fixture archive
        let reader = MmapReader::open(&fixture_path).unwrap();
        let sieve = QuerySieve::new(&reader);
        let agents = sieve.query_agent_registry(None);
        assert_eq!(agents.len(), 1);

        let (loaded_agent, _) = agents[0];
        // Confirm raw fixture record is unstamped legacy v1
        assert!(loaded_agent.is_legacy_unstamped());
        assert_eq!(loaded_agent._reserved, [0u8; 16]);

        // Verify permitted actions are allowed without inverted check failure
        assert!(loaded_agent
            .assert_allows(None, Some(TOOL_OPENALEX_QUERY))
            .is_ok());
        assert!(loaded_agent
            .assert_allows(None, Some(TOOL_READ_FILE))
            .is_ok());

        // Verify explicitly denied actions remain denied
        assert_eq!(
            loaded_agent.assert_allows(None, Some(TOOL_PUBLISH)),
            Err(AgentDenied::ToolExplicitlyDenied {
                tool_bit: TOOL_PUBLISH
            })
        );
        assert_eq!(
            loaded_agent.assert_allows(None, Some(TOOL_CREDENTIAL_EXPORT)),
            Err(AgentDenied::ToolExplicitlyDenied {
                tool_bit: TOOL_CREDENTIAL_EXPORT
            })
        );

        // Verify unallowlisted tools remain denied
        assert_eq!(
            loaded_agent.assert_allows(None, Some(TOOL_MUTATE_GRAPH)),
            Err(AgentDenied::ToolNotAllowlisted {
                tool_bit: TOOL_MUTATE_GRAPH
            })
        );
    }
}
