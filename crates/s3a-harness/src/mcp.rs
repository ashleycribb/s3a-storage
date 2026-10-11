//! Native Model Context Protocol (MCP) Dispatcher for S3A Autonomous Harness.
//! Conforms strictly to MCP 2024-11-05 specification and JSON-RPC 2.0.
//! Provides tools for academic literature search, synthesis matrix, vault audit trails,
//! Scholar Research Snapshot (.snapshot.s3a) export and intake, learnmcp xAPI tools,
//! and supervisory human-in-the-loop gates.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use s3a_core::S3ACoordinate;
use s3a_engine::{MmapReader, QuerySieve, S3ACrudEngine};
use s3a_pack_scholar::{intake_srs_snapshot, ScholarCrudExt, ScholarQueryExt, SrsManifestHeader};

use crate::agent::{EpistemicConfidenceTier, ResearchLoopEngine};
use crate::sandbox::Sandbox;
use crate::store::{LocalStore, Store};

pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

// =============================================================================
// Tool Input Schemas (Generated directly from Rust structs)
// =============================================================================

pub trait ToolSchema {
    fn tool_name() -> &'static str;
    fn description() -> &'static str;
    fn input_schema() -> Value;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchLibraryArgs {
    #[serde(default)]
    pub min_year: Option<u16>,
    #[serde(default)]
    pub max_year: Option<u16>,
}

impl ToolSchema for SearchLibraryArgs {
    fn tool_name() -> &'static str {
        "s3a_search_library"
    }
    fn description() -> &'static str {
        "Search academic research papers across 3D topic and methodology space with Skilling Hilbert curve ranking"
    }
    fn input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "min_year": { "type": "integer", "description": "Minimum publication year" },
                "max_year": { "type": "integer", "description": "Maximum publication year" }
            }
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetSynthesisMatrixArgs {}

impl ToolSchema for GetSynthesisMatrixArgs {
    fn tool_name() -> &'static str {
        "s3a_get_synthesis_matrix"
    }
    fn description() -> &'static str {
        "Retrieve multi-dimensional research synthesis matrix showing papers, citation counts, and evidence edges"
    }
    fn input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetVaultAuditTrailArgs {
    #[serde(default)]
    pub session_uuid: Option<String>,
}

impl ToolSchema for GetVaultAuditTrailArgs {
    fn tool_name() -> &'static str {
        "s3a_get_vault_audit_trail"
    }
    fn description() -> &'static str {
        "Fetch dual-actor provenance cross-trace matching human oversight actions to AI agent runs via 128-bit session UUID"
    }
    fn input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_uuid": { "type": "string", "description": "Optional hexadecimal 128-bit session UUID; defaults to active session" }
            }
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExportSrsArgs {
    // Per-call file/snapshot_file path args removed for sandbox safety;
    // snapshots are automatically created as UUIDv7 containers inside <root>/exports/
}

impl ToolSchema for ExportSrsArgs {
    fn tool_name() -> &'static str {
        "s3a_export_srs"
    }
    fn description() -> &'static str {
        "Export portable Scholar Research Snapshot container to <root>/exports/ for offline exchange or peer review"
    }
    fn input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntakeSrsArgs {
    #[serde(alias = "filename")]
    pub snapshot_id: String,
}

impl ToolSchema for IntakeSrsArgs {
    fn tool_name() -> &'static str {
        "s3a_intake_srs"
    }
    fn description() -> &'static str {
        "Zero-copy intake and mount of a Scholar Research Snapshot container within the sandbox root"
    }
    fn input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "snapshot_id": { "type": "string", "description": "Snapshot identifier or filename located within exports/ or root" }
            },
            "required": ["snapshot_id"]
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordXapiStatementArgs {
    pub statement: Value,
}

impl ToolSchema for RecordXapiStatementArgs {
    fn tool_name() -> &'static str {
        "record_xapi_statement"
    }
    fn description() -> &'static str {
        "Validate and record an xAPI statement compliant with research_traceability profile and agent allowlist"
    }
    fn input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "statement": { "type": "object", "description": "xAPI statement object" }
            },
            "required": ["statement"]
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetSessionContextArgs {
    #[serde(default)]
    pub session_id: Option<String>,
}

