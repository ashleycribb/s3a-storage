use s3a_core::{S3ACoordinate, ADDON_EDU};
use s3a_engine::S3ACrudEngine;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

use crate::ext::ScholarCrudExt;
use crate::records::{
    AcademicPaperRecord, AiTraceRecord, HumanLrsRecord, ResearchGraphEdgeRecord, SrsWorkspaceRecord,
};

/// Ingestion statistics returned after ingesting intermediate files into an S3A archive.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScholarIngestStats {
    pub papers_count: usize,
    pub edges_count: usize,
    pub human_count: usize,
    pub ai_count: usize,
    pub workspace_count: usize,
    pub total_tiles: u32,
}

/// Helper to parse a u64 field from a simple JSON string without full serde tree.
pub fn parse_json_u64(json: &str, key: &str) -> Option<u64> {
    let pattern = format!("\"{}\":", key);
    let start = json.find(&pattern)? + pattern.len();
    let rest = json[start..].trim_start();
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    if end == 0 {
        return None;
    }
    rest[..end].parse::<u64>().ok()
}

/// Helper to parse an f64 field from a simple JSON string without full serde tree.
pub fn parse_json_f64(json: &str, key: &str) -> Option<f64> {
    let pattern = format!("\"{}\":", key);
    let start = json.find(&pattern)? + pattern.len();
    let rest = json[start..].trim_start();
    let end = rest
        .find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-' && c != '+')
        .unwrap_or(rest.len());
    if end == 0 {
        return None;
    }
    rest[..end].parse::<f64>().ok()
}

/// Helper to parse a string field from a simple JSON string.
pub fn parse_json_str(json: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{}\":", key);
    let start = json.find(&pattern)? + pattern.len();
    let rest = json[start..].trim_start();
    if !rest.starts_with('"') {
        return None;
    }
    let inner = &rest[1..];
    let end = inner.find('"')?;
    Some(inner[..end].to_string())
}

