//! S3A Autonomous Research & Reasoning Loop with Epistemic Confidence Tiers.
//! Implements SECS-SEROM and SECS-SEMS rules: Human-First Gate, Evidence Before Generation,
//! and calibrated epistemic confidence (Very Low, Low, Moderate, High, Very High).

use std::sync::atomic::{AtomicU64, Ordering};
use s3a_core::{S3ACoordinate, HumanLrsRecord, AiTraceRecord, AcademicPaperRecord, ResearchGraphEdgeRecord};
use s3a_engine::S3ACrudEngine;

/// Epistemic Confidence Tier calibrated according to SECS-SEMS-1.0.0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpistemicConfidenceTier {
    VeryLow,    // 0.00 - 0.20 (Hypothetical speculation)
    Low,        // 0.20 - 0.40 (Emerging observation)
    Moderate,   // 0.40 - 0.70 (Corroborated by partial literature)
    High,       // 0.70 - 0.90 (Consistent empirical findings)
    VeryHigh,   // 0.90 - 1.00 (Replicated consensus or mathematical proof)
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
            EpistemicConfidenceTier::VeryLow => "Preliminary evidence suggests a speculative possibility",
            EpistemicConfidenceTier::Low => "Emerging observations indicate an initial pattern",
            EpistemicConfidenceTier::Moderate => "Literature corroborates moderate alignment",
            EpistemicConfidenceTier::High => "Substantial empirical evidence consistently demonstrates",
            EpistemicConfidenceTier::VeryHigh => "Conclusive replicated empirical findings establish",
        }
    }
}

/// Research Action in the autonomous inquiry loop.
#[derive(Debug, Clone)]
pub enum ResearchAction {
    FormulateHypothesis { question: String },
    SearchLiterature { query: String, min_year: u16, max_year: u16 },
    ExtractClaims { paper_id: u64, claims: Vec<String> },
    SynthesizeEvidence { claim_a: u64, claim_b: u64, predicate_id: u32, confidence: f32 },
    RequestHumanReview { reason: String, proposed_action: String },
}

/// Core autonomous loop engine coordinating inquiry, knowledge graph formation, and human gates.
pub struct ResearchLoopEngine {
    pub session_uuid: [u64; 2],
    pub agent_id: u64,
    pub step_counter: AtomicU64,
}

impl ResearchLoopEngine {
    pub fn new(session_uuid: [u64; 2], agent_id: u64) -> Self {
        Self {
            session_uuid,
            agent_id,
            step_counter: AtomicU64::new(1),
        }
    }

    /// Evaluates if human approval is required prior to applying critical knowledge state transitions.
    pub fn requires_human_gate(&self, confidence: f32, is_contradictory: bool) -> bool {
        // Human approval mandatory if confidence < 0.70 or if contradiction is flagged
        confidence < 0.70 || is_contradictory
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
        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
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
        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
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
        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
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
            (paper_id >> 32) as u64,
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
        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
        let edge = ResearchGraphEdgeRecord::new(
            subject_hash,
            object_hash,
            ts,
            0,
            predicate_id,
            weight,
        );
        let tile_id = engine.create_research_graph_edges(&[edge])?;
        Ok(S3ACoordinate::new(0, tile_id as u32, 0))
    }
}
