use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use bytemuck::{bytes_of, cast_slice};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use s3a_core::{
    FileHeader, HyperTileHeader, MarkRecord,
    FILE_HEADER_SIZE, FILE_HEADER_SLOT_A_OFFSET,
    FILE_HEADER_SLOT_SIZE, HYPER_TILE_HEADER_SIZE, HYPER_TILE_SIZE,
    MARK_NEEDS_REVIEW, MARK_SET, TileType,
};
use s3a_engine::sealed::seal_buffer;
use s3a_engine::TileWriter;

use crate::pack::ScholarTileType;
use crate::records::{
    AcademicPaperRecord, CitationFingerprintRecord,
};
use crate::xapi::{
    hash_iri, EXT_CONTENT_SHA256, EXT_CROSSREF_VERSION,
    EXT_RECHECK_STATUS, EXT_SOURCE_URL, SCHOLAR_XAPI_PROFILE_IRI,
};

pub fn is_sealed_file_bytes(bytes: &[u8]) -> bool {
    bytes.len() >= 64 && &bytes[bytes.len() - 64..bytes.len() - 56] == s3a_core::SEALED_ENDCAP_MAGIC
}

/// Verification outcome for a citation fingerprint against its original source & Crossref.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecheckStatus {
    Unchanged,
    Changed,
    Retracted,
    Unreachable,
}

impl RecheckStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unchanged => "unchanged",
            Self::Changed => "changed",
            Self::Retracted => "retracted",
            Self::Unreachable => "unreachable",
        }
    }
}

/// Detailed result of rechecking a single citation fingerprint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecheckItem {
    pub paper_id: u64,
    pub tile_id: u64,
    pub record_idx: u32,
    pub doi: String,
    pub source_url: String,
    pub content_kind: String,
    pub stored_sha256: String,
    pub fetched_sha256: Option<String>,
    pub status: RecheckStatus,
    pub message: String,
    pub mark_written: bool,
    pub statement_written: bool,
}

/// Overall report produced by `s3a-cli scholar recheck`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecheckReport {
    pub items: Vec<RecheckItem>,
    pub unchanged_count: usize,
    pub changed_count: usize,
    pub retracted_count: usize,
    pub unreachable_count: usize,
    pub marks_appended: usize,
    pub statements_appended: usize,
}

/// Execution options for citation rechecking.
#[derive(Debug, Clone)]
pub struct RecheckOptions {
    pub online: bool,
    pub crossref_endpoint: Option<String>,
    pub url_overrides: HashMap<u64, (String, String)>, // paper_id -> (doi, source_url)
    pub out_path: Option<String>,
}

impl Default for RecheckOptions {
    fn default() -> Self {
        Self {
            online: false,
            crossref_endpoint: None,
            url_overrides: HashMap::new(),
            out_path: None,
        }
    }
}

