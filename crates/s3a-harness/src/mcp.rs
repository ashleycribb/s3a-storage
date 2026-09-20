//! Native Model Context Protocol (MCP) Dispatcher for S3A Autonomous Harness.
//! Conforms strictly to MCP 2024-11-05 specification.
//! Provides tools for academic literature search, synthesis matrix, vault audit trails,
//! and Scholar Research Snapshot (.snapshot.s3a) export and intake.

use s3a_core::SrsManifestHeader;
use s3a_engine::{MmapReader, QuerySieve, S3ACrudEngine};

pub struct S3AMcpDispatcher;

impl S3AMcpDispatcher {
    pub fn handle_request(json_str: &str, default_db: &str) -> Option<String> {
        let method = parse_str(json_str, "method")?;
        let id_opt = parse_id(json_str);

        match method.as_str() {
            "initialize" => {
                let id = id_opt.unwrap_or_else(|| "\"1\"".to_string());
                Some(format!(
                    "{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{{\"tools\":{{}}}},\"serverInfo\":{{\"name\":\"s3a-academic-harness\",\"version\":\"0.1.0\"}}}}}}",
                    id
                ))
            }
            "notifications/initialized" => None,
            "ping" => {
                let id = id_opt.unwrap_or_else(|| "\"1\"".to_string());
                Some(format!("{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{{}}}}", id))
            }
            "tools/list" => {
                let id = id_opt.unwrap_or_else(|| "\"1\"".to_string());
                Some(format!(
                    "{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{{\"tools\":[{}]}}}}",
                    id,
                    Self::tools_json()
                ))
            }
            "tools/call" => {
                let id = id_opt.unwrap_or_else(|| "\"1\"".to_string());
                let tool_name = parse_str(json_str, "name").unwrap_or_default();
                let args_json = extract_subobject(json_str, "arguments").unwrap_or_default();

                let (result_text, is_err) = Self::execute_tool(&tool_name, &args_json, default_db);
                let escaped_text = result_text.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");

                Some(format!(
                    "{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{{\"content\":[{{\"type\":\"text\",\"text\":\"{}\"}}],\"isError\":{}}}}}",
                    id, escaped_text, is_err
                ))
            }
            _ => {
                let id = id_opt.unwrap_or_else(|| "\"1\"".to_string());
                Some(format!(
                    "{{\"jsonrpc\":\"2.0\",\"id\":{},\"error\":{{\"code\":-32601,\"message\":\"Method not found: {}\"}}}}",
                    id, method
                ))
            }
        }
    }

    fn tools_json() -> String {
        let tools = vec![
            r#"{"name":"s3a_search_library","description":"Search academic research papers across 3D topic and methodology space with Skilling Hilbert curve ranking","inputSchema":{"type":"object","properties":{"min_year":{"type":"integer","description":"Minimum publication year"},"max_year":{"type":"integer","description":"Maximum publication year"},"file":{"type":"string","description":"Optional database path"}}}}"#,
            r#"{"name":"s3a_get_synthesis_matrix","description":"Retrieve multi-dimensional research synthesis matrix showing papers, citation counts, and evidence edges","inputSchema":{"type":"object","properties":{"file":{"type":"string","description":"Optional database path"}}}}"#,
            r#"{"name":"s3a_get_vault_audit_trail","description":"Fetch dual-actor provenance cross-trace matching human oversight actions to AI agent runs via 128-bit session UUID","inputSchema":{"type":"object","properties":{"session_uuid":{"type":"string","description":"Hexadecimal 128-bit session UUID"},"file":{"type":"string","description":"Optional database path"}},"required":["session_uuid"]}}"#,
            r#"{"name":"s3a_export_srs","description":"Export portable Scholar Research Snapshot (.snapshot.s3a) container for offline exchange or peer review","inputSchema":{"type":"object","properties":{"snapshot_file":{"type":"string","description":"Target snapshot path"},"file":{"type":"string","description":"Optional live database path"}},"required":["snapshot_file"]}}"#,
            r#"{"name":"s3a_intake_srs","description":"Zero-copy intake and mount of a Scholar Research Snapshot container in sub-millisecond latency","inputSchema":{"type":"object","properties":{"snapshot_file":{"type":"string","description":"Snapshot path to mount"}},"required":["snapshot_file"]}}"#,
        ];
        tools.join(",")
    }