/// INGEST SCHOLAR INTERMEDIATE DIRECTORY (`scholar_ingest_v1`) INTO S3A ARCHIVE:
/// Reads NDJSON files (academic_papers.jsonl, graph_edges.jsonl, human_lrs.jsonl, ai_traces.jsonl, workspace.json)
/// from intermediate_dir, enables ADDON_EDU capability, and writes them into out_s3a_path.
///
/// If `fresh` is true, any existing archive at `out_s3a_path` is removed first to ensure idempotency.
pub fn ingest_scholar_intermediate_dir<P1: AsRef<Path>, P2: AsRef<Path>>(
    intermediate_dir: P1,
    out_s3a_path: P2,
    fresh: bool,
) -> io::Result<ScholarIngestStats> {
    let in_dir = intermediate_dir.as_ref();
    let out_file = out_s3a_path.as_ref();

    if !in_dir.exists() || !in_dir.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "Intermediate directory does not exist: {}",
                in_dir.display()
            ),
        ));
    }

    if fresh && out_file.exists() {
        std::fs::remove_file(out_file)?;
    }

    if let Some(parent) = out_file.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let engine = S3ACrudEngine::open_or_create(out_file)?;
    // Strictly enable ADDON_EDU capability before writing Scholar pack tiles
    engine.enable_addon(ADDON_EDU)?;

    let mut stats = ScholarIngestStats::default();
    const BATCH_SIZE: usize = 2000;

    // 1. Workspace (workspace.json or workspace.jsonl)
    let ws_file = in_dir.join("workspace.json");
    if ws_file.exists() {
        if let Ok(content) = std::fs::read_to_string(&ws_file) {
            let high = parse_json_u64(&content, "project_high").unwrap_or(0);
            let low = parse_json_u64(&content, "project_low").unwrap_or(0);
            let q_hash = parse_json_u64(&content, "question_hash").unwrap_or(0);
            let coll = parse_json_u64(&content, "collection_mask").unwrap_or(0);
            let cx = parse_json_f64(&content, "centroid_x").unwrap_or(0.0) as f32;
            let cy = parse_json_f64(&content, "centroid_y").unwrap_or(0.0) as f32;
            let cz = parse_json_f64(&content, "centroid_z").unwrap_or(0.0) as f32;
            let status = parse_json_u64(&content, "status").unwrap_or(1) as u32;

            let ws_rec = SrsWorkspaceRecord::new([high, low], q_hash, coll, [cx, cy, cz], status);
            engine.create_srs_workspace(&ws_rec)?;
            stats.workspace_count = 1;
        }
    }

    // 2. Academic Papers (academic_papers.jsonl or papers.jsonl)
    let papers_path = if in_dir.join("academic_papers.jsonl").exists() {
        in_dir.join("academic_papers.jsonl")
    } else {
        in_dir.join("papers.jsonl")
    };

    if papers_path.exists() {
        let file = File::open(&papers_path)?;
        let reader = BufReader::new(file);
        let mut batch = Vec::with_capacity(BATCH_SIZE);

        for line in reader.lines() {
            let l = line?;
            let trimmed = l.trim();
            if trimmed.is_empty() {
                continue;
            }

            let pid = parse_json_u64(trimmed, "paper_id").unwrap_or(0);
            let ts = parse_json_u64(trimmed, "timestamp_sec").unwrap_or(0);
            let h_idx = parse_json_u64(trimmed, "hilbert_index").unwrap_or(0);
            let tx = parse_json_f64(trimmed, "topic_x").unwrap_or(0.0) as f32;
            let ty = parse_json_f64(trimmed, "topic_y").unwrap_or(0.0) as f32;
            let tz = parse_json_f64(trimmed, "topic_z").unwrap_or(0.0) as f32;
            let cit = parse_json_u64(trimmed, "citation_count").unwrap_or(0) as u32;
            let year = parse_json_u64(trimmed, "year").unwrap_or(2024) as u16;
            let venue = parse_json_u64(trimmed, "venue_id").unwrap_or(0) as u16;
            let oa = parse_json_u64(trimmed, "open_access_flag").unwrap_or(0) as u16;
            let warn = parse_json_u64(trimmed, "warning_count").unwrap_or(0) as u16;
            let doi_pfx = parse_json_u64(trimmed, "doi_prefix_hash").unwrap_or(0);

            batch.push(AcademicPaperRecord::new(
                pid, ts, h_idx, tx, ty, tz, cit, year, venue, oa, warn, doi_pfx,
            ));
            stats.papers_count += 1;

            if batch.len() >= BATCH_SIZE {
                engine.create_academic_papers(&batch)?;
                batch.clear();
            }
        }
        if !batch.is_empty() {
            engine.create_academic_papers(&batch)?;
        }
    }

    // 3. Knowledge Graph Edges (graph_edges.jsonl or edges.jsonl)
    let edges_path = if in_dir.join("graph_edges.jsonl").exists() {
        in_dir.join("graph_edges.jsonl")
    } else {
        in_dir.join("edges.jsonl")
    };

    if edges_path.exists() {
        let file = File::open(&edges_path)?;
        let reader = BufReader::new(file);
        let mut batch = Vec::with_capacity(BATCH_SIZE);

        for line in reader.lines() {
            let l = line?;
            let trimmed = l.trim();
            if trimmed.is_empty() {
                continue;
            }

            let sub = parse_json_u64(trimmed, "subject_hash").unwrap_or(0);
            let obj = parse_json_u64(trimmed, "object_hash").unwrap_or(0);
            let ts = parse_json_u64(trimmed, "timestamp_sec").unwrap_or(0);
            let h_coord = parse_json_u64(trimmed, "hilbert_coord").unwrap_or(0);
            let pred = parse_json_u64(trimmed, "predicate_id").unwrap_or(1) as u32;
            let weight = parse_json_f64(trimmed, "weight").unwrap_or(1.0) as f32;

            batch.push(ResearchGraphEdgeRecord::new(
                sub, obj, ts, h_coord, pred, weight,
            ));
            stats.edges_count += 1;

            if batch.len() >= BATCH_SIZE {
                engine.create_research_graph_edges(&batch)?;
                batch.clear();
            }
        }
        if !batch.is_empty() {
            engine.create_research_graph_edges(&batch)?;
        }
    }

    // 4. Human LRS Activity (human_lrs.jsonl)
    let human_path = in_dir.join("human_lrs.jsonl");
    if human_path.exists() {
        let file = File::open(&human_path)?;
        let reader = BufReader::new(file);
        let mut batch = Vec::with_capacity(BATCH_SIZE);

        for line in reader.lines() {
            let l = line?;
            let trimmed = l.trim();
            if trimmed.is_empty() {
                continue;
            }

            let high = parse_json_u64(trimmed, "session_high").unwrap_or(0);
            let low = parse_json_u64(trimmed, "session_low").unwrap_or(0);
            let ts = parse_json_u64(trimmed, "timestamp_sec").unwrap_or(0);
            let actor = parse_json_u64(trimmed, "actor_hash").unwrap_or(0);
            let verb = parse_json_u64(trimmed, "verb_id").unwrap_or(1) as u32;
            let score = parse_json_f64(trimmed, "decision_score").unwrap_or(1.0) as f32;
            let obj = parse_json_u64(trimmed, "object_hash").unwrap_or(0) as u32;
            let ai_t = parse_json_u64(trimmed, "ai_tile").unwrap_or(0) as u32;
            let ai_r = parse_json_u64(trimmed, "ai_record").unwrap_or(0) as u32;
            let ai_coord = S3ACoordinate::new(0, ai_t, ai_r);

            batch.push(HumanLrsRecord::new(
                [high, low],
                ts,
                actor,
                verb,
                score,
                ai_coord,
                obj,
            ));
            stats.human_count += 1;

            if batch.len() >= BATCH_SIZE {
                engine.create_human_lrs(&batch)?;
                batch.clear();
            }
        }
        if !batch.is_empty() {
            engine.create_human_lrs(&batch)?;
        }
    }

    // 5. AI Agent Traces (ai_traces.jsonl)
    let ai_path = in_dir.join("ai_traces.jsonl");
    if ai_path.exists() {
        let file = File::open(&ai_path)?;
        let reader = BufReader::new(file);
        let mut batch = Vec::with_capacity(BATCH_SIZE);

        for line in reader.lines() {
            let l = line?;
            let trimmed = l.trim();
            if trimmed.is_empty() {
                continue;
            }

            let high = parse_json_u64(trimmed, "session_high").unwrap_or(0);
            let low = parse_json_u64(trimmed, "session_low").unwrap_or(0);
            let ts = parse_json_u64(trimmed, "timestamp_sec").unwrap_or(0);
            let agent = parse_json_u64(trimmed, "agent_id").unwrap_or(0);
            let step = parse_json_u64(trimmed, "step_type").unwrap_or(1) as u32;
            let conf = parse_json_f64(trimmed, "confidence").unwrap_or(0.9) as f32;
            let h_t = parse_json_u64(trimmed, "human_tile").unwrap_or(0) as u32;
            let h_r = parse_json_u64(trimmed, "human_record").unwrap_or(0) as u32;
            let h_coord = S3ACoordinate::new(0, h_t, h_r);
            let hilbert = parse_json_u64(trimmed, "hilbert_index").unwrap_or(0) as u32;

            batch.push(AiTraceRecord::new(
                [high, low],
                ts,
                agent,
                step,
                conf,
                h_coord,
                hilbert,
            ));
            stats.ai_count += 1;

            if batch.len() >= BATCH_SIZE {
                engine.create_ai_traces(&batch)?;
                batch.clear();
            }
        }
        if !batch.is_empty() {
            engine.create_ai_traces(&batch)?;
        }
    }

    // Query reader to get total tiles written
    let reader = s3a_engine::MmapReader::open(out_file)?;
    stats.total_tiles = reader.tile_count();

    Ok(stats)
}