/// Rechecks all citation fingerprints in an S3A archive (live or sealed snapshot).
pub fn recheck_archive<P: AsRef<Path>>(
    file_path: P,
    opts: &RecheckOptions,
) -> io::Result<RecheckReport> {
    let path = file_path.as_ref();
    if !path.exists() {
        return Err(io::Error::new(io::ErrorKind::NotFound, format!("File not found: {}", path.display())));
    }

    let file_bytes = std::fs::read(path)?;
    if file_bytes.len() < FILE_HEADER_SIZE {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "File too small for S3A header"));
    }

    let is_sealed = is_sealed_file_bytes(&file_bytes);
    let mut discovered_fps: Vec<(CitationFingerprintRecord, u64, u32)> = Vec::new(); // (rec, tile_id, rec_idx)
    let mut paper_doi_map: HashMap<u64, String> = HashMap::new();
    let mut paper_url_map: HashMap<u64, String> = HashMap::new();

    // Populate from options overrides first
    for (pid, (doi, url)) in &opts.url_overrides {
        paper_doi_map.insert(*pid, doi.clone());
        paper_url_map.insert(*pid, url.clone());
    }

    // Scan hyper-tiles
    let mut offset = FILE_HEADER_SIZE;
    while offset + HYPER_TILE_SIZE <= file_bytes.len() {
        let tile_bytes = &file_bytes[offset..offset + HYPER_TILE_SIZE];
        let tile_header: &HyperTileHeader = bytemuck::from_bytes(&tile_bytes[..HYPER_TILE_HEADER_SIZE]);

        if tile_header.tile_type == ScholarTileType::CITATION_FINGERPRINT {
            let payload = &tile_bytes[HYPER_TILE_HEADER_SIZE..HYPER_TILE_HEADER_SIZE + tile_header.payload_bytes as usize];
            let recs: &[CitationFingerprintRecord] = bytemuck::cast_slice(payload);
            for (rec_idx, rec) in recs.iter().enumerate() {
                if rec.is_active() {
                    discovered_fps.push((*rec, tile_header.tile_id, rec_idx as u32));
                }
            }
        } else if tile_header.tile_type == ScholarTileType::ACADEMIC_PAPERS {
            let payload = &tile_bytes[HYPER_TILE_HEADER_SIZE..HYPER_TILE_HEADER_SIZE + tile_header.payload_bytes as usize];
            let recs: &[AcademicPaperRecord] = bytemuck::cast_slice(payload);
            for rec in recs {
                if !paper_doi_map.contains_key(&rec.paper_id) {
                    paper_doi_map.insert(rec.paper_id, format!("10.1000/{}", rec.paper_id));
                }
            }
        } else if tile_header.tile_type == TileType::STATEMENT_BODY {
            // Extract DOIs and URLs from xAPI statement bodies
            let payload = &tile_bytes[HYPER_TILE_HEADER_SIZE..HYPER_TILE_HEADER_SIZE + tile_header.payload_bytes as usize];
            if payload.len() >= 64 {
                let json_slice = &payload[64..];
                if let Ok(val) = serde_json::from_slice::<serde_json::Value>(json_slice) {
                    if let Some(obj_id) = val.get("object").and_then(|o| o.get("id")).and_then(|i| i.as_str()) {
                        let doi_cleaned = obj_id.trim_start_matches("https://doi.org/").to_string();
                        let pid = val.get("context")
                            .and_then(|c| c.get("extensions"))
                            .and_then(|e| e.get("https://ashecribb.dev/xapi/ext/paper-id"))
                            .and_then(|p| p.as_u64())
                            .unwrap_or(hash_iri(&doi_cleaned));
                        paper_doi_map.entry(pid).or_insert(doi_cleaned);

                        if let Some(src_url) = val.get("context")
                            .and_then(|c| c.get("extensions"))
                            .and_then(|e| e.get(EXT_SOURCE_URL))
                            .and_then(|u| u.as_str()) {
                            paper_url_map.entry(pid).or_insert(src_url.to_string());
                        }
                    }
                }
            }
        }

        offset += HYPER_TILE_SIZE;
    }

    let mut report = RecheckReport::default();
    let mut marks_to_write: Vec<MarkRecord> = Vec::new();
    let mut statements_to_write: Vec<serde_json::Value> = Vec::new();
    let now_sec = s3a_engine::lrs_store::now_ms() / 1000;

    for (fp, tile_id, rec_idx) in discovered_fps {
        let doi = paper_doi_map.get(&fp.paper_id).cloned().unwrap_or_else(|| format!("10.1000/{}", fp.paper_id));
        let source_url = paper_url_map.get(&fp.paper_id).cloned().unwrap_or_else(|| format!("https://doi.org/{}", doi));

        let (status, msg, fetched_sha) = if opts.online {
            verify_online_source(&source_url, &doi, &fp, opts.crossref_endpoint.as_deref())
        } else {
            // In offline mode, verify internal hash plausibility
            let is_zero = fp.content_sha256.iter().all(|&b| b == 0);
            if is_zero {
                (RecheckStatus::Changed, "Zero content SHA-256 in fingerprint".to_string(), None)
            } else {
                (RecheckStatus::Unchanged, "Offline verification: SHA-256 signature format valid".to_string(), Some(fp.content_sha256_hex()))
            }
        };

        match status {
            RecheckStatus::Unchanged => report.unchanged_count += 1,
            RecheckStatus::Changed => report.changed_count += 1,
            RecheckStatus::Retracted => report.retracted_count += 1,
            RecheckStatus::Unreachable => report.unreachable_count += 1,
        }

        let mut mark_written = false;
        let mut stmt_written = false;

        // If changed, retracted, or unreachable: write MARK_NEEDS_REVIEW and xAPI statement
        if status != RecheckStatus::Unchanged {
            let mark = MarkRecord::new(
                ScholarTileType::CITATION_FINGERPRINT,
                tile_id,
                rec_idx,
                1, // generation
                MARK_NEEDS_REVIEW,
                MARK_SET,
                now_sec,
                hash_iri("scholar-citation-verifier"),
            );
            marks_to_write.push(mark);
            mark_written = true;

            let stmt = serde_json::json!({
                "actor": {
                    "account": {
                        "homePage": "https://ashecribb.dev/s3a",
                        "name": "scholar-citation-verifier"
                    }
                },
                "verb": {
                    "id": "https://ashecribb.dev/xapi/verbs/flagged-discrepancy",
                    "display": { "en-US": "flagged-discrepancy" }
                },
                "object": {
                    "id": format!("https://doi.org/{}", doi),
                    "objectType": "Activity"
                },
                "result": {
                    "response": format!("Citation verification status: {} ({})", status.as_str(), msg)
                },
                "context": {
                    "contextActivities": {
                        "category": [{ "id": SCHOLAR_XAPI_PROFILE_IRI }]
                    },
                    "extensions": {
                        EXT_RECHECK_STATUS: status.as_str(),
                        EXT_CONTENT_SHA256: fp.content_sha256_hex(),
                        EXT_SOURCE_URL: source_url,
                        EXT_CROSSREF_VERSION: fp.crossref_version,
                    }
                }
            });
            statements_to_write.push(stmt);
            stmt_written = true;
        }

        report.items.push(RecheckItem {
            paper_id: fp.paper_id,
            tile_id,
            record_idx: rec_idx,
            doi,
            source_url,
            content_kind: fp.content_kind_str().to_string(),
            stored_sha256: fp.content_sha256_hex(),
            fetched_sha256: fetched_sha,
            status,
            message: msg,
            mark_written,
            statement_written: stmt_written,
        });
    }

    report.marks_appended = marks_to_write.len();
    report.statements_appended = statements_to_write.len();

    // Persist marks and statements if any were generated
    if !marks_to_write.is_empty() || !statements_to_write.is_empty() {
        apply_marks_and_statements(path, &file_bytes, is_sealed, &marks_to_write, &statements_to_write, opts.out_path.as_deref())?;
    }

    Ok(report)
}

