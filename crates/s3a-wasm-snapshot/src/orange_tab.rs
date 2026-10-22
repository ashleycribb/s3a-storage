//! Orange Data Mining native `.tab` export (papers.tab + sessions.tab).
//!
//! Format (Orange docs, "Loading your Data" / Data Input):
//! <https://orange3.readthedocs.io/projects/orange-data-mining-library/en/latest/tutorial/data.html>
//! Row 1 = column names, row 2 = types (`continuous`/`discrete`/`string`/`time`),
//! row 3 = flags (`class`/`meta`/`ignore` or empty); values are tab-separated.
//!
//! This module is the column data model for a future S3A statistics app; the CLI is thin glue.
//!
//! Privacy rules enforced here:
//! - Tile 10/11 (human/AI) telemetry is exported only if the latest Tile-71 consent is an
//!   active, hash-valid Research grant for a study (same rule as `tracing_allowed`).
//! - Records under a set `MARK_CONSENT_WITHDRAWN` or `MARK_QUARANTINE` mark, or tombstoned,
//!   are never exported.
//! - No free text, emails, prompts or credentials: only numeric metadata and hashed ids.
//! - Actor and session ids are pseudonymised (salted SHA-256, 16 hex chars) by default.

use crate::agent_api::WasmAgentSnapshot;
use s3a_core::{MarkRecord, MARK_CONSENT_WITHDRAWN, MARK_QUARANTINE, MARK_TARGET_ALL_RECORDS};
use s3a_pack_scholar::ScholarTileType;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// Orange variable type (second header row).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrangeType {
    Continuous,
    Discrete,
    String,
    Time,
}

impl OrangeType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Continuous => "continuous",
            Self::Discrete => "discrete",
            Self::String => "string",
            Self::Time => "time",
        }
    }
}

/// Orange role flag (third header row).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrangeRole {
    Feature,
    Class,
    Meta,
    Ignore,
}

impl OrangeRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Feature => "",
            Self::Class => "class",
            Self::Meta => "meta",
            Self::Ignore => "ignore",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrangeColumn {
    pub name: &'static str,
    pub ty: OrangeType,
    pub role: OrangeRole,
}

const fn col(name: &'static str, ty: OrangeType, role: OrangeRole) -> OrangeColumn {
    OrangeColumn { name, ty, role }
}

/// A typed table: the reusable data model behind each `.tab` file.
#[derive(Debug, Clone, Default)]
pub struct OrangeTable {
    pub columns: Vec<OrangeColumn>,
    /// Row values; empty string = missing value.
    pub rows: Vec<Vec<String>>,
}

/// Escapes a cell: tabs, CR and LF (which would break the row/column structure) become spaces.
pub fn escape_cell(v: &str) -> String {
    v.chars()
        .map(|c| match c {
            '\t' | '\n' | '\r' => ' ',
            c => c,
        })
        .collect::<String>()
        .trim()
        .to_string()
}

impl OrangeTable {
    pub fn to_tab_string(&self) -> String {
        let mut out = String::new();
        let line = |cells: Vec<String>| cells.join("\t") + "\n";
        out += &line(self.columns.iter().map(|c| escape_cell(c.name)).collect());
        out += &line(self.columns.iter().map(|c| c.ty.as_str().into()).collect());
        out += &line(
            self.columns
                .iter()
                .map(|c| c.role.as_str().into())
                .collect(),
        );
        for r in &self.rows {
            out += &line(r.iter().map(|v| escape_cell(v)).collect());
        }
        out
    }

