//! `s3a-cli tour --html <archive> <out.html>`: one self-contained, offline HTML map of an archive.
//!
//! Read-only. The data is gathered with the same code as `tour --json` and embedded as JSON
//! in a `<script type="application/json">` block, escaped so no archive content can close the
//! block or run as markup. The page renders everything with `textContent`, never `innerHTML`.
use crate::tour::{tour_file, TourOptions};
use serde_json::{json, Value};
use std::io::{Read, Seek, SeekFrom};

/// Collects everything the page shows.
pub fn build_data(path: &str) -> Result<Value, Box<dyn std::error::Error>> {
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    let report = tour_file(&mut f, len, path, &TourOptions::default())?;
    let floors = report.floors.clone().unwrap_or_default();
    let mut details = Vec::new();
    for fl in &floors {
        let opts = TourOptions {
            floor: Some(fl.index),
            ..Default::default()
        };
        let d = tour_file(&mut f, len, path, &opts)
            .ok()
            .and_then(|r| r.floor_detail)
            .map(|d| serde_json::to_value(d).unwrap_or(Value::Null))
            .unwrap_or(Value::Null);
        details.push(d);
    }
    let mut head = vec![0u8; 128];
    f.seek(SeekFrom::Start(0))?;
    let n = f.read(&mut head)?;
    head.truncate(n);
    let slots: Vec<Value> = (0..2)
        .map(|s| {
            let Some(b) = head.get(s * 64..s * 64 + 64) else {
                return json!({"slot": s, "valid": false});
            };
            let u32le = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
            let gen = u64::from_le_bytes(b[8..16].try_into().unwrap());
            let crc = u32le(40);
            let c = crc32c::crc32c_append(crc32c::crc32c(&b[0..40]), &b[44..64]);
            let valid = &b[0..4] == b"S3A1" && crc != 0 && c == crc;
            json!({"slot": s, "valid": valid, "generation": gen, "tile_count": u32le(16)})
        })
        .collect();
    let (active_slot, record_roots) = match s3a_engine::MmapReader::open(path) {
        Ok(r) => {
            let rr = s3a_engine::record_roots::check(&r);
            let st = if !rr.ok() {
                "mismatch"
            } else if rr.present {
                "ok"
            } else {
                "absent"
            };
            (
                json!(r.active_slot()),
                json!({"status": st, "checked": rr.checked}),
            )
        }
        Err(_) => (Value::Null, json!({"status": "unknown", "checked": 0})),
    };
    Ok(json!({
        "file": path,
        "file_bytes": len,
        "lobby": report.lobby,
        "floors": floors,
        "details": details,
        "header": {"slots": slots, "active_slot": active_slot},
        "record_roots": record_roots,
    }))
}

/// JSON safe to place inside `<script type="application/json">`: no `<`, `>`, `&` or
/// line/paragraph separators survive literally, so archive text cannot end the block.
pub fn escape_json_for_html(json: &str) -> String {
    let mut out = String::with_capacity(json.len());
    for ch in json.chars() {
        match ch {
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c => out.push(c),
        }
    }
    out
}

/// HTML text escaping for the one string placed outside the JSON block (the title).
pub fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub fn render_html(data: &Value) -> String {
    let title = escape_html(data["file"].as_str().unwrap_or("archive"));
    let json = escape_json_for_html(&data.to_string());
    // DATA first, so a title containing the DATA marker cannot pull data into the title.
    TEMPLATE
        .replace("{{DATA}}", &json)
        .replacen("{{TITLE}}", &title, 1)
}

/// Writes the page; returns the number of tiles shown.
pub fn write_html(archive: &str, out: &str) -> Result<usize, Box<dyn std::error::Error>> {
    let data = build_data(archive)?;
    std::fs::write(out, render_html(&data))?;
    Ok(data["floors"].as_array().map(|a| a.len()).unwrap_or(0))
}

const TEMPLATE: &str = include_str!("tour_html_template.html");
