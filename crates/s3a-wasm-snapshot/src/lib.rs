//! S3A Additive In-Memory WebAssembly Snapshot Engine.
//!
//! Provides browser-safe zero-copy snapshot ingestion, query, pre-flight TA evaluation,
//! and export for the Scholar Explorer web application and AI agents.
//!
//! Conforms strictly to the Non-Core Additive Principle:
//! Does not pollute s3a-core or s3a-engine with browser/WASM-specific constructs.

pub mod agent_api;
pub mod bindgen;
pub mod snapshot_view;

pub use agent_api::WasmAgentSnapshot;
pub use bindgen::{
    export_bytes, from_bytes, iri_for_verb_id, new_session, verb_id_for_iri,
    TOOL_CREDENTIAL_EXPORT, TOOL_CROSSREF_QUERY, TOOL_EXPORT_SNAPSHOT, TOOL_EXTRACT_CLAIMS,
    TOOL_MUTATE_GRAPH, TOOL_OPENALEX_QUERY, TOOL_PUBLISH, TOOL_READ_FILE, TOOL_SYNTHESIZE_EVIDENCE,
    TOOL_TA_SUPERVISOR_CALL,
};
pub use snapshot_view::WasmSnapshotView;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn init_panic_hook() {
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::set_once();
}

#[cfg(test)]
mod tests {
    use super::*;
    use s3a_core::{
        MarkRecord, S3ACoordinate, TileType, MARK_CLEAR, MARK_CONSENT_WITHDRAWN, MARK_LEGAL_HOLD,
        MARK_QUARANTINE, MARK_SET,
    };
    use s3a_pack_ontology::{
        OntologyAxiomRecord, OntologyConceptRecord, KIND_CLASS, ONTOLOGY_PEDAGOGY,
        PREDICATE_HAS_APPROVED_ANALOGY,
    };
    use s3a_pack_scholar::{
        seed_default_agents, AcademicPaperRecord, AgentDenied, AiTraceRecord, HumanLrsRecord,
        ResearchGraphEdgeRecord, ScholarTileType, SrsDeltaEntry, SrsManifestHeader,
        SrsWorkspaceRecord, TOOL_OPENALEX_QUERY, TOOL_PUBLISH,
    };