    /// Minimal parser for round-trip checks: returns (names, types, flags, rows).
    pub fn parse_tab(text: &str) -> Result<ParsedTab, String> {
        let mut lines = text.split('\n');
        let mut next = |what: &str| {
            lines
                .next()
                .map(|l| l.split('\t').map(str::to_string).collect::<Vec<_>>())
                .ok_or(format!("missing {what} row"))
        };
        let names = next("names")?;
        let types = next("types")?;
        let flags = next("flags")?;
        let n = names.len();
        if types.len() != n || flags.len() != n {
            return Err("header width mismatch".into());
        }
        let mut rows = Vec::new();
        for l in lines.filter(|l| !l.is_empty()) {
            let r: Vec<String> = l.split('\t').map(str::to_string).collect();
            if r.len() != n {
                return Err(format!("row width {} != {}", r.len(), n));
            }
            rows.push(r);
        }
        Ok(ParsedTab {
            names,
            types,
            flags,
            rows,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ParsedTab {
    pub names: Vec<String>,
    pub types: Vec<String>,
    pub flags: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// Export options. `salt` keys the pseudonymisation; `raw_ids` disables it (not default).
#[derive(Debug, Clone, Default)]
pub struct ExportOptions {
    pub salt: Vec<u8>,
    pub raw_ids: bool,
}

#[derive(Debug, Clone)]
pub struct OrangeExport {
    pub papers: OrangeTable,
    pub sessions: OrangeTable,
    /// True when telemetry was exported (active Research consent present).
    pub telemetry_consented: bool,
    pub excluded_records: usize,
}

fn pseudo(opts: &ExportOptions, kind: &str, bytes: &[u8]) -> String {
    if opts.raw_ids {
        return hex16(bytes);
    }
    let mut h = Sha256::new();
    h.update(&opts.salt);
    h.update(kind.as_bytes());
    h.update(bytes);
    hex16(&h.finalize()[..8])
}

fn hex16(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// ISO-8601 UTC date-time from unix seconds (Orange `time` type).
pub fn iso_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Howard Hinnant's civil_from_days
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}",
        rem / 3600,
        (rem / 60) % 60,
        rem % 60
    )
}

/// Indices blocked by set withdrawn/quarantine marks for a tile type. `None` = whole tile.
fn blocked(marks: &[MarkRecord], tile: u32) -> Option<BTreeSet<u32>> {
    let mut set = BTreeSet::new();
    for m in marks.iter().filter(|m| {
        m.target_tile_type == tile
            && m.is_set()
            && (m.mark_kind == MARK_CONSENT_WITHDRAWN || m.mark_kind == MARK_QUARANTINE)
    }) {
        if m.record_index == MARK_TARGET_ALL_RECORDS {
            return None;
        }
        set.insert(m.record_index);
    }
    Some(set)
}

fn allowed(b: &Option<BTreeSet<u32>>, i: usize) -> bool {
    matches!(b, Some(s) if !s.contains(&(i as u32)))
}

const TOMBSTONE: u64 = s3a_core::FLAG_TOMBSTONE;

pub fn paper_columns() -> Vec<OrangeColumn> {
    use OrangeRole::*;
    use OrangeType::*;
    vec![
        col("paper_key", String, Meta),
        col("year", Continuous, Feature),
        col("published", Time, Feature),
        col("citation_count", Continuous, Feature),
        col("open_access", Discrete, Feature),
        col("venue_id", Discrete, Feature),
        col("warning_count", Continuous, Feature),
        col("topic_x", Continuous, Feature),
        col("topic_y", Continuous, Feature),
        col("topic_z", Continuous, Feature),
    ]
}

pub fn session_columns() -> Vec<OrangeColumn> {
    use OrangeRole::*;
    use OrangeType::*;
    vec![
        col("session_key", String, Meta),
        col("started", Time, Feature),
        col("ended", Time, Feature),
        col("duration_sec", Continuous, Feature),
        col("human_events", Continuous, Feature),
        col("ai_events", Continuous, Feature),
        col("distinct_humans", Continuous, Feature),
        col("distinct_agents", Continuous, Feature),
        col("mean_decision_score", Continuous, Feature),
        col("mean_ai_confidence", Continuous, Feature),
    ]
}

/// Round to 4 decimals so f32 noise (0.800000011920929) exports as 0.8.
fn num4(v: f64) -> String {
    num((v * 1e4).round() / 1e4)
}

fn num(v: f64) -> String {
    if v.is_finite() {
        format!("{v}")
    } else {
        String::new()
    }
}

#[derive(Default)]
struct Acc {
    first: u64,
    last: u64,
    humans: usize,
    ais: usize,
    actors: BTreeSet<u64>,
    agents: BTreeSet<u64>,
    score_sum: f64,
    conf_sum: f64,
}

impl Acc {
    fn touch(&mut self, ts: u64) {
        if self.first == 0 || ts < self.first {
            self.first = ts;
        }
        self.last = self.last.max(ts);
    }
}

/// Converts a snapshot into the two Orange tables, applying the privacy rules above.
pub fn export_snapshot(snap: &WasmAgentSnapshot, opts: &ExportOptions) -> OrangeExport {
    let mut excluded = 0usize;
    let marks = &snap.policy_marks;

    let pb = blocked(marks, ScholarTileType::ACADEMIC_PAPERS);
    let mut papers = OrangeTable {
        columns: paper_columns(),
        rows: Vec::new(),
    };
    for (i, p) in snap.papers.iter().enumerate() {
        if !allowed(&pb, i) || p.flags & TOMBSTONE != 0 {
            excluded += 1;
            continue;
        }
        papers.rows.push(vec![
            hex16(&p.paper_id.to_be_bytes()),
            p.year.to_string(),
            if p.timestamp_sec > 0 {
                iso_utc(p.timestamp_sec)
            } else {
                String::new()
            },
            p.citation_count.to_string(),
            if p.open_access_flag == 1 { "yes" } else { "no" }.into(),
            p.venue_id.to_string(),
            p.warning_count.to_string(),
            num(p.topic_x as f64),
            num(p.topic_y as f64),
            num(p.topic_z as f64),
        ]);
    }

    let consented = snap.tracing_allowed();
    let mut sessions = OrangeTable {
        columns: session_columns(),
        rows: Vec::new(),
    };
    let mut acc: BTreeMap<[u64; 2], Acc> = BTreeMap::new();
    if consented {
        let hb = blocked(marks, ScholarTileType::HUMAN_LRS);
        for (i, h) in snap.human_decisions.iter().enumerate() {
            if !allowed(&hb, i) || (h.flags as u64) & TOMBSTONE != 0 {
                excluded += 1;
                continue;
            }
            let a = acc.entry(h.session_uuid).or_default();
            a.touch(h.timestamp_sec);
            a.humans += 1;
            a.actors.insert(h.actor_hash);
            a.score_sum += h.decision_score as f64;
        }
        let ab = blocked(marks, ScholarTileType::AI_TRACE_LOG);
        for (i, t) in snap.ai_traces.iter().enumerate() {
            if !allowed(&ab, i) || (t.flags as u64) & TOMBSTONE != 0 {
                excluded += 1;
                continue;
            }
            let a = acc.entry(t.session_uuid).or_default();
            a.touch(t.timestamp_sec);
            a.ais += 1;
            a.agents.insert(t.agent_id);
            a.conf_sum += t.confidence as f64;
        }
    } else {
        excluded += snap.human_decisions.len() + snap.ai_traces.len();
    }
    for (uuid, a) in acc {
        let mut b = [0u8; 16];
        b[..8].copy_from_slice(&uuid[0].to_be_bytes());
        b[8..].copy_from_slice(&uuid[1].to_be_bytes());
        let mean = |s: f64, n: usize| {
            if n > 0 {
                num4(s / n as f64)
            } else {
                String::new()
            }
        };
        sessions.rows.push(vec![
            pseudo(opts, "session", &b),
            iso_utc(a.first),
            iso_utc(a.last),
            (a.last - a.first).to_string(),
            a.humans.to_string(),
            a.ais.to_string(),
            a.actors.len().to_string(),
            a.agents.len().to_string(),
            mean(a.score_sum, a.humans),
            mean(a.conf_sum, a.ais),
        ]);
    }

    OrangeExport {
        papers,
        sessions,
        telemetry_consented: consented,
        excluded_records: excluded,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use s3a_core::{ConsentPurpose, S3ACoordinate, MARK_SET};
    use s3a_pack_scholar::{AcademicPaperRecord, AiTraceRecord, HumanLrsRecord, SrsManifestHeader};

    fn base() -> WasmAgentSnapshot {
        WasmAgentSnapshot {
            manifest: SrsManifestHeader::new([1, 2], [3, 4], [0, 0], [1, 2], 1600000000, 1, 0),
            workspace: None,
            papers: vec![
                AcademicPaperRecord::new(11, 1_600_000_000, 0, 0.5, 0.25, 0.0, 7, 2020, 3, 1, 0, 0),
                AcademicPaperRecord::new(22, 0, 0, 1.0, 1.0, 1.0, 0, 2019, 4, 0, 2, 0),
            ],
            edges: vec![],
            human_decisions: vec![
                HumanLrsRecord::new([1, 1], 1000, 99, 3, 0.5, S3ACoordinate::default(), 5),
                HumanLrsRecord::new([1, 1], 1060, 99, 3, 1.0, S3ACoordinate::default(), 6),
            ],
            ai_traces: vec![AiTraceRecord::new(
                [1, 1],
                1030,
                42,
                1,
                0.8,
                S3ACoordinate::default(),
                0,
            )],
            agents: vec![],
            concepts: vec![],
            axioms: vec![],
            claims: vec![],
            citation_fingerprints: vec![],
            policy_marks: vec![],
            pending_delta: vec![],
            delta_seq: 1,
            last_applied_delta_seq: 0,
            statement_log: vec![],
            consents: vec![],
        }
    }

    fn consent(s: &mut WasmAgentSnapshot) {
        s.set_consent(true, ConsentPurpose::Research, 7, [0; 16], 900);
    }

    #[test]
    fn header_rows_are_names_types_flags() {
        let mut s = base();
        consent(&mut s);
        let e = export_snapshot(&s, &ExportOptions::default());
        let p = OrangeTable::parse_tab(&e.papers.to_tab_string()).unwrap();
        assert_eq!(p.names[0], "paper_key");
        assert_eq!(p.types[..3], ["string", "continuous", "time"]);
        assert_eq!(p.flags[0], "meta");
        assert_eq!(p.flags[1], "");
        let q = OrangeTable::parse_tab(&e.sessions.to_tab_string()).unwrap();
        assert_eq!(q.names.len(), session_columns().len());
        assert!(q
            .types
            .iter()
            .all(|t| ["continuous", "discrete", "string", "time"].contains(&t.as_str())));
    }

    #[test]
    fn escapes_tabs_and_newlines() {
        let t = OrangeTable {
            columns: vec![
                col("a", OrangeType::String, OrangeRole::Meta),
                col("b", OrangeType::Continuous, OrangeRole::Feature),
            ],
            rows: vec![vec!["x\ty\nz\r".into(), "1".into()]],
        };
        let p = OrangeTable::parse_tab(&t.to_tab_string()).unwrap();
        assert_eq!(p.rows, vec![vec!["x y z".to_string(), "1".to_string()]]);
    }

    #[test]
    fn round_trip_values() {
        let mut s = base();
        consent(&mut s);
        let e = export_snapshot(&s, &ExportOptions::default());
        let p = OrangeTable::parse_tab(&e.papers.to_tab_string()).unwrap();
        assert_eq!(p.rows.len(), 2);
        assert_eq!(p.rows[0][1], "2020");
        assert_eq!(p.rows[0][2], "2020-09-13T12:26:40");
        assert_eq!(p.rows[0][4], "yes");
        let q = OrangeTable::parse_tab(&e.sessions.to_tab_string()).unwrap();
        assert_eq!(q.rows.len(), 1);
        let r = &q.rows[0];
        assert_eq!(
            (r[3].as_str(), r[4].as_str(), r[5].as_str()),
            ("60", "2", "1")
        );
        assert_eq!(r[8], "0.75");
        assert_eq!(r[9], "0.8", "f32 confidence must be rounded");
        // Pseudonymous: session key is not the raw uuid.
        assert_ne!(r[0], "00000000000000010000000000000001");
        assert_eq!(r[0].len(), 16);
    }

    #[test]
    fn no_consent_exports_no_sessions() {
        let e = export_snapshot(&base(), &ExportOptions::default());
        assert!(!e.telemetry_consented);
        assert!(e.sessions.rows.is_empty());
        assert_eq!(e.papers.rows.len(), 2);
    }

    #[test]
    fn withdrawn_and_quarantined_excluded() {
        let mut s = base();
        consent(&mut s);
        s.policy_marks.push(MarkRecord::new(
            ScholarTileType::ACADEMIC_PAPERS,
            0,
            1,
            0,
            MARK_QUARANTINE,
            MARK_SET,
            1,
            0,
        ));
        s.policy_marks.push(MarkRecord::new(
            ScholarTileType::HUMAN_LRS,
            0,
            0,
            0,
            MARK_CONSENT_WITHDRAWN,
            MARK_SET,
            1,
            0,
        ));
        let e = export_snapshot(&s, &ExportOptions::default());
        assert_eq!(e.papers.rows.len(), 1);
        assert_eq!(e.sessions.rows[0][4], "1");
        // Withdrawal via Tile 71 stamps whole tiles and disables telemetry.
        s.set_consent(false, ConsentPurpose::Research, 7, [0; 16], 2000);
        let e = export_snapshot(&s, &ExportOptions::default());
        assert!(e.sessions.rows.is_empty());
    }
}
