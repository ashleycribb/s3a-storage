use std::collections::HashMap;
use std::io;
use std::path::Path;
use s3a_core::{ADDON_EDU, MarkRecord, TileType, MARK_TARGET_ALL_RECORDS};
use s3a_engine::{MmapReader, QuerySieve, S3ACrudEngine, TileWriter};

use crate::pack::ScholarTileType;
use crate::records::{
    AcademicPaperRecord, AgentRegistryRecord, AiTraceRecord, HumanLrsRecord, ResearchGraphEdgeRecord,
    ScholarNoteClaimRecord, SrsManifestHeader, SrsWorkspaceRecord, SCHOLAR_LAYOUT_VERSION_V1,
};
use crate::ext::{ScholarCrudExt, ScholarQueryExt};

/// SCHOLAR RESEARCH SNAPSHOT (SRS) BUNDLE.
/// Represents a fully reified, portable research project milestone containing
/// manifest, workspace context, literature corpus, evidence relationships, dual-actor provenance,
/// claims/notes, and carried policy marks (QUARANTINE, CONSENT_WITHDRAWN, LEGAL_HOLD).
#[derive(Debug, Clone, PartialEq)]
pub struct SrsSnapshotBundle {
    pub manifest: SrsManifestHeader,
    pub session_uuid: [u64; 2],
    pub base_snapshot_uuid: [u64; 2],
    pub last_applied_delta_seq: u64,
    pub workspace: Option<SrsWorkspaceRecord>,
    pub papers: Vec<AcademicPaperRecord>,
    pub evidence_edges: Vec<ResearchGraphEdgeRecord>,
    pub human_decisions: Vec<HumanLrsRecord>,
    pub ai_traces: Vec<AiTraceRecord>,
    pub agents: Vec<AgentRegistryRecord>,
    pub claims: Vec<ScholarNoteClaimRecord>,
    pub policy_marks: Vec<MarkRecord>,
}