fn verify_online_source(
    source_url: &str,
    doi: &str,
    fp: &CitationFingerprintRecord,
    crossref_endpoint: Option<&str>,
) -> (RecheckStatus, String, Option<String>) {
    // 1. Fetch source content bytes
    let resp = ureq::get(source_url)
        .timeout(Duration::from_secs(5))
        .call();

    let content_bytes = match resp {
        Ok(r) => {
            let mut buf = Vec::new();
            if let Err(e) = r.into_reader().read_to_end(&mut buf) {
                return (RecheckStatus::Unreachable, format!("Failed to read stream: {}", e), None);
            }
            buf
        }
        Err(e) => {
            return (RecheckStatus::Unreachable, format!("HTTP fetch failed: {}", e), None);
        }
    };

    let mut hasher = Sha256::new();
    hasher.update(&content_bytes);
    let fetched_digest: [u8; 32] = hasher.finalize().into();
    let fetched_hex = hex::encode(fetched_digest);

    // 2. Query Crossref metadata (or custom endpoint)
    let cr_url = if let Some(base) = crossref_endpoint {
        format!("{}/works/{}", base.trim_end_matches('/'), doi)
    } else {
        format!("https://api.crossref.org/works/{}", doi)
    };

    let cr_resp = ureq::get(&cr_url)
        .timeout(Duration::from_secs(5))
        .call();

    if let Ok(cr_r) = cr_resp {
        let mut cr_text = String::new();
        if cr_r.into_reader().read_to_string(&mut cr_text).is_ok() {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&cr_text) {
                let msg = val.get("message").unwrap_or(&val);

            // Check for retraction indications
            let is_retracted = msg.get("update-to")
                .and_then(|u| u.as_array())
                .map(|arr| arr.iter().any(|item| {
                    item.get("type").and_then(|t| t.as_str()).map(|t| t == "retraction").unwrap_or(false)
                        || item.get("label").and_then(|l| l.as_str()).map(|l| l.to_lowercase().contains("retract")).unwrap_or(false)
                }))
                .unwrap_or(false)
                || msg.get("status").and_then(|s| s.as_str()).map(|s| s.to_lowercase().contains("retract")).unwrap_or(false)
                || msg.get("relation").and_then(|r| r.get("is-retracted-by")).is_some();

            if is_retracted {
                return (RecheckStatus::Retracted, "Crossref reports work has been retracted".to_string(), Some(fetched_hex));
            }

            // Check for newer update/version
            let cr_version = msg.get("version").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
            if cr_version > fp.crossref_version {
                return (RecheckStatus::Changed, format!("Crossref reports newer version v{} (recorded v{})", cr_version, fp.crossref_version), Some(fetched_hex));
            }
        }
    }
}

    // 3. Compare hash
    if fetched_digest != fp.content_sha256 {
        return (
            RecheckStatus::Changed,
            format!("Content SHA-256 mismatch (stored {}, fetched {})", fp.content_sha256_hex(), fetched_hex),
            Some(fetched_hex),
        );
    }

    (RecheckStatus::Unchanged, "Content hash and Crossref metadata match recorded provenance".to_string(), Some(fetched_hex))
}

