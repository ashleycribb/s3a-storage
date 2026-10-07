//! S3A Autonomous Research & Reasoning Loop with Epistemic Confidence Tiers.
//! Implements SECS-SEROM and SECS-SEMS rules: Human-First Gate, Evidence Before Generation,
//! and calibrated epistemic confidence (Very Low, Low, Moderate, High, Very High).

use s3a_core::S3ACoordinate;
use s3a_engine::S3ACrudEngine;
use s3a_pack_scholar::xapi::{
    VERB_ID_APPROVED, VERB_ID_TA_APPROVED_DRAFT, VERB_ID_TA_ESCALATED, VERB_ID_TA_PROVIDED_ANALOGY,
    VERB_ID_TA_REDIRECTED,
};
use s3a_pack_scholar::{
    AcademicPaperRecord, AgentDenied, AgentRegistryRecord, AiTraceRecord, HumanLrsRecord,
    ResearchGraphEdgeRecord, ScholarCrudExt,
};
use std::sync::atomic::{AtomicU64, Ordering};

/// Epistemic Confidence Tier calibrated according to SECS-SEMS-1.0.0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpistemicConfidenceTier {
    VeryLow,  // 0.00 - 0.20 (Hypothetical speculation)
    Low,      // 0.20 - 0.40 (Emerging observation)
    Moderate, // 0.40 - 0.70 (Corroborated by partial literature)
    High,     // 0.70 - 0.90 (Consistent empirical findings)
    VeryHigh, // 0.90 - 1.00 (Replicated consensus or mathematical proof)
}

impl EpistemicConfidenceTier {
    pub fn from_score(score: f32) -> Self {
        if score < 0.20 {
            EpistemicConfidenceTier::VeryLow
        } else if score < 0.40 {
            EpistemicConfidenceTier::Low
        } else if score < 0.70 {
            EpistemicConfidenceTier::Moderate
        } else if score < 0.90 {
            EpistemicConfidenceTier::High
        } else {
            EpistemicConfidenceTier::VeryHigh
        }
    }

    pub fn epistemic_phrase(&self) -> &'static str {
        match self {
            EpistemicConfidenceTier::VeryLow => {
                "Preliminary evidence suggests a speculative possibility"
            }
            EpistemicConfidenceTier::Low => "Emerging observations indicate an initial pattern",
            EpistemicConfidenceTier::Moderate => "Literature corroborates moderate alignment",
            EpistemicConfidenceTier::High => {
                "Substantial empirical evidence consistently demonstrates"
            }
            EpistemicConfidenceTier::VeryHigh => {
                "Conclusive replicated empirical findings establish"
            }
        }
    }
}

/// The 4 Types of Pre-Flight Hallucination Risks detected by the Harness.
#[derive(Debug, Clone, PartialEq)]
pub enum HallucinationRisk {
    /// Type 4 / Inferential: Statistical confidence falls below epistemic floor (< 0.70)
    LowConfidence {
        score: f32,
        tier: EpistemicConfidenceTier,
    },
    /// Type 1 / Extrinsic: Analogy or metaphor has no pre-indexed isomorphism in the ontology
    UnverifiedAnalogy { proposed_analogy: String },
    /// Type 3 / Ontological: Entity or property is not grounded in the S3A corpus or ontology
    UnboundEntity { entity_hash: u64 },
    /// Type 2 / Contextual: Attempted instruction or assertion contradicts the Teacher's curriculum graph
    CurriculumContradiction { concept_id: u64, reason: String },
}

/// The Teacher Assistant's (TA) authoritative resolution provided back to the harness.
#[derive(Debug, Clone, PartialEq)]
pub enum TaGuidance {
    /// TA reviews the agent's proposed draft and approves it as pedagogically safe.
    ApproveDraft,
    /// TA provides an approved alternative analogy or everyday scaffold.
    ProvideAlternativeAnalogy {
        approved_analogy: String,
        pedagogical_edge: u32,
    },
    /// TA redirects the student/agent to a curated textbook excerpt coordinate in Tile 12.
    RedirectToExcerpt { target_coord: S3ACoordinate },
    /// TA flags the inquiry for in-person discussion or office hours.
    EscalateToOfficeHours { ta_note: String },
}