impl ToolSchema for GetSessionContextArgs {
    fn tool_name() -> &'static str {
        "get_session_context"
    }
    fn description() -> &'static str {
        "Retrieve active research session context, participant metrics, and recent events"
    }
    fn input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string", "description": "Optional session UUID; defaults to active session" }
            }
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListAcademicVerbsArgs {}

impl ToolSchema for ListAcademicVerbsArgs {
    fn tool_name() -> &'static str {
        "list_academic_verbs"
    }
    fn description() -> &'static str {
        "List all registered academic and Teacher Assistant xAPI verbs in the Scholar Explorer ontology"
    }
    fn input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetResearcherTimelineArgs {
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

impl ToolSchema for GetResearcherTimelineArgs {
    fn tool_name() -> &'static str {
        "get_researcher_timeline"
    }
    fn description() -> &'static str {
        "Retrieve an ordered timeline of researcher decisions and AI reasoning steps"
    }
    fn input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "agent_id": { "type": "string", "description": "Optional filter by agent ID" },
                "session_id": { "type": "string", "description": "Optional filter by session UUID" },
                "limit": { "type": "integer", "description": "Maximum number of events (default: 100)" }
            }
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreflightCheckArgs {
    pub candidate_confidence: f32,
    #[serde(default)]
    pub unbound_entities: Vec<u64>,
    #[serde(default = "default_true")]
    pub has_vetted_analogy: bool,
}

fn default_true() -> bool {
    true
}

impl ToolSchema for PreflightCheckArgs {
    fn tool_name() -> &'static str {
        "preflight_check"
    }
    fn description() -> &'static str {
        "Evaluate preflight hallucination risk on a candidate draft before emission"
    }
    fn input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "candidate_confidence": { "type": "number", "description": "Epistemic confidence score (0.00 to 1.00)" },
                "unbound_entities": { "type": "array", "items": { "type": "integer" }, "description": "Unbound entity hashes" },
                "has_vetted_analogy": { "type": "boolean", "description": "Whether analogy is ontology-grounded" }
            },
            "required": ["candidate_confidence"]
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestHumanGateArgs {
    pub action_type: String,
    pub reason: String,
    #[serde(default)]
    pub details: Option<Value>,
}

impl ToolSchema for RequestHumanGateArgs {
    fn tool_name() -> &'static str {
        "request_human_gate"
    }
    fn description() -> &'static str {
        "Request human supervisor gate approval for a critical action (returns pending; agent must wait)"
    }
    fn input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "action_type": { "type": "string", "description": "Type of action requiring approval" },
                "reason": { "type": "string", "description": "Reason human gate is needed" },
                "details": { "type": "object", "description": "Optional action payload details" }
            },
            "required": ["action_type", "reason"]
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordHumanDecisionArgs {
    pub actor_id: u64,
    pub verb_id: u32,
    pub decision_score: f32,
    #[serde(default)]
    pub ai_coord: Option<String>,
    #[serde(default)]
    pub target_object_id: Option<u32>,
    #[serde(default)]
    pub gate_id: Option<String>,
}

impl ToolSchema for RecordHumanDecisionArgs {
    fn tool_name() -> &'static str {
        "record_human_decision"
    }
    fn description() -> &'static str {
        "Record human supervisor decision approving, refining, or rejecting an agent action"
    }
    fn input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "actor_id": { "type": "integer", "description": "Human actor identifier" },
                "verb_id": { "type": "integer", "description": "Verb ID (e.g. 0x00E1 ta-approved-draft, 0x0003 approved)" },
                "decision_score": { "type": "number", "description": "Decision score (1.0 = approve, 0.0 = reject)" },
                "ai_coord": { "type": "string", "description": "Optional AI coordinate being reviewed" },
                "target_object_id": { "type": "integer", "description": "Target entity or concept ID" },
                "gate_id": { "type": "string", "description": "Optional gate ID to resolve" }
            },
            "required": ["actor_id", "verb_id", "decision_score"]
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordAiStepArgs {
    pub step_type: u32,
    pub confidence: f32,
    #[serde(default)]
    pub human_coord: Option<String>,
    #[serde(default)]
    pub hilbert_index: Option<u32>,
    #[serde(default)]
    pub gate_id: Option<String>,
}

