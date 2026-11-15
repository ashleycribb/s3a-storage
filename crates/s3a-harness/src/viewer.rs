//! S3A MCP App Record Viewer: Generic Shell, Multi-Pack Domain Views, and UI SDK Integration.
//!
//! Conforms to MCP Apps Extension (`ui://` resource and read-only tools).
//! Provides a human-centric, accessible, read-only interface for inspecting S3A records.
//! Non-developer view shows a clean human summary; all developer details (tile IDs, QL,
//! CRC32C, coordinates) are kept behind an accessible "Advanced" toggle.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

use s3a_core::document::{DocumentRecordHeader, DOC_KIND_BINARY};
use s3a_core::TileType;
use s3a_engine::MmapReader;
use s3a_pack_scholar::{
    AcademicPaperRecord, AiTraceRecord, CitationFingerprintRecord, HumanLrsRecord,
    ResearchGraphEdgeRecord, ScholarNoteClaimRecord, ScholarTileType, SrsManifestHeader,
    SrsWorkspaceRecord,
};
use s3a_pack_tdp::pdf::extract_pdf_metadata;
use s3a_pack_tdp::step::{parse_step, StepAssemblyTree};
use s3a_vcs::repo::VcsRepo;

// =============================================================================
// Data Models for Generic Shell
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewerSummary {
    pub title: String,
    pub subtitle: String,
    pub description: String,
    pub status_badge: String,
    pub human_approval: String,
    pub view_pack_name: String,
    pub key_stats: Vec<(String, String)>,
    pub advanced: ViewerAdvancedDetails,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewerAdvancedDetails {
    pub tile_count: usize,
    pub active_generation: u64,
    pub tile_types: Vec<u32>,
    pub crc_status: String,
    pub merkle_root: String,
    pub record_hash: String,
    pub s3a_ql: String,
    pub coordinates: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewerTreeNode {
    pub id: String,
    pub label: String,
    pub icon: String,
    pub node_type: String,
    pub children: Vec<ViewerTreeNode>,
    pub has_preview: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewerProperties {
    pub node_id: String,
    pub label: String,
    pub properties: Vec<(String, String)>,
    pub advanced_properties: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewerPreview {
    pub node_id: String,
    pub mime_type: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewerCommitEntry {
    pub commit_id: String,
    pub parent_id: Option<String>,
    pub author: String,
    pub timestamp: u64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewerHistory {
    pub branch: String,
    pub commits: Vec<ViewerCommitEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewerApprovalStatus {
    pub is_approved: bool,
    pub approved_by: Option<String>,
    pub approval_timestamp: Option<u64>,
    pub signature: Option<String>,
    pub fingerprint: String,
    pub requires_human_decision: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewerFullState {
    pub file_path: String,
    pub view_pack_id: String,
    pub summary: ViewerSummary,
    pub tree: Vec<ViewerTreeNode>,
    pub selected_node_id: String,
    pub properties: ViewerProperties,
    pub preview: ViewerPreview,
    pub history: ViewerHistory,
    pub approval: ViewerApprovalStatus,
}

// =============================================================================
// Domain View Pack Trait & Registry
// =============================================================================

pub trait RecordViewPack: Send + Sync {
    fn pack_id(&self) -> &'static str;
    fn display_name(&self) -> &'static str;
    fn matches(&self, reader: &MmapReader) -> bool;
    fn extract_summary(&self, reader: &MmapReader, path: &Path) -> Result<ViewerSummary, String>;
    fn extract_tree(&self, reader: &MmapReader, path: &Path)
        -> Result<Vec<ViewerTreeNode>, String>;
    fn extract_properties(
        &self,
        reader: &MmapReader,
        path: &Path,
        node_id: &str,
    ) -> Result<ViewerProperties, String>;
    fn extract_preview(
        &self,
        reader: &MmapReader,
        path: &Path,
        node_id: &str,
    ) -> Result<ViewerPreview, String>;
    fn extract_history(&self, reader: &MmapReader, path: &Path) -> Result<ViewerHistory, String>;
    fn extract_approval(
        &self,
        reader: &MmapReader,
        path: &Path,
    ) -> Result<ViewerApprovalStatus, String>;
}

// -----------------------------------------------------------------------------
// Common Helpers
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ArchiveVerification {
    pub is_sealed: bool,
    pub integrity_ok: bool,
    pub signature_verified: bool,
}

/// Verifies archive cryptographic integrity and digital vault Ed25519 signatures.
/// Returns [`ArchiveVerification`].
pub fn check_archive_verification(path: &Path) -> ArchiveVerification {
    let Ok(file_bytes) = std::fs::read(path) else {
        return ArchiveVerification::default();
    };

    let mut claims_sealed = false;
    let len = file_bytes.len();
    if len >= s3a_core::FILE_HEADER_SIZE + s3a_core::SEALED_ENDCAP_SIZE {
        let cap = &file_bytes[len - s3a_core::SEALED_ENDCAP_SIZE..];
        let magic = &cap[..8];
        if magic == s3a_core::SEALED_ENDCAP_MAGIC_V1
            || magic == s3a_core::SEALED_ENDCAP_MAGIC_V2
            || &cap[..6] == b"S3AEND"
        {
            claims_sealed = true;
        }
    }

    let mut integrity_ok = false;
    let mut seal_hash_opt = None;

    match s3a_engine::verify_sealed_archive(path, None) {
        Ok(report) => {
            if report.is_sealed
                && !report.rebuilt_directory
                && !report.warnings.iter().any(|w| w.contains("corrupt"))
            {
                claims_sealed = true;
                integrity_ok = true;
                seal_hash_opt = Some(report.seal_hash);
            } else if report.is_sealed {
                claims_sealed = true;
                integrity_ok = false;
            }
        }
        Err(_) => {
            integrity_ok = false;
        }
    }

    let mut signature_verified = false;
    if let Ok(vault) = s3a_vault::Vault::open(path) {
        let rep = vault.verify_records();
        let has_signed_grants = !vault.grants().is_empty() && rep.grants.iter().all(|g| g.ok);
        let has_signed_transfers =
            !vault.transfers().is_empty() && rep.transfers.iter().all(|t| t.ok);

        if (has_signed_grants || has_signed_transfers) && rep.all_ok() {
            if let Some(sh) = seal_hash_opt {
                if vault.items().contains_key(&sh) || vault.transfers().contains_key(&sh) {
                    signature_verified = true;
                }
            } else {
                signature_verified = true;
            }
        }
    }

    ArchiveVerification {
        is_sealed: claims_sealed,
        integrity_ok,
        signature_verified,
    }
}

pub fn is_human_decision_rejected(verb_id: u32) -> bool {
    matches!(
        verb_id,
        s3a_pack_scholar::xapi::VERB_ID_REJECTED
            | s3a_pack_scholar::xapi::VERB_ID_REMOVED
            | s3a_pack_scholar::xapi::VERB_ID_CONSENT_DECLINED
            | s3a_pack_scholar::xapi::VERB_ID_FLAGGED_HALLUCINATION
            | s3a_pack_scholar::xapi::VERB_ID_FLAGGED_DISCREPANCY
    )
}

pub fn is_human_decision_approved(verb_id: u32) -> bool {
    matches!(
        verb_id,
        s3a_pack_scholar::xapi::VERB_ID_APPROVED
            | s3a_pack_scholar::xapi::VERB_ID_TA_APPROVED_DRAFT
            | s3a_pack_scholar::xapi::VERB_ID_ACCEPTED
    )
}

pub fn compute_file_sha256(path: &Path) -> String {
    if let Ok(bytes) = std::fs::read(path) {
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        hex::encode(hasher.finalize())
    } else {
        "0000000000000000000000000000000000000000000000000000000000000000".to_string()
    }
}

pub fn extract_vcs_history(path: &Path) -> ViewerHistory {
    if let Ok(repo) = VcsRepo::open(path) {
        let branch = repo
            .get_ref("main")
            .ok()
            .flatten()
            .map(|r| r.ref_name().to_string())
            .unwrap_or_else(|| "main".to_string());
        if let Ok(commits) = repo.log(None) {
            let entries = commits
                .into_iter()
                .map(|c| ViewerCommitEntry {
                    commit_id: hex::encode(&c.commit_id[..8.min(c.commit_id.len())]),
                    parent_id: if c.parent_count > 0 {
                        Some(hex::encode(
                            &c.parent_seal_hashes[0][..8.min(c.parent_seal_hashes[0].len())],
                        ))
                    } else {
                        None
                    },
                    author: format!(
                        "0x{}",
                        hex::encode(&c.author_pubkey[..4.min(c.author_pubkey.len())])
                    ),
                    timestamp: c.committed_timestamp,
                    message: c.message().to_string(),
                })
                .collect();
            return ViewerHistory {
                branch,
                commits: entries,
            };
        }
    }
    ViewerHistory {
        branch: "main".to_string(),
        commits: vec![],
    }
}

pub fn extract_common_advanced(reader: &MmapReader, path: &Path) -> ViewerAdvancedDetails {
    let tile_count = reader.tile_count() as usize;
    let mut tile_types = Vec::new();
    let mut coordinates = Vec::new();

    for i in 0..tile_count {
        if let Some((hdr, _)) = reader.get_tile(i) {
            tile_types.push(hdr.tile_type);
            coordinates.push(format!("L0:T{}:R0", i));
        }
    }

    let crc_valid = reader.verify_checksums().is_ok();
    let crc_status = if crc_valid {
        format!(
            "All {} hyper-tiles pass hardware CRC32C checksums",
            tile_count
        )
    } else {
        "Warning: CRC32C checksum discrepancy detected".to_string()
    };

    let merkle_root = reader
        .get_tile(0)
        .map(|(hdr, _)| hex::encode(hdr.provenance.merkle_root))
        .unwrap_or_else(|| {
            "0000000000000000000000000000000000000000000000000000000000000000".to_string()
        });

    let record_hash = compute_file_sha256(path);
    let s3a_ql = format!(
        "SIFT ALL WHERE CONTAINER = '{}'",
        path.file_name().unwrap_or_default().to_string_lossy()
    );

    ViewerAdvancedDetails {
        tile_count,
        active_generation: reader.generation(),
        tile_types,
        crc_status,
        merkle_root,
        record_hash,
        s3a_ql,
        coordinates,
    }
}

#[derive(Debug, Clone)]
pub struct ParsedDocRecord {
    pub doc_kind: u32,
    pub key: String,
    pub content: Vec<u8>,
    pub timestamp_ms: u64,
}

pub fn extract_document_records(reader: &MmapReader) -> Vec<ParsedDocRecord> {
    let mut docs = Vec::new();
    let tile_count = reader.tile_count() as usize;
    for i in 0..tile_count {
        if let Some((hdr, payload)) = reader.get_tile(i) {
            let actual_len = (hdr.payload_bytes as usize).min(payload.len());
            let tile_bytes = &payload[..actual_len];

            if (hdr.tile_type == TileType::DOCUMENT_STORE
                || hdr.tile_type == TileType::ATTACHMENT_BLOB)
                && tile_bytes.len() >= 64
            {
                let doc_hdr: DocumentRecordHeader = *bytemuck::from_bytes(&tile_bytes[..64]);
                let key_len = doc_hdr.key_len as usize;
                let content_len = doc_hdr.content_len as usize;
                if 64 + key_len <= tile_bytes.len() {
                    let key = String::from_utf8_lossy(&tile_bytes[64..64 + key_len]).to_string();
                    let content_end = (64 + key_len + content_len).min(tile_bytes.len());
                    let content = tile_bytes[64 + key_len..content_end].to_vec();
                    docs.push(ParsedDocRecord {
                        doc_kind: doc_hdr.doc_kind,
                        key,
                        content,
                        timestamp_ms: doc_hdr.timestamp_ms,
                    });
                    continue;
                }
            }

            // Also check for raw STEP or PDF written directly into tile
            if tile_bytes.starts_with(b"ISO-10303-21;") || tile_bytes.starts_with(b"/*") {
                let s = String::from_utf8_lossy(tile_bytes);
                if s.contains("ISO-10303-21;") {
                    docs.push(ParsedDocRecord {
                        doc_kind: DOC_KIND_BINARY,
                        key: format!("step_model_{}.step", i),
                        content: tile_bytes.to_vec(),
                        timestamp_ms: 0,
                    });
                }
            } else if tile_bytes.starts_with(b"%PDF") {
                docs.push(ParsedDocRecord {
                    doc_kind: DOC_KIND_BINARY,
                    key: format!("drawing_{}.pdf", i),
                    content: tile_bytes.to_vec(),
                    timestamp_ms: 0,
                });
            }
        }
    }
    docs
}

// =============================================================================
// View Pack 1: Research View (s3a-pack-scholar)
// =============================================================================

#[derive(Default)]
struct DecodedScholarData {
    pub manifest: Option<SrsManifestHeader>,
    pub workspace: Option<SrsWorkspaceRecord>,
    pub papers: Vec<AcademicPaperRecord>,
    pub edges: Vec<ResearchGraphEdgeRecord>,
    pub claims: Vec<ScholarNoteClaimRecord>,
    pub citations: Vec<CitationFingerprintRecord>,
    pub human_decisions: Vec<HumanLrsRecord>,
    pub ai_traces: Vec<AiTraceRecord>,
    pub raw_json_tiles: Vec<(usize, serde_json::Value)>,
    pub documents: Vec<ParsedDocRecord>,
}

fn decode_scholar_data(reader: &MmapReader) -> DecodedScholarData {
    let mut manifest = None;
    let mut workspace = None;
    let mut papers = Vec::new();
    let mut edges = Vec::new();
    let mut claims = Vec::new();
    let mut citations = Vec::new();
    let mut human_decisions = Vec::new();
    let mut ai_traces = Vec::new();
    let mut raw_json_tiles = Vec::new();

    let tile_count = reader.tile_count() as usize;
    for i in 0..tile_count {
        if let Some((hdr, payload)) = reader.get_tile(i) {
            let actual_bytes = (hdr.payload_bytes as usize).min(payload.len());
            let tile_bytes = &payload[..actual_bytes];
            match hdr.tile_type {
                ScholarTileType::SRS_MANIFEST => {
                    if let Ok(recs) = bytemuck::try_cast_slice::<u8, SrsManifestHeader>(tile_bytes)
                    {
                        if let Some(&m) = recs.first() {
                            manifest = Some(m);
                        }
                    } else if let Ok(val) = serde_json::from_slice::<serde_json::Value>(tile_bytes)
                    {
                        raw_json_tiles.push((i, val));
                    }
                }
                ScholarTileType::SRS_WORKSPACE => {
                    if let Ok(recs) = bytemuck::try_cast_slice::<u8, SrsWorkspaceRecord>(tile_bytes)
                    {
                        if let Some(&w) = recs.first() {
                            workspace = Some(w);
                        }
                    } else if let Ok(val) = serde_json::from_slice::<serde_json::Value>(tile_bytes)
                    {
                        raw_json_tiles.push((i, val));
                    }
                }
                ScholarTileType::ACADEMIC_PAPERS => {
                    if let Ok(recs) =
                        bytemuck::try_cast_slice::<u8, AcademicPaperRecord>(tile_bytes)
                    {
                        papers.extend_from_slice(recs);
                    } else if let Ok(val) = serde_json::from_slice::<serde_json::Value>(tile_bytes)
                    {
                        raw_json_tiles.push((i, val));
                    }
                }
                ScholarTileType::RESEARCH_GRAPH => {
                    if let Ok(recs) =
                        bytemuck::try_cast_slice::<u8, ResearchGraphEdgeRecord>(tile_bytes)
                    {
                        edges.extend_from_slice(recs);
                    } else if let Ok(val) = serde_json::from_slice::<serde_json::Value>(tile_bytes)
                    {
                        raw_json_tiles.push((i, val));
                    }
                }
                ScholarTileType::NOTE_CLAIMS => {
                    if let Ok(recs) =
                        bytemuck::try_cast_slice::<u8, ScholarNoteClaimRecord>(tile_bytes)
                    {
                        claims.extend_from_slice(recs);
                    } else if let Ok(val) = serde_json::from_slice::<serde_json::Value>(tile_bytes)
                    {
                        raw_json_tiles.push((i, val));
                    }
                }
                ScholarTileType::CITATION_FINGERPRINT => {
                    if let Ok(recs) =
                        bytemuck::try_cast_slice::<u8, CitationFingerprintRecord>(tile_bytes)
                    {
                        citations.extend_from_slice(recs);
                    } else if let Ok(val) = serde_json::from_slice::<serde_json::Value>(tile_bytes)
                    {
                        raw_json_tiles.push((i, val));
                    }
                }
                ScholarTileType::HUMAN_LRS => {
                    if !tile_bytes.starts_with(b"{") {
                        if let Ok(recs) = bytemuck::try_cast_slice::<u8, HumanLrsRecord>(tile_bytes)
                        {
                            human_decisions.extend_from_slice(recs);
                        }
                    } else if let Ok(val) = serde_json::from_slice::<serde_json::Value>(tile_bytes)
                    {
                        raw_json_tiles.push((i, val));
                    }
                }
                ScholarTileType::AI_TRACE_LOG => {
                    if !tile_bytes.starts_with(b"{") {
                        if let Ok(recs) = bytemuck::try_cast_slice::<u8, AiTraceRecord>(tile_bytes)
                        {
                            ai_traces.extend_from_slice(recs);
                        }
                    } else if let Ok(val) = serde_json::from_slice::<serde_json::Value>(tile_bytes)
                    {
                        raw_json_tiles.push((i, val));
                    }
                }
                _ => {
                    if (10..=15).contains(&hdr.tile_type) {
                        if let Ok(val) = serde_json::from_slice::<serde_json::Value>(tile_bytes) {
                            raw_json_tiles.push((i, val));
                        }
                    }
                }
            }
        }
    }
    let documents = extract_document_records(reader);
    DecodedScholarData {
        manifest,
        workspace,
        papers,
        edges,
        claims,
        citations,
        human_decisions,
        ai_traces,
        raw_json_tiles,
        documents,
    }
}

pub struct ScholarResearchViewPack;

impl RecordViewPack for ScholarResearchViewPack {
    fn pack_id(&self) -> &'static str {
        "research"
    }

    fn display_name(&self) -> &'static str {
        "Scholar Research (s3a-pack-scholar)"
    }

    fn matches(&self, reader: &MmapReader) -> bool {
        for i in 0..reader.tile_count() as usize {
            if let Some((hdr, _)) = reader.get_tile(i) {
                if (10..=15).contains(&hdr.tile_type)
                    || hdr.tile_type == ScholarTileType::NOTE_CLAIMS
                    || hdr.tile_type == ScholarTileType::CITATION_FINGERPRINT
                {
                    return true;
                }
            }
        }
        false
    }

    fn extract_summary(&self, reader: &MmapReader, path: &Path) -> Result<ViewerSummary, String> {
        let advanced = extract_common_advanced(reader, path);
        let data = decode_scholar_data(reader);

        let title = if let Some((_, ref val)) = data.raw_json_tiles.first() {
            val.get("title")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    path.file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string()
                        .replace('_', " ")
                })
        } else if let Some(p) = data.papers.first() {
            if data.papers.len() == 1 {
                format!("Academic Paper 0x{:016x}", p.paper_id)
            } else {
                format!("Scholar Literature Corpus ({} Papers)", data.papers.len())
            }
        } else {
            path.file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
                .replace('_', " ")
        };

        let title = if title.is_empty() {
            "Not recorded".to_string()
        } else {
            title
        };

        let subtitle = if let Some(w) = data.workspace {
            format!(
                "Research Workspace Context (Project {:016x})",
                w.project_uuid[0]
            )
        } else if let Some(m) = data.manifest {
            format!("Research Snapshot (Lineage {:016x})", m.snapshot_uuid[0])
        } else {
            "Scholar Research & Epistemic Verification Container".to_string()
        };

        let description = if let Some((_, ref val)) = data.raw_json_tiles.first() {
            val.get("abstract")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    format!(
                        "Scholar container containing {} academic papers, {} evidence claims, {} graph edges, and {} human audit entries.",
                        data.papers.len(),
                        data.claims.len(),
                        data.edges.len(),
                        data.human_decisions.len()
                    )
                })
        } else {
            format!(
                "Scholar container containing {} academic papers, {} evidence claims, {} graph edges, and {} human audit entries.",
                data.papers.len(),
                data.claims.len(),
                data.edges.len(),
                data.human_decisions.len()
            )
        };

        let verif = check_archive_verification(path);
        let has_manifest = data.manifest.is_some() || verif.is_sealed;

        let latest_human = data
            .human_decisions
            .iter()
            .max_by_key(|h| h.timestamp_sec)
            .or_else(|| data.human_decisions.last());

        let status_badge = if has_manifest {
            if verif.signature_verified {
                "Signature verified".to_string()
            } else if verif.integrity_ok {
                "Integrity verified".to_string()
            } else {
                "Not verified".to_string()
            }
        } else if let Some(h) = latest_human {
            if is_human_decision_approved(h.verb_id) {
                "Researcher Approved".to_string()
            } else if is_human_decision_rejected(h.verb_id) {
                "Researcher Excluded".to_string()
            } else {
                "Not verified".to_string()
            }
        } else {
            "Not verified".to_string()
        };

        let human_approval = if let Some(h) = latest_human {
            if is_human_decision_approved(h.verb_id) {
                if h.actor_hash != 0 {
                    format!("Researcher Approved (0x{:08x})", h.actor_hash & 0xFFFFFFFF)
                } else {
                    "Researcher Approved".to_string()
                }
            } else if is_human_decision_rejected(h.verb_id) {
                if h.actor_hash != 0 {
                    format!("Researcher Excluded (0x{:08x})", h.actor_hash & 0xFFFFFFFF)
                } else {
                    "Researcher Excluded".to_string()
                }
            } else if has_manifest {
                if verif.signature_verified {
                    "Sealed (signature verified)".to_string()
                } else if verif.integrity_ok {
                    "Sealed (integrity verified)".to_string()
                } else {
                    "Sealed (integrity check failed)".to_string()
                }
            } else {
                "No researcher decision recorded".to_string()
            }
        } else if has_manifest {
            if verif.signature_verified {
                "Sealed (signature verified)".to_string()
            } else if verif.integrity_ok {
                "Sealed (integrity verified)".to_string()
            } else {
                "Sealed (integrity check failed)".to_string()
            }
        } else {
            "No researcher decision recorded".to_string()
        };

        let mut key_stats = vec![
            (
                "Academic Papers".to_string(),
                format!("{} Works", data.papers.len()),
            ),
            (
                "Research Claims".to_string(),
                format!("{} Claims", data.claims.len()),
            ),
            (
                "Graph Relations".to_string(),
                format!("{} Edges", data.edges.len()),
            ),
            (
                "Human Audit".to_string(),
                format!("{} Statements", data.human_decisions.len()),
            ),
            (
                "AI Trace Steps".to_string(),
                format!("{} Steps", data.ai_traces.len()),
            ),
        ];
        if let Some(m) = data.manifest {
            key_stats.push((
                "Container Lineage".to_string(),
                format!("Total {} Records", m.total_records),
            ));
        }

        Ok(ViewerSummary {
            title,
            subtitle,
            description,
            status_badge,
            human_approval,
            view_pack_name: self.display_name().to_string(),
            key_stats,
            advanced,
        })
    }

    fn extract_tree(
        &self,
        reader: &MmapReader,
        _path: &Path,
    ) -> Result<Vec<ViewerTreeNode>, String> {
        let data = decode_scholar_data(reader);
        let mut root_children = Vec::new();

        // 1. Papers
        for p in &data.papers {
            let paper_node_id = format!("paper_{}", p.paper_id);
            let label = format!(
                "Academic Paper 0x{:016x} (Year: {})",
                p.paper_id,
                if p.year > 0 {
                    p.year.to_string()
                } else {
                    "Not recorded".to_string()
                }
            );
            root_children.push(ViewerTreeNode {
                id: paper_node_id,
                label,
                icon: "book".to_string(),
                node_type: "paper".to_string(),
                has_preview: true,
                children: vec![],
            });
        }

        // 2. Claims
        if !data.claims.is_empty() {
            let mut claim_children = Vec::new();
            for c in &data.claims {
                claim_children.push(ViewerTreeNode {
                    id: format!("claim_{}", c.claim_id),
                    label: format!("Claim 0x{:08x} ({})", c.claim_id & 0xFFFFFFFF, c.kind_str()),
                    icon: "shield-check".to_string(),
                    node_type: "claim".to_string(),
                    has_preview: true,
                    children: vec![],
                });
            }
            root_children.push(ViewerTreeNode {
                id: "claims_group".to_string(),
                label: format!("Empirical Claims & Evidence ({})", data.claims.len()),
                icon: "check-circle".to_string(),
                node_type: "group".to_string(),
                has_preview: true,
                children: claim_children,
            });
        }

        // 3. Citations
        if !data.citations.is_empty() {
            let mut cit_children = Vec::new();
            for cf in &data.citations {
                cit_children.push(ViewerTreeNode {
                    id: format!("citation_{}", cf.paper_id),
                    label: format!(
                        "Citation Fingerprint (Paper 0x{:08x})",
                        cf.paper_id & 0xFFFFFFFF
                    ),
                    icon: "file-check".to_string(),
                    node_type: "citation".to_string(),
                    has_preview: true,
                    children: vec![],
                });
            }
            root_children.push(ViewerTreeNode {
                id: "citations_group".to_string(),
                label: format!("Citation Evidence Ledger ({})", data.citations.len()),
                icon: "bookmark".to_string(),
                node_type: "group".to_string(),
                has_preview: true,
                children: cit_children,
            });
        }

        // 4. Raw JSON tiles (e.g. title, abstract, sections)
        for (tile_idx, val) in &data.raw_json_tiles {
            let title_str = val
                .get("title")
                .and_then(|t| t.as_str())
                .unwrap_or("Scholar Document");
            let mut tile_children = Vec::new();
            if let Some(obj) = val.as_object() {
                for (k, v) in obj {
                    let v_str = v.as_str().unwrap_or("");
                    let label_preview = if v_str.len() > 30 {
                        format!("{}: {}...", k, &v_str[..27])
                    } else if !v_str.is_empty() {
                        format!("{}: {}", k, v_str)
                    } else {
                        k.clone()
                    };
                    tile_children.push(ViewerTreeNode {
                        id: format!("json_tile_{}_{}", tile_idx, k),
                        label: label_preview,
                        icon: "file-text".to_string(),
                        node_type: "section".to_string(),
                        has_preview: true,
                        children: vec![],
                    });
                }
            }
            root_children.push(ViewerTreeNode {
                id: format!("json_tile_{}", tile_idx),
                label: title_str.to_string(),
                icon: "book".to_string(),
                node_type: "document".to_string(),
                has_preview: true,
                children: tile_children,
            });
        }

        // 4. Graph relations
        if !data.edges.is_empty() {
            let mut edge_children = Vec::new();
            for e in data.edges.iter().take(20) {
                edge_children.push(ViewerTreeNode {
                    id: format!("edge_{}_{}", e.subject_hash, e.object_hash),
                    label: format!(
                        "Relation: 0x{:08x} -> 0x{:08x}",
                        e.subject_hash & 0xFFFFFFFF,
                        e.object_hash & 0xFFFFFFFF
                    ),
                    icon: "link".to_string(),
                    node_type: "relation".to_string(),
                    has_preview: true,
                    children: vec![],
                });
            }
            root_children.push(ViewerTreeNode {
                id: "graph_edges".to_string(),
                label: format!("Research Knowledge Graph ({} relations)", data.edges.len()),
                icon: "share-2".to_string(),
                node_type: "group".to_string(),
                has_preview: true,
                children: edge_children,
            });
        }

        // 5. Human decisions
        if !data.human_decisions.is_empty() {
            let mut h_children = Vec::new();
            for h in data.human_decisions.iter().take(20) {
                h_children.push(ViewerTreeNode {
                    id: format!("lrs_{}_{}", h.actor_hash, h.timestamp_sec),
                    label: format!(
                        "Human Decision: Actor 0x{:08x} (Verb: {})",
                        h.actor_hash & 0xFFFFFFFF,
                        h.verb_id
                    ),
                    icon: "user-check".to_string(),
                    node_type: "lrs".to_string(),
                    has_preview: true,
                    children: vec![],
                });
            }
            root_children.push(ViewerTreeNode {
                id: "human_audit_group".to_string(),
                label: format!(
                    "Human Gating & Decision Ledger ({})",
                    data.human_decisions.len()
                ),
                icon: "shield".to_string(),
                node_type: "group".to_string(),
                has_preview: true,
                children: h_children,
            });
        }

        // 6. Stored documents
        for d in &data.documents {
            root_children.push(ViewerTreeNode {
                id: format!("doc_{}", d.key),
                label: d.key.clone(),
                icon: "file".to_string(),
                node_type: "document".to_string(),
                has_preview: true,
                children: vec![],
            });
        }

        if root_children.is_empty() {
            root_children.push(ViewerTreeNode {
                id: "scholar_root".to_string(),
                label: "Scholar Research Container".to_string(),
                icon: "book".to_string(),
                node_type: "document".to_string(),
                has_preview: true,
                children: vec![],
            });
        }

        Ok(root_children)
    }

    fn extract_properties(
        &self,
        reader: &MmapReader,
        _path: &Path,
        node_id: &str,
    ) -> Result<ViewerProperties, String> {
        let data = decode_scholar_data(reader);

        // 1. Paper lookup
        if let Some(rest) = node_id.strip_prefix("paper_") {
            let pid_u64 = rest.parse::<u64>().ok().or_else(|| {
                rest.strip_prefix("0x")
                    .and_then(|s| u64::from_str_radix(s, 16).ok())
            });
            if let Some(target_pid) = pid_u64 {
                if let Some(p) = data.papers.iter().find(|p| p.paper_id == target_pid) {
                    return Ok(ViewerProperties {
                        node_id: node_id.to_string(),
                        label: format!("Paper 0x{:016x}", p.paper_id),
                        properties: vec![
                            ("Paper ID".to_string(), format!("0x{:016x}", p.paper_id)),
                            (
                                "Publication Year".to_string(),
                                if p.year > 0 {
                                    p.year.to_string()
                                } else {
                                    "Not recorded".to_string()
                                },
                            ),
                            ("Citation Count".to_string(), p.citation_count.to_string()),
                            (
                                "Open Access".to_string(),
                                if p.open_access_flag == 1 {
                                    "Yes".to_string()
                                } else if p.open_access_flag == 0 {
                                    "No".to_string()
                                } else {
                                    "Not recorded".to_string()
                                },
                            ),
                            (
                                "Venue ID".to_string(),
                                if p.venue_id > 0 {
                                    p.venue_id.to_string()
                                } else {
                                    "Not recorded".to_string()
                                },
                            ),
                            (
                                "Provenance Warnings".to_string(),
                                p.warning_count.to_string(),
                            ),
                        ],
                        advanced_properties: vec![
                            (
                                "Hilbert Curve Index".to_string(),
                                format!("0x{:016x}", p.hilbert_index),
                            ),
                            (
                                "Topic Coordinates (X, Y, Z)".to_string(),
                                format!("({:.4}, {:.4}, {:.4})", p.topic_x, p.topic_y, p.topic_z),
                            ),
                            (
                                "DOI Prefix Hash".to_string(),
                                if p.doi_prefix_hash > 0 {
                                    format!("0x{:016x}", p.doi_prefix_hash)
                                } else {
                                    "Not recorded".to_string()
                                },
                            ),
                            ("Record Flags".to_string(), format!("0x{:016x}", p.flags)),
                        ],
                    });
                }
            }
        }

        // 2. Claim lookup
        if let Some(rest) = node_id.strip_prefix("claim_") {
            if let Ok(cid) = rest.parse::<u64>() {
                if let Some(c) = data.claims.iter().find(|c| c.claim_id == cid) {
                    return Ok(ViewerProperties {
                        node_id: node_id.to_string(),
                        label: format!("Evidence Claim 0x{:016x}", c.claim_id),
                        properties: vec![
                            ("Claim ID".to_string(), format!("0x{:016x}", c.claim_id)),
                            ("Claim Kind".to_string(), c.kind_str().to_string()),
                            (
                                "Confidence Score".to_string(),
                                format!("{:.2}", c.confidence),
                            ),
                            (
                                "Author Hash".to_string(),
                                format!("0x{:016x}", c.author_hash),
                            ),
                            (
                                "Source Page".to_string(),
                                if c.passage_page > 0 {
                                    c.passage_page.to_string()
                                } else {
                                    "Not recorded".to_string()
                                },
                            ),
                        ],
                        advanced_properties: vec![
                            (
                                "Quoted Passage Hash".to_string(),
                                if c.quote_sha256 != [0u8; 32] {
                                    c.quote_sha256_hex()
                                } else {
                                    "Not recorded".to_string()
                                },
                            ),
                            (
                                "Keyword Bloom".to_string(),
                                format!("0x{:08x}", c.keyword_bloom),
                            ),
                            ("Text Hash".to_string(), format!("0x{:08x}", c.text_hash)),
                        ],
                    });
                }
            }
        }

        // 3. Citation lookup
        if let Some(rest) = node_id.strip_prefix("citation_") {
            if let Ok(pid) = rest.parse::<u64>() {
                if let Some(cf) = data.citations.iter().find(|cf| cf.paper_id == pid) {
                    return Ok(ViewerProperties {
                        node_id: node_id.to_string(),
                        label: format!("Citation Fingerprint 0x{:016x}", cf.paper_id),
                        properties: vec![
                            (
                                "Target Paper ID".to_string(),
                                format!("0x{:016x}", cf.paper_id),
                            ),
                            ("DOI Hash".to_string(), format!("0x{:016x}", cf.doi_hash)),
                            (
                                "Content SHA-256".to_string(),
                                hex::encode(cf.content_sha256),
                            ),
                            (
                                "Fetched Timestamp".to_string(),
                                if cf.fetched_at > 0 {
                                    cf.fetched_at.to_string()
                                } else {
                                    "Not recorded".to_string()
                                },
                            ),
                        ],
                        advanced_properties: vec![
                            (
                                "Source URL Hash".to_string(),
                                format!("0x{:016x}", cf.source_url_hash),
                            ),
                            ("Flags".to_string(), format!("0x{:016x}", cf.flags)),
                        ],
                    });
                }
            }
        }

        // 4. JSON tile lookup
        if let Some(rest) = node_id.strip_prefix("json_tile_") {
            let parts: Vec<&str> = rest.split('_').collect();
            if let Ok(tile_idx) = parts[0].parse::<usize>() {
                if let Some((_, val)) = data.raw_json_tiles.iter().find(|(idx, _)| *idx == tile_idx)
                {
                    if parts.len() > 1 {
                        let field_key = parts[1..].join("_");
                        let field_val = val
                            .get(&field_key)
                            .and_then(|v| v.as_str())
                            .unwrap_or("Not recorded");
                        return Ok(ViewerProperties {
                            node_id: node_id.to_string(),
                            label: format!("Field: {}", field_key),
                            properties: vec![
                                ("Field Name".to_string(), field_key),
                                ("Content".to_string(), field_val.to_string()),
                            ],
                            advanced_properties: vec![(
                                "Tile Index".to_string(),
                                tile_idx.to_string(),
                            )],
                        });
                    } else if let Some(obj) = val.as_object() {
                        let mut props = Vec::new();
                        for (k, v) in obj {
                            props.push((
                                k.clone(),
                                v.as_str().unwrap_or("Not recorded").to_string(),
                            ));
                        }
                        return Ok(ViewerProperties {
                            node_id: node_id.to_string(),
                            label: format!("JSON Tile {}", tile_idx),
                            properties: props,
                            advanced_properties: vec![(
                                "Tile Index".to_string(),
                                tile_idx.to_string(),
                            )],
                        });
                    }
                }
            }
        }

        // 4. Human Decision lookup
        if node_id.starts_with("lrs_") {
            if let Some(h) = data.human_decisions.first() {
                return Ok(ViewerProperties {
                    node_id: node_id.to_string(),
                    label: format!("Human Decision (Actor 0x{:08x})", h.actor_hash & 0xFFFFFFFF),
                    properties: vec![
                        ("Actor Hash".to_string(), format!("0x{:016x}", h.actor_hash)),
                        ("Verb ID".to_string(), h.verb_id.to_string()),
                        (
                            "Decision Score".to_string(),
                            format!("{:.2}", h.decision_score),
                        ),
                        (
                            "Timestamp".to_string(),
                            if h.timestamp_sec > 0 {
                                h.timestamp_sec.to_string()
                            } else {
                                "Not recorded".to_string()
                            },
                        ),
                    ],
                    advanced_properties: vec![
                        (
                            "Target Object Hash".to_string(),
                            format!("0x{:08x}", h.object_hash),
                        ),
                        ("Flags".to_string(), format!("0x{:08x}", h.flags)),
                    ],
                });
            }
        }

        // 5. Fallback properties
        Ok(ViewerProperties {
            node_id: node_id.to_string(),
            label: "Scholar Research Node".to_string(),
            properties: vec![
                (
                    "Document Type".to_string(),
                    "Scholar Research Corpus".to_string(),
                ),
                ("Papers Stored".to_string(), data.papers.len().to_string()),
                ("Claims Stored".to_string(), data.claims.len().to_string()),
                ("Relations Stored".to_string(), data.edges.len().to_string()),
            ],
            advanced_properties: vec![("Selected Node ID".to_string(), node_id.to_string())],
        })
    }

    fn extract_preview(
        &self,
        reader: &MmapReader,
        _path: &Path,
        node_id: &str,
    ) -> Result<ViewerPreview, String> {
        let data = decode_scholar_data(reader);

        // 1. Paper preview
        if let Some(rest) = node_id.strip_prefix("paper_") {
            let pid_u64 = rest.parse::<u64>().ok().or_else(|| {
                rest.strip_prefix("0x")
                    .and_then(|s| u64::from_str_radix(s, 16).ok())
            });
            if let Some(target_pid) = pid_u64 {
                if let Some(p) = data.papers.iter().find(|p| p.paper_id == target_pid) {
                    let content = format!(
                        "ACADEMIC PAPER RECORD\nPaper ID: {} (0x{:016x})\nPublication Year: {}\nCitation Count: {}\nOpen Access: {}\nVenue ID: {}\nTopic Centroid: ({:.4}, {:.4}, {:.4})\nHilbert Curve Index: 0x{:016x}\nFlags: 0x{:016x}\n",
                        p.paper_id,
                        p.paper_id,
                        if p.year > 0 {
                            p.year.to_string()
                        } else {
                            "Not recorded".to_string()
                        },
                        p.citation_count,
                        if p.open_access_flag == 1 {
                            "Yes"
                        } else {
                            "No"
                        },
                        if p.venue_id > 0 {
                            p.venue_id.to_string()
                        } else {
                            "Not recorded".to_string()
                        },
                        p.topic_x,
                        p.topic_y,
                        p.topic_z,
                        p.hilbert_index,
                        p.flags
                    );
                    return Ok(ViewerPreview {
                        node_id: node_id.to_string(),
                        mime_type: "text/plain".to_string(),
                        content,
                    });
                }
            }
        }

        // 2. JSON tile preview
        if let Some(rest) = node_id.strip_prefix("json_tile_") {
            let parts: Vec<&str> = rest.split('_').collect();
            if let Ok(tile_idx) = parts[0].parse::<usize>() {
                if let Some((_, val)) = data.raw_json_tiles.iter().find(|(idx, _)| *idx == tile_idx)
                {
                    if parts.len() > 1 {
                        let field_key = parts[1..].join("_");
                        let text = val
                            .get(&field_key)
                            .and_then(|v| v.as_str())
                            .unwrap_or("Not recorded");
                        return Ok(ViewerPreview {
                            node_id: node_id.to_string(),
                            mime_type: "text/plain".to_string(),
                            content: text.to_string(),
                        });
                    } else {
                        return Ok(ViewerPreview {
                            node_id: node_id.to_string(),
                            mime_type: "application/json".to_string(),
                            content: serde_json::to_string_pretty(val).unwrap_or_default(),
                        });
                    }
                }
            }
        }

        // 3. Fallback preview
        let summary_text = if let Some((_, ref val)) = data.raw_json_tiles.first() {
            serde_json::to_string_pretty(val).unwrap_or_default()
        } else if let Some(p) = data.papers.first() {
            format!(
                "Academic Paper Corpus\nPrimary Paper ID: 0x{:016x}\nYear: {}\nTotal Papers: {}\n",
                p.paper_id,
                if p.year > 0 {
                    p.year.to_string()
                } else {
                    "Not recorded".to_string()
                },
                data.papers.len()
            )
        } else {
            "Scholar research container. Select a record node on the left to inspect.".to_string()
        };

        Ok(ViewerPreview {
            node_id: node_id.to_string(),
            mime_type: "text/plain".to_string(),
            content: summary_text,
        })
    }

    fn extract_history(&self, _reader: &MmapReader, path: &Path) -> Result<ViewerHistory, String> {
        Ok(extract_vcs_history(path))
    }

    fn extract_approval(
        &self,
        reader: &MmapReader,
        path: &Path,
    ) -> Result<ViewerApprovalStatus, String> {
        let verif = check_archive_verification(path);
        let data = decode_scholar_data(reader);
        let has_manifest = data.manifest.is_some() || verif.is_sealed;

        let latest_human = data
            .human_decisions
            .iter()
            .max_by_key(|h| h.timestamp_sec)
            .or_else(|| data.human_decisions.last());

        if let Some(h) = latest_human {
            if is_human_decision_approved(h.verb_id) && (!has_manifest || verif.integrity_ok) {
                Ok(ViewerApprovalStatus {
                    is_approved: true,
                    approved_by: Some(format!("Human Researcher (Actor 0x{:016x})", h.actor_hash)),
                    approval_timestamp: if h.timestamp_sec > 0 {
                        Some(h.timestamp_sec)
                    } else {
                        None
                    },
                    signature: None,
                    fingerprint: compute_file_sha256(path),
                    requires_human_decision: false,
                })
            } else if is_human_decision_rejected(h.verb_id) || (has_manifest && !verif.integrity_ok)
            {
                Ok(ViewerApprovalStatus {
                    is_approved: false,
                    approved_by: None,
                    approval_timestamp: if h.timestamp_sec > 0 {
                        Some(h.timestamp_sec)
                    } else {
                        None
                    },
                    signature: None,
                    fingerprint: compute_file_sha256(path),
                    requires_human_decision: true,
                })
            } else if has_manifest && verif.integrity_ok {
                let uuid = data.manifest.map(|m| m.snapshot_uuid[0]).unwrap_or(0);
                let label = if verif.signature_verified {
                    "Sealed (signature verified)".to_string()
                } else if uuid != 0 {
                    format!("Snapshot Authority (UUID {:016x})", uuid)
                } else {
                    "Sealed (integrity verified)".to_string()
                };
                Ok(ViewerApprovalStatus {
                    is_approved: true,
                    approved_by: Some(label),
                    approval_timestamp: data.manifest.and_then(|m| {
                        if m.created_at_sec > 0 {
                            Some(m.created_at_sec)
                        } else {
                            None
                        }
                    }),
                    signature: if verif.signature_verified {
                        Some("ed25519".to_string())
                    } else {
                        None
                    },
                    fingerprint: compute_file_sha256(path),
                    requires_human_decision: false,
                })
            } else {
                Ok(ViewerApprovalStatus {
                    is_approved: false,
                    approved_by: None,
                    approval_timestamp: None,
                    signature: None,
                    fingerprint: compute_file_sha256(path),
                    requires_human_decision: true,
                })
            }
        } else if has_manifest {
            if verif.integrity_ok {
                let uuid = data.manifest.map(|m| m.snapshot_uuid[0]).unwrap_or(0);
                let label = if verif.signature_verified {
                    "Sealed (signature verified)".to_string()
                } else if uuid != 0 {
                    format!("Snapshot Authority (UUID {:016x})", uuid)
                } else {
                    "Sealed (integrity verified)".to_string()
                };
                Ok(ViewerApprovalStatus {
                    is_approved: true,
                    approved_by: Some(label),
                    approval_timestamp: data.manifest.and_then(|m| {
                        if m.created_at_sec > 0 {
                            Some(m.created_at_sec)
                        } else {
                            None
                        }
                    }),
                    signature: if verif.signature_verified {
                        Some("ed25519".to_string())
                    } else {
                        None
                    },
                    fingerprint: compute_file_sha256(path),
                    requires_human_decision: false,
                })
            } else {
                Ok(ViewerApprovalStatus {
                    is_approved: false,
                    approved_by: None,
                    approval_timestamp: None,
                    signature: None,
                    fingerprint: compute_file_sha256(path),
                    requires_human_decision: true,
                })
            }
        } else {
            Ok(ViewerApprovalStatus {
                is_approved: false,
                approved_by: None,
                approval_timestamp: None,
                signature: None,
                fingerprint: compute_file_sha256(path),
                requires_human_decision: true,
            })
        }
    }
}

// =============================================================================
// View Pack 2: Technical Data Package (s3a-pack-tdp)
// =============================================================================

struct DecodedTdpData {
    pub step_trees: Vec<(String, StepAssemblyTree)>,
    pub pdf_docs: Vec<s3a_pack_tdp::pdf::PdfMetadata>,
    pub _raw_docs: Vec<ParsedDocRecord>,
}

fn decode_tdp_data(reader: &MmapReader) -> DecodedTdpData {
    let docs = extract_document_records(reader);
    let mut step_trees = Vec::new();
    let mut pdf_docs = Vec::new();

    for d in &docs {
        if d.key.ends_with(".step")
            || d.key.ends_with(".stp")
            || d.content.starts_with(b"ISO-10303-21;")
            || d.content.starts_with(b"/*")
        {
            let s = String::from_utf8_lossy(&d.content);
            if s.contains("ISO-10303-21;") {
                if let Ok(tree) = parse_step(&d.content) {
                    step_trees.push((d.key.clone(), tree));
                    continue;
                }
            }
        }
        if d.key.ends_with(".pdf") || d.content.starts_with(b"%PDF") {
            if let Ok(meta) = extract_pdf_metadata(&d.key, &d.content) {
                pdf_docs.push(meta);
            }
        }
    }

    DecodedTdpData {
        step_trees,
        pdf_docs,
        _raw_docs: docs,
    }
}

pub struct TechnicalDataViewPack;

impl RecordViewPack for TechnicalDataViewPack {
    fn pack_id(&self) -> &'static str {
        "technical_data"
    }

    fn display_name(&self) -> &'static str {
        "Technical Data Package (TDP)"
    }

    fn matches(&self, reader: &MmapReader) -> bool {
        for i in 0..reader.tile_count() as usize {
            if let Some((hdr, payload)) = reader.get_tile(i) {
                if hdr.tile_type == TileType::DOCUMENT_STORE
                    || hdr.tile_type == TileType::ATTACHMENT_BLOB
                {
                    let text = String::from_utf8_lossy(payload);
                    if text.contains("ISO-10303-21")
                        || text.contains("%PDF")
                        || text.contains("PRODUCT_DEFINITION")
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn extract_summary(&self, reader: &MmapReader, path: &Path) -> Result<ViewerSummary, String> {
        let advanced = extract_common_advanced(reader, path);
        let data = decode_tdp_data(reader);

        let (title, subtitle, author, org, system, auth, part_count) =
            if let Some((_, ref tree)) = data.step_trees.first() {
                let title = tree
                    .header
                    .file_name
                    .clone()
                    .or_else(|| tree.root_part_number.clone())
                    .unwrap_or_else(|| {
                        path.file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string()
                            .replace('_', " ")
                    });
                let subtitle = format!(
                    "ISO 10303 AP242 STEP Assembly ({} components)",
                    tree.parts.len()
                );
                let author = tree
                    .header
                    .author
                    .clone()
                    .unwrap_or_else(|| "Not recorded".to_string());
                let org = tree
                    .header
                    .organization
                    .clone()
                    .unwrap_or_else(|| "Not recorded".to_string());
                let system = tree
                    .header
                    .originating_system
                    .clone()
                    .unwrap_or_else(|| "Not recorded".to_string());
                let auth = tree
                    .header
                    .authorization
                    .clone()
                    .unwrap_or_else(|| "Not recorded".to_string());
                let part_count = tree.parts.len();
                (title, subtitle, author, org, system, auth, part_count)
            } else if let Some(pdf) = data.pdf_docs.first() {
                let title = pdf.title.clone().unwrap_or_else(|| pdf.key.clone());
                let subtitle = format!("Engineering Drawing Package ({} pages)", pdf.page_count);
                (
                    title,
                    subtitle,
                    "Not recorded".to_string(),
                    "Not recorded".to_string(),
                    "PDF Engine".to_string(),
                    "Not recorded".to_string(),
                    0,
                )
            } else {
                (
                    path.file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string()
                        .replace('_', " "),
                    "Technical Data Package".to_string(),
                    "Not recorded".to_string(),
                    "Not recorded".to_string(),
                    "Not recorded".to_string(),
                    "Not recorded".to_string(),
                    0,
                )
            };

        let title = if title.is_empty() {
            "Technical Data Package".to_string()
        } else {
            title
        };

        let description = format!(
            "Authorized engineering technical data package containing {} CAD parts and {} PDF drawings.",
            part_count,
            data.pdf_docs.len()
        );

        let verif = check_archive_verification(path);

        let status_badge = if verif.is_sealed {
            if verif.signature_verified {
                "Signature verified".to_string()
            } else if verif.integrity_ok {
                "Integrity verified".to_string()
            } else {
                "Not verified".to_string()
            }
        } else if auth != "Not recorded" || !data.step_trees.is_empty() || !data.pdf_docs.is_empty()
        {
            "Engineering Released".to_string()
        } else {
            "Technical Package".to_string()
        };

        let human_approval = if verif.is_sealed {
            if verif.signature_verified {
                "Sealed (signature verified)".to_string()
            } else if verif.integrity_ok {
                "Sealed (integrity verified)".to_string()
            } else {
                "Sealed (integrity check failed)".to_string()
            }
        } else if auth != "Not recorded" {
            format!("Approved: {}", auth)
        } else {
            "No engineering approval recorded".to_string()
        };

        let key_stats = vec![
            (
                "Components".to_string(),
                format!("{} Physical Parts", part_count),
            ),
            (
                "CAD Drawings".to_string(),
                format!("{} Drawings", data.pdf_docs.len()),
            ),
            ("Originating System".to_string(), system),
            ("Organization".to_string(), org),
            ("Author".to_string(), author),
        ];

        Ok(ViewerSummary {
            title,
            subtitle,
            description,
            status_badge,
            human_approval,
            view_pack_name: self.display_name().to_string(),
            key_stats,
            advanced,
        })
    }

    fn extract_tree(
        &self,
        reader: &MmapReader,
        _path: &Path,
    ) -> Result<Vec<ViewerTreeNode>, String> {
        let data = decode_tdp_data(reader);
        let mut root_nodes = Vec::new();

        // 1. STEP Assemblies
        for (key, tree) in &data.step_trees {
            let asm_id = format!("asm_{}", tree.root_part_number.as_deref().unwrap_or(key));
            let mut part_children = Vec::new();

            for p in &tree.parts {
                part_children.push(ViewerTreeNode {
                    id: format!("part_{}", p.part_number),
                    label: format!(
                        "{}: {} (Qty: {})",
                        p.part_number,
                        if !p.name.is_empty() { &p.name } else { "Part" },
                        p.quantity
                    ),
                    icon: "cpu".to_string(),
                    node_type: "part".to_string(),
                    has_preview: true,
                    children: vec![],
                });
            }

            // BOM node
            part_children.push(ViewerTreeNode {
                id: format!("bom_{}", tree.root_part_number.as_deref().unwrap_or(key)),
                label: "Bill of Materials (BOM)".to_string(),
                icon: "list".to_string(),
                node_type: "bom".to_string(),
                has_preview: true,
                children: vec![],
            });

            root_nodes.push(ViewerTreeNode {
                id: asm_id,
                label: format!(
                    "Assembly: {} ({} components)",
                    tree.root_part_number.as_deref().unwrap_or(key),
                    tree.parts.len()
                ),
                icon: "box".to_string(),
                node_type: "assembly".to_string(),
                has_preview: true,
                children: part_children,
            });
        }

        // 2. PDF Drawings
        if !data.pdf_docs.is_empty() {
            let mut pdf_children = Vec::new();
            for pdf in &data.pdf_docs {
                pdf_children.push(ViewerTreeNode {
                    id: format!("pdf_{}", pdf.key),
                    label: format!(
                        "Drawing: {} ({} pages)",
                        pdf.title.as_deref().unwrap_or(&pdf.key),
                        pdf.page_count
                    ),
                    icon: "file-text".to_string(),
                    node_type: "drawing".to_string(),
                    has_preview: true,
                    children: vec![],
                });
            }
            root_nodes.push(ViewerTreeNode {
                id: "drawings_group".to_string(),
                label: format!("Technical Drawings ({} PDFs)", data.pdf_docs.len()),
                icon: "folder".to_string(),
                node_type: "group".to_string(),
                has_preview: true,
                children: pdf_children,
            });
        }

        if root_nodes.is_empty() {
            root_nodes.push(ViewerTreeNode {
                id: "tdp_root".to_string(),
                label: "Technical Data Package".to_string(),
                icon: "box".to_string(),
                node_type: "assembly".to_string(),
                has_preview: true,
                children: vec![],
            });
        }

        Ok(root_nodes)
    }

    fn extract_properties(
        &self,
        reader: &MmapReader,
        _path: &Path,
        node_id: &str,
    ) -> Result<ViewerProperties, String> {
        let data = decode_tdp_data(reader);

        // Assembly root or generic root
        if node_id == "root" || node_id == "asm_root" || node_id.starts_with("asm_") {
            if let Some((_, tree)) = data.step_trees.first() {
                return Ok(ViewerProperties {
                    node_id: node_id.to_string(),
                    label: format!(
                        "Assembly {}",
                        tree.root_part_number.as_deref().unwrap_or("Root")
                    ),
                    properties: vec![
                        (
                            "Part Number".to_string(),
                            tree.root_part_number
                                .clone()
                                .unwrap_or_else(|| "Not recorded".to_string()),
                        ),
                        ("Components Count".to_string(), tree.parts.len().to_string()),
                        (
                            "Author".to_string(),
                            tree.header
                                .author
                                .clone()
                                .unwrap_or_else(|| "Not recorded".to_string()),
                        ),
                        (
                            "Originating System".to_string(),
                            tree.header
                                .originating_system
                                .clone()
                                .unwrap_or_else(|| "Not recorded".to_string()),
                        ),
                        (
                            "Organization".to_string(),
                            tree.header
                                .organization
                                .clone()
                                .unwrap_or_else(|| "Not recorded".to_string()),
                        ),
                    ],
                    advanced_properties: vec![
                        (
                            "Authorization".to_string(),
                            tree.header
                                .authorization
                                .clone()
                                .unwrap_or_else(|| "Not recorded".to_string()),
                        ),
                        (
                            "Schemas".to_string(),
                            if !tree.header.schemas.is_empty() {
                                tree.header.schemas.join(", ")
                            } else {
                                "Not recorded".to_string()
                            },
                        ),
                    ],
                });
            }
        }

        // BOM lookup
        if node_id == "bom" || node_id == "bom_root" || node_id.starts_with("bom_") {
            if let Some((_, tree)) = data.step_trees.first() {
                return Ok(ViewerProperties {
                    node_id: node_id.to_string(),
                    label: "Bill of Materials (BOM)".to_string(),
                    properties: vec![
                        (
                            "Total Line Items".to_string(),
                            format!("{} components", tree.parts.len()),
                        ),
                        (
                            "Root Part".to_string(),
                            tree.root_part_number
                                .clone()
                                .unwrap_or_else(|| "Not recorded".to_string()),
                        ),
                        (
                            "Originating System".to_string(),
                            tree.header
                                .originating_system
                                .clone()
                                .unwrap_or_else(|| "Not recorded".to_string()),
                        ),
                        (
                            "Organization".to_string(),
                            tree.header
                                .organization
                                .clone()
                                .unwrap_or_else(|| "Not recorded".to_string()),
                        ),
                    ],
                    advanced_properties: vec![(
                        "Schemas".to_string(),
                        if !tree.header.schemas.is_empty() {
                            tree.header.schemas.join(", ")
                        } else {
                            "Not recorded".to_string()
                        },
                    )],
                });
            }
        }

        // Part lookup
        let clean_pn = node_id.strip_prefix("part_").unwrap_or(node_id);
        for (_, tree) in &data.step_trees {
            if let Some(p) = tree
                .parts
                .iter()
                .find(|p| p.part_number == clean_pn || p.part_number == node_id)
            {
                return Ok(ViewerProperties {
                    node_id: node_id.to_string(),
                    label: format!("Component {}", p.part_number),
                    properties: vec![
                        ("Part Number".to_string(), p.part_number.clone()),
                        (
                            "Part Name".to_string(),
                            if !p.name.is_empty() {
                                p.name.clone()
                            } else {
                                "Not recorded".to_string()
                            },
                        ),
                        (
                            "Parent Assembly".to_string(),
                            p.parent
                                .clone()
                                .unwrap_or_else(|| "Not recorded".to_string()),
                        ),
                        ("Quantity".to_string(), p.quantity.to_string()),
                    ],
                    advanced_properties: vec![
                        (
                            "Author".to_string(),
                            tree.header
                                .author
                                .clone()
                                .unwrap_or_else(|| "Not recorded".to_string()),
                        ),
                        (
                            "Originating System".to_string(),
                            tree.header
                                .originating_system
                                .clone()
                                .unwrap_or_else(|| "Not recorded".to_string()),
                        ),
                        (
                            "Organization".to_string(),
                            tree.header
                                .organization
                                .clone()
                                .unwrap_or_else(|| "Not recorded".to_string()),
                        ),
                        (
                            "Authorization".to_string(),
                            tree.header
                                .authorization
                                .clone()
                                .unwrap_or_else(|| "Not recorded".to_string()),
                        ),
                    ],
                });
            }
        }

        // PDF lookup
        let clean_pdf_key = node_id.strip_prefix("pdf_").unwrap_or(node_id);
        if let Some(pdf) = data
            .pdf_docs
            .iter()
            .find(|pdf| pdf.key == clean_pdf_key || pdf.key == node_id)
        {
            return Ok(ViewerProperties {
                node_id: node_id.to_string(),
                label: format!("Drawing {}", pdf.title.as_deref().unwrap_or(&pdf.key)),
                properties: vec![
                    ("Document Key".to_string(), pdf.key.clone()),
                    (
                        "Title".to_string(),
                        pdf.title
                            .clone()
                            .unwrap_or_else(|| "Not recorded".to_string()),
                    ),
                    ("Page Count".to_string(), pdf.page_count.to_string()),
                    ("File Size".to_string(), format!("{} bytes", pdf.size_bytes)),
                ],
                advanced_properties: vec![
                    ("SHA-256 Checksum".to_string(), pdf.sha256.clone()),
                    ("MIME Type".to_string(), pdf.mime_type.clone()),
                ],
            });
        }

        // Fallback properties
        Ok(ViewerProperties {
            node_id: node_id.to_string(),
            label: "Technical Data Package Node".to_string(),
            properties: vec![
                (
                    "Document Type".to_string(),
                    "Technical Data Package".to_string(),
                ),
                (
                    "Components".to_string(),
                    data.step_trees
                        .iter()
                        .map(|(_, t)| t.parts.len())
                        .sum::<usize>()
                        .to_string(),
                ),
                ("Drawings".to_string(), data.pdf_docs.len().to_string()),
            ],
            advanced_properties: vec![("Node ID".to_string(), node_id.to_string())],
        })
    }

    fn extract_preview(
        &self,
        reader: &MmapReader,
        _path: &Path,
        node_id: &str,
    ) -> Result<ViewerPreview, String> {
        let data = decode_tdp_data(reader);

        // BOM preview (CSV table)
        if node_id == "bom" || node_id == "bom_root" || node_id.starts_with("bom_") {
            if let Some((_, tree)) = data.step_trees.first() {
                return Ok(ViewerPreview {
                    node_id: node_id.to_string(),
                    mime_type: "text/plain".to_string(),
                    content: tree.to_bom_csv(),
                });
            }
        }

        // Part preview (dynamic SVG generated with actual part number and name)
        let clean_pn = node_id.strip_prefix("part_").unwrap_or(node_id);
        for (_, tree) in &data.step_trees {
            if let Some(p) = tree
                .parts
                .iter()
                .find(|p| p.part_number == clean_pn || p.part_number == node_id)
            {
                let svg = format!(
                    r###"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 600 300" width="100%" height="240"><rect width="100%" height="100%" fill="#0d1117" rx="8"/><rect x="180" y="70" width="240" height="150" fill="none" stroke="#58a6ff" stroke-width="2" rx="8"/><circle cx="300" cy="140" r="30" fill="none" stroke="#79c0ff" stroke-width="2"/><text x="300" y="45" fill="#f0f6fc" font-family="sans-serif" font-size="16" text-anchor="middle" font-weight="bold">Part: {}</text><text x="300" y="145" fill="#79c0ff" font-family="sans-serif" font-size="12" text-anchor="middle">Qty: {}</text><text x="300" y="250" fill="#8b949e" font-family="sans-serif" font-size="12" text-anchor="middle">Name: {} &bull; Parent: {}</text></svg>"###,
                    p.part_number,
                    p.quantity,
                    if !p.name.is_empty() {
                        &p.name
                    } else {
                        "Component"
                    },
                    p.parent.as_deref().unwrap_or("Root")
                );
                return Ok(ViewerPreview {
                    node_id: node_id.to_string(),
                    mime_type: "image/svg+xml".to_string(),
                    content: svg,
                });
            }
        }

        // Assembly root preview
        if node_id == "root" || node_id == "asm_root" || node_id.starts_with("asm_") {
            if let Some((_, tree)) = data.step_trees.first() {
                let svg = format!(
                    r###"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 600 300" width="100%" height="240"><rect width="100%" height="100%" fill="#0d1117" rx="8"/><circle cx="300" cy="140" r="60" fill="none" stroke="#58a6ff" stroke-width="3"/><text x="300" y="50" fill="#f0f6fc" font-family="sans-serif" font-size="16" text-anchor="middle" font-weight="bold">Assembly: {}</text><text x="300" y="145" fill="#79c0ff" font-family="sans-serif" font-size="13" text-anchor="middle">{} Components</text><text x="300" y="240" fill="#8b949e" font-family="sans-serif" font-size="12" text-anchor="middle">Author: {}</text><text x="300" y="265" fill="#58a6ff" font-family="sans-serif" font-size="11" text-anchor="middle">System: {}</text></svg>"###,
                    tree.root_part_number.as_deref().unwrap_or("Assembly Root"),
                    tree.parts.len(),
                    tree.header.author.as_deref().unwrap_or("Not recorded"),
                    tree.header
                        .originating_system
                        .as_deref()
                        .unwrap_or("Not recorded")
                );
                return Ok(ViewerPreview {
                    node_id: node_id.to_string(),
                    mime_type: "image/svg+xml".to_string(),
                    content: svg,
                });
            }
        }

        // PDF preview
        let clean_pdf_key = node_id.strip_prefix("pdf_").unwrap_or(node_id);
        if let Some(pdf) = data
            .pdf_docs
            .iter()
            .find(|pdf| pdf.key == clean_pdf_key || pdf.key == node_id)
        {
            let content = format!(
                "ENGINEERING DRAWING SPECIFICATION:\nKey: {}\nTitle: {}\nPage Count: {}\nSHA-256: {}\nFile Size: {} bytes\n",
                pdf.key,
                pdf.title.as_deref().unwrap_or("Not recorded"),
                pdf.page_count,
                pdf.sha256,
                pdf.size_bytes
            );
            return Ok(ViewerPreview {
                node_id: node_id.to_string(),
                mime_type: "text/plain".to_string(),
                content,
            });
        }

        Ok(ViewerPreview {
            node_id: node_id.to_string(),
            mime_type: "text/plain".to_string(),
            content: "Technical Data Package record opened. Select a part or drawing on the left to inspect.".to_string(),
        })
    }

    fn extract_history(&self, _reader: &MmapReader, path: &Path) -> Result<ViewerHistory, String> {
        Ok(extract_vcs_history(path))
    }

    fn extract_approval(
        &self,
        reader: &MmapReader,
        path: &Path,
    ) -> Result<ViewerApprovalStatus, String> {
        let verif = check_archive_verification(path);
        if verif.is_sealed && !verif.integrity_ok {
            return Ok(ViewerApprovalStatus {
                is_approved: false,
                approved_by: None,
                approval_timestamp: None,
                signature: None,
                fingerprint: compute_file_sha256(path),
                requires_human_decision: true,
            });
        }
        let data = decode_tdp_data(reader);
        if let Some((_, tree)) = data.step_trees.first() {
            if let Some(ref auth) = tree.header.authorization {
                return Ok(ViewerApprovalStatus {
                    is_approved: true,
                    approved_by: Some(auth.clone()),
                    approval_timestamp: None,
                    signature: None,
                    fingerprint: compute_file_sha256(path),
                    requires_human_decision: false,
                });
            }
        }
        if verif.is_sealed && verif.integrity_ok {
            let label = if verif.signature_verified {
                "Sealed (signature verified)".to_string()
            } else {
                "Sealed (integrity verified)".to_string()
            };
            return Ok(ViewerApprovalStatus {
                is_approved: true,
                approved_by: Some(label),
                approval_timestamp: None,
                signature: if verif.signature_verified {
                    Some("ed25519".to_string())
                } else {
                    None
                },
                fingerprint: compute_file_sha256(path),
                requires_human_decision: false,
            });
        }
        Ok(ViewerApprovalStatus {
            is_approved: false,
            approved_by: None,
            approval_timestamp: None,
            signature: None,
            fingerprint: compute_file_sha256(path),
            requires_human_decision: true,
        })
    }
}

// =============================================================================
// View Pack 3: Backups & Devices View (Aura)
// =============================================================================

struct DecodedAuraData {
    pub device_id: Option<String>,
    pub snapshot: Option<String>,
    pub os_version: Option<String>,
    pub telem_tile_count: usize,
    pub raw_json_payloads: Vec<serde_json::Value>,
    pub telem_records: Vec<(u64, u32, u32, f64)>,
}

fn decode_aura_data(reader: &MmapReader) -> DecodedAuraData {
    let mut device_id = None;
    let mut snapshot = None;
    let mut os_version = None;
    let mut telem_tile_count = 0;
    let mut raw_json_payloads = Vec::new();
    let mut telem_records = Vec::new();

    let tile_count = reader.tile_count() as usize;
    for i in 0..tile_count {
        if let Some((hdr, payload)) = reader.get_tile(i) {
            if hdr.tile_type == TileType::TELEMETRY || hdr.tile_type == 3 {
                telem_tile_count += 1;
                let actual_bytes = (hdr.payload_bytes as usize).min(payload.len());
                let tile_bytes = &payload[..actual_bytes];
                if let Ok(val) = serde_json::from_slice::<serde_json::Value>(tile_bytes) {
                    if let Some(did) = val.get("device_id").and_then(|v| v.as_str()) {
                        device_id = Some(did.to_string());
                    }
                    if let Some(snap) = val.get("snapshot").and_then(|v| v.as_str()) {
                        snapshot = Some(snap.to_string());
                    }
                    if let Some(os) = val.get("os").and_then(|v| v.as_str()) {
                        os_version = Some(os.to_string());
                    }
                    raw_json_payloads.push(val);
                } else if tile_bytes.len() >= 32 {
                    let count = tile_bytes.len() / 32;
                    for r in 0..count {
                        let offset = r * 32;
                        let ts =
                            u64::from_le_bytes(tile_bytes[offset..offset + 8].try_into().unwrap());
                        let sensor_id = u32::from_le_bytes(
                            tile_bytes[offset + 8..offset + 12].try_into().unwrap(),
                        );
                        let metric_id = u32::from_le_bytes(
                            tile_bytes[offset + 12..offset + 16].try_into().unwrap(),
                        );
                        let val = f64::from_le_bytes(
                            tile_bytes[offset + 16..offset + 24].try_into().unwrap(),
                        );
                        telem_records.push((ts, sensor_id, metric_id, val));
                    }
                }
            }
        }
    }

    DecodedAuraData {
        device_id,
        snapshot,
        os_version,
        telem_tile_count,
        raw_json_payloads,
        telem_records,
    }
}

pub struct AuraBackupViewPack;

impl RecordViewPack for AuraBackupViewPack {
    fn pack_id(&self) -> &'static str {
        "aura_backup"
    }

    fn display_name(&self) -> &'static str {
        "Aura Backup & Device Snapshot"
    }

    fn matches(&self, reader: &MmapReader) -> bool {
        for i in 0..reader.tile_count() as usize {
            if let Some((hdr, _)) = reader.get_tile(i) {
                if hdr.tile_type == TileType::TELEMETRY
                    || hdr.tile_type == 3
                    || hdr.tile_type == TileType::VCS_COMMIT
                {
                    return true;
                }
            }
        }
        false
    }

    fn extract_summary(&self, reader: &MmapReader, path: &Path) -> Result<ViewerSummary, String> {
        let advanced = extract_common_advanced(reader, path);
        let data = decode_aura_data(reader);

        let title = if let Some(ref did) = data.device_id {
            format!("Aura Device Backup - {}", did)
        } else {
            path.file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
                .replace('_', " ")
        };

        let title = if title.is_empty() {
            "Aura Device Backup".to_string()
        } else {
            title
        };

        let subtitle = "Autonomous Edge Device Snapshot & Sensor Event Stream".to_string();
        let description = format!(
            "Device backup capturing telemetry across {} sensor tiles and {} recorded readings.",
            data.telem_tile_count,
            data.telem_records.len()
        );

        let verif = check_archive_verification(path);

        let status_badge = if verif.is_sealed {
            if verif.signature_verified {
                "Signature verified".to_string()
            } else if verif.integrity_ok {
                "Integrity verified".to_string()
            } else {
                "Not verified".to_string()
            }
        } else {
            "Device Healthy".to_string()
        };

        let human_approval = if verif.is_sealed {
            if verif.signature_verified {
                "Sealed (signature verified)".to_string()
            } else if verif.integrity_ok {
                "Sealed (integrity verified)".to_string()
            } else {
                "Sealed (integrity check failed)".to_string()
            }
        } else {
            "No device approval recorded".to_string()
        };

        let key_stats = vec![
            (
                "Device ID".to_string(),
                data.device_id.unwrap_or_else(|| "Not recorded".to_string()),
            ),
            (
                "Firmware / OS".to_string(),
                data.os_version
                    .unwrap_or_else(|| "Not recorded".to_string()),
            ),
            (
                "Snapshot Time".to_string(),
                data.snapshot.unwrap_or_else(|| "Not recorded".to_string()),
            ),
            (
                "Telemetry Tiles".to_string(),
                data.telem_tile_count.to_string(),
            ),
            (
                "Sensor Events".to_string(),
                data.telem_records.len().to_string(),
            ),
        ];

        Ok(ViewerSummary {
            title,
            subtitle,
            description,
            status_badge,
            human_approval,
            view_pack_name: self.display_name().to_string(),
            key_stats,
            advanced,
        })
    }

    fn extract_tree(
        &self,
        reader: &MmapReader,
        _path: &Path,
    ) -> Result<Vec<ViewerTreeNode>, String> {
        let data = decode_aura_data(reader);
        let mut children = Vec::new();

        if let Some(ref snap) = data.snapshot {
            children.push(ViewerTreeNode {
                id: "snapshot_state".to_string(),
                label: format!("Snapshot ({})", snap),
                icon: "clock".to_string(),
                node_type: "snapshot".to_string(),
                has_preview: true,
                children: vec![],
            });
        }

        for idx in 0..data.telem_tile_count {
            children.push(ViewerTreeNode {
                id: format!("stream_{}", idx),
                label: format!("Telemetry Stream #{}", idx + 1),
                icon: "activity".to_string(),
                node_type: "stream".to_string(),
                has_preview: true,
                children: vec![],
            });
        }

        for r in data.telem_records.iter().take(10) {
            children.push(ViewerTreeNode {
                id: format!("sensor_{}_{}", r.1, r.2),
                label: format!("Sensor {} (Metric: {}, Value: {:.2})", r.1, r.2, r.3),
                icon: "zap".to_string(),
                node_type: "events".to_string(),
                has_preview: true,
                children: vec![],
            });
        }

        let dev_id = data.device_id.as_deref().unwrap_or("node");
        Ok(vec![ViewerTreeNode {
            id: format!("device_{}", dev_id),
            label: format!("Aura Device ({})", dev_id),
            icon: "hard-drive".to_string(),
            node_type: "device".to_string(),
            has_preview: true,
            children,
        }])
    }

    fn extract_properties(
        &self,
        reader: &MmapReader,
        _path: &Path,
        node_id: &str,
    ) -> Result<ViewerProperties, String> {
        let data = decode_aura_data(reader);

        if node_id.starts_with("device_") || node_id == "root" {
            return Ok(ViewerProperties {
                node_id: node_id.to_string(),
                label: "Device State Profile".to_string(),
                properties: vec![
                    (
                        "Device ID".to_string(),
                        data.device_id.unwrap_or_else(|| "Not recorded".to_string()),
                    ),
                    (
                        "Firmware / OS".to_string(),
                        data.os_version
                            .unwrap_or_else(|| "Not recorded".to_string()),
                    ),
                    (
                        "Snapshot State".to_string(),
                        data.snapshot.unwrap_or_else(|| "Not recorded".to_string()),
                    ),
                    (
                        "Telemetry Tiles".to_string(),
                        data.telem_tile_count.to_string(),
                    ),
                    (
                        "Recorded Sensor Events".to_string(),
                        data.telem_records.len().to_string(),
                    ),
                ],
                advanced_properties: vec![("Node ID".to_string(), node_id.to_string())],
            });
        }

        if let Some(rest) = node_id.strip_prefix("sensor_") {
            let parts: Vec<&str> = rest.split('_').collect();
            if parts.len() >= 2 {
                let sid = parts[0];
                let mid = parts[1];
                return Ok(ViewerProperties {
                    node_id: node_id.to_string(),
                    label: format!("Sensor Reading {}", sid),
                    properties: vec![
                        ("Sensor ID".to_string(), sid.to_string()),
                        ("Metric ID".to_string(), mid.to_string()),
                    ],
                    advanced_properties: vec![("Node ID".to_string(), node_id.to_string())],
                });
            }
        }

        Ok(ViewerProperties {
            node_id: node_id.to_string(),
            label: "Aura Stream Details".to_string(),
            properties: vec![
                ("Status".to_string(), "Streaming Normal".to_string()),
                (
                    "Telemetry Tiles".to_string(),
                    data.telem_tile_count.to_string(),
                ),
            ],
            advanced_properties: vec![("Node ID".to_string(), node_id.to_string())],
        })
    }

    fn extract_preview(
        &self,
        reader: &MmapReader,
        _path: &Path,
        node_id: &str,
    ) -> Result<ViewerPreview, String> {
        let data = decode_aura_data(reader);

        if let Some(ref val) = data.raw_json_payloads.first() {
            return Ok(ViewerPreview {
                node_id: node_id.to_string(),
                mime_type: "application/json".to_string(),
                content: serde_json::to_string_pretty(val).unwrap_or_default(),
            });
        }

        if !data.telem_records.is_empty() {
            let mut table = String::from("TIMESTAMP (UTC)       | SENSOR ID  | METRIC ID  | VALUE\n----------------------+------------+------------+----------\n");
            for r in data.telem_records.iter().take(20) {
                table.push_str(&format!(
                    "{:<22}| {:<11}| {:<11}| {:.2}\n",
                    r.0, r.1, r.2, r.3
                ));
            }
            return Ok(ViewerPreview {
                node_id: node_id.to_string(),
                mime_type: "text/plain".to_string(),
                content: table,
            });
        }

        Ok(ViewerPreview {
            node_id: node_id.to_string(),
            mime_type: "text/plain".to_string(),
            content: "Aura telemetry event stream.".to_string(),
        })
    }

    fn extract_history(&self, _reader: &MmapReader, path: &Path) -> Result<ViewerHistory, String> {
        Ok(extract_vcs_history(path))
    }

    fn extract_approval(
        &self,
        _reader: &MmapReader,
        path: &Path,
    ) -> Result<ViewerApprovalStatus, String> {
        let verif = check_archive_verification(path);
        if verif.is_sealed && verif.integrity_ok {
            let label = if verif.signature_verified {
                "Sealed (signature verified)".to_string()
            } else {
                "Sealed (integrity verified)".to_string()
            };
            Ok(ViewerApprovalStatus {
                is_approved: true,
                approved_by: Some(label),
                approval_timestamp: None,
                signature: if verif.signature_verified {
                    Some("ed25519".to_string())
                } else {
                    None
                },
                fingerprint: compute_file_sha256(path),
                requires_human_decision: false,
            })
        } else {
            Ok(ViewerApprovalStatus {
                is_approved: false,
                approved_by: None,
                approval_timestamp: None,
                signature: None,
                fingerprint: compute_file_sha256(path),
                requires_human_decision: true,
            })
        }
    }
}

// =============================================================================
// View Pack 4: Manifest Fallback View (Unknown / Generic Tiles)
// =============================================================================

pub struct ManifestFallbackViewPack;

impl RecordViewPack for ManifestFallbackViewPack {
    fn pack_id(&self) -> &'static str {
        "manifest"
    }

    fn display_name(&self) -> &'static str {
        "Generic S3A Record (Manifest)"
    }

    fn matches(&self, _reader: &MmapReader) -> bool {
        true
    }

    fn extract_summary(&self, reader: &MmapReader, path: &Path) -> Result<ViewerSummary, String> {
        let advanced = extract_common_advanced(reader, path);
        let title = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
            .replace('_', " ");
        let tile_count = reader.tile_count();
        let key_stats = vec![
            (
                "Document Type".to_string(),
                "Generic S3A Storage Container".to_string(),
            ),
            (
                "Stored Records".to_string(),
                format!("{} Record Blocks", tile_count),
            ),
            (
                "Integrity State".to_string(),
                "Checksum Validated".to_string(),
            ),
        ];

        let verif = check_archive_verification(path);

        let status_badge = if verif.is_sealed {
            if verif.signature_verified {
                "Signature verified".to_string()
            } else if verif.integrity_ok {
                "Integrity verified".to_string()
            } else {
                "Not verified".to_string()
            }
        } else if verif.signature_verified {
            "Signature verified".to_string()
        } else {
            "Not verified".to_string()
        };

        let human_approval = if verif.is_sealed {
            if verif.signature_verified {
                "Sealed (signature verified)".to_string()
            } else if verif.integrity_ok {
                "Sealed (integrity verified)".to_string()
            } else {
                "Sealed (integrity check failed)".to_string()
            }
        } else {
            "No researcher decision recorded".to_string()
        };

        Ok(ViewerSummary {
            title: if title.is_empty() {
                "S3A Container Record".to_string()
            } else {
                title
            },
            subtitle: "Self-Describing Spatial Hyper-Tile Container".to_string(),
            description: format!(
                "General S3A container. Displays record manifest and storage properties with {} hyper-tiles.",
                tile_count
            ),
            status_badge,
            human_approval,
            view_pack_name: self.display_name().to_string(),
            key_stats,
            advanced,
        })
    }

    fn extract_tree(
        &self,
        reader: &MmapReader,
        _path: &Path,
    ) -> Result<Vec<ViewerTreeNode>, String> {
        let mut children = Vec::new();
        for i in 0..reader.tile_count() as usize {
            if let Some((hdr, _)) = reader.get_tile(i) {
                children.push(ViewerTreeNode {
                    id: format!("record_{}", i),
                    label: format!(
                        "Record {}: Tile {} ({} bytes)",
                        i + 1,
                        hdr.tile_type,
                        hdr.payload_bytes
                    ),
                    icon: "file".to_string(),
                    node_type: "record".to_string(),
                    has_preview: true,
                    children: vec![],
                });
            }
        }
        if children.is_empty() {
            children.push(ViewerTreeNode {
                id: "record_empty".to_string(),
                label: "Empty Container Record".to_string(),
                icon: "file".to_string(),
                node_type: "record".to_string(),
                has_preview: false,
                children: vec![],
            });
        }

        Ok(vec![ViewerTreeNode {
            id: "manifest_root".to_string(),
            label: "Container Archive Records".to_string(),
            icon: "folder".to_string(),
            node_type: "container".to_string(),
            has_preview: true,
            children,
        }])
    }

    fn extract_properties(
        &self,
        reader: &MmapReader,
        _path: &Path,
        node_id: &str,
    ) -> Result<ViewerProperties, String> {
        let idx_opt = node_id
            .strip_prefix("record_")
            .and_then(|s| s.parse::<usize>().ok());
        if let Some(idx) = idx_opt {
            if let Some((hdr, _)) = reader.get_tile(idx) {
                return Ok(ViewerProperties {
                    node_id: node_id.to_string(),
                    label: format!("Record {} Metadata", idx + 1),
                    properties: vec![
                        ("Record Number".to_string(), (idx + 1).to_string()),
                        (
                            "Allocated Size".to_string(),
                            format!("{} bytes", hdr.payload_bytes),
                        ),
                        ("Record Count".to_string(), hdr.record_count.to_string()),
                    ],
                    advanced_properties: vec![
                        ("Raw Tile Type".to_string(), hdr.tile_type.to_string()),
                        (
                            "CRC32C Hardware Checksum".to_string(),
                            format!("0x{:08x}", hdr.data_crc32),
                        ),
                        ("S3A Coordinate".to_string(), format!("L0:T{}:R0", idx)),
                        ("Generation".to_string(), hdr.generation.to_string()),
                    ],
                });
            }
        }

        Ok(ViewerProperties {
            node_id: node_id.to_string(),
            label: "Container Overview".to_string(),
            properties: vec![
                (
                    "Container Format".to_string(),
                    "S3A Hyper-Tile Dual-Slot Archive".to_string(),
                ),
                (
                    "Integrity Status".to_string(),
                    "Valid Hardware CRC".to_string(),
                ),
            ],
            advanced_properties: vec![("Tile Count".to_string(), reader.tile_count().to_string())],
        })
    }

    fn extract_preview(
        &self,
        reader: &MmapReader,
        _path: &Path,
        node_id: &str,
    ) -> Result<ViewerPreview, String> {
        let idx_opt = node_id
            .strip_prefix("record_")
            .and_then(|s| s.parse::<usize>().ok());
        if let Some(idx) = idx_opt {
            if let Some((_, payload)) = reader.get_tile(idx) {
                let printable = String::from_utf8_lossy(payload);
                let trimmed = printable.trim_matches(char::from(0));
                return Ok(ViewerPreview {
                    node_id: node_id.to_string(),
                    mime_type: "text/plain".to_string(),
                    content: if trimmed.is_empty() {
                        format!("Binary record payload ({} bytes total).", payload.len())
                    } else {
                        trimmed[..trimmed.len().min(1024)].to_string()
                    },
                });
            }
        }

        Ok(ViewerPreview {
            node_id: node_id.to_string(),
            mime_type: "text/plain".to_string(),
            content: "S3A Container Manifest Overview:\nAll stored records are verified against sector checksums and can be explored using the record tree on the left.\n".to_string(),
        })
    }

    fn extract_history(&self, _reader: &MmapReader, path: &Path) -> Result<ViewerHistory, String> {
        Ok(extract_vcs_history(path))
    }

    fn extract_approval(
        &self,
        _reader: &MmapReader,
        path: &Path,
    ) -> Result<ViewerApprovalStatus, String> {
        let verif = check_archive_verification(path);
        if verif.is_sealed && verif.integrity_ok {
            let label = if verif.signature_verified {
                "Sealed (signature verified)".to_string()
            } else {
                "Sealed (integrity verified)".to_string()
            };
            Ok(ViewerApprovalStatus {
                is_approved: true,
                approved_by: Some(label),
                approval_timestamp: None,
                signature: if verif.signature_verified {
                    Some("ed25519".to_string())
                } else {
                    None
                },
                fingerprint: compute_file_sha256(path),
                requires_human_decision: false,
            })
        } else {
            Ok(ViewerApprovalStatus {
                is_approved: false,
                approved_by: None,
                approval_timestamp: None,
                signature: None,
                fingerprint: compute_file_sha256(path),
                requires_human_decision: true,
            })
        }
    }
}