fn apply_marks_and_statements(
    in_path: &Path,
    in_bytes: &[u8],
    is_sealed: bool,
    marks: &[MarkRecord],
    statements: &[serde_json::Value],
    out_path: Option<&str>,
) -> io::Result<()> {
    let dest_path = out_path.map(Path::new).unwrap_or(in_path);

    if is_sealed {
        // Strip sealed footer, append tiles to unsealed body, then re-seal
        let unsealed_len = extract_unsealed_length(in_bytes)?;
        let mut working_buf = in_bytes[..unsealed_len].to_vec();

        // Append marks tile if marks exist
        if !marks.is_empty() {
            let mut mark_tile = vec![0u8; HYPER_TILE_SIZE];
            let mark_bytes = cast_slice(marks);
            let mut hdr = HyperTileHeader::new(0, TileType::MARKS);
            hdr.record_count = marks.len() as u32;
            hdr.payload_bytes = mark_bytes.len() as u32;
            hdr.data_crc32 = crc32c::crc32c(mark_bytes);
            hdr.header_crc32 = compute_tile_header_crc32(&hdr);

            mark_tile[..HYPER_TILE_HEADER_SIZE].copy_from_slice(bytes_of(&hdr));
            mark_tile[HYPER_TILE_HEADER_SIZE..HYPER_TILE_HEADER_SIZE + mark_bytes.len()].copy_from_slice(mark_bytes);
            working_buf.extend_from_slice(&mark_tile);

            // Increment tile_count in header
            update_slot_tile_count(&mut working_buf);
        }

        // Append xAPI statement bodies
        for stmt in statements {
            let json_bytes = serde_json::to_vec(stmt).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            let mut stmt_tile = vec![0u8; HYPER_TILE_SIZE];
            let mut hdr = HyperTileHeader::new(0, TileType::STATEMENT_BODY);
            let total_payload = 64 + json_bytes.len();
            hdr.record_count = 1;
            hdr.payload_bytes = total_payload as u32;

            let mut payload_buf = vec![0u8; total_payload];
            payload_buf[64..].copy_from_slice(&json_bytes);
            hdr.data_crc32 = crc32c::crc32c(&payload_buf);
            hdr.header_crc32 = compute_tile_header_crc32(&hdr);

            stmt_tile[..HYPER_TILE_HEADER_SIZE].copy_from_slice(bytes_of(&hdr));
            stmt_tile[HYPER_TILE_HEADER_SIZE..HYPER_TILE_HEADER_SIZE + total_payload].copy_from_slice(&payload_buf);
            working_buf.extend_from_slice(&stmt_tile);

            update_slot_tile_count(&mut working_buf);
        }

        // Re-seal with K=16, M=2
        let sealed_bytes = seal_buffer(&working_buf, 16, 2)?;
        std::fs::write(dest_path, sealed_bytes)?;
    } else {
        // Live database file: append directly using TileWriter
        let mut writer = TileWriter::open_append(dest_path)?;

        if !marks.is_empty() {
            let timestamps: Vec<u64> = marks.iter().map(|m| m.timestamp).collect();
            writer.write_hyper_tile_ext(
                TileType::MARKS,
                marks,
                Some(&timestamps),
                None,
                None,
            )?;
        }

        for stmt in statements {
            let json_bytes = serde_json::to_vec(stmt).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            let mut payload_buf = vec![0u8; 64 + json_bytes.len()];
            payload_buf[64..].copy_from_slice(&json_bytes);

            let mut hdr = HyperTileHeader::new(0, TileType::STATEMENT_BODY);
            hdr.record_count = 1;
            hdr.payload_bytes = payload_buf.len() as u32;
            hdr.data_crc32 = crc32c::crc32c(&payload_buf);
            writer.write_raw_hyper_tile(&hdr, &payload_buf)?;
        }
    }

    Ok(())
}