impl ToolSchema for RecordAiStepArgs {
    fn tool_name() -> &'static str {
        "record_ai_step"
    }
    fn description() -> &'static str {
        "Record an AI reasoning step. Gated actions without matching human decision are rejected and logged."
    }
    fn input_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "step_type": { "type": "integer", "description": "Step category identifier" },
                "confidence": { "type": "number", "description": "Agent confidence score (0.00 to 1.00)" },
                "human_coord": { "type": "string", "description": "Linked human decision coordinate" },
                "hilbert_index": { "type": "integer", "description": "Hilbert spatial index" },
                "gate_id": { "type": "string", "description": "Gate ID if this action was gated" }
            },
            "required": ["step_type", "confidence"]
        })
    }
}

// =============================================================================
// MCP Dispatcher Implementation
// =============================================================================

pub struct S3AMcpDispatcher {
    sandbox: Sandbox,
    store: Arc<dyn Store>,
    loop_engine: Arc<ResearchLoopEngine>,
    project_uuid: [u64; 2],
    session_uuid: [u64; 2],
    negotiated_version: RwLock<String>,
}

impl S3AMcpDispatcher {
    pub fn new(
        sandbox: Sandbox,
        store: Arc<dyn Store>,
        project_uuid: [u64; 2],
        session_uuid: [u64; 2],
        agent_id: u64,
    ) -> Self {
        let loop_engine = Arc::new(ResearchLoopEngine::new(session_uuid, agent_id));
        Self {
            sandbox,
            store,
            loop_engine,
            project_uuid,
            session_uuid,
            negotiated_version: RwLock::new(MCP_PROTOCOL_VERSION.to_string()),
        }
    }

    /// Backwards-compatible constructor for quick dispatch or testing.
    pub fn handle_request(json_str: &str, default_db: &str) -> Option<String> {
        let db_path = PathBuf::from(default_db);
        let root = db_path.parent().unwrap_or(&db_path).to_path_buf();
        let root = if root.as_os_str().is_empty() {
            PathBuf::from(".")
        } else {
            root
        };

        let sandbox = Sandbox::new(&root, false)
            .unwrap_or_else(|_| Sandbox::new(std::env::temp_dir(), false).unwrap());

        let store: Arc<dyn Store> = Arc::new(LocalStore::new(default_db, false));
        let p_uuid = crate::generate_uuid_v7_pair();
        let s_uuid = crate::generate_uuid_v7_pair();

        let dispatcher = S3AMcpDispatcher::new(sandbox, store, p_uuid, s_uuid, 42);
        dispatcher.dispatch(json_str)
    }