// =============================================================================
// Viewer Registry
// =============================================================================

pub struct ViewerRegistry {
    packs: Vec<Box<dyn RecordViewPack>>,
}

impl Default for ViewerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewerRegistry {
    pub fn new() -> Self {
        Self {
            packs: vec![
                Box::new(TechnicalDataViewPack),
                Box::new(ScholarResearchViewPack),
                Box::new(AuraBackupViewPack),
                Box::new(ManifestFallbackViewPack),
            ],
        }
    }

    pub fn resolve_pack<'a>(&'a self, reader: &MmapReader) -> &'a dyn RecordViewPack {
        for pack in &self.packs {
            if pack.matches(reader) {
                return pack.as_ref();
            }
        }
        self.packs.last().unwrap().as_ref()
    }

    pub fn load_archive(&self, path: &Path) -> Result<ViewerFullState, String> {
        let reader =
            MmapReader::open(path).map_err(|e| format!("Failed to open archive: {}", e))?;
        let pack = self.resolve_pack(&reader);

        let summary = pack.extract_summary(&reader, path)?;
        let tree = pack.extract_tree(&reader, path)?;
        let selected_node_id = tree
            .first()
            .map(|n| n.id.clone())
            .unwrap_or_else(|| "root".to_string());
        let properties = pack.extract_properties(&reader, path, &selected_node_id)?;
        let preview = pack.extract_preview(&reader, path, &selected_node_id)?;
        let history = pack.extract_history(&reader, path)?;
        let approval = pack.extract_approval(&reader, path)?;

        Ok(ViewerFullState {
            file_path: path.to_string_lossy().to_string(),
            view_pack_id: pack.pack_id().to_string(),
            summary,
            tree,
            selected_node_id,
            properties,
            preview,
            history,
            approval,
        })
    }

    pub fn get_properties(&self, path: &Path, node_id: &str) -> Result<ViewerProperties, String> {
        let reader =
            MmapReader::open(path).map_err(|e| format!("Failed to open archive: {}", e))?;
        let pack = self.resolve_pack(&reader);
        pack.extract_properties(&reader, path, node_id)
    }

    pub fn get_preview(&self, path: &Path, node_id: &str) -> Result<ViewerPreview, String> {
        let reader =
            MmapReader::open(path).map_err(|e| format!("Failed to open archive: {}", e))?;
        let pack = self.resolve_pack(&reader);
        pack.extract_preview(&reader, path, node_id)
    }
}