    #[test]
    fn test_wasm_snapshot_in_memory_roundtrip_and_agent_check() {
        // 1. Construct a mock snapshot in memory
        let p_uuid = [0x1111_2222, 0x3333_4444];
        let s_uuid = [0x5555_6666, 0x7777_8888];
        let manifest = SrsManifestHeader::new(p_uuid, s_uuid, [0, 0], [0, 0], 1600000000, 9, 6);

        let ws = SrsWorkspaceRecord::new(p_uuid, 888, 1, [0.1, 0.2, 0.3], 1);
        let paper =
            AcademicPaperRecord::new(0x1234, 1600000000, 50, 0.1, 0.2, 0.3, 10, 2024, 1, 1, 0, 0);
        let edge = ResearchGraphEdgeRecord::new(0x1234, 0x5678, 1600000000, 0, 1, 1.0);
        let human = HumanLrsRecord::new(
            [0xAAAA, 0xBBBB],
            1600000000,
            0x05E8,
            0x00E2,
            1.0,
            S3ACoordinate::new(0, 0, 0),
            0x104,
        );
        let ai = AiTraceRecord::new(
            [0xAAAA, 0xBBBB],
            1600000000,
            42,
            1,
            0.65,
            S3ACoordinate::new(0, 0, 0),
            10,
        );

        let default_agents = seed_default_agents();
        let agent = default_agents[0]; // lab-assistant
        let claim_checker = default_agents[2]; // claim-checker (REQUIRES_HUMAN_GATE)

        let concept_id = 0x104; // IdealGasLaw
        let analogy_id = 0x999; // BouncingTennisBalls
        let concept =
            OntologyConceptRecord::new(concept_id, 0, ONTOLOGY_PEDAGOGY, KIND_CLASS, 1, 200);
        let axiom = OntologyAxiomRecord::new(
            concept_id,
            analogy_id,
            PREDICATE_HAS_APPROVED_ANALOGY,
            2,
            0x3C00,
            10,
            s3a_core::ADDON_ONTOLOGY as u16,
        );

        let agent_snap = WasmAgentSnapshot {
            manifest,
            workspace: Some(ws),
            papers: vec![paper],
            edges: vec![edge],
            human_decisions: vec![human],
            ai_traces: vec![ai],
            agents: vec![agent, claim_checker],
            concepts: vec![concept],
            axioms: vec![axiom],
            claims: Vec::new(),
            citation_fingerprints: Vec::new(),
            policy_marks: Vec::new(),
            pending_delta: Vec::new(),
            delta_seq: 1,
            last_applied_delta_seq: 0,
            statement_log: Vec::new(),
        };

        // 2. Export to byte buffer (simulates web app packaging)
        let bytes = agent_snap.export_bytes().unwrap();
        assert!(bytes.len() >= s3a_core::FILE_HEADER_SIZE);

        // 3. Read back from byte buffer using WasmSnapshotView
        let view = WasmSnapshotView::parse(&bytes).unwrap();
        assert_eq!(view.papers.len(), 1);
        assert_eq!(view.papers[0].paper_id, 0x1234);
        assert_eq!(view.agents.len(), 2);
        assert_eq!(view.agents[0].agent_id, agent.agent_id);
        assert_eq!(view.concepts.len(), 1);
        assert_eq!(view.axioms.len(), 1);

        // 4. Reconstruct WasmAgentSnapshot from bytes
        let loaded_agent_snap = WasmAgentSnapshot::from_bytes(&bytes).unwrap();
        assert_eq!(loaded_agent_snap.manifest.snapshot_uuid, s_uuid);
        assert_eq!(loaded_agent_snap.human_decisions.len(), 1);
        assert_eq!(loaded_agent_snap.agents.len(), 2);
        assert_eq!(loaded_agent_snap.agents[0].agent_id, agent.agent_id);

        // 5. Test Agent Allowlist Enforcement inside WASM
        assert!(loaded_agent_snap
            .assert_agent_allows(agent.agent_id, Some(1 << 5), Some(TOOL_OPENALEX_QUERY))
            .is_ok());
        let denied =
            loaded_agent_snap.assert_agent_allows(agent.agent_id, None, Some(TOOL_PUBLISH));
        assert!(matches!(
            denied,
            Err(AgentDenied::ToolExplicitlyDenied { .. })
        ));

        // Test Gated Action: claim-checker requires human gate approval
        let unapproved = loaded_agent_snap.assert_agent_allows_gated(
            claim_checker.agent_id,
            Some(1 << 5),
            None,
            false,
        );
        assert!(matches!(unapproved, Err(AgentDenied::RequiresHumanGate)));

        let approved = loaded_agent_snap.assert_agent_allows_gated(
            claim_checker.agent_id,
            Some(1 << 5),
            None,
            true,
        );
        assert!(approved.is_ok());

        // 6. Test Preflight Agent Checks inside WASM:
        // Case A: Low confidence (< 0.70) triggers Raise Hand to TA
        let check_low = loaded_agent_snap.preflight_agent_check(0.50, concept_id, true);
        assert!(check_low.is_err());
        assert!(check_low.unwrap_err().contains("RAISE_HAND_TO_TA"));

        // Case B: Unapproved analogy triggers Raise Hand to TA
        let check_unapproved = loaded_agent_snap.preflight_agent_check(0.85, 0x999999, false);
        assert!(check_unapproved.is_err());
        assert!(check_unapproved.unwrap_err().contains("RAISE_HAND_TO_TA"));

        // Case C: Grounded concept with approved analogy passes
        let check_ok = loaded_agent_snap.preflight_agent_check(0.90, concept_id, false);
        assert!(check_ok.is_ok());

        // 7. Test finding approved analogies
        let analogies = loaded_agent_snap.find_approved_analogies(concept_id);
        assert_eq!(analogies.len(), 1);
        assert_eq!(analogies[0].object_concept_id, analogy_id);
    }

