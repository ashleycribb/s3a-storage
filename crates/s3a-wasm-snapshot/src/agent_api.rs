use bytemuck::{bytes_of, cast_slice};
use s3a_core::{
    EmbeddingRecord128, FileHeader, HyperTileHeader, MarkRecord, TileType,
    ADDON_EDU, ADDON_ONTOLOGY, FILE_HEADER_SIZE, FILE_HEADER_SLOT_A_OFFSET,
    FILE_HEADER_SLOT_SIZE, HYPER_TILE_HEADER_SIZE, HYPER_TILE_SIZE,
};
use s3a_pack_scholar::{
    AcademicPaperRecord, AgentDenied, AgentRegistryRecord, AiTraceRecord, CitationFingerprintRecord,
    HumanLrsRecord, ResearchGraphEdgeRecord, ScholarNoteClaimRecord, ScholarTileType, SrsDeltaEntry,
    SrsManifestHeader, SrsSyncDeltaPacket, SrsWorkspaceRecord,
    SCHOLAR_LAYOUT_VERSION_V1, STEP_CLAIM,
};
use s3a_pack_ontology::{
    OntologyAxiomRecord, OntologyConceptRecord, OntologyTileType,
    PREDICATE_HAS_APPROVED_ANALOGY,
};
use s3a_simd::cosine_similarity;
use crate::snapshot_view::WasmSnapshotView;

fn compute_file_header_crc32(header: &FileHeader) -> u32 {
    let bytes = bytes_of(header);
    let crc_offset = core::mem::offset_of!(FileHeader, header_crc32);
    let mut crc = crc32c::crc32c(&bytes[..crc_offset]);
    crc = crc32c::crc32c_append(crc, &bytes[crc_offset + 4..]);
    crc
}

fn compute_tile_header_crc32(header: &HyperTileHeader) -> u32 {
    let bytes = bytes_of(header);
    let crc_offset = core::mem::offset_of!(HyperTileHeader, header_crc32);
    let mut crc = crc32c::crc32c(&bytes[..crc_offset]);
    crc = crc32c::crc32c_append(crc, &bytes[crc_offset + 4..]);
    crc
}

/// Packs vector embedding and text payload into a standard 528-byte `EmbeddingRecord128` (Tile 1).
/// Guarantees that vector[0..16] contains valid floats for compactor convex hull computation,
/// while vector[16..128] stores embedding dimensions and UTF-8 claim text.
pub fn encode_claim_embedding(
    claim_id: u64,
    timestamp_sec: u64,
    embedding: Option<&[f32]>,
    text: &str,
) -> EmbeddingRecord128 {
    let mut rec = EmbeddingRecord128::new(claim_id, timestamp_sec, [0.0f32; 128]);
    let emb_slice = embedding.unwrap_or(&[]);
    let emb_dim = emb_slice.len().min(64);
    for i in 0..emb_dim.min(16) {
        rec.vector[i] = emb_slice[i];
    }
    let raw_bytes: &mut [u8; 512] = bytemuck::cast_mut(&mut rec.vector);
    raw_bytes[64..66].copy_from_slice(&(emb_dim as u16).to_le_bytes());
    let text_bytes = text.as_bytes();
    let emb_extra_bytes = if emb_dim > 16 { (emb_dim - 16) * 4 } else { 0 };
    let max_text_bytes = (512 - 68 - emb_extra_bytes).max(0);
    let actual_text_len = text_bytes.len().min(max_text_bytes);
    raw_bytes[66..68].copy_from_slice(&(actual_text_len as u16).to_le_bytes());

    if emb_dim > 16 {
        let extra_floats = &emb_slice[16..emb_dim];
        let extra_bytes: &[u8] = bytemuck::cast_slice(extra_floats);
        raw_bytes[68..68 + emb_extra_bytes].copy_from_slice(extra_bytes);
    }
    let text_start = 68 + emb_extra_bytes;
    raw_bytes[text_start..text_start + actual_text_len].copy_from_slice(&text_bytes[..actual_text_len]);
    rec
}

