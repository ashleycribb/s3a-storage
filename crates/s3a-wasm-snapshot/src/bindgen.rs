use s3a_core::S3ACoordinate;
use s3a_pack_scholar::{
    seed_default_agents, AcademicPaperRecord, AiTraceRecord, HumanLrsRecord, SrsManifestHeader,
    SrsWorkspaceRecord, TOOL_CREDENTIAL_EXPORT as CORE_TOOL_CREDENTIAL_EXPORT,
    TOOL_CROSSREF_QUERY as CORE_TOOL_CROSSREF_QUERY,
    TOOL_EXPORT_SNAPSHOT as CORE_TOOL_EXPORT_SNAPSHOT,
    TOOL_EXTRACT_CLAIMS as CORE_TOOL_EXTRACT_CLAIMS, TOOL_MUTATE_GRAPH as CORE_TOOL_MUTATE_GRAPH,
    TOOL_OPENALEX_QUERY as CORE_TOOL_OPENALEX_QUERY, TOOL_PUBLISH as CORE_TOOL_PUBLISH,
    TOOL_READ_FILE as CORE_TOOL_READ_FILE,
    TOOL_SYNTHESIZE_EVIDENCE as CORE_TOOL_SYNTHESIZE_EVIDENCE,
    TOOL_TA_SUPERVISOR_CALL as CORE_TOOL_TA_SUPERVISOR_CALL,
};
use wasm_bindgen::prelude::*;

use crate::agent_api::WasmAgentSnapshot as InnerSnapshot;

#[wasm_bindgen]
#[allow(non_camel_case_types)]
#[derive(Clone, Copy)]
pub enum ScholarTool {
    TOOL_READ_FILE = 1,
    TOOL_OPENALEX_QUERY = 2,
    TOOL_EXTRACT_CLAIMS = 4,
    TOOL_SYNTHESIZE_EVIDENCE = 8,
    TOOL_CROSSREF_QUERY = 16,
    TOOL_EXPORT_SNAPSHOT = 32,
    TOOL_TA_SUPERVISOR_CALL = 64,
    TOOL_MUTATE_GRAPH = 128,
    TOOL_PUBLISH = 65536,
    TOOL_CREDENTIAL_EXPORT = 131072,
}

pub const TOOL_READ_FILE: u32 = CORE_TOOL_READ_FILE;
pub const TOOL_OPENALEX_QUERY: u32 = CORE_TOOL_OPENALEX_QUERY;
pub const TOOL_EXTRACT_CLAIMS: u32 = CORE_TOOL_EXTRACT_CLAIMS;
pub const TOOL_SYNTHESIZE_EVIDENCE: u32 = CORE_TOOL_SYNTHESIZE_EVIDENCE;
pub const TOOL_CROSSREF_QUERY: u32 = CORE_TOOL_CROSSREF_QUERY;
pub const TOOL_EXPORT_SNAPSHOT: u32 = CORE_TOOL_EXPORT_SNAPSHOT;
pub const TOOL_TA_SUPERVISOR_CALL: u32 = CORE_TOOL_TA_SUPERVISOR_CALL;
pub const TOOL_MUTATE_GRAPH: u32 = CORE_TOOL_MUTATE_GRAPH;
pub const TOOL_PUBLISH: u32 = CORE_TOOL_PUBLISH;
pub const TOOL_CREDENTIAL_EXPORT: u32 = CORE_TOOL_CREDENTIAL_EXPORT;

#[wasm_bindgen]
pub fn tool_publish() -> u32 {
    CORE_TOOL_PUBLISH
}

#[wasm_bindgen]
pub fn tool_openalex_query() -> u32 {
    CORE_TOOL_OPENALEX_QUERY
}

fn js_error(msg: &str) -> JsValue {
    #[cfg(target_arch = "wasm32")]
    {
        JsValue::from_str(msg)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = msg;
        JsValue::NULL
    }
}