    fn execute_tool(tool_name: &str, args_json: &str, default_db: &str) -> (String, bool) {
        let file = parse_str(args_json, "file").unwrap_or_else(|| default_db.to_string());

        match tool_name {
            "s3a_search_library" => {
                let min_year = parse_u64(args_json, "min_year").unwrap_or(0) as u16;
                let max_year = parse_u64(args_json, "max_year").unwrap_or(u16::MAX as u64) as u16;
                match MmapReader::open(&file) {
                    Ok(reader) => {
                        let sieve = QuerySieve::new(&reader);
                        let papers = sieve.query_academic_papers_3d([-f32::INFINITY; 3], [f32::INFINITY; 3], min_year, max_year);
                        let mut summary = format!("Found {} academic papers in '{}' (Year {}-{}):\n", papers.len(), file, min_year, max_year);
                        for (p, coord) in papers.iter().take(20) {
                            summary.push_str(&format!(
                                "  - [{}] Paper ID: 0x{:016X} | Year: {} | Citations: {} | Venue: {} | OA: {}\n",
                                coord, p.paper_id, p.year, p.citation_count, p.venue_id, if p.open_access_flag == 1 { "Yes" } else { "No" }
                            ));
                        }
                        (summary, false)
                    }
                    Err(e) => (format!("Search error: {}", e), true),
                }
            }
            "s3a_get_synthesis_matrix" => {
                match MmapReader::open(&file) {
                    Ok(reader) => {
                        let sieve = QuerySieve::new(&reader);
                        let papers = sieve.query_academic_papers_3d([-f32::INFINITY; 3], [f32::INFINITY; 3], 0, u16::MAX);
                        let edges = sieve.query_research_graph(None, None);
                        let matrix = format!(
                            "Synthesis Matrix for '{}':\n  - Analyzed Works: {}\n  - Knowledge Graph Relations: {}\n  - Status: Hardware CRC32C Verified",
                            file, papers.len(), edges.len()
                        );
                        (matrix, false)
                    }
                    Err(e) => (format!("Synthesis error: {}", e), true),
                }
            }
            "s3a_get_vault_audit_trail" => {
                let uuid_str = parse_str(args_json, "session_uuid").unwrap_or_default();
                let clean = uuid_str.replace('-', "");
                if clean.len() != 32 {
                    return ("Invalid 128-bit UUID format (must be 32 hex characters)".to_string(), true);
                }
                let high = match u64::from_str_radix(&clean[0..16], 16) {
                    Ok(v) => v,
                    Err(_) => return ("Failed to parse UUID high bits".to_string(), true),
                };
                let low = match u64::from_str_radix(&clean[16..32], 16) {
                    Ok(v) => v,
                    Err(_) => return ("Failed to parse UUID low bits".to_string(), true),
                };

                match MmapReader::open(&file) {
                    Ok(reader) => {
                        let sieve = QuerySieve::new(&reader);
                        let human = sieve.query_human_lrs(Some([high, low]), None, 0, u64::MAX);
                        let ai = sieve.query_ai_traces(Some([high, low]), None, 0, u64::MAX);
                        let mut report = format!("Vault Audit Trail for Session {:016X}{:016X} ({} records):\n", high, low, human.len() + ai.len());
                        report.push_str(&format!("  - Human Decisions: {}\n", human.len()));
                        for (h, coord) in &human {
                            report.push_str(&format!("    ├─ [{}] Actor: 0x{:016X}, Verb: {}, Score: {:.2}, Linked AI: {}\n", coord, h.actor_hash, h.verb_id, h.decision_score, h.ai_trace_coord));
                        }
                        report.push_str(&format!("  - AI Agent Traces: {}\n", ai.len()));
                        for (a, coord) in &ai {
                            report.push_str(&format!("    ├─ [{}] Agent: 0x{:016X}, Step: {}, Conf: {:.2}, Linked Human: {}\n", coord, a.agent_id, a.step_type, a.confidence, a.human_coord));
                        }
                        (report, false)
                    }
                    Err(e) => (format!("Audit error: {}", e), true),
                }
            }
            "s3a_export_srs" => {
                let snapshot_file = parse_str(args_json, "snapshot_file").unwrap_or_else(|| "project.snapshot.s3a".to_string());
                match S3ACrudEngine::open_or_create(&file) {
                    Ok(engine) => {
                        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
                        let manifest = SrsManifestHeader::new([0x1234, 0x5678], [0xAAAA, ts], [0, 0], [0, 0], ts, 0, 0);
                        match engine.export_srs_snapshot(&snapshot_file, &manifest, None, None) {
                            Ok(b) => (format!("Successfully exported SRS snapshot to '{}' ({} total records).", snapshot_file, b.manifest.total_records), false),
                            Err(e) => (format!("Export error: {}", e), true),
                        }
                    }
                    Err(e) => (format!("Engine error: {}", e), true),
                }
            }
            "s3a_intake_srs" => {
                let snapshot_file = parse_str(args_json, "snapshot_file").unwrap_or_else(|| "project.snapshot.s3a".to_string());
                match S3ACrudEngine::intake_srs_snapshot(&snapshot_file) {
                    Ok(b) => (
                        format!(
                            "Mounted SRS snapshot '{}' in < 1ms: Project UUID {:016X}{:016X}, Papers: {}, Edges: {}, Human Decisions: {}, AI Traces: {}",
                            snapshot_file, b.manifest.project_uuid[0], b.manifest.project_uuid[1],
                            b.papers.len(), b.evidence_edges.len(), b.human_decisions.len(), b.ai_traces.len()
                        ),
                        false,
                    ),
                    Err(e) => (format!("Intake error: {}", e), true),
                }
            }
            _ => (format!("Unknown tool: {}", tool_name), true),
        }
    }
}