/// Decodes vector embedding and text payload from a standard 528-byte `EmbeddingRecord128` (Tile 1).
pub fn decode_claim_embedding(rec: &EmbeddingRecord128) -> (Option<Vec<f32>>, String) {
    let raw_bytes: &[u8; 512] = bytemuck::cast_ref(&rec.vector);
    let emb_dim = u16::from_le_bytes([raw_bytes[64], raw_bytes[65]]) as usize;
    let text_len = u16::from_le_bytes([raw_bytes[66], raw_bytes[67]]) as usize;

    let mut emb = None;
    if emb_dim > 0 && emb_dim <= 64 {
        let mut v = Vec::with_capacity(emb_dim);
        for i in 0..emb_dim.min(16) {
            v.push(rec.vector[i]);
        }
        if emb_dim > 16 {
            let extra_count = emb_dim - 16;
            let extra_bytes = &raw_bytes[68..68 + extra_count * 4];
            let extra_floats: &[f32] = bytemuck::cast_slice(extra_bytes);
            v.extend_from_slice(extra_floats);
        }
        emb = Some(v);
    }

    let emb_extra_bytes = if emb_dim > 16 && emb_dim <= 64 { (emb_dim - 16) * 4 } else { 0 };
    let text_start = 68 + emb_extra_bytes;
    let text = if text_len > 0 && text_start + text_len <= 512 {
        String::from_utf8_lossy(&raw_bytes[text_start..text_start + text_len]).to_string()
    } else {
        String::new()
    };

    (emb, text)
}

/// In-memory representation of an indexed claim or research note with optional vector embedding.
#[derive(Debug, Clone, PartialEq)]
pub struct ClaimItem {
    pub record: ScholarNoteClaimRecord,
    pub text: String,
    pub embedding: Option<Vec<f32>>,
}

/// Search match returned from vector, lexical, or hybrid claim queries.
#[derive(Debug, Clone, PartialEq)]
pub struct ClaimSearchResult {
    pub claim_id: u64,
    pub text: String,
    pub confidence: f32,
    pub kind: u16,
    pub score: f32,
}

/// In-Memory Scholar Explorer AI Agent API.
/// Provides browser-side agents with zero-copy queries, epistemic pre-flight checks,
/// full-text and vector search over claims/notes, and incremental delta synchronization.
#[derive(Debug, Clone)]
pub struct WasmAgentSnapshot {
    pub manifest: SrsManifestHeader,
    pub workspace: Option<SrsWorkspaceRecord>,
    pub papers: Vec<AcademicPaperRecord>,
    pub edges: Vec<ResearchGraphEdgeRecord>,
    pub human_decisions: Vec<HumanLrsRecord>,
    pub ai_traces: Vec<AiTraceRecord>,
    pub agents: Vec<AgentRegistryRecord>,
    pub concepts: Vec<OntologyConceptRecord>,
    pub axioms: Vec<OntologyAxiomRecord>,
    pub claims: Vec<ClaimItem>,
    pub citation_fingerprints: Vec<CitationFingerprintRecord>,
    pub policy_marks: Vec<MarkRecord>,
    pub pending_delta: Vec<SrsDeltaEntry>,
    pub delta_seq: u64,
    pub last_applied_delta_seq: u64,
    pub statement_log: Vec<SrsDeltaEntry>,
}

impl WasmAgentSnapshot {
    /// Loads snapshot from in-memory byte slice.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, &'static str> {
        let view = WasmSnapshotView::parse(bytes)?;
        let manifest = view.manifest.ok_or("Snapshot missing authoritative Tile 15 (SRS_MANIFEST)")?;

        // Reconstruct claims by pairing ScholarNoteClaimRecord with corresponding EmbeddingRecord128
        let mut claims = Vec::new();
        for claim_rec in &view.claims {
            let matching_emb = view.embeddings.iter().find(|e| e.id == claim_rec.claim_id);
            let (emb, text) = match matching_emb {
                Some(e) => decode_claim_embedding(e),
                None => (None, String::new()),
            };
            claims.push(ClaimItem {
                record: **claim_rec,
                text,
                embedding: emb,
            });
        }

        let policy_marks: Vec<MarkRecord> = view.policy_marks.into_iter().copied().collect();

        let agents = if view.agents.is_empty() {
            s3a_pack_scholar::seed_default_agents()
        } else {
            view.agents.into_iter().copied().collect()
        };