/// EXPORT SRS SNAPSHOT: Packs all active SEROM records from a live database into an immutable .snapshot.s3a container.
///
/// Tile 15 (SrsManifestHeader) is authoritative and written into the container after all payload tiles.
pub fn export_srs_snapshot<P1: AsRef<Path>, P2: AsRef<Path>>(
    live_db_path: P1,
    dest_path: P2,
    manifest: &SrsManifestHeader,
    workspace: Option<&SrsWorkspaceRecord>,
    project_uuid: Option<[u64; 2]>,
) -> io::Result<SrsSnapshotBundle> {
    let reader = MmapReader::open(live_db_path)?;
    let sieve = QuerySieve::new(&reader).with_hidden_marks(true);

    let mut tile_remap: HashMap<u64, u64> = HashMap::new();
    let mut record_remap: HashMap<(u64, u32), (u64, u32)> = HashMap::new();

    let ws_pairs = if let Some(ws) = workspace {
        vec![(*ws, s3a_core::S3ACoordinate::new(0, 0, 0))]
    } else {
        sieve.query_srs_workspace(project_uuid)
    };
    let paper_pairs = sieve.query_academic_papers_3d([-f32::INFINITY; 3], [f32::INFINITY; 3], 0, u16::MAX);
    let edge_pairs = sieve.query_research_graph(None, None);
    let human_pairs = sieve.query_human_lrs(None, None, 0, u64::MAX);
    let ai_pairs = sieve.query_ai_traces(None, None, 0, u64::MAX);
    let agent_pairs = sieve.query_agent_registry(None);
    let claim_pairs = sieve.query_note_claims(None, None);

    let ws_records: Vec<SrsWorkspaceRecord> = ws_pairs.iter().map(|(r, _)| *r).collect();
    let papers: Vec<AcademicPaperRecord> = paper_pairs.iter().map(|(r, _)| *r).collect();
    let edges: Vec<ResearchGraphEdgeRecord> = edge_pairs.iter().map(|(r, _)| *r).collect();
    let human: Vec<HumanLrsRecord> = human_pairs.iter().map(|(r, _)| *r).collect();
    let ai: Vec<AiTraceRecord> = ai_pairs.iter().map(|(r, _)| *r).collect();
    let agents: Vec<AgentRegistryRecord> = agent_pairs.iter().map(|(r, _)| *r).collect();
    let claims: Vec<ScholarNoteClaimRecord> = claim_pairs.iter().map(|(r, _)| *r).collect();

    let mut writer = TileWriter::create(&dest_path)?;
    // Enable Scholar Pack capability on the snapshot container
    writer.enable_addon(ADDON_EDU)?;

    if !ws_records.is_empty() {
        let written_tid = writer.write_hyper_tile(ScholarTileType::SRS_WORKSPACE, &ws_records, None, None)?;
        for (idx, (_, ptr)) in ws_pairs.iter().enumerate() {
            let actual_old_tid = reader.get_tile(ptr.tile_id as usize).map(|(h, _)| h.tile_id).unwrap_or(ptr.tile_id as u64);
            tile_remap.insert(ptr.tile_id as u64, written_tid);
            tile_remap.insert(actual_old_tid, written_tid);
            record_remap.insert((ptr.tile_id as u64, ptr.record_offset), (written_tid, idx as u32));
            record_remap.insert((actual_old_tid, ptr.record_offset), (written_tid, idx as u32));
        }
    }
    if !papers.is_empty() {
        let timestamps: Vec<u64> = papers.iter().map(|p| p.timestamp_sec).collect();
        let written_tid = writer.write_hyper_tile(ScholarTileType::ACADEMIC_PAPERS, &papers, Some(&timestamps), None)?;
        for (idx, (_, ptr)) in paper_pairs.iter().enumerate() {
            let actual_old_tid = reader.get_tile(ptr.tile_id as usize).map(|(h, _)| h.tile_id).unwrap_or(ptr.tile_id as u64);
            tile_remap.insert(ptr.tile_id as u64, written_tid);
            tile_remap.insert(actual_old_tid, written_tid);
            record_remap.insert((ptr.tile_id as u64, ptr.record_offset), (written_tid, idx as u32));
            record_remap.insert((actual_old_tid, ptr.record_offset), (written_tid, idx as u32));
        }
    }
    if !edges.is_empty() {
        let timestamps: Vec<u64> = edges.iter().map(|e| e.timestamp_sec).collect();
        let written_tid = writer.write_hyper_tile(ScholarTileType::RESEARCH_GRAPH, &edges, Some(&timestamps), None)?;
        for (idx, (_, ptr)) in edge_pairs.iter().enumerate() {
            let actual_old_tid = reader.get_tile(ptr.tile_id as usize).map(|(h, _)| h.tile_id).unwrap_or(ptr.tile_id as u64);
            tile_remap.insert(ptr.tile_id as u64, written_tid);
            tile_remap.insert(actual_old_tid, written_tid);
            record_remap.insert((ptr.tile_id as u64, ptr.record_offset), (written_tid, idx as u32));
            record_remap.insert((actual_old_tid, ptr.record_offset), (written_tid, idx as u32));
        }
    }
    if !human.is_empty() {
        let timestamps: Vec<u64> = human.iter().map(|h| h.timestamp_sec).collect();
        let written_tid = writer.write_hyper_tile(ScholarTileType::HUMAN_LRS, &human, Some(&timestamps), None)?;
        for (idx, (_, ptr)) in human_pairs.iter().enumerate() {
            let actual_old_tid = reader.get_tile(ptr.tile_id as usize).map(|(h, _)| h.tile_id).unwrap_or(ptr.tile_id as u64);
            tile_remap.insert(ptr.tile_id as u64, written_tid);
            tile_remap.insert(actual_old_tid, written_tid);
            record_remap.insert((ptr.tile_id as u64, ptr.record_offset), (written_tid, idx as u32));
            record_remap.insert((actual_old_tid, ptr.record_offset), (written_tid, idx as u32));
        }
    }
    if !ai.is_empty() {
        let timestamps: Vec<u64> = ai.iter().map(|a| a.timestamp_sec).collect();
        let written_tid = writer.write_hyper_tile(ScholarTileType::AI_TRACE_LOG, &ai, Some(&timestamps), None)?;
        for (idx, (_, ptr)) in ai_pairs.iter().enumerate() {
            let actual_old_tid = reader.get_tile(ptr.tile_id as usize).map(|(h, _)| h.tile_id).unwrap_or(ptr.tile_id as u64);
            tile_remap.insert(ptr.tile_id as u64, written_tid);
            tile_remap.insert(actual_old_tid, written_tid);
            record_remap.insert((ptr.tile_id as u64, ptr.record_offset), (written_tid, idx as u32));
            record_remap.insert((actual_old_tid, ptr.record_offset), (written_tid, idx as u32));
        }
    }
    if !agents.is_empty() {
        let written_tid = writer.write_hyper_tile(ScholarTileType::AGENT_REGISTRY, &agents, None, None)?;
        for (idx, (_, ptr)) in agent_pairs.iter().enumerate() {
            let actual_old_tid = reader.get_tile(ptr.tile_id as usize).map(|(h, _)| h.tile_id).unwrap_or(ptr.tile_id as u64);
            tile_remap.insert(ptr.tile_id as u64, written_tid);
            tile_remap.insert(actual_old_tid, written_tid);
            record_remap.insert((ptr.tile_id as u64, ptr.record_offset), (written_tid, idx as u32));
            record_remap.insert((actual_old_tid, ptr.record_offset), (written_tid, idx as u32));
        }
    }
    if !claims.is_empty() {
        let timestamps: Vec<u64> = claims.iter().map(|c| c.timestamp_sec).collect();
        let written_tid = writer.write_hyper_tile(ScholarTileType::NOTE_CLAIMS, &claims, Some(&timestamps), None)?;
        for (idx, (_, ptr)) in claim_pairs.iter().enumerate() {
            let actual_old_tid = reader.get_tile(ptr.tile_id as usize).map(|(h, _)| h.tile_id).unwrap_or(ptr.tile_id as u64);
            tile_remap.insert(ptr.tile_id as u64, written_tid);
            tile_remap.insert(actual_old_tid, written_tid);
            record_remap.insert((ptr.tile_id as u64, ptr.record_offset), (written_tid, idx as u32));
            record_remap.insert((actual_old_tid, ptr.record_offset), (written_tid, idx as u32));
        }
    }

    // Carried policy marks: QUARANTINE, CONSENT_WITHDRAWN, LEGAL_HOLD. Ephemeral marks like ACCESS_HEAT are dropped.
    let all_active_marks = sieve.active_marks();
    let mut policy_marks: Vec<MarkRecord> = Vec::new();
    for mark in all_active_marks {
        if !mark.is_policy_mark() {
            continue;
        }
        let mut remapped = mark;
        if mark.record_index == MARK_TARGET_ALL_RECORDS {
            if let Some(&new_tid) = tile_remap.get(&mark.target_tile_id) {
                remapped.target_tile_id = new_tid;
                policy_marks.push(remapped);
            }
        } else if let Some(&(new_tid, new_idx)) = record_remap.get(&(mark.target_tile_id, mark.record_index)) {
            remapped.target_tile_id = new_tid;
            remapped.record_index = new_idx;
            policy_marks.push(remapped);
        }
    }

    if !policy_marks.is_empty() {
        writer.write_hyper_tile(TileType::MARKS, &policy_marks, None, None)?;
    }

    let payload_tiles = writer.tile_count();
    let final_tile_count = payload_tiles + 1; // including Tile 15 (manifest)
    let total_records = (ws_records.len() + papers.len() + edges.len() + human.len() + ai.len() + agents.len() + claims.len() + policy_marks.len()) as u64;

    let mut final_manifest = *manifest;
    final_manifest.magic = SrsManifestHeader::MAGIC;
    final_manifest.schema_version = SCHOLAR_LAYOUT_VERSION_V1;
    final_manifest.tile_count = final_tile_count;
    final_manifest.total_records = total_records;
    final_manifest.crc32c_checksum = final_manifest.compute_checksum();

    // Persist authoritative Tile 15 into the snapshot archive
    writer.write_hyper_tile(ScholarTileType::SRS_MANIFEST, &[final_manifest], None, None)?;
    drop(writer);

    // Snapshot exports are sealed by default per S3A spec v1.1-draft
    let temp_seal = dest_path.as_ref().with_extension("tmp.seal");
    s3a_engine::seal_archive(&dest_path, &temp_seal, s3a_engine::DEFAULT_SEAL_K, s3a_engine::DEFAULT_SEAL_M)?;
    s3a_engine::atomic_replace(&temp_seal, &dest_path)?;

    Ok(SrsSnapshotBundle {
        manifest: final_manifest,
        session_uuid: final_manifest.origin_session_uuid,
        base_snapshot_uuid: final_manifest.snapshot_uuid,
        last_applied_delta_seq: 0,
        workspace: ws_records.into_iter().next(),
        papers,
        evidence_edges: edges,
        human_decisions: human,
        ai_traces: ai,
        agents,
        claims,
        policy_marks,
    })
}