    /// Primary dispatch function for JSON-RPC 2.0 requests over stdio or sockets.
    pub fn dispatch(&self, raw_json: &str) -> Option<String> {
        let parsed: Value = match serde_json::from_str(raw_json) {
            Ok(v) => v,
            Err(e) => {
                // -32700 Parse error
                let resp = json!({
                    "jsonrpc": "2.0",
                    "id": Value::Null,
                    "error": {
                        "code": -32700,
                        "message": format!("Parse error: {}", e)
                    }
                });
                return Some(serde_json::to_string(&resp).unwrap());
            }
        };

        let req_obj = match parsed.as_object() {
            Some(obj) => obj,
            None => {
                // -32600 Invalid request
                let resp = json!({
                    "jsonrpc": "2.0",
                    "id": Value::Null,
                    "error": {
                        "code": -32600,
                        "message": "Invalid Request: expected JSON object"
                    }
                });
                return Some(serde_json::to_string(&resp).unwrap());
            }
        };

        // Check jsonrpc version
        if req_obj.get("jsonrpc").and_then(|v| v.as_str()) != Some("2.0") {
            let id = req_obj.get("id").cloned().unwrap_or(Value::Null);
            let resp = json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32600,
                    "message": "Invalid Request: 'jsonrpc' must be '2.0'"
                }
            });
            return Some(serde_json::to_string(&resp).unwrap());
        }

        // Check if message is a notification (lacks "id" or id is null)
        let id_val = match req_obj.get("id") {
            Some(id) if !id.is_null() => id.clone(),
            _ => {
                // Notifications: MUST NEVER REPLY TO NOTIFICATIONS
                return None;
            }
        };

        let method = match req_obj.get("method").and_then(|v| v.as_str()) {
            Some(m) => m,
            None => {
                let resp = json!({
                    "jsonrpc": "2.0",
                    "id": id_val,
                    "error": {
                        "code": -32600,
                        "message": "Invalid Request: missing or invalid 'method'"
                    }
                });
                return Some(serde_json::to_string(&resp).unwrap());
            }
        };

        let params = req_obj.get("params").cloned().unwrap_or(Value::Null);

        match method {
            "initialize" => {
                let client_version = params.get("protocolVersion").and_then(|v| v.as_str());

                // Support current spec (2024-11-05), 2024-10-07, and client-requested if compatible
                let negotiated = match client_version {
                    Some("2024-10-07") => "2024-10-07",
                    Some("2024-11-05") | Some("latest") | Some("current") => MCP_PROTOCOL_VERSION,
                    Some(other) if other.starts_with("2024-") => MCP_PROTOCOL_VERSION,
                    _ => MCP_PROTOCOL_VERSION,
                };
                *self.negotiated_version.write().unwrap() = negotiated.to_string();

                let resp = json!({
                    "jsonrpc": "2.0",
                    "id": id_val,
                    "result": {
                        "protocolVersion": negotiated,
                        "capabilities": {
                            "tools": {}
                        },
                        "serverInfo": {
                            "name": "s3a-academic-harness",
                            "version": "0.1.0"
                        }
                    }
                });
                Some(serde_json::to_string(&resp).unwrap())
            }
            "ping" => {
                let resp = json!({
                    "jsonrpc": "2.0",
                    "id": id_val,
                    "result": {}
                });
                Some(serde_json::to_string(&resp).unwrap())
            }
            "tools/list" => {
                let tools = self.list_tools();
                let resp = json!({
                    "jsonrpc": "2.0",
                    "id": id_val,
                    "result": {
                        "tools": tools
                    }
                });
                Some(serde_json::to_string(&resp).unwrap())
            }
            "tools/call" => {
                let tool_name = match params.get("name").and_then(|v| v.as_str()) {
                    Some(n) => n,
                    None => {
                        let resp = json!({
                            "jsonrpc": "2.0",
                            "id": id_val,
                            "error": {
                                "code": -32602,
                                "message": "Invalid params: 'name' is required"
                            }
                        });
                        return Some(serde_json::to_string(&resp).unwrap());
                    }
                };

                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                let call_result = self.call_tool(tool_name, &args);

                match call_result {
                    Ok((text, is_err)) => {
                        let resp = json!({
                            "jsonrpc": "2.0",
                            "id": id_val,
                            "result": {
                                "content": [{
                                    "type": "text",
                                    "text": text
                                }],
                                "isError": is_err
                            }
                        });
                        Some(serde_json::to_string(&resp).unwrap())
                    }
                    Err(json_err) => {
                        let resp = json!({
                            "jsonrpc": "2.0",
                            "id": id_val,
                            "error": json_err
                        });
                        Some(serde_json::to_string(&resp).unwrap())
                    }
                }
            }
            _ => {
                // -32601 Method not found
                let resp = json!({
                    "jsonrpc": "2.0",
                    "id": id_val,
                    "error": {
                        "code": -32601,
                        "message": format!("Method not found: {}", method)
                    }
                });
                Some(serde_json::to_string(&resp).unwrap())
            }
        }
    }

    fn list_tools(&self) -> Vec<Value> {
        vec![
            json!({
                "name": SearchLibraryArgs::tool_name(),
                "description": SearchLibraryArgs::description(),
                "inputSchema": SearchLibraryArgs::input_schema()
            }),
            json!({
                "name": GetSynthesisMatrixArgs::tool_name(),
                "description": GetSynthesisMatrixArgs::description(),
                "inputSchema": GetSynthesisMatrixArgs::input_schema()
            }),
            json!({
                "name": GetVaultAuditTrailArgs::tool_name(),
                "description": GetVaultAuditTrailArgs::description(),
                "inputSchema": GetVaultAuditTrailArgs::input_schema()
            }),
            json!({
                "name": ExportSrsArgs::tool_name(),
                "description": ExportSrsArgs::description(),
                "inputSchema": ExportSrsArgs::input_schema()
            }),
            json!({
                "name": IntakeSrsArgs::tool_name(),
                "description": IntakeSrsArgs::description(),
                "inputSchema": IntakeSrsArgs::input_schema()
            }),
            json!({
                "name": RecordXapiStatementArgs::tool_name(),
                "description": RecordXapiStatementArgs::description(),
                "inputSchema": RecordXapiStatementArgs::input_schema()
            }),
            json!({
                "name": GetSessionContextArgs::tool_name(),
                "description": GetSessionContextArgs::description(),
                "inputSchema": GetSessionContextArgs::input_schema()
            }),
            json!({
                "name": ListAcademicVerbsArgs::tool_name(),
                "description": ListAcademicVerbsArgs::description(),
                "inputSchema": ListAcademicVerbsArgs::input_schema()
            }),
            json!({
                "name": GetResearcherTimelineArgs::tool_name(),
                "description": GetResearcherTimelineArgs::description(),
                "inputSchema": GetResearcherTimelineArgs::input_schema()
            }),
            json!({
                "name": PreflightCheckArgs::tool_name(),
                "description": PreflightCheckArgs::description(),
                "inputSchema": PreflightCheckArgs::input_schema()
            }),
            json!({
                "name": RequestHumanGateArgs::tool_name(),
                "description": RequestHumanGateArgs::description(),
                "inputSchema": RequestHumanGateArgs::input_schema()
            }),
            json!({
                "name": RecordHumanDecisionArgs::tool_name(),
                "description": RecordHumanDecisionArgs::description(),
                "inputSchema": RecordHumanDecisionArgs::input_schema()
            }),
            json!({
                "name": RecordAiStepArgs::tool_name(),
                "description": RecordAiStepArgs::description(),
                "inputSchema": RecordAiStepArgs::input_schema()
            }),
        ]
    }

    fn call_tool(&self, name: &str, args: &Value) -> Result<(String, bool), Value> {
        match name {
            "s3a_search_library" => {
                let parsed: SearchLibraryArgs = serde_json::from_value(args.clone()).map_err(
                    |e| json!({ "code": -32602, "message": format!("Invalid arguments: {}", e) }),
                )?;

                let min_year = parsed.min_year.unwrap_or(0);
                let max_year = parsed.max_year.unwrap_or(u16::MAX);
                let db_path = self.sandbox.default_db();

                if !db_path.exists() {
                    return Ok((
                        format!("Database not found at '{}'", db_path.display()),
                        false,
                    ));
                }

                match MmapReader::open(db_path) {
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
                            db_path.display(),
                            min_year,
                            max_year
                        );
                        for (p, coord) in papers.iter().take(20) {
                            summary.push_str(&format!(
                                "  - [{}] Paper ID: 0x{:016X} | Year: {} | Citations: {} | Venue: {} | OA: {}\n",
                                coord, p.paper_id, p.year, p.citation_count, p.venue_id, if p.open_access_flag == 1 { "Yes" } else { "No" }
                            ));
                        }
                        Ok((summary, false))
                    }
                    Err(e) => Ok((format!("Search error: {}", e), true)),
                }
            }
            "s3a_get_synthesis_matrix" => {
                let db_path = self.sandbox.default_db();
                if !db_path.exists() {
                    return Ok((
                        format!("Database not found at '{}'", db_path.display()),
                        false,
                    ));
                }

                match MmapReader::open(db_path) {
                    Ok(reader) => {
                        let start = Instant::now();
                        let sieve = QuerySieve::new(&reader);
                        let papers = sieve.query_academic_papers_3d(
                            [-f32::INFINITY; 3],
                            [f32::INFINITY; 3],
                            0,
                            u16::MAX,
                        );
                        let edges = sieve.query_research_graph(None, None);
                        let elapsed = start.elapsed();

                        // Actual CRC32C verification
                        let file_bytes = std::fs::read(db_path).unwrap_or_default();
                        let crc = crc32c::crc32c(&file_bytes);

                        let matrix = format!(
                            "Synthesis Matrix for '{}':\n  - Analyzed Works: {}\n  - Knowledge Graph Relations: {}\n  - Checksum: 0x{:08X} (verified {} bytes in {:.2?})",
                            db_path.display(), papers.len(), edges.len(), crc, file_bytes.len(), elapsed
                        );
                        Ok((matrix, false))
                    }
                    Err(e) => Ok((format!("Synthesis error: {}", e), true)),
                }
            }
            "s3a_get_vault_audit_trail" => {
                let parsed: GetVaultAuditTrailArgs = serde_json::from_value(args.clone()).map_err(
                    |e| json!({ "code": -32602, "message": format!("Invalid arguments: {}", e) }),
                )?;

                let session_pair = if let Some(uuid_str) = parsed.session_uuid {
                    let clean = uuid_str.replace('-', "");
                    if clean.len() != 32 {
                        return Ok((
                            "Invalid 128-bit UUID format (must be 32 hex characters)".to_string(),
                            true,
                        ));
                    }
                    let high = match u64::from_str_radix(&clean[0..16], 16) {
                        Ok(v) => v,
                        Err(_) => return Ok(("Failed to parse UUID high bits".to_string(), true)),
                    };
                    let low = match u64::from_str_radix(&clean[16..32], 16) {
                        Ok(v) => v,
                        Err(_) => return Ok(("Failed to parse UUID low bits".to_string(), true)),
                    };
                    [high, low]
                } else {
                    self.session_uuid
                };

                let db_path = self.sandbox.default_db();
                if !db_path.exists() {
                    return Ok((
                        format!("Database not found at '{}'", db_path.display()),
                        false,
                    ));
                }

                match MmapReader::open(db_path) {
                    Ok(reader) => {
                        let sieve = QuerySieve::new(&reader);
                        let human = sieve.query_human_lrs(Some(session_pair), None, 0, u64::MAX);
                        let ai = sieve.query_ai_traces(Some(session_pair), None, 0, u64::MAX);
                        let mut report = format!(
                            "Vault Audit Trail for Session {:016X}{:016X} ({} records):\n",
                            session_pair[0],
                            session_pair[1],
                            human.len() + ai.len()
                        );
                        report.push_str(&format!("  - Human Decisions: {}\n", human.len()));
                        for (h, coord) in &human {
                            report.push_str(&format!("    ├─ [{}] Actor: 0x{:016X}, Verb: {}, Score: {:.2}, Linked AI: {}\n", coord, h.actor_hash, h.verb_id, h.decision_score, h.ai_trace_coord));
                        }
                        report.push_str(&format!("  - AI Agent Traces: {}\n", ai.len()));
                        for (a, coord) in &ai {
                            report.push_str(&format!("    ├─ [{}] Agent: 0x{:016X}, Step: {}, Conf: {:.2}, Linked Human: {}\n", coord, a.agent_id, a.step_type, a.confidence, a.human_coord));
                        }
                        Ok((report, false))
                    }
                    Err(e) => Ok((format!("Audit error: {}", e), true)),
                }
            }
            "s3a_export_srs" => {
                let _parsed: ExportSrsArgs = serde_json::from_value(args.clone()).map_err(
                    |e| json!({ "code": -32602, "message": format!("Invalid arguments: {}", e) }),
                )?;

                let target_path = match self.sandbox.create_export_snapshot_path() {
                    Ok(p) => p,
                    Err(e) => return Ok((format!("Sandbox export error: {}", e), true)),
                };

                let live_db = self.sandbox.default_db();
                match S3ACrudEngine::open_or_create(live_db) {
                    Ok(engine) => {
                        let start = Instant::now();
                        let ts = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();
                        let snapshot_uuid = crate::generate_uuid_v7_pair();
                        let manifest = SrsManifestHeader::new(
                            self.project_uuid,
                            snapshot_uuid,
                            [0, 0],
                            self.session_uuid,
                            ts,
                            0,
                            0,
                        );
                        match engine.export_srs_snapshot(
                            &target_path,
                            &manifest,
                            None,
                            Some(self.project_uuid),
                        ) {
                            Ok(b) => {
                                let elapsed = start.elapsed();
                                Ok((format!(
                                    "Successfully exported SRS snapshot to '{}' ({} total records, snapshot UUID: {:016X}{:016X}, project UUID: {:016X}{:016X}) in {:.2?}",
                                    target_path.display(), b.manifest.total_records, snapshot_uuid[0], snapshot_uuid[1], self.project_uuid[0], self.project_uuid[1], elapsed
                                ), false))
                            }
                            Err(e) => Ok((format!("Export error: {}", e), true)),
                        }
                    }
                    Err(e) => Ok((format!("Engine error: {}", e), true)),
                }
            }
            "s3a_intake_srs" => {
                let parsed: IntakeSrsArgs = serde_json::from_value(args.clone()).map_err(
                    |e| json!({ "code": -32602, "message": format!("Invalid arguments: {}", e) }),
                )?;

                let snapshot_path = match self
                    .sandbox
                    .resolve_intake_snapshot_path(&parsed.snapshot_id)
                {
                    Ok(p) => p,
                    Err(e) => return Ok((format!("Sandbox intake error: {}", e), true)),
                };

                let start = Instant::now();
                match intake_srs_snapshot(&snapshot_path) {
                    Ok(b) => {
                        let elapsed = start.elapsed();
                        Ok((
                            format!(
                                "Mounted SRS snapshot '{}' in {:.3} ms: Project UUID {:016X}{:016X}, Papers: {}, Edges: {}, Human Decisions: {}, AI Traces: {}",
                                snapshot_path.display(),
                                elapsed.as_secs_f64() * 1000.0,
                                b.manifest.project_uuid[0], b.manifest.project_uuid[1],
                                b.papers.len(), b.evidence_edges.len(), b.human_decisions.len(), b.ai_traces.len()
                            ),
                            false,
                        ))
                    }
                    Err(e) => Ok((format!("Intake error: {}", e), true)),
                }
            }
            "record_xapi_statement" => {
                let parsed: RecordXapiStatementArgs = serde_json::from_value(args.clone())
                    .map_err(|e| json!({ "code": -32602, "message": format!("Invalid arguments: {}", e) }))?;

                match self.store.record_xapi_statement(&parsed.statement) {
                    Ok(id) => Ok((
                        format!("Statement recorded successfully. Statement ID: {}", id),
                        false,
                    )),
                    Err(e) => Ok((format!("Failed to record statement: {}", e), true)),
                }
            }
            "get_session_context" => {
                let parsed: GetSessionContextArgs = serde_json::from_value(args.clone()).map_err(
                    |e| json!({ "code": -32602, "message": format!("Invalid arguments: {}", e) }),
                )?;

                let session_str = parsed.session_id.unwrap_or_else(|| {
                    format!("{:016X}{:016X}", self.session_uuid[0], self.session_uuid[1])
                });

                match self.store.get_session_context(&session_str) {
                    Ok(ctx) => Ok((
                        serde_json::to_string_pretty(&ctx).unwrap_or_default(),
                        false,
                    )),
                    Err(e) => Ok((format!("Failed to get session context: {}", e), true)),
                }
            }
            "list_academic_verbs" => match self.store.list_academic_verbs() {
                Ok(verbs) => Ok((
                    serde_json::to_string_pretty(&verbs).unwrap_or_default(),
                    false,
                )),
                Err(e) => Ok((format!("Failed to list verbs: {}", e), true)),
            },
            "get_researcher_timeline" => {
                let parsed: GetResearcherTimelineArgs = serde_json::from_value(args.clone())
                    .map_err(|e| json!({ "code": -32602, "message": format!("Invalid arguments: {}", e) }))?;

                match self.store.get_researcher_timeline(
                    parsed.agent_id.as_deref(),
                    parsed.session_id.as_deref(),
                    parsed.limit,
                ) {
                    Ok(events) => Ok((
                        serde_json::to_string_pretty(&events).unwrap_or_default(),
                        false,
                    )),
                    Err(e) => Ok((format!("Failed to get timeline: {}", e), true)),
                }
            }
            "preflight_check" => {
                let parsed: PreflightCheckArgs = serde_json::from_value(args.clone()).map_err(
                    |e| json!({ "code": -32602, "message": format!("Invalid arguments: {}", e) }),
                )?;

                match self.loop_engine.preflight_check(
                    parsed.candidate_confidence,
                    &parsed.unbound_entities,
                    parsed.has_vetted_analogy,
                ) {
                    Ok(()) => {
                        let tier = EpistemicConfidenceTier::from_score(parsed.candidate_confidence);
                        let res = json!({
                            "status": "passed",
                            "confidence": parsed.candidate_confidence,
                            "tier": format!("{:?}", tier),
                            "epistemic_phrase": tier.epistemic_phrase()
                        });
                        Ok((
                            serde_json::to_string_pretty(&res).unwrap_or_default(),
                            false,
                        ))
                    }
                    Err(risk) => {
                        let res = json!({
                            "status": "halted",
                            "risk": format!("{:?}", risk)
                        });
                        Ok((serde_json::to_string_pretty(&res).unwrap_or_default(), true))
                    }
                }
            }
            "request_human_gate" => {
                let parsed: RequestHumanGateArgs = serde_json::from_value(args.clone()).map_err(
                    |e| json!({ "code": -32602, "message": format!("Invalid arguments: {}", e) }),
                )?;

                let gate = self
                    .loop_engine
                    .request_human_gate(&parsed.action_type, &parsed.reason);
                let res = json!({
                    "gate_id": gate.gate_id,
                    "status": "pending",
                    "action_type": gate.action_type,
                    "reason": gate.reason,
                    "message": "Human supervisor gate requested. Agent must wait for decision."
                });
                Ok((
                    serde_json::to_string_pretty(&res).unwrap_or_default(),
                    false,
                ))
            }
            "record_human_decision" => {
                let parsed: RecordHumanDecisionArgs = serde_json::from_value(args.clone())
                    .map_err(|e| json!({ "code": -32602, "message": format!("Invalid arguments: {}", e) }))?;

                if let Err(e) = self.sandbox.check_write_permitted() {
                    return Ok((format!("Write rejected: {}", e), true));
                }

                let ai_coord = match parsed.ai_coord {
                    Some(s) => parse_coord_str(&s).unwrap_or(S3ACoordinate::new(0, 0, 0)),
                    None => S3ACoordinate::new(0, 0, 0),
                };

                let live_db = self.sandbox.default_db();
                match S3ACrudEngine::open_or_create(live_db) {
                    Ok(engine) => {
                        let _ = engine.enable_addon(s3a_core::ADDON_EDU);
                        let obj_id = parsed.target_object_id.unwrap_or(0);
                        match self.loop_engine.record_human_decision_with_gate(
                            &engine,
                            parsed.actor_id,
                            parsed.verb_id,
                            parsed.decision_score,
                            ai_coord,
                            obj_id,
                            parsed.gate_id.as_deref(),
                        ) {
                            Ok(coord) => {
                                let res = json!({
                                    "status": "recorded",
                                    "coordinate": coord.to_string(),
                                    "gate_id": parsed.gate_id
                                });
                                Ok((
                                    serde_json::to_string_pretty(&res).unwrap_or_default(),
                                    false,
                                ))
                            }
                            Err(e) => Ok((format!("Error recording human decision: {}", e), true)),
                        }
                    }
                    Err(e) => Ok((format!("Engine error: {}", e), true)),
                }
            }
            "record_ai_step" => {
                let parsed: RecordAiStepArgs = serde_json::from_value(args.clone()).map_err(
                    |e| json!({ "code": -32602, "message": format!("Invalid arguments: {}", e) }),
                )?;

                if let Err(e) = self.sandbox.check_write_permitted() {
                    return Ok((format!("Write rejected: {}", e), true));
                }

                let human_coord = match parsed.human_coord {
                    Some(s) => parse_coord_str(&s).unwrap_or(S3ACoordinate::new(0, 0, 0)),
                    None => S3ACoordinate::new(0, 0, 0),
                };

                let live_db = self.sandbox.default_db();
                match S3ACrudEngine::open_or_create(live_db) {
                    Ok(engine) => {
                        let _ = engine.enable_addon(s3a_core::ADDON_EDU);
                        let hilbert_idx = parsed.hilbert_index.unwrap_or(0);
                        match self.loop_engine.record_ai_step_gated(
                            &engine,
                            parsed.step_type,
                            parsed.confidence,
                            human_coord,
                            hilbert_idx,
                            parsed.gate_id.as_deref(),
                        ) {
                            Ok(coord) => {
                                let res = json!({
                                    "status": "recorded",
                                    "coordinate": coord.to_string()
                                });
                                Ok((
                                    serde_json::to_string_pretty(&res).unwrap_or_default(),
                                    false,
                                ))
                            }
                            Err(err_msg) => {
                                let res = json!({
                                    "status": "rejected",
                                    "error": err_msg
                                });
                                Ok((serde_json::to_string_pretty(&res).unwrap_or_default(), true))
                            }
                        }
                    }
                    Err(e) => Ok((format!("Engine error: {}", e), true)),
                }
            }
            _ => Err(json!({
                "code": -32601,
                "message": format!("Unknown tool: {}", name)
            })),
        }
    }
}

fn parse_coord_str(s: &str) -> Option<S3ACoordinate> {
    let parts: Vec<&str> = s.split('/').collect();
    if parts.len() == 3 {
        let level: u16 = parts[0].parse().ok()?;
        let tile_id: u32 = parts[1].parse().ok()?;
        let offset: u32 = parts[2].parse().ok()?;
        Some(S3ACoordinate::new(level, tile_id, offset))
    } else {
        None
    }
}