fn parse_uuid_hex(hex_str: &str) -> Result<[u64; 2], JsValue> {
    let clean: String = hex_str.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if clean.len() != 32 {
        return Err(js_error(&format!(
            "Invalid session UUID hex length: expected 32 hex characters, got {}",
            clean.len()
        )));
    }
    let high = u64::from_str_radix(&clean[0..16], 16)
        .map_err(|e| js_error(&format!("Invalid UUID high 64 bits: {}", e)))?;
    let low = u64::from_str_radix(&clean[16..32], 16)
        .map_err(|e| js_error(&format!("Invalid UUID low 64 bits: {}", e)))?;
    Ok([high, low])
}

fn js_value_to_u64(val: &JsValue) -> Result<u64, JsValue> {
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(n) = val.as_f64() {
            Ok(n as u64)
        } else if let Some(s) = val.as_string() {
            s.parse::<u64>()
                .map_err(|e| js_error(&format!("Invalid u64 string: {}", e)))
        } else if let Some(b) = js_sys::BigInt::new(val).ok() {
            if let Ok(s) = b.to_string(10) {
                if let Some(st) = s.as_string() {
                    if let Ok(n) = st.parse::<u64>() {
                        return Ok(n);
                    }
                }
            }
            Err(js_error("Invalid BigInt for u64"))
        } else {
            Err(js_error("Expected number, string, or BigInt for u64"))
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = val;
        Ok(0)
    }
}

fn current_timestamp_sec() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        (js_sys::Date::now() / 1000.0) as u64
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
}

/// Browser-side WebAssembly wrapper around `WasmAgentSnapshot`.
#[wasm_bindgen]
pub struct WasmAgentSnapshot {
    pub(crate) inner: InnerSnapshot,
    session_uuid: [u64; 2],
}