    #[test]
    fn test_wasm_delta_incremental_sync() {
        let mut snap1 = WasmAgentSnapshot::from_bytes(
            &WasmAgentSnapshot {
                manifest: SrsManifestHeader::new([1, 2], [3, 4], [0, 0], [1, 2], 1600000000, 1, 0),
                workspace: None,
                papers: Vec::new(),
                edges: Vec::new(),
                human_decisions: Vec::new(),
                ai_traces: Vec::new(),
                agents: Vec::new(),
                concepts: Vec::new(),
                axioms: Vec::new(),
                claims: Vec::new(),
                citation_fingerprints: Vec::new(),
                policy_marks: Vec::new(),
                pending_delta: Vec::new(),
                delta_seq: 1,
                last_applied_delta_seq: 0,
                statement_log: Vec::new(),
            }
            .export_bytes()
            .unwrap(),
        )
        .unwrap();

        // Mutate snap1: add a claim and an AI trace
        snap1.add_claim(
            101,
            0xCAFE,
            0.95,
            s3a_pack_scholar::CLAIM_KIND_EMPIRICAL,
            "Evidence for room temperature superconductor refuted",
            None,
        );
        // Pushes AI trace + Note Claim
        assert_eq!(snap1.pending_delta.len(), 2);

        // Export incremental delta: compact byte packet
        let delta_bytes = snap1.export_delta_bytes();
        assert_eq!(snap1.pending_delta.len(), 0);

        // Apply delta to snap2
        let mut snap2 = WasmAgentSnapshot {
            manifest: SrsManifestHeader::new([1, 2], [3, 4], [0, 0], [1, 2], 1600000000, 1, 0),
            workspace: None,
            papers: Vec::new(),
            edges: Vec::new(),
            human_decisions: Vec::new(),
            ai_traces: Vec::new(),
            agents: Vec::new(),
            concepts: Vec::new(),
            axioms: Vec::new(),
            claims: Vec::new(),
            citation_fingerprints: Vec::new(),
            policy_marks: Vec::new(),
            pending_delta: Vec::new(),
            delta_seq: 1,
            last_applied_delta_seq: 0,
            statement_log: Vec::new(),
        };
        assert_eq!(snap2.ai_traces.len(), 0);

        let applied = snap2.apply_delta_bytes(&delta_bytes).expect("apply delta");
        assert_eq!(applied, 2);
        assert_eq!(snap2.ai_traces.len(), 1);
        assert_eq!(snap2.ai_traces[0].agent_id, 0xCAFE);
        assert_eq!(snap2.ai_traces[0].confidence, 0.95);
        assert_eq!(snap2.claims.len(), 1);
        assert_eq!(snap2.claims[0].record.claim_id, 101);
    }

