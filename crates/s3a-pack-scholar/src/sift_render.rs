//! Human-readable rendering of decoded Scholar records for S3A-QL `SIFT`
//! results (usability fix B3, SIFT side).
//!
//! Before this module, every Scholar `SIFT` handler returned only a count
//! (`"Found 3 academic paper records"`). The CLI prints the handler's
//! `summary` verbatim (and, in `--json` mode, its `json_payload`), so
//! rendering lives here: each handler builds its `PackData` from
//! [`render_papers`]/[`render_human_lrs`]/[`render_ai_traces`], which emit
//! one readable line per decoded record plus a machine-readable JSON
//! payload with the same fields.
//!
//! ## Honesty constraint
//!
//! `AcademicPaperRecord` stores hashes only — no title, author, or DOI
//! string exists in the 64-byte record, so none of those can be recovered
//! from an archive. The renderers show every decoded field that *does*
//! exist (year, citations, venue, open-access flag, topic coordinates,
//! Hilbert index, both hashes, timestamp) and say so explicitly, in both
//! the text summary and the JSON payload (`"title": null, "doi": null`
//! plus a `note`). Nothing is invented to fill the gap. Showing real
//! titles/DOIs needs a design decision that does not exist yet: storing
//! dossier `paper_identity` (title/DOI/author) alongside each Tile 12
//! record, or in a companion tile the renderer can join against.

use s3a_core::S3ACoordinate;

use crate::records::{
    AcademicPaperRecord, AiTraceRecord, HumanLrsRecord, STEP_CLAIM, STEP_HALLUCINATION, STEP_NOTE,
    STEP_REASONING, STEP_TOOL_CALL,
};

/// Maximum number of decoded rows rendered in one SIFT summary. Beyond
/// this the summary notes how many rows were omitted (the caller can page
/// with `LIMIT n`); the JSON payload carries the same rendered rows.
pub const MAX_SIFT_RENDERED_ROWS: usize = 50;

/// Note attached to paper results, in both text and JSON forms.
pub const PAPER_IDENTITY_NOTE: &str = "Titles, authors, and DOI strings are NOT stored in Tile 12 records (hashes only) and cannot be recovered from this archive. Showing them needs dossier paper_identity stored alongside each record — a design decision, not yet implemented.";

/// Formats a 128-bit `[u64; 2]` UUID the way
/// `ScholarNoteClaimRecord::verification_statement_uuid_string` does.
pub fn format_uuid128(uuid: [u64; 2]) -> String {
    if uuid == [0u64; 2] {
        return "00000000-0000-0000-0000-000000000000".to_string();
    }
    let b0 = uuid[0].to_be_bytes();
    let b1 = uuid[1].to_be_bytes();
    let mut bytes = [0u8; 16];
    bytes[0..8].copy_from_slice(&b0);
    bytes[8..16].copy_from_slice(&b1);
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5],
        bytes[6], bytes[7],
        bytes[8], bytes[9],
        bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    )
}

/// Human-readable name for an `AiTraceRecord` step type.
pub fn step_type_str(step_type: u32) -> &'static str {
    match step_type {
        STEP_REASONING => "reasoning",
        STEP_TOOL_CALL => "tool_call",
        STEP_CLAIM => "claim",
        STEP_NOTE => "note",
        STEP_HALLUCINATION => "hallucination",
        _ => "unknown",
    }
}

fn year_str(year: u16) -> String {
    if year == 0 {
        "year unknown".to_string()
    } else {
        format!("year {year}")
    }
}

/// One readable text line for an academic paper record, plus a second line
/// with its hashes. No title or DOI is shown: none is stored.
pub fn paper_row(rec: &AcademicPaperRecord, coord: &S3ACoordinate) -> String {
    format!(
        "  {coord} · {year} · {} citations · OA {} · venue {} · warnings {} · topic ({:.3}, {:.3}, {:.3}) · hilbert {}\n    paper_id 0x{:016x} · doi_prefix_hash 0x{:016x} · added epoch {}",
        rec.citation_count,
        if rec.open_access_flag == 1 { "yes" } else { "no" },
        rec.venue_id,
        rec.warning_count,
        rec.topic_x,
        rec.topic_y,
        rec.topic_z,
        rec.hilbert_index,
        rec.paper_id,
        rec.doi_prefix_hash,
        rec.timestamp_sec,
        year = year_str(rec.year),
    )
}