#[wasm_bindgen]
impl WasmAgentSnapshot {
    /// Creates a new session from a 128-bit hex UUID and seeds the registry with default agents.
    #[wasm_bindgen]
    pub fn new_session(session_uuid_hex: &str) -> Result<WasmAgentSnapshot, JsValue> {
        let session_uuid = parse_uuid_hex(session_uuid_hex)?;
        let project_uuid = session_uuid;
        let snapshot_uuid = [
            session_uuid[0] ^ 0xA5A5_A5A5_5A5A_5A5A,
            session_uuid[1] ^ 0x5A5A_5A5A_A5A5_A5A5,
        ];
        let parent_snapshot_uuid = [0u64, 0u64];
        let origin_session_uuid = session_uuid;
        let created_at_sec = current_timestamp_sec();

        let default_agents = seed_default_agents();
        let total_records = (1 + default_agents.len()) as u64; // workspace + agents
        let manifest = SrsManifestHeader::new(
            project_uuid,
            snapshot_uuid,
            parent_snapshot_uuid,
            origin_session_uuid,
            created_at_sec,
            2, // initial estimate: workspace + agents + manifest
            total_records,
        );

        let workspace = Some(SrsWorkspaceRecord::new(
            session_uuid,
            0,
            0,
            [0.0, 0.0, 0.0],
            1,
        ));

        let inner = InnerSnapshot {
            manifest,
            workspace,
            papers: Vec::new(),
            edges: Vec::new(),
            human_decisions: Vec::new(),
            ai_traces: Vec::new(),
            agents: default_agents,
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

        Ok(Self {
            inner,
            session_uuid,
        })
    }

    /// Records a human activity statement / decision.
    #[wasm_bindgen]
    pub fn record_human(
        &mut self,
        verb_id: u32,
        actor_hash: JsValue,
        object_hash: u32,
        ai_trace_index: Option<u32>,
    ) -> Result<u32, JsValue> {
        if verb_id == s3a_pack_scholar::xapi::VERB_ID_AI_ANSWERED {
            return Err(js_error(
                "Verb 'ai-answered' is AI-only and cannot be used in record_human",
            ));
        }

        let actor = js_value_to_u64(&actor_hash)?;
        let ai_trace_coord = match ai_trace_index {
            Some(idx) => S3ACoordinate::new(0, 0, idx),
            None => S3ACoordinate::default(),
        };

        let now = current_timestamp_sec();
        let rec = HumanLrsRecord::new(
            self.session_uuid,
            now,
            actor,
            verb_id,
            1.0,
            ai_trace_coord,
            object_hash,
        );

        self.inner.human_decisions.push(rec);
        if let Ok(entry) = s3a_pack_scholar::SrsDeltaEntry::from_record(
            s3a_pack_scholar::ScholarTileType::HUMAN_LRS,
            &rec,
        ) {
            self.inner.pending_delta.push(entry);
            self.inner.statement_log.push(entry);
        }
        Ok((self.inner.human_decisions.len() - 1) as u32)
    }

    /// Records an AI agent reasoning / tool-call step.
    #[wasm_bindgen]
    pub fn record_ai(
        &mut self,
        agent_slug: &str,
        step_type: u32,
        confidence: f32,
        human_index: Option<u32>,
    ) -> Result<u32, JsValue> {
        if step_type == s3a_pack_scholar::xapi::VERB_ID_APPROVED
            || step_type == s3a_pack_scholar::xapi::VERB_ID_REJECTED
            || step_type == s3a_pack_scholar::xapi::VERB_ID_REVIEWED
        {
            return Err(js_error(
                "AI steps can never use judgment verbs (approved, rejected, reviewed)",
            ));
        }

        let agent_id = s3a_pack_scholar::xapi::hash_iri(agent_slug);
        let human_coord = match human_index {
            Some(idx) => S3ACoordinate::new(0, 0, idx),
            None => S3ACoordinate::default(),
        };

        let now = current_timestamp_sec();
        let rec = AiTraceRecord::new(
            self.session_uuid,
            now,
            agent_id,
            step_type,
            confidence,
            human_coord,
            0,
        );

        self.inner.ai_traces.push(rec);
        if let Ok(entry) = s3a_pack_scholar::SrsDeltaEntry::from_record(
            s3a_pack_scholar::ScholarTileType::AI_TRACE_LOG,
            &rec,
        ) {
            self.inner.pending_delta.push(entry);
            self.inner.statement_log.push(entry);
        }
        Ok((self.inner.ai_traces.len() - 1) as u32)
    }

    /// Adds an academic paper metadata record into the snapshot.
    #[wasm_bindgen]
    pub fn add_paper(
        &mut self,
        openalex_id: &str,
        doi: &str,
        year: u16,
        cited_by_count: u32,
        flags: Option<u32>,
    ) -> Result<u32, JsValue> {
        let paper_id = s3a_pack_scholar::xapi::hash_iri(openalex_id);
        let doi_prefix_hash = s3a_pack_scholar::xapi::hash_iri(doi);
        let now = current_timestamp_sec();

        let mut rec = AcademicPaperRecord::new(
            paper_id,
            now,
            0,
            0.0,
            0.0,
            0.0,
            cited_by_count,
            year,
            1,
            1,
            0,
            doi_prefix_hash,
        );

        if let Some(f) = flags {
            rec.flags = f as u64;
        } else {
            rec.flags = AcademicPaperRecord::SOURCE_OPENALEX;
        }

        self.inner.papers.push(rec);
        if let Ok(entry) = s3a_pack_scholar::SrsDeltaEntry::from_record(
            s3a_pack_scholar::ScholarTileType::ACADEMIC_PAPERS,
            &rec,
        ) {
            self.inner.pending_delta.push(entry);
            self.inner.statement_log.push(entry);
        }
        Ok((self.inner.papers.len() - 1) as u32)
    }

    /// Asserts that the specified agent slug is authorized for the given verb_id and tool_bit.
    /// Returns an error on denial.
    #[wasm_bindgen]
    pub fn assert_agent_allows(
        &self,
        agent_slug: &str,
        verb_id: Option<u32>,
        tool_bit: Option<u32>,
    ) -> Result<(), JsValue> {
        if let Some(vid) = verb_id {
            if vid == s3a_pack_scholar::xapi::VERB_ID_APPROVED
                || vid == s3a_pack_scholar::xapi::VERB_ID_REJECTED
                || vid == s3a_pack_scholar::xapi::VERB_ID_REVIEWED
            {
                return Err(js_error(
                    "AI steps can never use judgment verbs (approved, rejected, reviewed)",
                ));
            }
        }

        let agent_id = s3a_pack_scholar::xapi::hash_iri(agent_slug);
        let agent = self
            .inner
            .agents
            .iter()
            .find(|a| a.agent_id == agent_id)
            .ok_or_else(|| {
                js_error(&format!(
                    "Agent '{}' (id 0x{:016X}) not found in snapshot registry",
                    agent_slug, agent_id
                ))
            })?;

        let verb_bit = match verb_id {
            Some(vid) if vid > 0 => {
                if let Some(spec) = s3a_pack_scholar::xapi::resolve_verb_by_id(vid) {
                    Some(spec.verb_bit)
                } else {
                    Some(vid)
                }
            }
            _ => None,
        };

        let tool_opt = match tool_bit {
            Some(t) if t > 0 => Some(t),
            _ => None,
        };

        agent
            .assert_allows(verb_bit, tool_opt)
            .map_err(|denied| JsValue::from_str(&denied.to_string()))
    }

    /// Checks whether an agent requires human supervisory gate clearance.
    #[wasm_bindgen]
    pub fn requires_human_gate(&self, agent_slug: &str) -> Result<bool, JsValue> {
        let agent_id = s3a_pack_scholar::xapi::hash_iri(agent_slug);
        let agent = self
            .inner
            .agents
            .iter()
            .find(|a| a.agent_id == agent_id)
            .ok_or_else(|| {
                JsValue::from_str(&format!(
                    "Agent '{}' (id 0x{:016X}) not found in snapshot registry",
                    agent_slug, agent_id
                ))
            })?;
        Ok(agent.requires_human_gate())
    }

    /// Pre-flight hallucination and epistemic confidence evaluation.
    /// Returns "ok" on success or the risk message if risk detected.
    #[wasm_bindgen]
    pub fn preflight(
        &self,
        confidence: f32,
        concept_id: Option<f64>,
        has_approved_analogy: Option<bool>,
    ) -> String {
        let cid = concept_id.map(|n| n as u64).unwrap_or(0);
        let approved = has_approved_analogy.unwrap_or(true);

        match self.inner.preflight_agent_check(confidence, cid, approved) {
            Ok(()) => "ok".to_string(),
            Err(risk) => risk.to_string(),
        }
    }

    /// Exports the snapshot as an immutable .snapshot.s3a Uint8Array buffer.
    #[wasm_bindgen]
    pub fn export_bytes(&mut self) -> Result<Vec<u8>, JsValue> {
        self.inner.manifest.total_records = (if self.inner.workspace.is_some() { 1 } else { 0 })
            + self.inner.papers.len() as u64
            + self.inner.edges.len() as u64
            + self.inner.human_decisions.len() as u64
            + self.inner.ai_traces.len() as u64
            + self.inner.agents.len() as u64
            + self.inner.concepts.len() as u64
            + self.inner.axioms.len() as u64
            + (self.inner.claims.len() * 2) as u64
            + self.inner.policy_marks.len() as u64;

        self.inner
            .export_bytes()
            .map_err(|e| JsValue::from_str(&format!("Snapshot export error: {}", e)))
    }

    /// Loads a snapshot from a Uint8Array byte slice.
    #[wasm_bindgen]
    pub fn from_bytes(bytes: &[u8]) -> Result<WasmAgentSnapshot, JsValue> {
        let inner = InnerSnapshot::from_bytes(bytes)
            .map_err(|e| JsValue::from_str(&format!("Snapshot deserialization error: {}", e)))?;
        let session_uuid = inner.manifest.origin_session_uuid;
        Ok(Self {
            inner,
            session_uuid,
        })
    }

    /// Returns the number of uncommitted delta records pending synchronization.
    #[wasm_bindgen]
    pub fn pending_delta_count(&self) -> usize {
        self.inner.pending_delta.len()
    }

    /// Exports pending delta records since the last sync as a compact byte buffer.
    #[wasm_bindgen]
    pub fn export_delta(&mut self) -> Result<Vec<u8>, JsValue> {
        Ok(self.inner.export_delta_bytes())
    }

    /// Exports only statements and delta records recorded after `cursor`.
    /// Returns a `WasmSyncBatch` containing the binary delta packet, new cursor, and record count.
    #[wasm_bindgen]
    pub fn export_since(&self, cursor: u64) -> Result<WasmSyncBatch, JsValue> {
        let (bytes, new_cursor, record_count) =
            self.inner.export_since(cursor).map_err(|e| js_error(&e))?;
        Ok(WasmSyncBatch {
            bytes,
            new_cursor,
            record_count,
        })
    }

    /// Applies an incoming sync delta buffer into the local session.
    #[wasm_bindgen]
    pub fn apply_delta(&mut self, delta_bytes: &[u8]) -> Result<usize, JsValue> {
        self.inner
            .apply_delta_bytes(delta_bytes)
            .map_err(|e| JsValue::from_str(&format!("Apply delta error: {}", e)))
    }

    /// Adds a research claim or note with optional vector embedding.
    #[wasm_bindgen]
    pub fn add_claim(
        &mut self,
        agent_slug: &str,
        text: &str,
        confidence: f32,
        kind: Option<u16>,
        embedding: Option<Vec<f32>>,
    ) -> Result<u32, JsValue> {
        let author_hash = s3a_pack_scholar::xapi::hash_iri(agent_slug);
        let claim_id = crc32c::crc32c(text.as_bytes()) as u64;
        let claim_kind = kind.unwrap_or(s3a_pack_scholar::CLAIM_KIND_EMPIRICAL);
        self.inner.add_claim(
            claim_id,
            author_hash,
            confidence,
            claim_kind,
            text,
            embedding.as_deref(),
        );
        Ok((self.inner.claims.len() - 1) as u32)
    }

    /// Searches indexed claims and notes by keyword using token Bloom filter and lexical scoring.
    #[wasm_bindgen]
    pub fn search_claims_text(
        &self,
        query: &str,
        top_k: Option<usize>,
        include_hidden: Option<bool>,
    ) -> Result<JsValue, JsValue> {
        let k = top_k.unwrap_or(10);
        let hidden = include_hidden.unwrap_or(false);
        let results = self.inner.search_claims_text(query, k, hidden);
        format_claim_results(&results)
    }

    /// Searches indexed claims by vector embedding using SIMD-accelerated cosine similarity.
    #[wasm_bindgen]
    pub fn search_claims_vector(
        &self,
        query_embedding: Vec<f32>,
        top_k: Option<usize>,
        min_score: Option<f32>,
        include_hidden: Option<bool>,
    ) -> Result<JsValue, JsValue> {
        let k = top_k.unwrap_or(10);
        let min_s = min_score.unwrap_or(0.0);
        let hidden = include_hidden.unwrap_or(false);
        let results = self
            .inner
            .search_claims_vector(&query_embedding, k, min_s, hidden);
        format_claim_results(&results)
    }

    /// Performs hybrid search combining keyword match and vector cosine similarity.
    #[wasm_bindgen]
    pub fn search_claims_hybrid(
        &self,
        query: &str,
        query_embedding: Option<Vec<f32>>,
        top_k: Option<usize>,
        include_hidden: Option<bool>,
    ) -> Result<JsValue, JsValue> {
        let k = top_k.unwrap_or(10);
        let hidden = include_hidden.unwrap_or(false);
        let results =
            self.inner
                .search_claims_hybrid(query, query_embedding.as_deref(), 0.5, 0.5, k, hidden);
        format_claim_results(&results)
    }

    /// Resolves an xAPI IRI or label to its 32-bit integer verb_id.
    #[wasm_bindgen]
    pub fn verb_id_for_iri(&self, iri: &str) -> Option<u32> {
        verb_id_for_iri(iri)
    }

    /// Resolves a 32-bit integer verb_id to its authoritative xAPI IRI string.
    #[wasm_bindgen]
    pub fn iri_for_verb_id(&self, id: u32) -> Option<String> {
        iri_for_verb_id(id)
    }
}

/// Standalone factory function to create a new session.
#[wasm_bindgen]
pub fn new_session(session_uuid_hex: &str) -> Result<WasmAgentSnapshot, JsValue> {
    WasmAgentSnapshot::new_session(session_uuid_hex)
}

/// Standalone factory function to parse a snapshot from a Uint8Array.
#[wasm_bindgen]
pub fn from_bytes(bytes: &[u8]) -> Result<WasmAgentSnapshot, JsValue> {
    WasmAgentSnapshot::from_bytes(bytes)
}

/// Standalone function exporting the snapshot as an immutable .snapshot.s3a Uint8Array buffer.
#[wasm_bindgen]
pub fn export_bytes(snapshot: &mut WasmAgentSnapshot) -> Result<Vec<u8>, JsValue> {
    snapshot.export_bytes()
}

/// Standalone function resolving an xAPI IRI or label to its 32-bit integer verb_id.
#[wasm_bindgen]
pub fn verb_id_for_iri(iri: &str) -> Option<u32> {
    s3a_pack_scholar::xapi::resolve_verb_by_iri(iri)
        .or_else(|| s3a_pack_scholar::xapi::resolve_verb_by_label(iri))
        .map(|v| v.verb_id)
}

/// Standalone function resolving a 32-bit integer verb_id to its canonical IRI.
#[wasm_bindgen]
pub fn iri_for_verb_id(id: u32) -> Option<String> {
    s3a_pack_scholar::xapi::resolve_verb_by_id(id).map(|v| v.iri.to_string())
}

fn format_claim_results(
    results: &[crate::agent_api::ClaimSearchResult],
) -> Result<JsValue, JsValue> {
    let arr = js_sys::Array::new();
    for res in results {
        let obj = js_sys::Object::new();
        js_sys::Reflect::set(
            &obj,
            &JsValue::from_str("claimId"),
            &JsValue::from_f64(res.claim_id as f64),
        )?;
        js_sys::Reflect::set(
            &obj,
            &JsValue::from_str("text"),
            &JsValue::from_str(&res.text),
        )?;
        js_sys::Reflect::set(
            &obj,
            &JsValue::from_str("confidence"),
            &JsValue::from_f64(res.confidence as f64),
        )?;
        js_sys::Reflect::set(
            &obj,
            &JsValue::from_str("kind"),
            &JsValue::from_f64(res.kind as f64),
        )?;
        js_sys::Reflect::set(
            &obj,
            &JsValue::from_str("score"),
            &JsValue::from_f64(res.score as f64),
        )?;
        arr.push(&obj);
    }
    Ok(arr.into())
}

/// Batched sync delta payload for Scholar Explorer.
#[wasm_bindgen]
#[derive(Clone, Debug)]
pub struct WasmSyncBatch {
    bytes: Vec<u8>,
    new_cursor: u64,
    record_count: u32,
}

#[wasm_bindgen]
impl WasmSyncBatch {
    #[wasm_bindgen(getter)]
    pub fn bytes(&self) -> Vec<u8> {
        self.bytes.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn new_cursor(&self) -> u64 {
        self.new_cursor
    }

    #[wasm_bindgen(getter)]
    pub fn record_count(&self) -> u32 {
        self.record_count
    }
}

/// Standalone function exporting statements recorded after `cursor`.
#[wasm_bindgen]
pub fn export_since(snapshot: &WasmAgentSnapshot, cursor: u64) -> Result<WasmSyncBatch, JsValue> {
    snapshot.export_since(cursor)
}