// Well-known Teacher Assistant xAPI action verb IDs from authoritative xAPI registry
pub const TA_VERB_APPROVED_DRAFT: u32 = VERB_ID_TA_APPROVED_DRAFT;
pub const TA_VERB_PROVIDED_ANALOGY: u32 = VERB_ID_TA_PROVIDED_ANALOGY;
pub const TA_VERB_REDIRECTED: u32 = VERB_ID_TA_REDIRECTED;
pub const TA_VERB_ESCALATED: u32 = VERB_ID_TA_ESCALATED;

/// Research Action in the autonomous inquiry loop.
#[derive(Debug, Clone)]
pub enum ResearchAction {
    FormulateHypothesis {
        question: String,
    },
    SearchLiterature {
        query: String,
        min_year: u16,
        max_year: u16,
    },
    ExtractClaims {
        paper_id: u64,
        claims: Vec<String>,
    },
    SynthesizeEvidence {
        claim_a: u64,
        claim_b: u64,
        predicate_id: u32,
        confidence: f32,
    },
    RequestHumanReview {
        reason: String,
        proposed_action: String,
    },
}

use std::collections::HashMap;
use std::sync::RwLock;

/// Gate state for human-in-the-loop oversight.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GateStatus {
    Pending,
    Approved,
    Rejected,
}

/// A human gate request awaiting supervisor intervention.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HumanGateRequest {
    pub gate_id: String,
    pub action_type: String,
    pub reason: String,
    pub status: GateStatus,
    pub created_at: u64,
    pub resolved_at: Option<u64>,
    pub decision_actor: Option<u64>,
}

/// Audit log entry for a rejected gated action without matching human decision.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GateRejectionLog {
    pub timestamp: u64,
    pub session_uuid: [u64; 2],
    pub agent_id: u64,
    pub step_type: u32,
    pub confidence: f32,
    pub reason: String,
}

/// Core autonomous loop engine coordinating inquiry, knowledge graph formation, and human gates.
pub struct ResearchLoopEngine {
    pub session_uuid: [u64; 2],
    pub agent_id: u64,
    pub step_counter: AtomicU64,
    pub gates: RwLock<HashMap<String, HumanGateRequest>>,
    pub rejections: RwLock<Vec<GateRejectionLog>>,
}

impl ResearchLoopEngine {
    pub fn new(session_uuid: [u64; 2], agent_id: u64) -> Self {
        Self {
            session_uuid,
            agent_id,
            step_counter: AtomicU64::new(1),
            gates: RwLock::new(HashMap::new()),
            rejections: RwLock::new(Vec::new()),
        }
    }

    /// Submits a request for human supervisor gate review. Returns pending gate.
    pub fn request_human_gate(&self, action_type: &str, reason: &str) -> HumanGateRequest {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let gate_id = s3a_engine::lrs_store::generate_uuid_v4();
        let gate = HumanGateRequest {
            gate_id: gate_id.clone(),
            action_type: action_type.to_string(),
            reason: reason.to_string(),
            status: GateStatus::Pending,
            created_at: ts,
            resolved_at: None,
            decision_actor: None,
        };
        self.gates.write().unwrap().insert(gate_id, gate.clone());
        gate
    }

    /// Retrieves status of a previously registered human gate.
    pub fn get_gate_status(&self, gate_id: &str) -> Option<HumanGateRequest> {
        self.gates.read().unwrap().get(gate_id).cloned()
    }

    /// Records human decision and associates it with a pending gate if specified.
    #[allow(clippy::too_many_arguments)]
    pub fn record_human_decision_with_gate(
        &self,
        engine: &S3ACrudEngine,
        actor_id: u64,
        verb_id: u32,
        decision_score: f32,
        ai_coord: S3ACoordinate,
        target_object_id: u32,
        gate_id: Option<&str>,
    ) -> Result<S3ACoordinate, std::io::Error> {
        let coord = self.record_human_decision(
            engine,
            actor_id,
            verb_id,
            decision_score,
            ai_coord,
            target_object_id,
        )?;

        if let Some(gid) = gate_id {
            let mut map = self.gates.write().unwrap();
            if let Some(gate) = map.get_mut(gid) {
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                gate.status = if decision_score >= 0.70
                    || verb_id == VERB_ID_APPROVED
                    || verb_id == VERB_ID_TA_APPROVED_DRAFT
                {
                    GateStatus::Approved
                } else {
                    GateStatus::Rejected
                };
                gate.resolved_at = Some(ts);
                gate.decision_actor = Some(actor_id);
            }
        }

        Ok(coord)
    }