/// INTAKE SRS SNAPSHOT: Reads and validates a portable .snapshot.s3a container via zero-copy mmap.
///
/// Strictly enforces:
/// 1. Hardware CRC32C validation over all headers and tile payloads.
/// 2. ADDON_EDU capability bitmask on the container.
/// 3. Authoritative Tile 15 (SrsManifestHeader) existence, magic, schema version, and checksum.
pub fn intake_srs_snapshot<P: AsRef<Path>>(snapshot_path: P) -> io::Result<SrsSnapshotBundle> {
    let reader = MmapReader::open(snapshot_path)?;
    reader
        .verify_checksums()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("Checksum failure: {}", e)))?;

    let header = reader.file_header();
    if (header.addon_mask & ADDON_EDU) == 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "Snapshot archive does not have ADDON_EDU capability enabled (addon_mask=0x{:08X})",
                header.addon_mask
            ),
        ));
    }

    let sieve = QuerySieve::new(&reader);

    // Tile 15 is authoritative: must exist and pass validation
    let manifest = sieve.query_srs_manifest().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "Authoritative Tile 15 (SRS_MANIFEST) is missing from snapshot archive",
        )
    })?;

    if manifest.magic != SrsManifestHeader::MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "Invalid SRS manifest magic: expected {:?}, found {:?}",
                SrsManifestHeader::MAGIC,
                manifest.magic
            ),
        ));
    }

    if manifest.schema_version != SCHOLAR_LAYOUT_VERSION_V1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "Unsupported SRS schema layout version: {} (current supported: {})",
                manifest.schema_version, SCHOLAR_LAYOUT_VERSION_V1
            ),
        ));
    }

    if !manifest.verify_checksum() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "SRS manifest CRC32C checksum mismatch (expected 0x{:08X}, computed 0x{:08X})",
                manifest.crc32c_checksum,
                manifest.compute_checksum()
            ),
        ));
    }

    let ws = sieve.query_srs_workspace(None).into_iter().next().map(|(r, _)| r);
    let papers: Vec<AcademicPaperRecord> = sieve
        .query_academic_papers_3d([-f32::INFINITY; 3], [f32::INFINITY; 3], 0, u16::MAX)
        .into_iter()
        .map(|(r, _)| r)
        .collect();
    let edges: Vec<ResearchGraphEdgeRecord> = sieve
        .query_research_graph(None, None)
        .into_iter()
        .map(|(r, _)| r)
        .collect();
    let human: Vec<HumanLrsRecord> = sieve
        .query_human_lrs(None, None, 0, u64::MAX)
        .into_iter()
        .map(|(r, _)| r)
        .collect();
    let ai: Vec<AiTraceRecord> = sieve
        .query_ai_traces(None, None, 0, u64::MAX)
        .into_iter()
        .map(|(r, _)| r)
        .collect();
    let agents: Vec<AgentRegistryRecord> = sieve
        .query_agent_registry(None)
        .into_iter()
        .map(|(mut r, _)| {
            if r.is_legacy_unstamped() {
                r.sync_inverted_fields();
            }
            r
        })
        .collect();
    let claims: Vec<ScholarNoteClaimRecord> = sieve
        .query_note_claims(None, None)
        .into_iter()
        .map(|(r, _)| r)
        .collect();
    let policy_marks: Vec<MarkRecord> = sieve
        .active_marks()
        .into_iter()
        .filter(|m| m.is_policy_mark())
        .collect();

    Ok(SrsSnapshotBundle {
        manifest,
        session_uuid: manifest.origin_session_uuid,
        base_snapshot_uuid: manifest.snapshot_uuid,
        last_applied_delta_seq: 0,
        workspace: ws,
        papers,
        evidence_edges: edges,
        human_decisions: human,
        ai_traces: ai,
        agents,
        claims,
        policy_marks,
    })
}