    #[test]
    fn test_wasm_claims_roundtrip_persistence_and_search() {
        let mut snap = WasmAgentSnapshot {
            manifest: SrsManifestHeader::new([1, 2], [3, 4], [0, 0], [1, 2], 1600000000, 1, 0),
            workspace: None,
            papers: Vec::new(),
            edges: Vec::new(),
            human_decisions: Vec::new(),
            ai_traces: Vec::new(),
            agents: Vec::new(),
            concepts: Vec::new(),
            axioms: Vec::new(),
            claims: Vec::new(),
            citation_fingerprints: Vec::new(),
            policy_marks: Vec::new(),
            pending_delta: Vec::new(),
            delta_seq: 1,
            last_applied_delta_seq: 0,
            statement_log: Vec::new(),
        };

        // Claim 1: Quantum physics with vector [1.0, 0.0, 0.0, 0.0]
        snap.add_claim(
            1,
            0x111,
            0.95,
            s3a_pack_scholar::CLAIM_KIND_EMPIRICAL,
            "Majorana zero modes verified in topological superconducting nanowires",
            Some(&[1.0, 0.0, 0.0, 0.0]),
        );

        // Claim 2: Deep Learning with vector [0.0, 1.0, 0.0, 0.0]
        snap.add_claim(
            2,
            0x222,
            0.88,
            s3a_pack_scholar::CLAIM_KIND_THEORETICAL,
            "Transformer attention heads exhibit sparsity under linear projection",
            Some(&[0.0, 1.0, 0.0, 0.0]),
        );

        // Export snapshot to bytes and reload from bytes
        let bytes = snap.export_bytes().unwrap();
        let loaded = WasmAgentSnapshot::from_bytes(&bytes).expect("deserialization");

        assert_eq!(loaded.claims.len(), 2);
        assert_eq!(loaded.claims[0].record.claim_id, 1);
        assert_eq!(
            loaded.claims[0].text,
            "Majorana zero modes verified in topological superconducting nanowires"
        );
        assert!(loaded.claims[0].embedding.is_some());
        assert_eq!(loaded.claims[0].embedding.as_ref().unwrap()[0], 1.0);

        // 1. Text Search (Bloom + Lexical)
        let text_results = loaded.search_claims_text("Majorana nanowires", 5, false);
        assert_eq!(text_results.len(), 1);
        assert_eq!(text_results[0].claim_id, 1);
        assert_eq!(text_results[0].score, 1.0);

        let dl_results = loaded.search_claims_text("Transformer sparsity", 5, false);
        assert_eq!(dl_results.len(), 1);
        assert_eq!(dl_results[0].claim_id, 2);

        let empty_results = loaded.search_claims_text("unrelated biology topic", 5, false);
        assert_eq!(empty_results.len(), 0);

        // 2. Vector Search (Cosine Similarity)
        let query_vec = [0.95, 0.05, 0.0, 0.0];
        let vec_results = loaded.search_claims_vector(&query_vec, 5, 0.5, false);
        assert_eq!(vec_results.len(), 1);
        assert_eq!(vec_results[0].claim_id, 1);
        assert!(vec_results[0].score > 0.9);

        // 3. Hybrid Search
        let hybrid_results =
            loaded.search_claims_hybrid("nanowires", Some(&query_vec), 0.5, 0.5, 5, false);
        assert!(!hybrid_results.is_empty());
        assert_eq!(hybrid_results[0].claim_id, 1);
        assert!(hybrid_results[0].score > 0.8);
    }

    #[test]
    fn test_wasm_claims_search_mark_filtering() {
        let mut snap = WasmAgentSnapshot {
            manifest: SrsManifestHeader::new([1, 2], [3, 4], [0, 0], [1, 2], 1600000000, 1, 0),
            workspace: None,
            papers: Vec::new(),
            edges: Vec::new(),
            human_decisions: Vec::new(),
            ai_traces: Vec::new(),
            agents: Vec::new(),
            concepts: Vec::new(),
            axioms: Vec::new(),
            claims: Vec::new(),
            citation_fingerprints: Vec::new(),
            policy_marks: Vec::new(),
            pending_delta: Vec::new(),
            delta_seq: 1,
            last_applied_delta_seq: 0,
            statement_log: Vec::new(),
        };

        snap.add_claim(
            1,
            0x111,
            0.95,
            s3a_pack_scholar::CLAIM_KIND_EMPIRICAL,
            "Majorana zero modes verified in topological superconducting nanowires",
            Some(&[1.0, 0.0, 0.0, 0.0]),
        );

        // Baseline: visible
        assert_eq!(snap.search_claims_text("Majorana", 5, false).len(), 1);

        // Mark claim 1 with QUARANTINE
        let quarantine_mark = MarkRecord::new(
            ScholarTileType::NOTE_CLAIMS,
            1, // target_tile_id matches claim_id
            0,
            1,
            MARK_QUARANTINE,
            MARK_SET,
            1600000010,
            0xAD01,
        );
        snap.policy_marks.push(quarantine_mark);

        // Now hidden by default
        assert_eq!(snap.search_claims_text("Majorana", 5, false).len(), 0);
        // Visible with opt-in
        assert_eq!(snap.search_claims_text("Majorana", 5, true).len(), 1);

        let query_vec = [1.0, 0.0, 0.0, 0.0];
        assert_eq!(
            snap.search_claims_vector(&query_vec, 5, 0.5, false).len(),
            0
        );
        assert_eq!(snap.search_claims_vector(&query_vec, 5, 0.5, true).len(), 1);

        // Export and reload: verify quarantine mark is preserved and filtering holds
        let bytes = snap.export_bytes().unwrap();
        let loaded = WasmAgentSnapshot::from_bytes(&bytes).expect("reload");
        assert_eq!(loaded.search_claims_text("Majorana", 5, false).len(), 0);
        assert_eq!(loaded.search_claims_text("Majorana", 5, true).len(), 1);
    }