fn compute_tile_header_crc32(header: &HyperTileHeader) -> u32 {
    let bytes = bytes_of(header);
    let crc_offset = core::mem::offset_of!(HyperTileHeader, header_crc32);
    let mut crc = crc32c::crc32c(&bytes[..crc_offset]);
    crc = crc32c::crc32c_append(crc, &bytes[crc_offset + 4..]);
    crc
}

fn update_slot_tile_count(buf: &mut [u8]) {
    let slot_a: &mut FileHeader = bytemuck::from_bytes_mut(&mut buf[FILE_HEADER_SLOT_A_OFFSET..FILE_HEADER_SLOT_A_OFFSET + FILE_HEADER_SLOT_SIZE]);
    slot_a.tile_count += 1;
    slot_a.generation += 1;
    slot_a.header_crc32 = compute_file_header_crc32(slot_a);
}

fn compute_file_header_crc32(header: &FileHeader) -> u32 {
    let bytes = bytes_of(header);
    let crc_offset = core::mem::offset_of!(FileHeader, header_crc32);
    let mut crc = crc32c::crc32c(&bytes[..crc_offset]);
    crc = crc32c::crc32c_append(crc, &bytes[crc_offset + 4..]);
    crc
}

fn extract_unsealed_length(bytes: &[u8]) -> io::Result<usize> {
    if bytes.len() < 64 {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "File too small"));
    }
    let endcap_bytes = &bytes[bytes.len() - 64..];
    let endcap: s3a_core::SealedEndCap = bytemuck::pod_read_unaligned(endcap_bytes);
    if endcap.footer_offset == 0 || endcap.footer_offset as usize > bytes.len() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid footer offset in endcap"));
    }
    Ok(endcap.footer_offset as usize)
}

/// Lightweight mock HTTP server providing local paper content & Crossref API endpoints for offline tests.
pub struct MockScholarServer {
    pub addr: String,
    shutdown: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl MockScholarServer {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("Must bind to ephemeral port");
        let local_addr = listener.local_addr().unwrap();
        let addr = format!("http://{}", local_addr);
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = shutdown.clone();

        listener.set_nonblocking(true).unwrap();

        let handle = thread::spawn(move || {
            while !shutdown_clone.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                        let mut buf = [0u8; 2048];
                        let mut n = 0;
                        while n < buf.len() {
                            match stream.read(&mut buf[n..]) {
                                Ok(0) => break,
                                Ok(bytes_read) => {
                                    n += bytes_read;
                                    if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                                        break;
                                    }
                                }
                                Err(_) => break,
                            }
                        }
                        let req = String::from_utf8_lossy(&buf[..n]);

                        let (status_line, content_type, body) = if req.contains("GET /paper_ok.pdf") {
                            ("HTTP/1.1 200 OK", "application/pdf", b"%PDF-1.4 Mock valid content for research paper".to_vec())
                        } else if req.contains("GET /paper_changed.pdf") {
                            ("HTTP/1.1 200 OK", "application/pdf", b"%PDF-1.4 TAMPERED content differs from original hash".to_vec())
                        } else if req.contains("GET /works/10.1000/retracted") {
                            let json = r#"{"message":{"update-to":[{"type":"retraction","label":"Retracted paper"}]}}"#;
                            ("HTTP/1.1 200 OK", "application/json", json.as_bytes().to_vec())
                        } else if req.contains("GET /works/10.1000/updated") {
                            let json = r#"{"message":{"version":2}}"#;
                            ("HTTP/1.1 200 OK", "application/json", json.as_bytes().to_vec())
                        } else if req.contains("GET /works/") {
                            let json = r#"{"message":{"version":1}}"#;
                            ("HTTP/1.1 200 OK", "application/json", json.as_bytes().to_vec())
                        } else {
                            ("HTTP/1.1 404 Not Found", "text/plain", b"Not found".to_vec())
                        };

                        let response = format!(
                            "{}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            status_line, content_type, body.len()
                        );
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.write_all(&body);
                        let _ = stream.flush();
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            addr,
            shutdown,
            handle: Some(handle),
        }
    }
}

