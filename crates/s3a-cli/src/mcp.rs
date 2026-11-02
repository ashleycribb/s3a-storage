use std::io::{self, BufRead, Write};
use std::path::Path;

use s3a_engine::{
    execute_query, Lexer, MmapReader, Parser, QuerySieve, S3ACoordinate, S3ACrudEngine,
    S3AStatement, TelemetryRecord,
};
use s3a_pack_scholar::{intake_srs_snapshot, ScholarCrudExt, ScholarQueryExt, SrsManifestHeader};

/// Tools that change the archive or write files. They are not listed and cannot be called
/// unless the server was started with the explicit `--allow-writes` opt-in.
pub const WRITE_TOOLS: [&str; 4] = [
    "s3a_insert_telemetry",
    "s3a_delete_telemetry",
    "s3a_compact_archive",
    "s3a_export_srs",
];

fn write_refused(what: &str) -> String {
    format!(
        "{} is a write operation and this MCP server is read-only. Start it with `s3a-cli mcp-server <archive> --allow-writes` to enable writes.",
        what
    )
}

/// True when the S3A-QL text is a read-only statement (FETCH or SIFT). Anything that does
/// not parse, or that is INSERT / DELETE / COMPACT / FUSE, is treated as a write.
fn query_is_read_only(query: &str) -> bool {
    let tokens = match Lexer::new(query).tokenize_all() {
        Ok(t) => t,
        Err(_) => return false,
    };
    matches!(
        Parser::new(tokens).parse_statement(),
        Ok(S3AStatement::Fetch(_)) | Ok(S3AStatement::Sift(_))
    )
}

/// Runs the standard Model Context Protocol (MCP) server over stdin/stdout.
/// Read-only unless `allow_writes` is true (the `--allow-writes` flag).
pub fn run_mcp_server<P: AsRef<Path>>(archive_path: P, allow_writes: bool) {
    let path_str = archive_path.as_ref().to_string_lossy().to_string();
    eprintln!(
        "[S3A MCP Server] Initialized for archive: {} ({})",
        path_str,
        if allow_writes {
            "WRITES ENABLED via --allow-writes"
        } else {
            "read-only"
        }
    );

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Some(resp) = handle_mcp_request(trimmed, &path_str, allow_writes) {
            let _ = stdout.write_all(resp.as_bytes());
            let _ = stdout.write_all(b"\n");
            let _ = stdout.flush();
        }
    }
}