    #[test]
    fn test_wasm_delta_security_and_idempotency() {
        let mut snap = WasmAgentSnapshot {
            manifest: SrsManifestHeader::new([1, 2], [3, 4], [0, 0], [1, 2], 1600000000, 1, 0),
            workspace: None,
            papers: Vec::new(),
            edges: Vec::new(),
            human_decisions: Vec::new(),
            ai_traces: Vec::new(),
            agents: Vec::new(),
            concepts: Vec::new(),
            axioms: Vec::new(),
            claims: Vec::new(),
            citation_fingerprints: Vec::new(),
            policy_marks: Vec::new(),
            pending_delta: Vec::new(),
            delta_seq: 1,
            last_applied_delta_seq: 0,
            statement_log: Vec::new(),
        };

        // 1. Delta with AGENT_REGISTRY is rejected fail-closed
        let mut packet = s3a_pack_scholar::SrsSyncDeltaPacket::new([1, 2], [3, 4], 1);
        let default_agents = seed_default_agents();
        packet.entries.push(
            SrsDeltaEntry::from_record(ScholarTileType::AGENT_REGISTRY, &default_agents[0])
                .unwrap(),
        );
        let res = snap.apply_delta_bytes(&packet.export_bytes());
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("ForbiddenTileInDelta"));

        // 2. Delta attempting to clear LEGAL_HOLD is rejected
        let mut mark_packet = s3a_pack_scholar::SrsSyncDeltaPacket::new([1, 2], [3, 4], 1);
        let illegal_clear = MarkRecord::new(
            ScholarTileType::NOTE_CLAIMS,
            1,
            0,
            1,
            MARK_LEGAL_HOLD,
            MARK_CLEAR,
            1600000000,
            0xBAD,
        );
        mark_packet
            .entries
            .push(SrsDeltaEntry::from_record(TileType::MARKS, &illegal_clear).unwrap());
        let mark_res = snap.apply_delta_bytes(&mark_packet.export_bytes());
        assert!(mark_res.is_err());
        assert!(mark_res.unwrap_err().contains("UnauthorizedMark"));

        // 3. Valid delta applied successfully
        let mut valid_packet = s3a_pack_scholar::SrsSyncDeltaPacket::new([1, 2], [3, 4], 1);
        let consent_mark = MarkRecord::new(
            ScholarTileType::NOTE_CLAIMS,
            1,
            0,
            1,
            MARK_CONSENT_WITHDRAWN,
            MARK_SET,
            1600000000,
            0xBEEF,
        );
        valid_packet
            .entries
            .push(SrsDeltaEntry::from_record(TileType::MARKS, &consent_mark).unwrap());
        let valid_bytes = valid_packet.export_bytes();
        let ok_res = snap.apply_delta_bytes(&valid_bytes).unwrap();
        assert_eq!(ok_res, 1);
        assert_eq!(snap.last_applied_delta_seq, 1);

        // 4. Replay of same delta is an idempotent no-op (Ok(0))
        let replay_res = snap.apply_delta_bytes(&valid_bytes).unwrap();
        assert_eq!(replay_res, 0);