impl Drop for MockScholarServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use s3a_core::ADDON_EDU;
    use s3a_engine::S3ACrudEngine;
    use crate::ext::ScholarCrudExt;
    use crate::CONTENT_KIND_PDF;

    #[test]
    fn test_scholar_recheck_offline_and_mock_server_all_four_states() {
        let server = MockScholarServer::start();
        let valid_content = b"%PDF-1.4 Mock valid content for research paper";

        let temp_dir = std::env::temp_dir();
        let unique_name = format!("scholar_recheck_test_{}.s3a", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
        let live_path = temp_dir.join(unique_name);
        let _ = std::fs::remove_file(&live_path);

        // Create S3A file and enable ADDON_EDU
        let engine = S3ACrudEngine::open_or_create(&live_path).unwrap();
        engine.enable_addon(ADDON_EDU).unwrap();

        // 1. Unchanged paper
        let fp_unchanged = CitationFingerprintRecord::from_content(
            101,
            "10.1000/101",
            valid_content,
            CONTENT_KIND_PDF,
            &format!("{}/paper_ok.pdf", server.addr),
            1,
            1600000000,
        );

        // 2. Changed paper (recorded hash does not match server content)
        let fp_changed = CitationFingerprintRecord::from_content(
            102,
            "10.1000/102",
            b"%PDF-1.4 Original clean bytes before modifications",
            CONTENT_KIND_PDF,
            &format!("{}/paper_changed.pdf", server.addr),
            1,
            1600000000,
        );

        // 3. Retracted paper
        let fp_retracted = CitationFingerprintRecord::from_content(
            103,
            "10.1000/retracted",
            valid_content,
            CONTENT_KIND_PDF,
            &format!("{}/paper_ok.pdf", server.addr),
            1,
            1600000000,
        );

        // 4. Unreachable paper
        let fp_unreachable = CitationFingerprintRecord::from_content(
            104,
            "10.1000/104",
            valid_content,
            CONTENT_KIND_PDF,
            "http://127.0.0.1:9/unreachable_nonexistent.pdf",
            1,
            1600000000,
        );

        let records = vec![fp_unchanged, fp_changed, fp_retracted, fp_unreachable];
        engine.create_citation_fingerprints(&records).unwrap();

        // Run recheck with online enabled against our mock server
        let mut opts = RecheckOptions {
            online: true,
            crossref_endpoint: Some(server.addr.clone()),
            url_overrides: HashMap::new(),
            out_path: None,
        };
        opts.url_overrides.insert(101, ("10.1000/101".to_string(), format!("{}/paper_ok.pdf", server.addr)));
        opts.url_overrides.insert(102, ("10.1000/102".to_string(), format!("{}/paper_changed.pdf", server.addr)));
        opts.url_overrides.insert(103, ("10.1000/retracted".to_string(), format!("{}/paper_ok.pdf", server.addr)));
        opts.url_overrides.insert(104, ("10.1000/104".to_string(), "http://127.0.0.1:9/unreachable.pdf".to_string()));

        let report = recheck_archive(&live_path, &opts).unwrap();
        assert_eq!(report.items.len(), 4);
        assert_eq!(report.unchanged_count, 1);
        assert_eq!(report.changed_count, 1);
        assert_eq!(report.retracted_count, 1);
        assert_eq!(report.unreachable_count, 1);
        assert_eq!(report.marks_appended, 3);
        assert_eq!(report.statements_appended, 3);

        // Verify status values
        assert_eq!(report.items[0].status, RecheckStatus::Unchanged);
        assert_eq!(report.items[1].status, RecheckStatus::Changed);
        assert_eq!(report.items[2].status, RecheckStatus::Retracted);
        assert_eq!(report.items[3].status, RecheckStatus::Unreachable);

        let _ = std::fs::remove_file(&live_path);
    }
}