/// INTAKE SRS SNAPSHOT INTO LIVE DATABASE:
/// Validates snapshot container, ensures ADDON_EDU capability on live database,
/// and appends snapshot records into the live database.
pub fn intake_srs_snapshot_into_live<P1: AsRef<Path>, P2: AsRef<Path>>(
    snapshot_path: P1,
    live_db_path: P2,
) -> io::Result<SrsSnapshotBundle> {
    let bundle = intake_srs_snapshot(snapshot_path)?;
    let engine = S3ACrudEngine::open_or_create(live_db_path)?;
    // Fail closed if unable to enable ADDON_EDU capability
    engine.enable_addon(ADDON_EDU)?;

    if let Some(ws) = &bundle.workspace {
        engine.create_srs_workspace(ws)?;
    }
    if !bundle.papers.is_empty() {
        engine.create_academic_papers(&bundle.papers)?;
    }
    if !bundle.evidence_edges.is_empty() {
        engine.create_research_graph_edges(&bundle.evidence_edges)?;
    }
    if !bundle.human_decisions.is_empty() {
        engine.create_human_lrs(&bundle.human_decisions)?;
    }
    if !bundle.ai_traces.is_empty() {
        engine.create_ai_traces(&bundle.ai_traces)?;
    }
    if !bundle.agents.is_empty() {
        engine.create_agent_registry(&bundle.agents)?;
    }
    if !bundle.claims.is_empty() {
        engine.create_note_claims(&bundle.claims)?;
    }
    if !bundle.policy_marks.is_empty() {
        engine.create_marks(&bundle.policy_marks)?;
    }

    Ok(bundle)
}