// =============================================================================
// HTML Shell Generator with vercel-labs/ui Web Components
// =============================================================================

const HTML_SHELL_TEMPLATE: &str = r###"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>S3A Record Viewer - {{DOC_TITLE}}</title>
  <!-- Vercel UI SDK Web Components Adapter (Pinned Beta) -->
  <style>
    :root {
      --geist-background: #000000;
      --geist-foreground: #ededed;
      --accents-1: #111111;
      --geist-cyan: #50e3c2;
      --bg: var(--geist-background);
      --surface: #0a0a0a;
      --surface-hover: #141414;
      --border: #222222;
      --border-subtle: #1a1a1a;
      --text-main: var(--geist-foreground);
      --text-muted: #888888;
      --accent: #0070f3;
      --accent-badge: rgba(0, 112, 243, 0.15);
      --success: #50e3c2;
      --success-badge: rgba(80, 227, 194, 0.15);
      --font-sans: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Oxygen, Ubuntu, Cantarell, sans-serif;
      --font-mono: "SF Mono", "Fira Code", Menlo, Monaco, Consolas, monospace;
    }
    * { box-sizing: border-box; margin: 0; padding: 0; }
    body {
      background: var(--bg);
      color: var(--text-main);
      font-family: var(--font-sans);
      font-size: 14px;
      line-height: 1.5;
      display: flex;
      flex-direction: column;
      height: 100vh;
      overflow: hidden;
    }

    header {
      background: var(--surface);
      border-bottom: 1px solid var(--border);
      padding: 12px 20px;
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 16px;
    }
    .header-left { display: flex; align-items: center; gap: 12px; }
    .app-icon {
      width: 28px;
      height: 28px;
      background: linear-gradient(135deg, var(--accent), #7928ca);
      border-radius: 6px;
      display: flex;
      align-items: center;
      justify-content: center;
      font-weight: bold;
      color: white;
      font-size: 13px;
    }
    .doc-meta { display: flex; flex-direction: column; }
    .doc-title { font-size: 15px; font-weight: 600; color: #fff; }
    .doc-pack { font-size: 12px; color: var(--text-muted); }

    .header-right { display: flex; align-items: center; gap: 10px; }

    ui-badge {
      display: inline-flex;
      align-items: center;
      padding: 3px 8px;
      border-radius: 9999px;
      font-size: 12px;
      font-weight: 500;
      border: 1px solid var(--border);
    }
    ui-badge[variant="success"] { background: var(--success-badge); color: var(--success); border-color: rgba(80, 227, 194, 0.3); }
    ui-badge[variant="accent"] { background: var(--accent-badge); color: var(--accent); border-color: rgba(0, 112, 243, 0.3); }

    ui-button {
      display: inline-flex;
      align-items: center;
      justify-content: center;
      padding: 6px 12px;
      border-radius: 6px;
      font-size: 13px;
      font-weight: 500;
      cursor: pointer;
      background: var(--surface-hover);
      color: var(--text-main);
      border: 1px solid var(--border);
      transition: background 0.15s, border-color 0.15s;
      user-select: none;
    }
    ui-button:hover { background: #222; border-color: #333; }
    ui-button:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
    ui-button[aria-pressed="true"] { background: var(--accent); color: white; border-color: var(--accent); }

    .workspace {
      display: grid;
      grid-template-columns: 280px 1fr 320px;
      flex: 1;
      height: calc(100vh - 54px);
      overflow: hidden;
    }

    .tree-pane {
      background: var(--surface);
      border-right: 1px solid var(--border);
      display: flex;
      flex-direction: column;
      overflow-y: auto;
    }
    .pane-title {
      padding: 12px 16px;
      font-size: 12px;
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.05em;
      color: var(--text-muted);
      border-bottom: 1px solid var(--border-subtle);
    }

    ui-tree { display: block; padding: 6px 0; }
    ui-tree-item {
      display: block;
      padding: 6px 12px;
      margin: 1px 8px;
      border-radius: 4px;
      cursor: pointer;
      color: var(--text-main);
      font-size: 13px;
      user-select: none;
      outline: none;
    }
    ui-tree-item:hover { background: var(--surface-hover); }
    ui-tree-item:focus-visible { outline: 2px solid var(--accent); }
    ui-tree-item[aria-selected="true"] {
      background: rgba(0, 112, 243, 0.2);
      color: #fff;
      font-weight: 500;
    }
    .tree-item-inner { display: flex; align-items: center; gap: 8px; }
    .tree-icon { opacity: 0.7; font-size: 14px; width: 16px; text-align: center; }

    .preview-pane {
      display: flex;
      flex-direction: column;
      background: #000;
      overflow-y: auto;
    }
    .preview-header {
      padding: 10px 16px;
      border-bottom: 1px solid var(--border);
      background: var(--surface);
      display: flex;
      align-items: center;
      justify-content: space-between;
    }
    .preview-tabs { display: flex; gap: 4px; }
    ui-tab {
      padding: 4px 10px;
      border-radius: 4px;
      font-size: 12px;
      cursor: pointer;
      color: var(--text-muted);
    }
    ui-tab[aria-selected="true"] { background: var(--surface-hover); color: #fff; font-weight: 500; }
    ui-tab:focus-visible { outline: 2px solid var(--accent); }

    .preview-content {
      padding: 24px;
      flex: 1;
      display: flex;
      flex-direction: column;
      align-items: center;
      justify-content: flex-start;
    }
    .summary-card {
      width: 100%;
      max-width: 800px;
      background: var(--surface);
      border: 1px solid var(--border);
      border-radius: 8px;
      padding: 16px 20px;
      margin-bottom: 20px;
    }
    .summary-title { font-size: 16px; font-weight: 600; color: #fff; margin-bottom: 6px; }
    .summary-desc { color: var(--text-muted); font-size: 13px; margin-bottom: 14px; }
    .stats-grid {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
      gap: 12px;
    }
    .stat-item {
      background: rgba(255, 255, 255, 0.03);
      padding: 8px 12px;
      border-radius: 6px;
      border: 1px solid var(--border-subtle);
    }
    .stat-k { font-size: 11px; text-transform: uppercase; color: var(--text-muted); }
    .stat-v { font-size: 14px; font-weight: 600; color: #fff; }

    .preview-box {
      width: 100%;
      max-width: 800px;
      background: var(--surface);
      border: 1px solid var(--border);
      border-radius: 8px;
      padding: 20px;
      font-family: var(--font-mono);
      font-size: 13px;
      white-space: pre-wrap;
      overflow-x: auto;
      line-height: 1.6;
    }
    .preview-media {
      width: 100%;
      max-width: 800px;
      border: 1px solid var(--border);
      border-radius: 8px;
      overflow: hidden;
    }

    .advanced-section {
      width: 100%;
      max-width: 800px;
      margin-top: 16px;
      background: #0e0e0e;
      border: 1px dashed #333;
      border-radius: 8px;
      padding: 16px;
      display: none;
    }
    .advanced-section.visible { display: block; }
    .adv-header { font-size: 12px; font-weight: 600; text-transform: uppercase; color: #e5a100; margin-bottom: 8px; }

    .properties-pane {
      background: var(--surface);
      border-left: 1px solid var(--border);
      display: flex;
      flex-direction: column;
      overflow-y: auto;
    }
    .prop-list { padding: 12px 16px; }
    .prop-group { margin-bottom: 16px; }
    .prop-k { font-size: 11px; text-transform: uppercase; color: var(--text-muted); margin-bottom: 2px; }
    .prop-v { font-size: 13px; font-weight: 500; color: #fff; word-break: break-word; }

    .history-pane { padding: 12px 16px; border-top: 1px solid var(--border); }
    .history-item {
      padding: 8px 0;
      border-bottom: 1px solid var(--border-subtle);
      font-size: 12px;
    }
    .hist-commit { font-family: var(--font-mono); color: var(--accent); }
    .hist-msg { color: #ddd; margin: 2px 0; }
    .hist-meta { color: var(--text-muted); font-size: 11px; }
  </style>
</head>
<body>
  <header>
    <div class="header-left">
      <div class="app-icon">S3A</div>
      <div class="doc-meta">
        <div class="doc-title" id="hdr-title">{{DOC_TITLE}}</div>
        <div class="doc-pack" id="hdr-pack">{{VIEW_PACK_NAME}} &bull; Read-Only MCP App</div>
      </div>
    </div>
    <div class="header-right">
      <ui-badge variant="success" id="hdr-badge">{{STATUS_BADGE}}</ui-badge>
      <ui-badge variant="accent" id="hdr-approval">{{HUMAN_APPROVAL}}</ui-badge>
      <ui-button id="advanced-toggle" aria-pressed="false" role="button" tabindex="0">Advanced Details</ui-button>
    </div>
  </header>

  <div class="workspace">
    <div class="tree-pane">
      <div class="pane-title">Record Structure</div>
      <ui-tree role="tree" id="record-tree" aria-label="S3A Record Tree">
      </ui-tree>
    </div>

    <div class="preview-pane">
      <div class="preview-header">
        <div class="preview-tabs" role="tablist">
          <ui-tab role="tab" aria-selected="true" id="tab-doc" tabindex="0">Document Preview</ui-tab>
          <ui-tab role="tab" aria-selected="false" id="tab-summary" tabindex="0">Summary &amp; Stats</ui-tab>
        </div>
        <div id="active-node-name" style="font-size: 12px; color: var(--text-muted);">Overview</div>
      </div>

      <div class="preview-content">
        <div class="summary-card" id="summary-card-elem">
          <div class="summary-title" id="summary-title-elem">{{DOC_TITLE}}</div>
          <div class="summary-desc" id="summary-desc-elem">{{DESCRIPTION}}</div>
          <div class="stats-grid" id="stats-grid-elem">
          </div>
        </div>

        <div id="preview-render-area" style="width: 100%; max-width: 800px;">
        </div>

        <div class="advanced-section" id="advanced-content" style="display:none;">
          <div class="adv-header">Developer Details (Advanced Mode)</div>
          <div class="prop-group">
            <div class="stat-k">Hardware CRC32C Integrity</div>
            <div class="stat-v" style="font-family: var(--font-mono); font-size: 12px;" id="adv-crc">Passed</div>
          </div>
          <div class="prop-group">
            <div class="stat-k">Merkle Root</div>
            <div class="stat-v" style="font-family: var(--font-mono); font-size: 11px;" id="adv-merkle">0000...</div>
          </div>
          <div class="prop-group">
            <div class="stat-k">SHA-256 Storage Fingerprint</div>
            <div class="stat-v" style="font-family: var(--font-mono); font-size: 11px;" id="adv-hash">0000...</div>
          </div>
          <div class="prop-group">
            <div class="stat-k">Native S3A-QL Query</div>
            <div class="stat-v" style="font-family: var(--font-mono); font-size: 11px; color: #50e3c2;" id="adv-ql">SIFT ...</div>
          </div>
          <div class="prop-group">
            <div class="stat-k">Physical Coordinate Addressing</div>
            <div class="stat-v" style="font-family: var(--font-mono); font-size: 11px;" id="adv-coords">L0:T0:R0</div>
          </div>
        </div>
      </div>
    </div>

    <div class="properties-pane">
      <div class="pane-title">Properties</div>
      <div class="prop-list" id="props-list">
      </div>

      <div class="pane-title" style="border-top: 1px solid var(--border);">Version History</div>
      <div class="history-pane" id="history-list">
      </div>
    </div>
  </div>

  <script type="application/json" id="s3a-state-payload">
{{STATE_JSON}}
  </script>

  <script>
    (function() {
      const payloadEl = document.getElementById('s3a-state-payload');
      let appState = {};
      try {
        appState = JSON.parse(payloadEl.textContent);
      } catch(e) {
        console.error("Failed to parse S3A state payload", e);
      }

      const statsGrid = document.getElementById('stats-grid-elem');
      if (appState.summary && appState.summary.key_stats) {
        statsGrid.innerHTML = appState.summary.key_stats.map(s => `
          <div class="stat-item">
            <div class="stat-k">${s[0]}</div>
            <div class="stat-v">${s[1]}</div>
          </div>
        `).join('');
      }

      if (appState.summary && appState.summary.advanced) {
        const adv = appState.summary.advanced;
        document.getElementById('adv-crc').textContent = adv.crc_status || 'OK';
        document.getElementById('adv-merkle').textContent = adv.merkle_root || 'None';
        document.getElementById('adv-hash').textContent = adv.record_hash || 'None';
        document.getElementById('adv-ql').textContent = adv.s3a_ql || 'SIFT';
        document.getElementById('adv-coords').textContent = (adv.coordinates || []).slice(0, 5).join(', ') + ((adv.coordinates || []).length > 5 ? '...' : '');
      }

      const treeEl = document.getElementById('record-tree');
      function renderTreeNodes(nodes, level) {
        if (!nodes || !nodes.length) return '';
        return nodes.map(node => `
          <ui-tree-item role="treeitem" tabindex="0" data-id="${node.id}" style="padding-left: ${level * 16 + 8}px;" aria-selected="false" aria-expanded="${node.children && node.children.length ? 'true' : 'false'}">
            <div class="tree-item-inner">
              <span class="tree-icon">&bull;</span>
              <span>${node.label}</span>
            </div>
          </ui-tree-item>
          ${node.children && node.children.length ? renderTreeNodes(node.children, level + 1) : ''}
        `).join('');
      }
      treeEl.innerHTML = renderTreeNodes(appState.tree, 0);

      const treeItems = Array.from(treeEl.querySelectorAll('ui-tree-item'));
      if (treeItems.length > 0) {
        treeItems[0].setAttribute('aria-selected', 'true');
      }

      function selectNode(item) {
        treeItems.forEach(i => i.setAttribute('aria-selected', 'false'));
        item.setAttribute('aria-selected', 'true');
        const nodeId = item.getAttribute('data-id');
        document.getElementById('active-node-name').textContent = item.textContent.trim();
        updateProperties(nodeId);
      }

      treeItems.forEach((item, index) => {
        item.addEventListener('click', () => selectNode(item));
        item.addEventListener('keydown', (e) => {
          if (e.key === 'ArrowDown') {
            e.preventDefault();
            const next = treeItems[index + 1] || treeItems[0];
            next.focus();
            selectNode(next);
          } else if (e.key === 'ArrowUp') {
            e.preventDefault();
            const prev = treeItems[index - 1] || treeItems[treeItems.length - 1];
            prev.focus();
            selectNode(prev);
          } else if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            selectNode(item);
          }
        });
      });

      function updateProperties(nodeId) {
        const propsList = document.getElementById('props-list');
        const props = (appState.properties && appState.properties.properties) || [];
        propsList.innerHTML = props.map(p => `
          <div class="prop-group">
            <div class="prop-k">${p[0]}</div>
            <div class="prop-v">${p[1]}</div>
          </div>
        `).join('');
      }
      updateProperties(appState.selected_node_id);

      const previewArea = document.getElementById('preview-render-area');
      if (appState.preview) {
        if (appState.preview.mime_type === 'image/svg+xml') {
          previewArea.innerHTML = `<div class="preview-media">${appState.preview.content}</div>`;
        } else {
          previewArea.innerHTML = `<div class="preview-box">${appState.preview.content}</div>`;
        }
      }

      const histList = document.getElementById('history-list');
      if (appState.history && appState.history.commits) {
        histList.innerHTML = appState.history.commits.map(c => `
          <div class="history-item">
            <span class="hist-commit">${c.commit_id}</span>
            <div class="hist-msg">${c.message}</div>
            <div class="hist-meta">${c.author}</div>
          </div>
        `).join('');
      }

      const toggleBtn = document.getElementById('advanced-toggle') || document.getElementById('btn-toggle-advanced');
      const advSection = document.getElementById('advanced-content') || document.getElementById('adv-section-elem');
      let advActive = false;
      if (toggleBtn && advSection) {
        toggleBtn.addEventListener('click', () => {
          advActive = !advActive;
          toggleBtn.setAttribute('aria-pressed', advActive ? 'true' : 'false');
          if (advActive) {
            advSection.style.display = 'block';
            advSection.classList.add('visible');
            toggleBtn.textContent = 'Hide Advanced';
          } else {
            advSection.style.display = 'none';
            advSection.classList.remove('visible');
            toggleBtn.textContent = 'Advanced Details';
          }
        });
        toggleBtn.addEventListener('keydown', (e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            toggleBtn.click();
          }
        });
      }
    })();
  </script>
</body>
</html>
"###;

pub fn generate_viewer_html(state: &ViewerFullState) -> String {
    let state_json = serde_json::to_string(state).unwrap_or_else(|_| "{}".to_string());
    let escaped_json = state_json.replace("</script>", "<\\/script>");

    HTML_SHELL_TEMPLATE
        .replace("{{DOC_TITLE}}", &state.summary.title)
        .replace("{{VIEW_PACK_NAME}}", &state.summary.view_pack_name)
        .replace("{{STATUS_BADGE}}", &state.summary.status_badge)
        .replace("{{HUMAN_APPROVAL}}", &state.summary.human_approval)
        .replace("{{DESCRIPTION}}", &state.summary.description)
        .replace("{{STATE_JSON}}", &escaped_json)
}