    /// Gated AI step recording.
    /// If confidence < 0.70 or action requires a gate, rejects and logs if no approved human decision exists.
    pub fn record_ai_step_gated(
        &self,
        engine: &S3ACrudEngine,
        step_type: u32,
        confidence: f32,
        human_coord: S3ACoordinate,
        hilbert_index: u32,
        gate_id: Option<&str>,
    ) -> Result<S3ACoordinate, String> {
        let is_gated = self.requires_human_gate(confidence, false);
        if is_gated {
            let approved = match gate_id {
                Some(gid) => {
                    let map = self.gates.read().unwrap();
                    map.get(gid)
                        .map(|g| g.status == GateStatus::Approved)
                        .unwrap_or(false)
                }
                None => false,
            };

            if !approved {
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let reason = format!(
                    "Gated action rejected: confidence {:.2} < 0.70 requires human gate approval; gate_id={:?}",
                    confidence, gate_id
                );
                let log = GateRejectionLog {
                    timestamp: ts,
                    session_uuid: self.session_uuid,
                    agent_id: self.agent_id,
                    step_type,
                    confidence,
                    reason: reason.clone(),
                };
                self.rejections.write().unwrap().push(log);
                return Err(reason);
            }
        }

        self.record_ai_step(engine, step_type, confidence, human_coord, hilbert_index)
            .map_err(|e| e.to_string())
    }

    /// Evaluates if human/TA approval is required prior to applying critical knowledge state transitions.
    pub fn requires_human_gate(&self, confidence: f32, is_contradictory: bool) -> bool {
        // Human approval mandatory if confidence < 0.70 or if contradiction is flagged
        confidence < 0.70 || is_contradictory
    }

    /// Asserts that a registered task agent is permitted to execute the given action verb and tool.
    /// Hard denial: returns Err(AgentDenied) on any violation; never warns and continues.
    pub fn assert_agent_allows(
        &self,
        agent: &AgentRegistryRecord,
        verb_bit: Option<u32>,
        tool_bit: Option<u32>,
    ) -> Result<(), AgentDenied> {
        agent.assert_allows(verb_bit, tool_bit)
    }

    /// Complete bounding assertion enforcing both static allowlists/denials
    /// AND dynamic supervisory clearance when AGENT_FLAG_REQUIRES_HUMAN_GATE is set.
    pub fn assert_agent_allows_gated(
        &self,
        agent: &AgentRegistryRecord,
        verb_bit: Option<u32>,
        tool_bit: Option<u32>,
        is_human_cleared: bool,
    ) -> Result<(), AgentDenied> {
        agent.assert_allows_gated(verb_bit, tool_bit, is_human_cleared)
    }

    /// Pre-Flight Hallucination Gate: Evaluates candidate draft before emission to student.
    pub fn preflight_check(
        &self,
        candidate_confidence: f32,
        unbound_entities: &[u64],
        has_vetted_analogy: bool,
    ) -> Result<(), HallucinationRisk> {
        if candidate_confidence < 0.70 {
            return Err(HallucinationRisk::LowConfidence {
                score: candidate_confidence,
                tier: EpistemicConfidenceTier::from_score(candidate_confidence),
            });
        }
        if let Some(&unbound) = unbound_entities.first() {
            return Err(HallucinationRisk::UnboundEntity {
                entity_hash: unbound,
            });
        }
        if !has_vetted_analogy {
            return Err(HallucinationRisk::UnverifiedAnalogy {
                proposed_analogy: "Unvetted analogy attempted".into(),
            });
        }
        Ok(())
    }

    /// The Agent "Raises Its Hand" to the Teacher Assistant (TA) in the harness.
    /// Halts generation, alerts the TA, and awaits authoritative guidance.
    pub fn raise_hand_to_ta(
        &self,
        risk: HallucinationRisk,
        _student_question: &str,
        _candidate_draft: &str,
        fallback_guidance: Option<TaGuidance>,
    ) -> TaGuidance {
        // If a specific TA guidance was provided (e.g. interactive supervisor input or mock policy), return it.
        // Otherwise, apply the safe fallback: redirect to curated curriculum.
        fallback_guidance.unwrap_or_else(|| match &risk {
            HallucinationRisk::UnverifiedAnalogy { .. } => TaGuidance::ProvideAlternativeAnalogy {
                approved_analogy: "Standard textbook model: observe baseline equilibrium".into(),
                pedagogical_edge: s3a_pack_ontology::PREDICATE_HAS_APPROVED_ANALOGY,
            },
            _ => TaGuidance::EscalateToOfficeHours {
                ta_note: "Concept requires instructor clarification".into(),
            },
        })
    }