/// One readable text line for a human LRS decision record.
pub fn human_lrs_row(rec: &HumanLrsRecord, coord: &S3ACoordinate) -> String {
    let verb = crate::xapi::resolve_verb_by_id(rec.verb_id)
        .map(|v| v.label)
        .unwrap_or("unknown-verb");
    format!(
        "  {coord} · session {} · actor 0x{:016x} · verb {} ({}) · score {:.2} · object 0x{:08x} · ai_trace {} · at epoch {}",
        format_uuid128(rec.session_uuid),
        rec.actor_hash,
        verb,
        rec.verb_id,
        rec.decision_score,
        rec.object_hash,
        rec.ai_trace_coord,
        rec.timestamp_sec,
    )
}

/// One readable text line for an AI trace record.
pub fn ai_trace_row(rec: &AiTraceRecord, coord: &S3ACoordinate) -> String {
    format!(
        "  {coord} · step {} ({}) · agent 0x{:016x} · confidence {:.2} · human {} · hilbert {} · at epoch {}",
        step_type_str(rec.step_type),
        rec.step_type,
        rec.agent_id,
        rec.confidence,
        rec.human_coord,
        rec.hilbert_index,
        rec.timestamp_sec,
    )
}

fn paper_json(rec: &AcademicPaperRecord, coord: &S3ACoordinate) -> serde_json::Value {
    serde_json::json!({
        "coord": coord.to_string(),
        // Deliberately null: no title/DOI string is stored in the record.
        "title": null,
        "doi": null,
        "year": if rec.year == 0 { serde_json::Value::Null } else { serde_json::json!(rec.year) },
        "citation_count": rec.citation_count,
        "open_access": rec.open_access_flag == 1,
        "venue_id": rec.venue_id,
        "warning_count": rec.warning_count,
        "topic": [rec.topic_x, rec.topic_y, rec.topic_z],
        "hilbert_index": rec.hilbert_index,
        "paper_id": format!("0x{:016x}", rec.paper_id),
        "doi_prefix_hash": format!("0x{:016x}", rec.doi_prefix_hash),
        "timestamp_sec": rec.timestamp_sec,
    })
}

fn human_lrs_json(rec: &HumanLrsRecord, coord: &S3ACoordinate) -> serde_json::Value {
    serde_json::json!({
        "coord": coord.to_string(),
        "session_uuid": format_uuid128(rec.session_uuid),
        "actor_hash": format!("0x{:016x}", rec.actor_hash),
        "verb_id": rec.verb_id,
        "verb": crate::xapi::resolve_verb_by_id(rec.verb_id).map(|v| v.label),
        "decision_score": rec.decision_score,
        "object_hash": format!("0x{:08x}", rec.object_hash),
        "ai_trace_coord": rec.ai_trace_coord.to_string(),
        "timestamp_sec": rec.timestamp_sec,
    })
}

fn ai_trace_json(rec: &AiTraceRecord, coord: &S3ACoordinate) -> serde_json::Value {
    serde_json::json!({
        "coord": coord.to_string(),
        "session_uuid": format_uuid128(rec.session_uuid),
        "agent_id": format!("0x{:016x}", rec.agent_id),
        "step_type": rec.step_type,
        "step": step_type_str(rec.step_type),
        "confidence": rec.confidence,
        "human_coord": rec.human_coord.to_string(),
        "hilbert_index": rec.hilbert_index,
        "timestamp_sec": rec.timestamp_sec,
    })
}

/// A rendered SIFT result: the human-readable `summary` the CLI prints,
/// and the machine-readable `json_payload` it embeds in `--json` mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedSift {
    pub summary: String,
    pub json_payload: String,
}

