//! Deterministic cleaning into a separate, signed cleaned version.
use crate::archive;
use crate::raw::RawSnapshot;
use crate::rulebook::{Category, Rule, Rulebook};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

/// Tile type for the cleaned version (core generic RECORD tile; no new tile IDs).
pub const CLEAN_TILE: u32 = s3a_core::TileType::RECORD;

/// Who made rule-based changes.
pub const RULE_ENGINE: &str = "rule-engine";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CleanedRecord {
    /// Citation: the exact raw item this record came from.
    pub raw_item_sha256: String,
    pub raw_snapshot_hash: String,
    pub source_path: String,
    /// 1-based data row in the source file (header is row 0).
    pub row: usize,
    pub category: String,
    pub fields: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Change {
    pub raw_item_sha256: String,
    pub row: usize,
    pub field: String,
    pub before: String,
    pub after: String,
    pub rule_id: String,
    pub rule_version: u32,
    pub made_by: String,
    /// The approval basis: the human-maintained rulebook version and hash.
    pub approved_by: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum QueueTag {
    /// No rulebook category covers this file.
    #[serde(rename = "UNCLASSIFIED")]
    Unclassified,
    /// Covered, but the rulebook can't decide (ambiguous, invalid, unknown column).
    #[serde(rename = "NEEDS_RULE")]
    NeedsRule,
}

/// A review-queue entry for a human to resolve by updating the rulebook.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QueueEntry {
    pub tag: QueueTag,
    /// Existing core mark: `MARK_NEEDS_REVIEW` for unclassified files,
    /// `MARK_QUARANTINE` for records held out of the cleaned set.
    pub mark: u32,
    pub raw_item_sha256: String,
    pub source_path: String,
    pub row: Option<usize>,
    pub field: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CleanBody {
    pub format: String,
    pub raw_snapshot_hash: String,
    pub rulebook_version: String,
    pub rulebook_sha256: String,
    pub records: Vec<CleanedRecord>,
    pub changes: Vec<Change>,
    pub review_queue: Vec<QueueEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CleanedVersion {
    #[serde(flatten)]
    pub body: CleanBody,
    /// SHA-256 of the canonical body. Same raw + same rulebook => same hash.
    pub cleaned_hash: String,
    pub signer_pubkey: String,
    pub signature: String,
}

/// Build the unsigned cleaned body (pure and deterministic).
pub fn clean_body(
    raw: &RawSnapshot,
    blobs: &BTreeMap<String, Vec<u8>>,
    rb: &Rulebook,
    rb_bytes: &[u8],
) -> CleanBody {
    let rb_sha = crate::sha256_hex(rb_bytes);
    let approved_by = format!("rulebook:{}@sha256:{}", rb.rulebook_version, rb_sha);
    let mut body = CleanBody {
        format: "s3a-clean-v1".into(),
        raw_snapshot_hash: raw.snapshot_hash.clone(),
        rulebook_version: rb.rulebook_version.clone(),
        rulebook_sha256: rb_sha,
        records: vec![],
        changes: vec![],
        review_queue: vec![],
    };
    for it in &raw.items {
        let q = |tag, mark, row, field: Option<&str>, reason: String| QueueEntry {
            tag,
            mark,
            raw_item_sha256: it.item_sha256.clone(),
            source_path: it.source_path.clone(),
            row,
            field: field.map(str::to_string),
            reason,
        };
        let name = it.file_name.to_ascii_lowercase();
        let ext = name.rsplit_once('.').map(|x| x.1).unwrap_or("");
        let cats: Vec<&Category> = rb
            .categories
            .iter()
            .filter(|c| {
                c.matcher
                    .extensions
                    .iter()
                    .any(|e| e.eq_ignore_ascii_case(ext))
                    && c.matcher
                        .name_prefix
                        .as_deref()
                        .is_none_or(|p| name.starts_with(&p.to_ascii_lowercase()))
            })
            .collect();
        let cat = match cats.len() {
            0 => {
                body.review_queue.push(q(
                    QueueTag::Unclassified,
                    s3a_core::MARK_NEEDS_REVIEW,
                    None,
                    None,
                    format!("no rulebook category covers '{}'", it.file_name),
                ));
                continue;
            }
            1 => cats[0],
            _ => {
                let names: Vec<&str> = cats.iter().map(|c| c.name.as_str()).collect();
                body.review_queue.push(q(
                    QueueTag::NeedsRule,
                    s3a_core::MARK_QUARANTINE,
                    None,
                    None,
                    format!("ambiguous: matches categories {:?}", names),
                ));
                continue;
            }
        };
        if ext != "csv" {
            body.review_queue.push(q(
                QueueTag::NeedsRule,
                s3a_core::MARK_QUARANTINE,
                None,
                None,
                format!(
                    "category '{}' matched but only CSV content is parsed in v1",
                    cat.name
                ),
            ));
            continue;
        }
        let text = match std::str::from_utf8(&blobs[&it.item_sha256]) {
            Ok(t) => t,
            Err(_) => {
                body.review_queue.push(q(
                    QueueTag::NeedsRule,
                    s3a_core::MARK_QUARANTINE,
                    None,
                    None,
                    "not UTF-8".into(),
                ));
                continue;
            }
        };
        let rows = parse_csv(text);
        let Some(header) = rows.first() else {
            body.review_queue.push(q(
                QueueTag::NeedsRule,
                s3a_core::MARK_QUARANTINE,
                None,
                None,
                "empty file".into(),
            ));
            continue;
        };
        let header: Vec<String> = header.iter().map(|h| h.trim().to_string()).collect();
        let unknown: Vec<&String> = header
            .iter()
            .filter(|h| !cat.fields.iter().any(|f| &f.name == *h))
            .collect();
        if !unknown.is_empty() {
            body.review_queue.push(q(
                QueueTag::NeedsRule,
                s3a_core::MARK_QUARANTINE,
                None,
                None,
                format!(
                    "columns not in rulebook category '{}': {:?}",
                    cat.name, unknown
                ),
            ));
            continue;
        }
        'row: for (ri, cells) in rows.iter().enumerate().skip(1) {
            if cells.len() != header.len() {
                body.review_queue.push(q(
                    QueueTag::NeedsRule,
                    s3a_core::MARK_QUARANTINE,
                    Some(ri),
                    None,
                    format!("expected {} cells, got {}", header.len(), cells.len()),
                ));
                continue;
            }
            let mut fields: BTreeMap<String, String> =
                header.iter().cloned().zip(cells.iter().cloned()).collect();
            let mut changes = vec![];
            for r in &cat.rules {
                let targets: Vec<String> = match &r.field {
                    Some(f) => vec![f.clone()],
                    None => header.clone(),
                };
                for f in targets {
                    let Some(v) = fields.get(&f).cloned() else {
                        continue;
                    };
                    match apply(r, &v) {
                        Ok(nv) if nv != v => {
                            changes.push(Change {
                                raw_item_sha256: it.item_sha256.clone(),
                                row: ri,
                                field: f.clone(),
                                before: v,
                                after: nv.clone(),
                                rule_id: r.id.clone(),
                                rule_version: r.version,
                                made_by: RULE_ENGINE.into(),
                                approved_by: approved_by.clone(),
                            });
                            fields.insert(f, nv);
                        }
                        Ok(_) => {}
                        Err(why) => {
                            body.review_queue.push(q(
                                QueueTag::NeedsRule,
                                s3a_core::MARK_QUARANTINE,
                                Some(ri),
                                Some(&f),
                                format!("rule {} v{}: {}", r.id, r.version, why),
                            ));
                            continue 'row;
                        }
                    }
                }
            }
            for fd in &cat.fields {
                let v = fields.get(&fd.name).map(String::as_str).unwrap_or("");
                let why = if v.is_empty() {
                    fd.required.then(|| "required field is empty".to_string())
                } else if !type_ok(&fd.ty, v) {
                    Some(format!("'{}' is not a valid {}", v, fd.ty))
                } else if !fd.allowed.is_empty() && !fd.allowed.iter().any(|a| a == v) {
                    Some(format!("'{}' not in allowed values {:?}", v, fd.allowed))
                } else {
                    None
                };
                if let Some(why) = why {
                    body.review_queue.push(q(
                        QueueTag::NeedsRule,
                        s3a_core::MARK_QUARANTINE,
                        Some(ri),
                        Some(&fd.name),
                        why,
                    ));
                    continue 'row;
                }
            }
            body.changes.extend(changes);
            body.records.push(CleanedRecord {
                raw_item_sha256: it.item_sha256.clone(),
                raw_snapshot_hash: raw.snapshot_hash.clone(),
                source_path: it.source_path.clone(),
                row: ri,
                category: cat.name.clone(),
                fields,
            });
        }
    }
    body
}

pub fn body_hash(b: &CleanBody) -> String {
    crate::sha256_hex(&crate::canonical(b))
}

/// Clean and sign.
pub fn clean(
    raw: &RawSnapshot,
    blobs: &BTreeMap<String, Vec<u8>>,
    rb: &Rulebook,
    rb_bytes: &[u8],
    key: &SigningKey,
) -> CleanedVersion {
    let body = clean_body(raw, blobs, rb, rb_bytes);
    let h = body_hash(&body);
    CleanedVersion {
        signature: hex::encode(key.sign(h.as_bytes()).to_bytes()),
        signer_pubkey: hex::encode(key.verifying_key().to_bytes()),
        cleaned_hash: h,
        body,
    }
}

/// Verify hash, signature, and that every record/change/queue entry cites an item of `raw`.
pub fn verify(cv: &CleanedVersion, raw: &RawSnapshot) -> Result<(), String> {
    if body_hash(&cv.body) != cv.cleaned_hash {
        return Err("cleaned hash mismatch (provenance or records altered)".into());
    }
    let pk: [u8; 32] = hex::decode(&cv.signer_pubkey)
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or("bad signer key")?;
    let sig: [u8; 64] = hex::decode(&cv.signature)
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or("bad signature")?;
    VerifyingKey::from_bytes(&pk)
        .map_err(|_| "bad signer key")?
        .verify(cv.cleaned_hash.as_bytes(), &Signature::from_bytes(&sig))
        .map_err(|_| "bad signature")?;
    if cv.body.raw_snapshot_hash != raw.snapshot_hash {
        return Err("cleaned version cites a different raw snapshot".into());
    }
    let cites = cv
        .body
        .records
        .iter()
        .map(|r| (&r.raw_item_sha256, Some(&r.raw_snapshot_hash)))
        .chain(cv.body.changes.iter().map(|c| (&c.raw_item_sha256, None)))
        .chain(
            cv.body
                .review_queue
                .iter()
                .map(|q| (&q.raw_item_sha256, None)),
        );
    for (sha, snap) in cites {
        if raw.item(sha).is_none() || snap.is_some_and(|s| s != &raw.snapshot_hash) {
            return Err(format!("entry cites unknown raw item {sha}"));
        }
    }
    Ok(())
}

/// Write the cleaned version to a new sealed archive.
pub fn write(path: &Path, cv: &CleanedVersion) -> io::Result<()> {
    let key: [u8; 32] = hex::decode(&cv.cleaned_hash).unwrap().try_into().unwrap();
    archive::write_sealed(
        path,
        &archive::chunk_tiles(CLEAN_TILE, &key, &crate::canonical(cv)),
        "S3A Clean Pack cleaned version",
    )
}

/// Read a cleaned version from its archive.
pub fn read(path: &Path) -> io::Result<CleanedVersion> {
    let m = archive::unchunk(&archive::read_tiles(path, CLEAN_TILE)?)?;
    let (_, v) = m
        .into_iter()
        .next()
        .ok_or_else(|| io::Error::other("no cleaned version"))?;
    serde_json::from_slice(&v).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn apply(r: &Rule, v: &str) -> Result<String, String> {
    Ok(match r.op.as_str() {
        "trim" => v.trim().to_string(),
        "uppercase" => v.to_uppercase(),
        "lowercase" => v.to_lowercase(),
        "map" => r.map.get(v).cloned().unwrap_or_else(|| v.to_string()),
        "date_iso" => {
            if v.is_empty() {
                return Ok(String::new());
            }
            let mut outs: Vec<String> = r.formats.iter().filter_map(|f| parse_date(f, v)).collect();
            outs.sort();
            outs.dedup();
            match outs.len() {
                0 => return Err(format!("date '{v}' matches none of {:?}", r.formats)),
                1 => outs.remove(0),
                _ => return Err(format!("ambiguous date '{v}': could be {:?}", outs)),
            }
        }
        o => return Err(format!("unknown op {o}")),
    })
}

fn parse_date(fmt: &str, v: &str) -> Option<String> {
    let (sep, order) = match fmt {
        "YYYY-MM-DD" => ('-', [0, 1, 2]),
        "MM/DD/YYYY" => ('/', [2, 0, 1]),
        "DD/MM/YYYY" => ('/', [2, 1, 0]),
        "DD.MM.YYYY" => ('.', [2, 1, 0]),
        _ => return None,
    };
    let p: Vec<&str> = v.trim().split(sep).collect();
    if p.len() != 3
        || p.iter()
            .any(|s| s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let (y, m, d) = (p[order[0]], p[order[1]], p[order[2]]);
    if y.len() != 4 {
        return None;
    }
    let (yi, mi, di): (u32, u32, u32) = (y.parse().ok()?, m.parse().ok()?, d.parse().ok()?);
    let dim = match mi {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (yi % 4 == 0 && yi % 100 != 0) || yi % 400 == 0 => 29,
        2 => 28,
        _ => return None,
    };
    (1..=dim)
        .contains(&di)
        .then(|| format!("{yi:04}-{mi:02}-{di:02}"))
}

fn type_ok(ty: &str, v: &str) -> bool {
    match ty {
        "integer" => v.parse::<i64>().is_ok(),
        "decimal" => v.parse::<f64>().map(|f| f.is_finite()).unwrap_or(false),
        "date" => parse_date("YYYY-MM-DD", v).as_deref() == Some(v),
        "bool" => v == "true" || v == "false",
        _ => true,
    }
}

/// Small RFC 4180 CSV reader (quotes, escaped quotes, CRLF). Blank lines are skipped.
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = vec![];
    let (mut row, mut cell, mut inq) = (vec![], String::new(), false);
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        match (inq, c) {
            (true, '"') if it.peek() == Some(&'"') => {
                cell.push('"');
                it.next();
            }
            (true, '"') => inq = false,
            (true, c) => cell.push(c),
            (false, '"') => inq = true,
            (false, ',') => row.push(std::mem::take(&mut cell)),
            (false, '\r') => {}
            (false, '\n') => {
                row.push(std::mem::take(&mut cell));
                if !(row.len() == 1 && row[0].is_empty()) {
                    rows.push(std::mem::take(&mut row));
                } else {
                    row.clear();
                }
            }
            (false, c) => cell.push(c),
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }
    rows
}