    /// Records an authoritative Teacher Assistant (TA) oversight decision linked to an AI trace coordinate.
    pub fn record_ta_decision(
        &self,
        engine: &S3ACrudEngine,
        ta_id: u64,
        verb_id: u32,
        decision_score: f32,
        ai_coord: S3ACoordinate,
        target_object_id: u32,
    ) -> Result<S3ACoordinate, std::io::Error> {
        self.record_human_decision(
            engine,
            ta_id,
            verb_id,
            decision_score,
            ai_coord,
            target_object_id,
        )
    }

    /// Records an atomic dual-actor step: creates AI trace and links to human decision coordinate.
    pub fn record_ai_step(
        &self,
        engine: &S3ACrudEngine,
        step_type: u32,
        confidence: f32,
        human_coord: S3ACoordinate,
        hilbert_index: u32,
    ) -> Result<S3ACoordinate, std::io::Error> {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let trace = AiTraceRecord::new(
            self.session_uuid,
            ts,
            self.agent_id,
            step_type,
            confidence,
            human_coord,
            hilbert_index,
        );
        let tile_id = engine.create_ai_traces(&[trace])?;
        self.step_counter.fetch_add(1, Ordering::Relaxed);
        Ok(S3ACoordinate::new(0, tile_id as u32, 0))
    }

    /// Records an explicit human oversight decision approving or refining AI outputs.
    pub fn record_human_decision(
        &self,
        engine: &S3ACrudEngine,
        actor_id: u64,
        verb_id: u32,
        decision_score: f32,
        ai_coord: S3ACoordinate,
        target_object_id: u32,
    ) -> Result<S3ACoordinate, std::io::Error> {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let record = HumanLrsRecord::new(
            self.session_uuid,
            ts,
            actor_id,
            verb_id,
            decision_score,
            ai_coord,
            target_object_id,
        );
        let tile_id = engine.create_human_lrs(&[record])?;
        Ok(S3ACoordinate::new(0, tile_id as u32, 0))
    }

    /// Registers a newly ingested academic paper into the hyper-dimensional topic space.
    #[allow(clippy::too_many_arguments)]
    pub fn register_paper(
        &self,
        engine: &S3ACrudEngine,
        paper_id: u64,
        topic_coords: [f32; 3],
        citation_count: u32,
        year: u16,
        venue_id: u16,
        is_oa: bool,
    ) -> Result<S3ACoordinate, std::io::Error> {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let hilbert_idx = s3a_core::point_to_hilbert_3d(
            ((topic_coords[0].clamp(0.0, 1.0)) * 1023.0) as u32,
            ((topic_coords[1].clamp(0.0, 1.0)) * 1023.0) as u32,
            ((topic_coords[2].clamp(0.0, 1.0)) * 1023.0) as u32,
            10,
        );

        let paper = AcademicPaperRecord::new(
            paper_id,
            ts,
            hilbert_idx,
            topic_coords[0],
            topic_coords[1],
            topic_coords[2],
            citation_count,
            year,
            venue_id,
            if is_oa { 1 } else { 0 },
            0,
            paper_id >> 32,
        );
        let tile_id = engine.create_academic_papers(&[paper])?;
        Ok(S3ACoordinate::new(0, tile_id as u32, 0))
    }

    /// Links two research objects in the knowledge graph with an evidence relationship edge.
    pub fn link_evidence(
        &self,
        engine: &S3ACrudEngine,
        subject_hash: u64,
        object_hash: u64,
        predicate_id: u32,
        weight: f32,
    ) -> Result<S3ACoordinate, std::io::Error> {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let edge =
            ResearchGraphEdgeRecord::new(subject_hash, object_hash, ts, 0, predicate_id, weight);
        let tile_id = engine.create_research_graph_edges(&[edge])?;
        Ok(S3ACoordinate::new(0, tile_id as u32, 0))
    }
}