fn render_pack(
    headline: String,
    rows: &[String],
    total: usize,
    note: Option<&str>,
    json_records: Vec<serde_json::Value>,
) -> RenderedSift {
    let mut summary = headline;
    for row in rows {
        summary.push('\n');
        summary.push_str(row);
    }
    if total > rows.len() {
        summary.push_str(&format!(
            "\n  … and {} more record(s) not shown (re-run with LIMIT {} to page through them)",
            total - rows.len(),
            MAX_SIFT_RENDERED_ROWS
        ));
    }
    if let Some(note) = note {
        summary.push_str("\n  Note: ");
        summary.push_str(note);
    }
    let json_payload = serde_json::json!({
        "records": json_records,
        "rendered": rows.len(),
        "total": total,
        "note": note,
    })
    .to_string();
    RenderedSift {
        summary,
        json_payload,
    }
}

/// Renders decoded academic-paper rows for `SIFT ACADEMIC_PAPERS`.
pub fn render_papers(
    from_path: &str,
    rows: &[(AcademicPaperRecord, S3ACoordinate)],
) -> RenderedSift {
    let rendered: Vec<_> = rows.iter().take(MAX_SIFT_RENDERED_ROWS).collect();
    let text_rows: Vec<String> = rendered
        .iter()
        .map(|(rec, coord)| paper_row(rec, coord))
        .collect();
    let json_rows: Vec<serde_json::Value> = rendered
        .iter()
        .map(|(rec, coord)| paper_json(rec, coord))
        .collect();
    render_pack(
        format!(
            "Found {} academic paper records in '{}'",
            rows.len(),
            from_path
        ),
        &text_rows,
        rows.len(),
        if rows.is_empty() {
            None
        } else {
            Some(PAPER_IDENTITY_NOTE)
        },
        json_rows,
    )
}

/// Renders decoded human LRS rows for `SIFT HUMAN_LRS` /
/// `SIFT LEARNING_ACTIVITIES`.
pub fn render_human_lrs(from_path: &str, rows: &[(HumanLrsRecord, S3ACoordinate)]) -> RenderedSift {
    let rendered: Vec<_> = rows.iter().take(MAX_SIFT_RENDERED_ROWS).collect();
    let text_rows: Vec<String> = rendered
        .iter()
        .map(|(rec, coord)| human_lrs_row(rec, coord))
        .collect();
    let json_rows: Vec<serde_json::Value> = rendered
        .iter()
        .map(|(rec, coord)| human_lrs_json(rec, coord))
        .collect();
    render_pack(
        format!(
            "Found {} human decision/LRS records in '{}'",
            rows.len(),
            from_path
        ),
        &text_rows,
        rows.len(),
        None,
        json_rows,
    )
}