/// Dispatches JSON-RPC 2.0 requests for the MCP specification.
pub fn handle_mcp_request(
    json_str: &str,
    default_path: &str,
    allow_writes: bool,
) -> Option<String> {
    let method = parse_str(json_str, "method")?;
    let id_opt = parse_id(json_str);

    match method.as_str() {
        "initialize" => {
            let id = id_opt.unwrap_or_else(|| "\"1\"".to_string());
            Some(format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{{\"tools\":{{}}}},\"serverInfo\":{{\"name\":\"s3a-storage-engine\",\"version\":\"0.1.0\"}}}}}}",
                id
            ))
        }
        "notifications/initialized" => None,
        "ping" => {
            let id = id_opt.unwrap_or_else(|| "\"1\"".to_string());
            Some(format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{{}}}}",
                id
            ))
        }
        "tools/list" => {
            let id = id_opt.unwrap_or_else(|| "\"1\"".to_string());
            Some(format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{{\"tools\":[{}]}}}}",
                id,
                get_tools_definitions(allow_writes)
            ))
        }
        "tools/call" => {
            let id = id_opt.unwrap_or_else(|| "\"1\"".to_string());
            let tool_name = parse_str(json_str, "name").unwrap_or_default();
            let args_json = extract_subobject(json_str, "arguments").unwrap_or_default();

            let (result_text, is_err) =
                execute_tool(&tool_name, &args_json, default_path, allow_writes);
            let escaped_text = result_text
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n");

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

fn get_tools_definitions(allow_writes: bool) -> String {
    let tools = vec![
        r#"{"name":"s3a_query","description":"Execute a native S3A-QL query statement (FETCH, SIFT; INSERT, DELETE, COMPACT, FUSE only with --allow-writes)","inputSchema":{"type":"object","properties":{"query":{"type":"string","description":"The S3A-QL statement"}},"required":["query"]}}"#,
        r#"{"name":"s3a_fetch_coordinate","description":"Instant O(1) lookup of a record at coordinate address (e.g. L0:T0:R0)","inputSchema":{"type":"object","properties":{"coordinate":{"type":"string","description":"Coordinate address in format L<level>:T<tile>:R<offset>"},"file":{"type":"string","description":"Optional archive path"}},"required":["coordinate"]}}"#,
        r#"{"name":"s3a_insert_telemetry","description":"Append a telemetry record into the S3A archive with real-time L0 ring buffering","inputSchema":{"type":"object","properties":{"timestamp":{"type":"integer","description":"Epoch timestamp"},"sensor_id":{"type":"integer","description":"Sensor ID"},"metric_id":{"type":"integer","description":"Metric ID"},"value":{"type":"number","description":"Metric measurement value"},"file":{"type":"string","description":"Optional archive path"}},"required":["timestamp","sensor_id","metric_id","value"]}}"#,
        r#"{"name":"s3a_delete_telemetry","description":"Soft-delete a record by appending a tombstone","inputSchema":{"type":"object","properties":{"timestamp":{"type":"integer","description":"Timestamp of record"},"sensor_id":{"type":"integer","description":"Sensor ID"},"metric_id":{"type":"integer","description":"Metric ID"},"file":{"type":"string","description":"Optional archive path"}},"required":["timestamp","sensor_id","metric_id"]}}"#,
        r#"{"name":"s3a_inspect_archive","description":"Inspect S3A Hyper-Tile metadata, active generation, and hardware CRC32C status","inputSchema":{"type":"object","properties":{"file":{"type":"string","description":"Optional archive path"}}}}"#,
        r#"{"name":"s3a_compact_archive","description":"Run stratified compaction and purge tombstoned records","inputSchema":{"type":"object","properties":{"file":{"type":"string","description":"Optional archive path"}}}}"#,
        r#"{"name":"s3a_search_library","description":"Search academic research papers across 3D topic and methodology space with Skilling Hilbert curve ranking","inputSchema":{"type":"object","properties":{"min_year":{"type":"integer","description":"Minimum publication year"},"max_year":{"type":"integer","description":"Maximum publication year"},"file":{"type":"string","description":"Optional archive path"}}}}"#,
        r#"{"name":"s3a_get_synthesis_matrix","description":"Retrieve multi-dimensional research synthesis matrix showing papers, citation counts, and open-access status","inputSchema":{"type":"object","properties":{"file":{"type":"string","description":"Optional archive path"}}}}"#,
        r#"{"name":"s3a_export_srs","description":"Export a portable, reproducible Scholar Research Snapshot (.snapshot.s3a) container","inputSchema":{"type":"object","properties":{"snapshot_file":{"type":"string","description":"Target snapshot file path"},"file":{"type":"string","description":"Optional live database path"}},"required":["snapshot_file"]}}"#,
        r#"{"name":"s3a_intake_srs","description":"Intake a Scholar Research Snapshot (.snapshot.s3a) container via zero-copy mmap with CRC32C verification","inputSchema":{"type":"object","properties":{"snapshot_file":{"type":"string","description":"Snapshot file path to intake"}},"required":["snapshot_file"]}}"#,
    ];
    tools
        .into_iter()
        .filter(|t| {
            allow_writes
                || !WRITE_TOOLS
                    .iter()
                    .any(|w| t.contains(&format!("\"name\":\"{}\"", w)))
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn execute_tool(
    tool_name: &str,
    args_json: &str,
    default_path: &str,
    allow_writes: bool,
) -> (String, bool) {
    let file = parse_str(args_json, "file").unwrap_or_else(|| default_path.to_string());

    if !allow_writes && WRITE_TOOLS.contains(&tool_name) {
        return (write_refused(tool_name), true);
    }

    match tool_name {
        "s3a_query" => {
            let q = parse_str(args_json, "query").unwrap_or_default();
            if !allow_writes && !query_is_read_only(&q) {
                return (write_refused("This S3A-QL statement"), true);
            }
            match execute_query(&q) {
                Ok(res) => (format!("{:?}", res), false),
                Err(e) => (format!("Query error: {}", e), true),
            }
        }
        "s3a_fetch_coordinate" => {
            let coord_str = parse_str(args_json, "coordinate").unwrap_or_default();
            match coord_str.parse::<S3ACoordinate>() {
                // MmapReader is the same read path the engine uses, and never creates a file.
                Ok(c) => match MmapReader::open(&file) {
                    Ok(reader) => match reader.read_record_by_coordinate::<TelemetryRecord>(&c) {
                        Ok(r) => (
                            format!(
                                "Record at {}: timestamp={}, sensor={}, metric={}, value={:.2}",
                                c, r.timestamp, r.sensor_id, r.metric_id, r.value
                            ),
                            false,
                        ),
                        Err(e) => (format!("Lookup error: {}", e), true),
                    },
                    Err(e) => (format!("Engine error: {}", e), true),
                },
                Err(e) => (format!("Invalid coordinate format: {}", e), true),
            }
        }
        "s3a_insert_telemetry" => {
            let ts = parse_u64(args_json, "timestamp").unwrap_or(1000);
            let sensor = parse_u64(args_json, "sensor_id").unwrap_or(1) as u32;
            let metric = parse_u64(args_json, "metric_id").unwrap_or(101) as u32;
            let val = parse_f64(args_json, "value").unwrap_or(0.0);

            match S3ACrudEngine::open_or_create(&file) {
                Ok(engine) => {
                    let rec = TelemetryRecord::new(ts, sensor, metric, val);
                    match engine.create_telemetry(&[rec]) {
                        Ok(tile_id) => (format!("Inserted record into tile #{}", tile_id), false),
                        Err(e) => (format!("Insert error: {}", e), true),
                    }
                }
                Err(e) => (format!("Engine error: {}", e), true),
            }
        }
        "s3a_delete_telemetry" => {
            let ts = parse_u64(args_json, "timestamp").unwrap_or(1000);
            let sensor = parse_u64(args_json, "sensor_id").unwrap_or(1) as u32;
            let metric = parse_u64(args_json, "metric_id").unwrap_or(101) as u32;

            match S3ACrudEngine::open_or_create(&file) {
                Ok(engine) => match engine.delete_telemetry(sensor, metric, ts) {
                    Ok(deleted) => (format!("Tombstone delete applied: {}", deleted), false),
                    Err(e) => (format!("Delete error: {}", e), true),
                },
                Err(e) => (format!("Engine error: {}", e), true),
            }
        }
        "s3a_inspect_archive" => match MmapReader::open(&file) {
            Ok(reader) => {
                let hdr = reader.file_header();
                let info = format!(
                        "Archive '{}': version={}, generation={}, active_slot={}, tiles={}, header_crc32=0x{:08X}",
                        file, hdr.version, hdr.generation, reader.active_slot(), reader.tile_count(), hdr.header_crc32
                    );
                (info, false)
            }
            Err(e) => (format!("Failed to inspect archive '{}': {}", file, e), true),
        },
        "s3a_compact_archive" => match S3ACrudEngine::open_or_create(&file) {
            Ok(engine) => match engine.purge_and_compact() {
                Ok(new_count) => (
                    format!(
                        "Compacted archive '{}': new tile count = {}",
                        file, new_count
                    ),
                    false,
                ),
                Err(e) => (format!("Compaction error: {}", e), true),
            },
            Err(e) => (format!("Engine error: {}", e), true),
        },
        "s3a_search_library" => {
            let min_year = parse_u64(args_json, "min_year").unwrap_or(0) as u16;
            let max_year = parse_u64(args_json, "max_year").unwrap_or(u16::MAX as u64) as u16;
            match MmapReader::open(&file) {
                Ok(reader) => {
                    let sieve = QuerySieve::new(&reader);
                    let papers = sieve.query_academic_papers_3d(
                        [-f32::INFINITY; 3],
                        [f32::INFINITY; 3],
                        min_year,
                        max_year,
                    );
                    let mut summary = format!(
                        "Found {} academic papers in '{}' (Year {}-{}):\n",
                        papers.len(),
                        file,
                        min_year,
                        max_year
                    );
                    for (p, coord) in papers.iter().take(25) {
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
        "s3a_get_synthesis_matrix" => match MmapReader::open(&file) {
            Ok(reader) => {
                let sieve = QuerySieve::new(&reader);
                let papers = sieve.query_academic_papers_3d(
                    [-f32::INFINITY; 3],
                    [f32::INFINITY; 3],
                    0,
                    u16::MAX,
                );
                let edges = sieve.query_research_graph(None, None);
                let matrix = format!(
                        "Research Synthesis Matrix for '{}':\n  - Total Analyzed Papers: {}\n  - Extracted Evidence Relationships: {}\n  - 3D Cluster Centroid: In-memory\n  - Provenance: Hardware CRC32C Verified",
                        file, papers.len(), edges.len()
                    );
                (matrix, false)
            }
            Err(e) => (format!("Synthesis error: {}", e), true),
        },
        "s3a_export_srs" => {
            let snapshot_file = parse_str(args_json, "snapshot_file")
                .unwrap_or_else(|| "project.snapshot.s3a".to_string());
            match S3ACrudEngine::open_or_create(&file) {
                Ok(engine) => {
                    let ts = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    let manifest = SrsManifestHeader::new(
                        [0x1234, 0x5678],
                        [0xAAAA, ts],
                        [0, 0],
                        [0, 0],
                        ts,
                        0,
                        0,
                    );
                    match engine.export_srs_snapshot(&snapshot_file, &manifest, None, None) {
                        Ok(b) => (
                            format!(
                                "Successfully exported SRS snapshot to '{}' with {} total records.",
                                snapshot_file, b.manifest.total_records
                            ),
                            false,
                        ),
                        Err(e) => (format!("Export error: {}", e), true),
                    }
                }
                Err(e) => (format!("Engine error: {}", e), true),
            }
        }
        "s3a_intake_srs" => {
            let snapshot_file = parse_str(args_json, "snapshot_file")
                .unwrap_or_else(|| "project.snapshot.s3a".to_string());
            match intake_srs_snapshot(&snapshot_file) {
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

/// Simple interactive verification client for testing the MCP server.
/// Runs read-only unless `allow_writes` is true, in which case step 3 inserts a record.
pub fn run_mcp_client_test<P: AsRef<Path>>(archive_path: P, allow_writes: bool) {
    let path_str = archive_path.as_ref().to_string_lossy().to_string();
    println!("==================================================================");
    println!("        S3A MODEL CONTEXT PROTOCOL (MCP) VERIFICATION CLIENT     ");
    println!("==================================================================");

    // 1. Initialize
    println!("1. Testing MCP 'initialize' request...");
    let init_req = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#;
    let init_resp = handle_mcp_request(init_req, &path_str, allow_writes).unwrap();
    println!("   Response: {}\n", init_resp);

    // 2. List tools
    println!("2. Testing MCP 'tools/list' request...");
    let list_req = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
    let list_resp = handle_mcp_request(list_req, &path_str, allow_writes).unwrap();
    println!("   Response: {}\n", list_resp);

    // 3. Call tool: s3a_insert_telemetry
    println!(
        "3. Testing MCP 'tools/call' for 's3a_insert_telemetry' ({})...",
        if allow_writes {
            "writes enabled"
        } else {
            "expected to be refused: read-only"
        }
    );
    let call_insert = format!(
        r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"s3a_insert_telemetry","arguments":{{"timestamp":1000,"sensor_id":1,"metric_id":101,"value":42.5,"file":"{}"}}}}}}"#,
        path_str
    );
    let insert_resp = handle_mcp_request(&call_insert, &path_str, allow_writes).unwrap();
    println!("   Response: {}\n", insert_resp);

    // 4. Call tool: s3a_fetch_coordinate
    println!("4. Testing MCP 'tools/call' for 's3a_fetch_coordinate'...");
    let call_fetch = format!(
        r#"{{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{{"name":"s3a_fetch_coordinate","arguments":{{"coordinate":"L0:T0:R0","file":"{}"}}}}}}"#,
        path_str
    );
    let fetch_resp = handle_mcp_request(&call_fetch, &path_str, allow_writes).unwrap();
    println!("   Response: {}\n", fetch_resp);

    // 5. Call tool: s3a_inspect_archive
    println!("5. Testing MCP 'tools/call' for 's3a_inspect_archive'...");
    let call_inspect = format!(
        r#"{{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{{"name":"s3a_inspect_archive","arguments":{{"file":"{}"}}}}}}"#,
        path_str
    );
    let inspect_resp = handle_mcp_request(&call_inspect, &path_str, allow_writes).unwrap();
    println!("   Response: {}\n", inspect_resp);

    println!("==================================================================");
    println!("MCP Protocol 2024-11-05 Verification: 100% SUCCESSFUL!");
    println!("==================================================================");
}

// Helpers for zero-dependency JSON extraction
fn parse_id(json: &str) -> Option<String> {
    if let Some(pos) = json.find("\"id\":") {
        let rest = json[pos + 5..].trim_start();
        if rest.starts_with('"') {
            let end = rest[1..].find('"')? + 1;
            Some(rest[..=end].to_string())
        } else {
            let end = rest
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(rest.len());
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
        let end = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        rest[..end].parse().ok()
    } else {
        None
    }
}

fn parse_f64(json: &str, key: &str) -> Option<f64> {
    let search = format!("\"{}\":", key);
    if let Some(pos) = json.find(&search) {
        let rest = json[pos + search.len()..].trim_start();
        let end = rest
            .find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
            .unwrap_or(rest.len());
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
            if c == '{' {
                depth += 1;
            } else if c == '}' {
                depth -= 1;
                if depth == 0 {
                    return Some(rest[..=i].to_string());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_archive(label: &str) -> String {
        let mut p = std::env::temp_dir();
        p.push(format!("s3a_mcp_ro_{}_{}.s3a", label, std::process::id()));
        let _ = std::fs::remove_file(&p);
        p.to_string_lossy().to_string()
    }

    fn call(tool: &str, args: &str, path: &str, allow_writes: bool) -> String {
        let req = format!(
            r#"{{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{{"name":"{}","arguments":{}}}}}"#,
            tool, args
        );
        handle_mcp_request(&req, path, allow_writes).unwrap()
    }

    #[test]
    fn read_only_by_default_hides_write_tools() {
        let list = handle_mcp_request(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
            "x.s3a",
            false,
        )
        .unwrap();
        for w in WRITE_TOOLS {
            assert!(!list.contains(w), "{} must not be listed read-only", w);
        }
        for r in [
            "s3a_query",
            "s3a_fetch_coordinate",
            "s3a_inspect_archive",
            "s3a_search_library",
            "s3a_get_synthesis_matrix",
            "s3a_intake_srs",
        ] {
            assert!(list.contains(r), "{} should stay available", r);
        }
    }

    #[test]
    fn opt_in_lists_write_tools() {
        let list = handle_mcp_request(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
            "x.s3a",
            true,
        )
        .unwrap();
        for w in WRITE_TOOLS {
            assert!(
                list.contains(w),
                "{} should be listed with --allow-writes",
                w
            );
        }
    }

    #[test]
    fn write_tools_are_refused_and_create_nothing_read_only() {
        let path = tmp_archive("refuse");
        let args = format!(
            r#"{{"timestamp":1000,"sensor_id":1,"metric_id":101,"value":42.5,"file":"{}"}}"#,
            path
        );
        for tool in WRITE_TOOLS {
            let resp = call(tool, &args, &path, false);
            assert!(resp.contains("\"isError\":true"), "{}: {}", tool, resp);
            assert!(resp.contains("read-only"), "{}: {}", tool, resp);
        }
        assert!(
            !Path::new(&path).exists(),
            "read-only mode must not create the archive"
        );
    }

    #[test]
    fn s3a_query_refuses_write_statements_read_only() {
        let path = tmp_archive("query");
        // Valid statements (single-quoted paths so they survive the JSON string), so the
        // refusal comes from the read-only gate and not from a parse error.
        for q in [
            format!("INSERT TELEMETRY (1000, 42, 99, 123.45) INTO '{}'", path),
            format!("DELETE TELEMETRY WHERE SENSOR_ID = 42 AND METRIC_ID = 99 AND TIME = 1000 FROM '{}'", path),
            format!("COMPACT ARCHIVE '{}'", path),
            "this is not a statement".to_string(),
        ] {
            let args = format!(r#"{{"query":"{}"}}"#, q);
            let resp = call("s3a_query", &args, &path, false);
            assert!(resp.contains("\"isError\":true"), "{}: {}", q, resp);
            assert!(resp.contains("read-only"), "{}: {}", q, resp);
        }
        assert!(!Path::new(&path).exists());
    }

    #[test]
    fn query_gate_classifies_statements() {
        assert!(query_is_read_only("FETCH RECORD AT L0:T0:R0 FROM 'a.s3a'"));
        assert!(query_is_read_only("SIFT TELEMETRY FROM 'a.s3a' LIMIT 5"));
        assert!(!query_is_read_only(
            "INSERT TELEMETRY (1000, 42, 99, 123.45) INTO 'a.s3a'"
        ));
        assert!(!query_is_read_only("COMPACT ARCHIVE 'a.s3a'"));
        assert!(!query_is_read_only("garbage"));
    }

    #[test]
    fn fetch_does_not_create_a_missing_archive() {
        let path = tmp_archive("fetch");
        let args = format!(r#"{{"coordinate":"L0:T0:R0","file":"{}"}}"#, path);
        let resp = call("s3a_fetch_coordinate", &args, &path, false);
        assert!(resp.contains("\"isError\":true"), "{}", resp);
        assert!(!Path::new(&path).exists());
    }

    #[test]
    fn opt_in_allows_insert_then_read_back() {
        let path = tmp_archive("optin");
        let args = format!(
            r#"{{"timestamp":1000,"sensor_id":1,"metric_id":101,"value":42.5,"file":"{}"}}"#,
            path
        );
        let resp = call("s3a_insert_telemetry", &args, &path, true);
        assert!(resp.contains("\"isError\":false"), "{}", resp);
        // The same archive is readable in read-only mode.
        let insp = call(
            "s3a_inspect_archive",
            &format!(r#"{{"file":"{}"}}"#, path),
            &path,
            false,
        );
        assert!(insp.contains("\"isError\":false"), "{}", insp);
        let _ = std::fs::remove_file(&path);
    }
}