        // 5. Sequence gap is rejected
        let gap_packet = s3a_pack_scholar::SrsSyncDeltaPacket::new([1, 2], [3, 4], 3); // expected 2
        let gap_res = snap.apply_delta_bytes(&gap_packet.export_bytes());
        assert!(gap_res.is_err());
        assert!(gap_res.unwrap_err().contains("SequenceGap"));
    }

    #[test]
    fn test_wasm_verb_registry_roundtrip() {
        // Round-trip tests for IDs 1..=21
        for id in 1..=21 {
            let iri =
                iri_for_verb_id(id).unwrap_or_else(|| panic!("IRI must exist for verb id {}", id));
            let resolved_id = verb_id_for_iri(&iri)
                .unwrap_or_else(|| panic!("Verb ID must resolve for IRI {}", iri));
            assert_eq!(resolved_id, id, "Round-trip mismatch for verb id {}", id);
        }

        // Test alias mappings resolve correctly
        assert_eq!(
            verb_id_for_iri("https://ashecribb.dev/xapi/verbs/created"),
            Some(15)
        );
        assert_eq!(
            verb_id_for_iri("http://activitystrea.ms/schema/1.0/create"),
            Some(15)
        );
        assert_eq!(
            verb_id_for_iri("https://ashecribb.dev/xapi/verbs/asked"),
            Some(17)
        );
        assert_eq!(
            verb_id_for_iri("http://adlnet.gov/expapi/verbs/asked"),
            Some(17)
        );
        assert_eq!(
            verb_id_for_iri("https://ashecribb.dev/xapi/verbs/logged-out"),
            Some(20)
        );
        assert_eq!(
            verb_id_for_iri("http://adlnet.gov/expapi/verbs/logged-out"),
            Some(20)
        );

        // Test label resolution
        assert_eq!(verb_id_for_iri("searched"), Some(10));
        assert_eq!(verb_id_for_iri("opened"), Some(11));
        assert_eq!(verb_id_for_iri("viewed"), Some(12));
        assert_eq!(verb_id_for_iri("saved"), Some(13));
        assert_eq!(verb_id_for_iri("removed"), Some(14));
        assert_eq!(verb_id_for_iri("created"), Some(15));
        assert_eq!(verb_id_for_iri("edited"), Some(16));
        assert_eq!(verb_id_for_iri("asked"), Some(17));
        assert_eq!(verb_id_for_iri("ai-answered"), Some(18));
        assert_eq!(verb_id_for_iri("accepted"), Some(19));
        assert_eq!(verb_id_for_iri("logged-out"), Some(20));
        assert_eq!(verb_id_for_iri("consent-declined"), Some(21));
    }

    #[test]
    fn test_record_ai_rejects_judgment_verbs() {
        let mut session =
            crate::bindgen::WasmAgentSnapshot::new_session("0123456789abcdef0123456789abcdef")
                .unwrap();

        // 1. record_ai must reject judgment verbs: approved (3), rejected (4), reviewed (2)
        let judgment_verbs = [
            s3a_pack_scholar::xapi::VERB_ID_APPROVED,
            s3a_pack_scholar::xapi::VERB_ID_REJECTED,
            s3a_pack_scholar::xapi::VERB_ID_REVIEWED,
        ];

        for &verb in &judgment_verbs {
            let res = session.record_ai("lab-assistant", verb, 0.95, None);
            assert!(
                res.is_err(),
                "record_ai should reject judgment verb {}",
                verb
            );
        }

        // 2. assert_agent_allows must reject judgment verbs for AI agents
        for &verb in &judgment_verbs {
            let res = session.assert_agent_allows("lab-assistant", Some(verb), None);
            assert!(
                res.is_err(),
                "assert_agent_allows should reject judgment verb {}",
                verb
            );
        }

        // 3. record_ai accepts valid non-judgment verbs (e.g. ai-answered: 18, experienced: 1)
        let ok_res = session.record_ai(
            "lab-assistant",
            s3a_pack_scholar::xapi::VERB_ID_AI_ANSWERED,
            0.9,
            None,
        );
        assert!(ok_res.is_ok(), "record_ai should accept ai-answered");

        // 4. record_human rejects ai-answered (18) because it is AI-only
        let human_res = session.record_human(
            s3a_pack_scholar::xapi::VERB_ID_AI_ANSWERED,
            wasm_bindgen::JsValue::NULL,
            0,
            None,
        );
        assert!(human_res.is_err(), "record_human should reject ai-answered");

        // 5. record_human accepts judgment verbs (e.g. approved: 3, reviewed: 2, rejected: 4)
        for &verb in &judgment_verbs {
            let res = session.record_human(verb, wasm_bindgen::JsValue::NULL, 0, None);
            assert!(
                res.is_ok(),
                "record_human should accept judgment verb {}",
                verb
            );
        }
    }

    #[test]
    fn test_reproduce_export_from_bytes_roundtrip() {
        let mut session =
            crate::bindgen::WasmAgentSnapshot::new_session("0123456789abcdef0123456789abcdef")
                .unwrap();

        // Record a few human + AI steps (including new verbs 10..=21 and ai_trace_index links)
        let ai_idx1 = session
            .record_ai(
                "lab-assistant",
                s3a_pack_scholar::xapi::VERB_ID_AI_ANSWERED,
                0.95,
                None,
            )
            .unwrap();
        let human_idx1 = session
            .record_human(
                s3a_pack_scholar::xapi::VERB_ID_SEARCHED, // 10
                wasm_bindgen::JsValue::NULL,
                12345,
                Some(ai_idx1),
            )
            .unwrap();

        let ai_idx2 = session
            .record_ai(
                "lab-assistant",
                s3a_pack_scholar::xapi::VERB_ID_CREATED,
                0.88,
                Some(human_idx1),
            )
            .unwrap();
        let _human_idx2 = session
            .record_human(
                s3a_pack_scholar::xapi::VERB_ID_ACCEPTED, // 19
                wasm_bindgen::JsValue::NULL,
                67890,
                Some(ai_idx2),
            )
            .unwrap();

        // Export bytes
        let bytes1 = session.export_bytes().unwrap();
        println!("Export 1 bytes len: {}", bytes1.len());

        // Test exporting twice in a row with no changes
        let bytes1_again = session.export_bytes().unwrap();
        assert_eq!(bytes1.len(), bytes1_again.len());

        // from_bytes
        let mut restored = crate::bindgen::WasmAgentSnapshot::from_bytes(&bytes1)
            .expect("from_bytes should succeed");

        // Export immediately from restored (test that exports twice in a row with no changes / before new calls)
        let bytes_restored_1 = restored.export_bytes().unwrap();
        println!(
            "Export restored immediately len: {}",
            bytes_restored_1.len()
        );

        // Record more steps
        let ai_idx3 = restored
            .record_ai(
                "lab-assistant",
                s3a_pack_scholar::xapi::VERB_ID_AI_ANSWERED,
                0.92,
                None,
            )
            .unwrap();
        let _human_idx3 = restored
            .record_human(
                s3a_pack_scholar::xapi::VERB_ID_EDITED, // 16
                wasm_bindgen::JsValue::NULL,
                11111,
                Some(ai_idx3),
            )
            .unwrap();

        // Export bytes again
        let bytes2 = restored.export_bytes().unwrap();
        println!("Export 2 bytes len: {}", bytes2.len());
        assert!(bytes2.len() >= bytes1.len());
    }

    #[test]
    fn test_export_since_deltas() {
        let mut session =
            crate::bindgen::WasmAgentSnapshot::new_session("0123456789abcdef0123456789abcdef")
                .unwrap();

        // Record 5 statements: 3 AI traces, 2 Human decisions
        let a0 = session
            .record_ai(
                "lab-assistant",
                s3a_pack_scholar::xapi::VERB_ID_AI_ANSWERED,
                0.95,
                None,
            )
            .unwrap();
        let h0 = session
            .record_human(
                s3a_pack_scholar::xapi::VERB_ID_SEARCHED,
                wasm_bindgen::JsValue::NULL,
                100,
                Some(a0),
            )
            .unwrap();
        let a1 = session
            .record_ai(
                "lab-assistant",
                s3a_pack_scholar::xapi::VERB_ID_CREATED,
                0.90,
                Some(h0),
            )
            .unwrap();
        let h1 = session
            .record_human(
                s3a_pack_scholar::xapi::VERB_ID_ACCEPTED,
                wasm_bindgen::JsValue::NULL,
                200,
                Some(a1),
            )
            .unwrap();
        let _a2 = session
            .record_ai(
                "lab-assistant",
                s3a_pack_scholar::xapi::VERB_ID_AI_ANSWERED,
                0.88,
                Some(h1),
            )
            .unwrap();

        // Full delta (cursor = 0): should contain all 5 statements
        let full = session.export_since(0).expect("full delta should succeed");
        assert_eq!(full.record_count(), 5);
        assert_eq!(full.new_cursor(), 5);
        assert!(!full.bytes().is_empty());

        let full_packet = s3a_pack_scholar::SrsSyncDeltaPacket::parse_bytes(&full.bytes())
            .expect("packet deserialization");
        assert_eq!(full_packet.entries.len(), 5);

        // Partial delta (cursor = 2): should contain only the remaining 3 statements
        let partial = session
            .export_since(2)
            .expect("partial delta should succeed");
        assert_eq!(partial.record_count(), 3);
        assert_eq!(partial.new_cursor(), 5);
        assert!(!partial.bytes().is_empty());

        let partial_packet = s3a_pack_scholar::SrsSyncDeltaPacket::parse_bytes(&partial.bytes())
            .expect("packet deserialization");
        assert_eq!(partial_packet.entries.len(), 3);

        // Empty delta (cursor = 5): should contain 0 statements, cursor remains 5
        let empty = session.export_since(5).expect("empty delta should succeed");
        assert_eq!(empty.record_count(), 0);
        assert_eq!(empty.new_cursor(), 5);

        let empty_packet = s3a_pack_scholar::SrsSyncDeltaPacket::parse_bytes(&empty.bytes())
            .expect("packet deserialization");
        assert_eq!(empty_packet.entries.len(), 0);

        // Cursor out-of-bounds (cursor > 5): should return an error
        let invalid = session.export_since(6);
        assert!(
            invalid.is_err(),
            "cursor ahead of statement log should fail"
        );
    }

    #[test]
    fn test_export_from_bytes_100_iterations_roundtrip() {
        let mut session =
            crate::bindgen::WasmAgentSnapshot::new_session("0123456789abcdef0123456789abcdef")
                .unwrap();
        let mut last_size = 0;

        for i in 0..100 {
            let ai_idx = session
                .record_ai(
                    "lab-assistant",
                    s3a_pack_scholar::xapi::VERB_ID_AI_ANSWERED,
                    0.95,
                    None,
                )
                .unwrap();
            let _human_idx = session
                .record_human(
                    s3a_pack_scholar::xapi::VERB_ID_SEARCHED,
                    wasm_bindgen::JsValue::NULL,
                    (1000 + i) as u32,
                    Some(ai_idx),
                )
                .unwrap();

            let bytes = session.export_bytes().unwrap();
            assert!(
                bytes.len() >= last_size,
                "Size must grow or stay valid at iteration {}",
                i
            );
            last_size = bytes.len();

            let restored = crate::bindgen::WasmAgentSnapshot::from_bytes(&bytes)
                .expect("from_bytes should succeed");
            assert_eq!(restored.inner.ai_traces.len(), (i + 1) as usize);
            assert_eq!(restored.inner.human_decisions.len(), (i + 1) as usize);
            assert_eq!(restored.inner.statement_log.len(), ((i + 1) * 2) as usize);

            session = restored;
        }

        let final_bytes = session.export_bytes().unwrap();
        assert!(!final_bytes.is_empty());
    }

    #[test]
    fn test_wasm_roundtrip_new_session_record_export_from_bytes_export() {
        let mut session =
            crate::bindgen::WasmAgentSnapshot::new_session("0123456789abcdef0123456789abcdef")
                .expect("new_session should succeed");

        let h_idx = session
            .record_human(
                s3a_pack_scholar::xapi::VERB_ID_SEARCHED,
                wasm_bindgen::JsValue::NULL,
                0x12345678,
                None,
            )
            .expect("record_human should succeed");

        let _ai_idx = session
            .record_ai(
                "lab-assistant",
                18, // verb 18: ai-answered
                0.95,
                Some(h_idx),
            )
            .expect("record_ai should succeed");

        let bytes1 = session
            .export_bytes()
            .expect("first export_bytes should succeed");
        assert!(!bytes1.is_empty(), "first export must not be empty");
        println!("Export 1 size: {} bytes", bytes1.len());

        let mut restored = crate::bindgen::WasmAgentSnapshot::from_bytes(&bytes1)
            .expect("from_bytes should succeed");

        let bytes2 = restored
            .export_bytes()
            .expect("second export_bytes should succeed");
        assert!(!bytes2.is_empty(), "second export must not be empty");
        println!("Export 2 size: {} bytes", bytes2.len());

        assert_eq!(
            bytes1.len(),
            bytes2.len(),
            "both exports must succeed and lengths must be stable"
        );
    }
}