/// Renders decoded AI trace rows for `SIFT AI_TRACES`.
pub fn render_ai_traces(from_path: &str, rows: &[(AiTraceRecord, S3ACoordinate)]) -> RenderedSift {
    let rendered: Vec<_> = rows.iter().take(MAX_SIFT_RENDERED_ROWS).collect();
    let text_rows: Vec<String> = rendered
        .iter()
        .map(|(rec, coord)| ai_trace_row(rec, coord))
        .collect();
    let json_rows: Vec<serde_json::Value> = rendered
        .iter()
        .map(|(rec, coord)| ai_trace_json(rec, coord))
        .collect();
    render_pack(
        format!("Found {} AI trace records in '{}'", rows.len(), from_path),
        &text_rows,
        rows.len(),
        None,
        json_rows,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::records::{ScholarNoteClaimRecord, CLAIM_KIND_EMPIRICAL};

    #[test]
    fn test_paper_row_shows_decoded_fields_and_no_invented_title() {
        let rec = AcademicPaperRecord::new(
            0x1234,
            1_700_000_000,
            424242,
            0.1,
            0.2,
            0.3,
            25,
            2024,
            1,
            1,
            0,
            0x9999,
        );
        let coord = S3ACoordinate::new(0, 0, 0);
        let row = paper_row(&rec, &coord);
        assert!(row.contains("year 2024"), "{row}");
        assert!(row.contains("25 citations"), "{row}");
        assert!(row.contains("OA yes"), "{row}");
        assert!(row.contains("0.100, 0.200, 0.300"), "{row}");
        assert!(row.contains("hilbert 424242"), "{row}");
        assert!(row.contains("paper_id 0x0000000000001234"), "{row}");
        assert!(row.contains("doi_prefix_hash 0x0000000000009999"), "{row}");
        // No title or DOI string appears anywhere in the row.
        assert!(!row.to_ascii_lowercase().contains("title"));
        assert!(!row.contains("10."));
    }

    #[test]
    fn test_paper_row_year_zero_renders_unknown() {
        let rec = AcademicPaperRecord::new(1, 100, 0, 0.0, 0.0, 0.0, 0, 0, 0, 0, 0, 0);
        let row = paper_row(&rec, &S3ACoordinate::new(0, 0, 0));
        assert!(row.contains("year unknown"), "{row}");
        assert!(!row.contains("year 0"));
    }

    #[test]
    fn test_render_papers_note_and_json_nulls() {
        let rec = AcademicPaperRecord::new(
            0x1234,
            1_700_000_000,
            7,
            0.5,
            0.5,
            0.5,
            3,
            2024,
            2,
            0,
            1,
            0x42,
        );
        let rendered = render_papers("test.s3a", &[(rec, S3ACoordinate::new(0, 0, 0))]);
        assert!(rendered.summary.contains("Found 1 academic paper records"));
        assert!(rendered.summary.contains("paper_identity"));
        assert!(rendered.summary.contains("cannot be recovered"));
        let payload: serde_json::Value = serde_json::from_str(&rendered.json_payload).unwrap();
        assert_eq!(payload["total"], 1);
        assert_eq!(payload["records"][0]["title"], serde_json::Value::Null);
        assert_eq!(payload["records"][0]["doi"], serde_json::Value::Null);
        assert_eq!(payload["records"][0]["year"], 2024);
        assert_eq!(payload["records"][0]["paper_id"], "0x0000000000001234");
        assert!(payload["note"].as_str().unwrap().contains("paper_identity"));
    }

    #[test]
    fn test_render_papers_empty_has_no_note_and_caps_rows() {
        let empty = render_papers("test.s3a", &[]);
        assert!(empty.summary.contains("Found 0 academic paper records"));
        assert!(!empty.summary.contains("Note:"));

        let many: Vec<(AcademicPaperRecord, S3ACoordinate)> = (0..60)
            .map(|i| {
                (
                    AcademicPaperRecord::new(i, 100, i, 0.0, 0.0, 0.0, 0, 2024, 0, 0, 0, 0),
                    S3ACoordinate::new(0, 0, i as u32),
                )
            })
            .collect();
        let rendered = render_papers("test.s3a", &many);
        assert!(rendered.summary.contains("and 10 more record(s) not shown"));
        let payload: serde_json::Value = serde_json::from_str(&rendered.json_payload).unwrap();
        assert_eq!(payload["rendered"], MAX_SIFT_RENDERED_ROWS);
        assert_eq!(payload["total"], 60);
    }

    #[test]
    fn test_human_and_ai_rows_resolve_labels() {
        let human = HumanLrsRecord::new(
            [0x1111, 0x2222],
            1_600_000_000,
            999,
            crate::xapi::VERB_ID_REVIEWED,
            1.0,
            S3ACoordinate::new(0, 1, 0),
            42,
        );
        let row = human_lrs_row(&human, &S3ACoordinate::new(0, 0, 0));
        assert!(row.contains("reviewed"), "{row}");
        assert!(row.contains("1111"), "{row}");
        assert!(row.contains("L0:T1:R0"), "{row}");

        let ai = AiTraceRecord::new(
            [0, 0],
            1_600_000_001,
            888,
            STEP_HALLUCINATION,
            0.42,
            S3ACoordinate::new(0, 0, 0),
            500,
        );
        let ai_row_text = ai_trace_row(&ai, &S3ACoordinate::new(0, 1, 0));
        assert!(ai_row_text.contains("hallucination"), "{ai_row_text}");
        assert!(ai_row_text.contains("0.42"), "{ai_row_text}");
        // Keep the claim-record import used (guards the step constants' provenance).
        assert_eq!(CLAIM_KIND_EMPIRICAL, 1);
        let _ = ScholarNoteClaimRecord::new(1, 0, [0, 0], 0, 0.0, CLAIM_KIND_EMPIRICAL, "");
    }
}