        // Reconstruct chronological statement log from constituent tiles
        let mut statement_log = Vec::new();
        for p in &view.papers {
            if let Ok(e) = SrsDeltaEntry::from_record(ScholarTileType::ACADEMIC_PAPERS, *p) {
                statement_log.push(e);
            }
        }
        for h in &view.human_decisions {
            if let Ok(e) = SrsDeltaEntry::from_record(ScholarTileType::HUMAN_LRS, *h) {
                statement_log.push(e);
            }
        }
        for a in &view.ai_traces {
            if let Ok(e) = SrsDeltaEntry::from_record(ScholarTileType::AI_TRACE_LOG, *a) {
                statement_log.push(e);
            }
        }
        for c in &claims {
            if let Ok(e) = SrsDeltaEntry::from_record(ScholarTileType::NOTE_CLAIMS, &c.record) {
                statement_log.push(e);
            }
        }

        Ok(Self {
            manifest,
            workspace: view.workspace,
            papers: view.papers.into_iter().copied().collect(),
            edges: view.edges.into_iter().copied().collect(),
            human_decisions: view.human_decisions.into_iter().copied().collect(),
            ai_traces: view.ai_traces.into_iter().copied().collect(),
            agents,
            concepts: view.concepts.into_iter().copied().collect(),
            axioms: view.axioms.into_iter().copied().collect(),
            claims,
            citation_fingerprints: view.citation_fingerprints.into_iter().copied().collect(),
            policy_marks,
            pending_delta: Vec::new(),
            delta_seq: 1,
            last_applied_delta_seq: 0,
            statement_log,
        })
    }

    /// Exports only statements and delta records recorded after `cursor`.
    /// Returns (serialized_bytes, new_cursor, record_count).
    pub fn export_since(&self, cursor: u64) -> Result<(Vec<u8>, u64, u32), String> {
        let total = self.statement_log.len() as u64;
        if cursor > total {
            return Err(format!(
                "Cursor {} is ahead of current statement count {}",
                cursor, total
            ));
        }

        let slice = &self.statement_log[cursor as usize..];
        let record_count = slice.len() as u32;
        let new_cursor = total;

        let mut packet = SrsSyncDeltaPacket::with_timestamp(
            self.manifest.origin_session_uuid,
            self.manifest.snapshot_uuid,
            cursor + 1,
            self.manifest.created_at_sec,
        );
        packet.entries = slice.to_vec();
        let bytes = packet.export_bytes();

        Ok((bytes, new_cursor, record_count))
    }


    /// Evaluates if an agent's proposed generation requires raising its hand to the Teacher Assistant.
    pub fn preflight_agent_check(
        &self,
        candidate_confidence: f32,
        concept_id: u64,
        has_approved_analogy: bool,
    ) -> Result<(), &'static str> {
        if candidate_confidence < 0.70 {
            return Err("RAISE_HAND_TO_TA: Epistemic confidence below 0.70 threshold");
        }

        // Verify concept is grounded in the snapshot ontology
        if !self.concepts.is_empty() && !self.concepts.iter().any(|c| c.concept_id == concept_id) {
            return Err("RAISE_HAND_TO_TA: Concept ID unbound in snapshot ontology");
        }

        // Check if an unverified analogy is being attempted
        if !has_approved_analogy {
            let has_approved = self.axioms.iter().any(|a| {
                a.subject_concept_id == concept_id && a.predicate_id == PREDICATE_HAS_APPROVED_ANALOGY
            });
            if !has_approved {
                return Err("RAISE_HAND_TO_TA: Proposed analogy is not approved in snapshot axioms");
            }
        }

        Ok(())
    }

    /// Evaluates if an agent is authorized by the registry to perform an action.
    pub fn assert_agent_allows(
        &self,
        agent_id: u64,
        verb_bit: Option<u32>,
        tool_bit: Option<u32>,
    ) -> Result<(), AgentDenied> {
        let agent = self.agents.iter().find(|a| a.agent_id == agent_id).ok_or(AgentDenied::AgentDisabled)?;
        agent.assert_allows(verb_bit, tool_bit)
    }

    /// Complete bounding assertion enforcing both static allowlists/denials
    /// AND dynamic supervisory clearance when AGENT_FLAG_REQUIRES_HUMAN_GATE is set.
    pub fn assert_agent_allows_gated(
        &self,
        agent_id: u64,
        verb_bit: Option<u32>,
        tool_bit: Option<u32>,
        is_human_cleared: bool,
    ) -> Result<(), AgentDenied> {
        let agent = self.agents.iter().find(|a| a.agent_id == agent_id).ok_or(AgentDenied::AgentDisabled)?;
        agent.assert_allows_gated(verb_bit, tool_bit, is_human_cleared)
    }

    /// Finds all approved analogy records for a concept.
    pub fn find_approved_analogies(&self, concept_id: u64) -> Vec<OntologyAxiomRecord> {
        self.axioms
            .iter()
            .filter(|a| a.subject_concept_id == concept_id && a.predicate_id == PREDICATE_HAS_APPROVED_ANALOGY)
            .copied()
            .collect()
    }

    /// Exports the snapshot into an immutable .snapshot.s3a byte buffer.
    pub fn export_bytes(&self) -> Vec<u8> {
        let mut tiles: Vec<(u32, Vec<u8>, u32, u64, u64)> = Vec::new();

        if let Some(ref ws) = self.workspace {
            tiles.push((ScholarTileType::SRS_WORKSPACE, bytes_of(ws).to_vec(), 1, 0, 0));
        }
        if !self.papers.is_empty() {
            let min_ts = self.papers.iter().map(|p| p.timestamp_sec).min().unwrap_or(0);
            let max_ts = self.papers.iter().map(|p| p.timestamp_sec).max().unwrap_or(0);
            tiles.push((ScholarTileType::ACADEMIC_PAPERS, cast_slice(&self.papers).to_vec(), self.papers.len() as u32, min_ts, max_ts));
        }
        if !self.edges.is_empty() {
            let min_ts = self.edges.iter().map(|e| e.timestamp_sec).min().unwrap_or(0);
            let max_ts = self.edges.iter().map(|e| e.timestamp_sec).max().unwrap_or(0);
            tiles.push((ScholarTileType::RESEARCH_GRAPH, cast_slice(&self.edges).to_vec(), self.edges.len() as u32, min_ts, max_ts));
        }
        if !self.human_decisions.is_empty() {
            let min_ts = self.human_decisions.iter().map(|h| h.timestamp_sec).min().unwrap_or(0);
            let max_ts = self.human_decisions.iter().map(|h| h.timestamp_sec).max().unwrap_or(0);
            tiles.push((ScholarTileType::HUMAN_LRS, cast_slice(&self.human_decisions).to_vec(), self.human_decisions.len() as u32, min_ts, max_ts));
        }
        if !self.ai_traces.is_empty() {
            let min_ts = self.ai_traces.iter().map(|a| a.timestamp_sec).min().unwrap_or(0);
            let max_ts = self.ai_traces.iter().map(|a| a.timestamp_sec).max().unwrap_or(0);
            tiles.push((ScholarTileType::AI_TRACE_LOG, cast_slice(&self.ai_traces).to_vec(), self.ai_traces.len() as u32, min_ts, max_ts));
        }
        if !self.agents.is_empty() {
            tiles.push((ScholarTileType::AGENT_REGISTRY, cast_slice(&self.agents).to_vec(), self.agents.len() as u32, 0, 0));
        }
        if !self.concepts.is_empty() {
            tiles.push((OntologyTileType::ONTOLOGY_CONCEPT, cast_slice(&self.concepts).to_vec(), self.concepts.len() as u32, 0, 0));
        }
        if !self.axioms.is_empty() {
            tiles.push((OntologyTileType::ONTOLOGY_AXIOM, cast_slice(&self.axioms).to_vec(), self.axioms.len() as u32, 0, 0));
        }
        if !self.claims.is_empty() {
            let claim_records: Vec<ScholarNoteClaimRecord> = self.claims.iter().map(|c| c.record).collect();
            let min_ts = claim_records.iter().map(|c| c.timestamp_sec).min().unwrap_or(0);
            let max_ts = claim_records.iter().map(|c| c.timestamp_sec).max().unwrap_or(0);
            tiles.push((ScholarTileType::NOTE_CLAIMS, cast_slice(&claim_records).to_vec(), claim_records.len() as u32, min_ts, max_ts));

            let embedding_records: Vec<EmbeddingRecord128> = self.claims.iter().map(|c| {
                encode_claim_embedding(c.record.claim_id, c.record.timestamp_sec, c.embedding.as_deref(), &c.text)
            }).collect();
            tiles.push((TileType::EMBEDDING, cast_slice(&embedding_records).to_vec(), embedding_records.len() as u32, min_ts, max_ts));
        }
        if !self.policy_marks.is_empty() {
            let min_ts = self.policy_marks.iter().map(|m| m.timestamp).min().unwrap_or(0);
            let max_ts = self.policy_marks.iter().map(|m| m.timestamp).max().unwrap_or(0);
            tiles.push((TileType::MARKS, cast_slice(&self.policy_marks).to_vec(), self.policy_marks.len() as u32, min_ts, max_ts));
        }
        if !self.citation_fingerprints.is_empty() {
            let min_ts = self.citation_fingerprints.iter().map(|f| f.fetched_at).min().unwrap_or(0);
            let max_ts = self.citation_fingerprints.iter().map(|f| f.fetched_at).max().unwrap_or(0);
            tiles.push((ScholarTileType::CITATION_FINGERPRINT, cast_slice(&self.citation_fingerprints).to_vec(), self.citation_fingerprints.len() as u32, min_ts, max_ts));
        }

        let payload_tile_count = tiles.len();
        let total_tile_count = (payload_tile_count + 1) as u32; // including Tile 15 manifest

        let mut manifest = self.manifest;
        manifest.magic = SrsManifestHeader::MAGIC;
        manifest.schema_version = SCHOLAR_LAYOUT_VERSION_V1;
        manifest.archive_flags |= SrsManifestHeader::FLAG_WASM_EXPORTED;
        manifest.tile_count = total_tile_count;
        manifest.crc32c_checksum = manifest.compute_checksum();

        tiles.push((ScholarTileType::SRS_MANIFEST, bytes_of(&manifest).to_vec(), 1, manifest.created_at_sec, manifest.created_at_sec));

        // Allocate total file buffer: 512-byte header + total_tile_count * 128 KB
        let total_size = FILE_HEADER_SIZE + (total_tile_count as usize) * HYPER_TILE_SIZE;
        let mut out = vec![0u8; total_size];

        // 1. Populate FileHeader Slot A
        let created_ts = if self.manifest.created_at_sec > 0 { self.manifest.created_at_sec } else { 1600000000 };
        let mut file_header = FileHeader::new(total_tile_count, created_ts);
        file_header.addon_mask = ADDON_EDU | ADDON_ONTOLOGY;
        file_header.file_uuid = self.manifest.snapshot_uuid;
        file_header.header_crc32 = compute_file_header_crc32(&file_header);

        let header_bytes = bytes_of(&file_header);
        out[FILE_HEADER_SLOT_A_OFFSET..FILE_HEADER_SLOT_A_OFFSET + FILE_HEADER_SLOT_SIZE]
            .copy_from_slice(header_bytes);

        // 2. Write each Hyper-Tile (512B HyperTileHeader + payload + padding)
        for (idx, (tile_type, payload, count, min_ts, max_ts)) in tiles.iter().enumerate() {
            let offset = FILE_HEADER_SIZE + idx * HYPER_TILE_SIZE;

            let mut tile_hdr = HyperTileHeader::new((idx + 1) as u64, *tile_type);
            tile_hdr.record_count = *count;
            tile_hdr.min_timestamp = *min_ts;
            tile_hdr.max_timestamp = *max_ts;
            tile_hdr.payload_bytes = payload.len() as u32;
            tile_hdr.generation = 1;
            tile_hdr.tile_index = idx as u32;
            tile_hdr.file_uuid = self.manifest.snapshot_uuid;
            tile_hdr.data_crc32 = crc32c::crc32c(payload);
            tile_hdr.header_crc32 = compute_tile_header_crc32(&tile_hdr);

            let hdr_bytes = bytes_of(&tile_hdr);
            out[offset..offset + HYPER_TILE_HEADER_SIZE].copy_from_slice(hdr_bytes);

            let payload_start = offset + HYPER_TILE_HEADER_SIZE;
            out[payload_start..payload_start + payload.len()].copy_from_slice(payload);
        }

        s3a_engine::seal_buffer(&out, s3a_engine::DEFAULT_SEAL_K, s3a_engine::DEFAULT_SEAL_M).unwrap_or(out)
    }

    /// Exports pending delta records since the last sync as a compact byte packet.
    pub fn export_delta_bytes(&mut self) -> Vec<u8> {
        let mut packet = SrsSyncDeltaPacket::with_timestamp(
            self.manifest.origin_session_uuid,
            self.manifest.snapshot_uuid,
            self.delta_seq,
            self.manifest.created_at_sec,
        );
        packet.entries = std::mem::take(&mut self.pending_delta);
        self.delta_seq += 1;
        packet.export_bytes()
    }

    /// Applies an incoming sync delta packet into the local snapshot state.
    /// Follows strict two-phase commit: validates session, base snapshot, monotonic delta sequence,
    /// and all tile entries before mutating state.
    pub fn apply_delta_bytes(&mut self, bytes: &[u8]) -> Result<usize, String> {
        let packet = SrsSyncDeltaPacket::parse_bytes(bytes).map_err(|e| format!("{}", e))?;

        // 1. Session and Base Snapshot validation
        if packet.header.session_uuid != self.manifest.origin_session_uuid {
            return Err("SessionMismatch".to_string());
        }
        if packet.header.base_snapshot_uuid != self.manifest.snapshot_uuid {
            return Err("BaseSnapshotMismatch".to_string());
        }

        // 2. Monotonic sequence check and idempotency
        if packet.header.delta_seq <= self.last_applied_delta_seq {
            // Already applied: idempotent no-op
            return Ok(0);
        }
        if packet.header.delta_seq > self.last_applied_delta_seq + 1 {
            return Err(format!(
                "SequenceGap: expected {}, got {}",
                self.last_applied_delta_seq + 1,
                packet.header.delta_seq
            ));
        }

        // 3. Phase 1: Validate all entries (Fail closed)
        for entry in &packet.entries {
            match entry.tile_type {
                ScholarTileType::AGENT_REGISTRY => {
                    return Err(format!("ForbiddenTileInDelta({})", entry.tile_type));
                }
                TileType::MARKS => {
                    let mark: MarkRecord = *bytemuck::from_bytes(&entry.record_bytes()[..64]);
                    // Security policy for marks in deltas:
                    // Only NEEDS_REVIEW and CONSENT_WITHDRAWN can be SET
                    if mark.is_set() {
                        if mark.mark_kind != s3a_core::MARK_NEEDS_REVIEW
                            && mark.mark_kind != s3a_core::MARK_CONSENT_WITHDRAWN
                        {
                            return Err(format!(
                                "UnauthorizedMark: cannot set mark_kind {}",
                                mark.mark_kind
                            ));
                        }
                    }
                    // Cannot CLEAR legal hold, quarantine, or consent withdrawn
                    if mark.is_cleared() {
                        if mark.mark_kind == s3a_core::MARK_LEGAL_HOLD
                            || mark.mark_kind == s3a_core::MARK_QUARANTINE
                            || mark.mark_kind == s3a_core::MARK_CONSENT_WITHDRAWN
                        {
                            return Err(format!(
                                "UnauthorizedMark: cannot clear mark_kind {}",
                                mark.mark_kind
                            ));
                        }
                    }
                }
                ScholarTileType::HUMAN_LRS
                | ScholarTileType::AI_TRACE_LOG
                | ScholarTileType::ACADEMIC_PAPERS
                | ScholarTileType::RESEARCH_GRAPH
                | ScholarTileType::SRS_WORKSPACE
                | ScholarTileType::NOTE_CLAIMS
                | ScholarTileType::CITATION_FINGERPRINT
                | TileType::EMBEDDING
                | OntologyTileType::ONTOLOGY_CONCEPT
                | OntologyTileType::ONTOLOGY_AXIOM => {
                    // Valid tile types
                }
                other => {
                    return Err(format!("UnknownTileType({})", other));
                }
            }
        }

        // 4. Phase 2: Commit all entries
        let mut count = 0;
        for entry in &packet.entries {
            match entry.tile_type {
                ScholarTileType::HUMAN_LRS => {
                    let r: HumanLrsRecord = *bytemuck::from_bytes(&entry.record_bytes()[..64]);
                    self.human_decisions.push(r);
                    count += 1;
                }
                ScholarTileType::AI_TRACE_LOG => {
                    let r: AiTraceRecord = *bytemuck::from_bytes(&entry.record_bytes()[..64]);
                    self.ai_traces.push(r);
                    count += 1;
                }
                ScholarTileType::ACADEMIC_PAPERS => {
                    let r: AcademicPaperRecord = *bytemuck::from_bytes(&entry.record_bytes()[..64]);
                    self.papers.push(r);
                    count += 1;
                }
                ScholarTileType::RESEARCH_GRAPH => {
                    let r: ResearchGraphEdgeRecord = *bytemuck::from_bytes(&entry.record_bytes()[..64]);
                    self.edges.push(r);
                    count += 1;
                }
                ScholarTileType::SRS_WORKSPACE => {
                    let r: SrsWorkspaceRecord = *bytemuck::from_bytes(&entry.record_bytes()[..64]);
                    self.workspace = Some(r);
                    count += 1;
                }
                ScholarTileType::NOTE_CLAIMS => {
                    let r: ScholarNoteClaimRecord = *bytemuck::from_bytes(&entry.record_bytes()[..160]);
                    self.claims.push(ClaimItem {
                        record: r,
                        text: String::new(),
                        embedding: None,
                    });
                    count += 1;
                }
                ScholarTileType::CITATION_FINGERPRINT => {
                    let r: CitationFingerprintRecord = *bytemuck::from_bytes(&entry.record_bytes()[..96]);
                    self.citation_fingerprints.push(r);
                    count += 1;
                }
                TileType::EMBEDDING => {
                    // Embeddings are not transported in standard SrsDeltaEntry due to size limits
                }
                TileType::MARKS => {
                    let mark: MarkRecord = *bytemuck::from_bytes(&entry.record_bytes()[..64]);
                    self.policy_marks.push(mark);
                    count += 1;
                }
                OntologyTileType::ONTOLOGY_CONCEPT => {
                    let r: OntologyConceptRecord = *bytemuck::from_bytes(&entry.record_bytes()[..64]);
                    self.concepts.push(r);
                    count += 1;
                }
                OntologyTileType::ONTOLOGY_AXIOM => {
                    let r: OntologyAxiomRecord = *bytemuck::from_bytes(&entry.record_bytes()[..64]);
                    self.axioms.push(r);
                    count += 1;
                }
                _ => {}
            }
        }

        self.statement_log.extend(packet.entries.iter().cloned());
        self.last_applied_delta_seq = packet.header.delta_seq;
        self.manifest.total_records += count as u64;
        Ok(count)
    }

    /// Adds a research claim or note with integrated Bloom filtering and optional vector embedding.
    pub fn add_claim(
        &mut self,
        claim_id: u64,
        author_hash: u64,
        confidence: f32,
        kind: u16,
        text: &str,
        embedding: Option<&[f32]>,
    ) -> u64 {
        let now = self.manifest.created_at_sec;
        let record = ScholarNoteClaimRecord::new(
            claim_id,
            now,
            self.manifest.origin_session_uuid,
            author_hash,
            confidence,
            kind,
            text,
        );

        // Record as an AI Trace claim step for provenance
        let trace = AiTraceRecord::new(
            self.manifest.origin_session_uuid,
            now,
            author_hash,
            STEP_CLAIM,
            confidence,
            s3a_core::S3ACoordinate::default(),
            (claim_id % 1000) as u32,
        );
        self.ai_traces.push(trace);
        if let Ok(entry) = SrsDeltaEntry::from_record(ScholarTileType::AI_TRACE_LOG, &trace) {
            self.pending_delta.push(entry);
            self.statement_log.push(entry);
        }

        // Push claim record to delta
        if let Ok(entry) = SrsDeltaEntry::from_record(ScholarTileType::NOTE_CLAIMS, &record) {
            self.pending_delta.push(entry);
            self.statement_log.push(entry);
        }

        self.claims.push(ClaimItem {
            record,
            text: text.to_string(),
            embedding: embedding.map(|e| e.to_vec()),
        });


        claim_id
    }

    /// Evaluates whether a claim is hidden by active policy marks (CONSENT_WITHDRAWN or QUARANTINE).
    pub fn is_claim_hidden(&self, claim_id: u64, claim_idx: usize) -> bool {
        self.is_claim_marked(claim_id, claim_idx, s3a_core::MARK_CONSENT_WITHDRAWN)
            || self.is_claim_marked(claim_id, claim_idx, s3a_core::MARK_QUARANTINE)
    }

    /// Evaluates the active status of a mark_kind on a claim.
    /// Follows append-only semantics where the latest mark per target wins.
    pub fn is_claim_marked(&self, claim_id: u64, claim_idx: usize, mark_kind: u32) -> bool {
        let mut latest_state: Option<bool> = None;
        for m in &self.policy_marks {
            if m.mark_kind != mark_kind {
                continue;
            }
            let matches_type = m.target_tile_type == ScholarTileType::NOTE_CLAIMS
                || m.target_tile_type == TileType::EMBEDDING
                || m.target_tile_type == 0;
            if !matches_type {
                continue;
            }
            let matches_target = m.target_tile_id == claim_id
                || (m.record_index == claim_idx as u32)
                || (m.target_tile_id == 0 && m.record_index == claim_id as u32)
                || m.is_whole_tile();
            if matches_target {
                latest_state = Some(m.is_set());
            }
        }
        latest_state.unwrap_or(false)
    }

    /// Full-text search over indexed claims using token Bloom filtering followed by lexical scoring.
    pub fn search_claims_text(&self, query: &str, top_k: usize, include_hidden: bool) -> Vec<ClaimSearchResult> {
        let mut results = Vec::new();
        let query_words: Vec<String> = query
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(|w| w.to_ascii_lowercase())
            .collect();

        if query_words.is_empty() {
            return results;
        }

        for (idx, claim) in self.claims.iter().enumerate() {
            if !include_hidden && self.is_claim_hidden(claim.record.claim_id, idx) {
                continue;
            }

            // Instant 32-bit Bloom filter check
            if !claim.record.matches_query(query) {
                continue;
            }

            let claim_lower = claim.text.to_ascii_lowercase();
            let mut matches = 0;
            for qw in &query_words {
                if claim_lower.contains(qw) {
                    matches += 1;
                }
            }

            if matches > 0 {
                let score = matches as f32 / query_words.len() as f32;
                results.push(ClaimSearchResult {
                    claim_id: claim.record.claim_id,
                    text: claim.text.clone(),
                    confidence: claim.record.confidence,
                    kind: claim.record.kind,
                    score,
                });
            }
        }

        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(core::cmp::Ordering::Equal));
        if results.len() > top_k {
            results.truncate(top_k);
        }
        results
    }

    /// Dense vector search over indexed claims using SIMD-accelerated cosine similarity.
    pub fn search_claims_vector(
        &self,
        query_embedding: &[f32],
        top_k: usize,
        min_score: f32,
        include_hidden: bool,
    ) -> Vec<ClaimSearchResult> {
        let mut results = Vec::new();

        for (idx, claim) in self.claims.iter().enumerate() {
            if !include_hidden && self.is_claim_hidden(claim.record.claim_id, idx) {
                continue;
            }

            if let Some(ref emb) = claim.embedding {
                let sim = cosine_similarity(query_embedding, emb);
                if sim >= min_score {
                    results.push(ClaimSearchResult {
                        claim_id: claim.record.claim_id,
                        text: claim.text.clone(),
                        confidence: claim.record.confidence,
                        kind: claim.record.kind,
                        score: sim,
                    });
                }
            }
        }

        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(core::cmp::Ordering::Equal));
        if results.len() > top_k {
            results.truncate(top_k);
        }
        results
    }

    /// Hybrid search combining lexical Bloom token matching with SIMD vector cosine similarity.
    pub fn search_claims_hybrid(
        &self,
        query: &str,
        query_embedding: Option<&[f32]>,
        weight_text: f32,
        weight_vector: f32,
        top_k: usize,
        include_hidden: bool,
    ) -> Vec<ClaimSearchResult> {
        let query_words: Vec<String> = query
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(|w| w.to_ascii_lowercase())
            .collect();

        let mut results = Vec::new();

        for (idx, claim) in self.claims.iter().enumerate() {
            if !include_hidden && self.is_claim_hidden(claim.record.claim_id, idx) {
                continue;
            }

            let text_score = if query_words.is_empty() {
                0.0
            } else if claim.record.matches_query(query) {
                let claim_lower = claim.text.to_ascii_lowercase();
                let matches = query_words.iter().filter(|qw| claim_lower.contains(*qw)).count();
                matches as f32 / query_words.len() as f32
            } else {
                0.0
            };

            let vec_score = match (query_embedding, &claim.embedding) {
                (Some(q_emb), Some(c_emb)) => cosine_similarity(q_emb, c_emb),
                _ => 0.0,
            };

            let total_score = weight_text * text_score + weight_vector * vec_score;
            if total_score > 0.0 {
                results.push(ClaimSearchResult {
                    claim_id: claim.record.claim_id,
                    text: claim.text.clone(),
                    confidence: claim.record.confidence,
                    kind: claim.record.kind,
                    score: total_score,
                });
            }
        }

        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(core::cmp::Ordering::Equal));
        if results.len() > top_k {
            results.truncate(top_k);
        }
        results
    }
}

