use bytemuck::from_bytes;
use s3a_core::{
    EmbeddingRecord128, FileHeader, HyperTileHeader, MarkRecord, TileType,
    FILE_HEADER_SIZE, FILE_HEADER_SLOT_A_OFFSET, FILE_HEADER_SLOT_B_OFFSET,
    FILE_HEADER_SLOT_SIZE, HYPER_TILE_HEADER_SIZE, HYPER_TILE_SIZE, ADDON_EDU,
};
use s3a_pack_scholar::{
    AcademicPaperRecord, AgentRegistryRecord, AiTraceRecord, CitationFingerprintRecord,
    HumanLrsRecord, ResearchGraphEdgeRecord, ScholarNoteClaimRecord, ScholarTileType,
    SrsManifestHeader, SrsWorkspaceRecord,
};
use s3a_pack_ontology::{OntologyAxiomRecord, OntologyConceptRecord, OntologyTileType};

/// In-Memory Zero-Copy WebAssembly Snapshot Viewer.
/// Operates directly on browser-provided memory buffers (Uint8Array / &[u8]) without filesystem IO.
#[derive(Debug, Clone)]
pub struct WasmSnapshotView<'a> {
    pub file_header: FileHeader,
    pub manifest: Option<SrsManifestHeader>,
    pub workspace: Option<SrsWorkspaceRecord>,
    pub papers: Vec<&'a AcademicPaperRecord>,
    pub edges: Vec<&'a ResearchGraphEdgeRecord>,
    pub human_decisions: Vec<&'a HumanLrsRecord>,
    pub ai_traces: Vec<&'a AiTraceRecord>,
    pub agents: Vec<&'a AgentRegistryRecord>,
    pub concepts: Vec<&'a OntologyConceptRecord>,
    pub axioms: Vec<&'a OntologyAxiomRecord>,
    pub claims: Vec<&'a ScholarNoteClaimRecord>,
    pub citation_fingerprints: Vec<&'a CitationFingerprintRecord>,
    pub embeddings: Vec<&'a EmbeddingRecord128>,
    pub policy_marks: Vec<&'a MarkRecord>,
}

impl<'a> WasmSnapshotView<'a> {
    /// Parses an in-memory S3A snapshot byte buffer.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, &'static str> {
        if bytes.len() < FILE_HEADER_SIZE {
            return Err("Buffer too small for S3A dual file header");
        }

        let slot_a: &FileHeader = from_bytes(&bytes[FILE_HEADER_SLOT_A_OFFSET..FILE_HEADER_SLOT_A_OFFSET + FILE_HEADER_SLOT_SIZE]);
        let slot_b: &FileHeader = from_bytes(&bytes[FILE_HEADER_SLOT_B_OFFSET..FILE_HEADER_SLOT_B_OFFSET + FILE_HEADER_SLOT_SIZE]);

        let header = match (slot_a.verify(), slot_b.verify()) {
            (Ok(_), Ok(_)) => {
                if slot_a.generation >= slot_b.generation { *slot_a } else { *slot_b }
            }
            (Ok(_), Err(_)) => *slot_a,
            (Err(_), Ok(_)) => *slot_b,
            (Err(_), Err(_)) => return Err("Both S3A file header slots are corrupted"),
        };

        if (header.addon_mask & ADDON_EDU) == 0 {
            return Err("Snapshot missing mandatory ADDON_EDU capability mask");
        }

        let mut manifest = None;
        let mut workspace = None;
        let mut papers = Vec::new();
        let mut edges = Vec::new();
        let mut human_decisions = Vec::new();
        let mut ai_traces = Vec::new();
        let mut agents = Vec::new();
        let mut concepts = Vec::new();
        let mut axioms = Vec::new();
        let mut claims = Vec::new();
        let mut citation_fingerprints = Vec::new();
        let mut embeddings = Vec::new();
        let mut policy_marks = Vec::new();

        let tile_count = header.tile_count as usize;
        for i in 0..tile_count {
            let offset = FILE_HEADER_SIZE + i * HYPER_TILE_SIZE;
            if offset + HYPER_TILE_SIZE > bytes.len() {
                break; // Premature EOF
            }

            let tile_header: &HyperTileHeader = from_bytes(&bytes[offset..offset + HYPER_TILE_HEADER_SIZE]);
            let payload_start = offset + HYPER_TILE_HEADER_SIZE;
            let payload_end = payload_start + tile_header.payload_bytes as usize;
            if payload_end > offset + HYPER_TILE_SIZE {
                return Err("Corrupted hyper-tile payload length exceeds tile size");
            }
            let payload = &bytes[payload_start..payload_end];

            match tile_header.tile_type {
                ScholarTileType::SRS_MANIFEST => {
                    let recs: &[SrsManifestHeader] = bytemuck::cast_slice(payload);
                    if let Some(&m) = recs.first() {
                        manifest = Some(m);
                    }
                }
                ScholarTileType::SRS_WORKSPACE => {
                    let recs: &[SrsWorkspaceRecord] = bytemuck::cast_slice(payload);
                    if let Some(&w) = recs.first() {
                        workspace = Some(w);
                    }
                }
                ScholarTileType::ACADEMIC_PAPERS => {
                    let recs: &[AcademicPaperRecord] = bytemuck::cast_slice(payload);
                    papers.extend(recs.iter());
                }
                ScholarTileType::RESEARCH_GRAPH => {
                    let recs: &[ResearchGraphEdgeRecord] = bytemuck::cast_slice(payload);
                    edges.extend(recs.iter());
                }
                ScholarTileType::HUMAN_LRS => {
                    let recs: &[HumanLrsRecord] = bytemuck::cast_slice(payload);
                    human_decisions.extend(recs.iter());
                }
                ScholarTileType::AI_TRACE_LOG => {
                    let recs: &[AiTraceRecord] = bytemuck::cast_slice(payload);
                    ai_traces.extend(recs.iter());
                }
                ScholarTileType::AGENT_REGISTRY => {
                    let recs: &[AgentRegistryRecord] = bytemuck::cast_slice(payload);
                    agents.extend(recs.iter());
                }
                ScholarTileType::NOTE_CLAIMS => {
                    let recs: &[ScholarNoteClaimRecord] = bytemuck::cast_slice(payload);
                    claims.extend(recs.iter());
                }
                ScholarTileType::CITATION_FINGERPRINT => {
                    let recs: &[CitationFingerprintRecord] = bytemuck::cast_slice(payload);
                    citation_fingerprints.extend(recs.iter());
                }
                TileType::EMBEDDING => {
                    let recs: &[EmbeddingRecord128] = bytemuck::cast_slice(payload);
                    embeddings.extend(recs.iter());
                }
                TileType::MARKS => {
                    let recs: &[MarkRecord] = bytemuck::cast_slice(payload);
                    policy_marks.extend(recs.iter());
                }
                OntologyTileType::ONTOLOGY_CONCEPT => {
                    let recs: &[OntologyConceptRecord] = bytemuck::cast_slice(payload);
                    concepts.extend(recs.iter());
                }
                OntologyTileType::ONTOLOGY_AXIOM => {
                    let recs: &[OntologyAxiomRecord] = bytemuck::cast_slice(payload);
                    axioms.extend(recs.iter());
                }
                _ => {}
            }
        }

        Ok(Self {
            file_header: header,
            manifest,
            workspace,
            papers,
            edges,
            human_decisions,
            ai_traces,
            agents,
            concepts,
            axioms,
            claims,
            citation_fingerprints,
            embeddings,
            policy_marks,
        })
    }
}