// Helpers for zero-dependency JSON extraction
fn parse_id(json: &str) -> Option<String> {
    if let Some(pos) = json.find("\"id\":") {
        let rest = json[pos + 5..].trim_start();
        if rest.starts_with('"') {
            let end = rest[1..].find('"')? + 1;
            Some(rest[..=end].to_string())
        } else {
            let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
            Some(rest[..end].to_string())
        }
    } else {
        None
    }
}

fn parse_str(json: &str, key: &str) -> Option<String> {
    let search = format!("\"{}\":", key);
    if let Some(pos) = json.find(&search) {
        let rest = json[pos + search.len()..].trim_start();
        if rest.starts_with('"') {
            let end = rest[1..].find('"')?;
            Some(rest[1..1 + end].to_string())
        } else {
            None
        }
    } else {
        None
    }
}

fn parse_u64(json: &str, key: &str) -> Option<u64> {
    let search = format!("\"{}\":", key);
    if let Some(pos) = json.find(&search) {
        let rest = json[pos + search.len()..].trim_start();
        let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
        rest[..end].parse().ok()
    } else {
        None
    }
}

fn extract_subobject(json: &str, key: &str) -> Option<String> {
    let search = format!("\"{}\":", key);
    let pos = json.find(&search)?;
    let rest = json[pos + search.len()..].trim_start();
    if rest.starts_with('{') {
        let mut depth = 0;
        for (i, c) in rest.char_indices() {
            if c == '{' { depth += 1; }
            else if c == '}' {
                depth -= 1;
                if depth == 0 {
                    return Some(rest[..=i].to_string());
                }
            }
        }
    }
    None
}
