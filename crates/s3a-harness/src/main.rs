use std::env;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use s3a_engine::S3ACrudEngine;
use s3a_harness::{
    EpistemicConfidenceTier, LocalStore, LrsStore, ResearchLoopEngine, S3AMcpDispatcher, Sandbox,
    SrsContainerManager, Store,
};
use s3a_pack_scholar::SrsWorkspaceRecord;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_usage();
        return;
    }

    match args[1].as_str() {
        "mcp" | "mcp-server" => {
            run_mcp_server(&args[2..]);
        }
        "run" => {
            run_agent_cli(&args[2..]);
        }
        "resume" => {
            resume_agent_cli(&args[2..]);
        }
        "shadow-report" => {
            run_shadow_report_cli(&args[2..]);
        }
        "doctor" => {
            run_doctor_cli(&args[2..]);
        }
        "diff" => {
            run_diff_cli(&args[2..]);
        }
        "export-trajectories" => {
            run_export_trajectories_cli(&args[2..]);
        }
        "eval" => {
            run_eval_cli(&args[2..]);
        }
        "package" | "export" => {
            run_package(&args[2..]);
        }
        "intake" | "mount" => {
            run_intake(&args[2..]);
        }
        "loop-demo" => {
            run_autonomous_loop_demo();
        }
        "--help" | "-h" | "help" => {
            print_usage();
        }
        _ => {
            eprintln!("Unknown command: '{}'", args[1]);
            print_usage();
        }
    }
}

fn print_usage() {
    println!("S3A Standalone Research Agent Harness & MCP Server");
    println!("Usage:");
    println!("  s3a-harness run [OPTIONS]             Run autonomous agent loop from config");
    println!("    --config <path>                     Path to agent TOML config (default: agents/scholar_reviewer.toml)");
    println!("    --root <dir>                        Sandbox root directory (default: current directory)");
    println!("    --goal <text>                       Research goal / task description");
    println!("    --base-snapshot <id>                Base sealed snapshot ID to fork this run branch from");
    println!("    --consent <purpose>                 Grant a data-use purpose for this run (repeatable, e.g. model_training). Default: none");
    println!(
        "    --mock                              Use internal MockModel for offline/test execution"
    );
    println!("    --shadow <model>                    Run candidate model in shadow mode concurrently (never acts)");
    println!("    --model-url <url>                   Base URL for OpenAI-compatible endpoint (default: http://localhost:11434/v1)");
    println!("    --db <filename>                     Local S3A database filename inside root");
    println!("    --lrs-endpoint <url>                Optional remote LRS HTTP endpoint");
    println!(
        "    --verbose, -v                       Enable detailed structured tracing diagnostics"
    );
    println!();
    println!("  s3a-harness doctor [OPTIONS]          Perform health and operability diagnostics");
    println!("    --config <path>                     Agent config to validate (default: agents/scholar_reviewer.toml)");
    println!("    --no-config                         Skip config file validation");
    println!("    --root <dir>                        Sandbox root directory");
    println!("    --mock                              Test against mock model backend");
    println!("    --model-url <url>                   Model backend URL to test");
    println!("    --db <filename>                     Local database file to check");
    println!("    --lrs-endpoint <url>                Optional LRS endpoint to check");
    println!("    --json                              Output diagnostic results as JSON");
    println!();
    println!(
        "  s3a-harness diff <runA> <runB> [OPT]  Compare two agent runs and snapshot branches"
    );
    println!("    --root <dir>                        Sandbox root directory");
    println!("    --json                              Output comparison diff report as JSON");
    println!();
    println!("  s3a-harness export-trajectories [OPT] Export consent-filtered trajectories for fine-tuning");
    println!("    --format <chatml|sharegpt>          Output formatting (default: chatml)");
    println!("    --root <dir>                        Sandbox root directory");
    println!("    --out <file>                        Output destination path (default: trajectories.jsonl)");
    println!("    --purpose <name>                    Required consent purpose (default: model_training)");
    println!();
    println!(
        "  s3a-harness eval [OPTIONS]            Run action-asserted benchmark evaluation suite"
    );
    println!("    --agent <id>                        Agent ID or config path to evaluate");
    println!("    --set <path>                        Path to evals/*.jsonl evaluation suite");
    println!("    --root <dir>                        Sandbox root directory");
    println!("    --mock                              Use internal mock model");
    println!("    --model-url <url>                   Model backend URL");
    println!("    --json                              Output evaluation results as JSON");
    println!();
    println!("  s3a-harness resume <run_id> [OPTIONS] Resume paused agent run after human review");
    println!("    --decision <text>                   Supervisor decision (default: Approved)");
    println!("    --root <dir>                        Sandbox root directory");
    println!();
    println!("  s3a-harness shadow-report [OPTIONS]   Compare shadow vs live model agreement, confidence, and outcomes");
    println!("    --run <run_id>                      Specific run ID to evaluate (default: all runs in sandbox)");
    println!("    --root <dir>                        Sandbox root directory");
    println!();
    println!("  s3a-harness mcp [OPTIONS]             Start Model Context Protocol (MCP) server over stdin/stdout");
    println!("    --root <dir>                        Sandbox root directory (default: current directory)");
    println!("    --read-only                         Enforce read-only mode (rejects writes)");
    println!("    --db <filename>                     Default S3A database filename inside root (default: scholar_lab.s3a)");
    println!("    --lrs-endpoint <url>                Optional remote LRS HTTP endpoint (e.g. http://127.0.0.1:8080)");
    println!("    --allow-insecure-http               Allow HTTP connection to non-localhost remote hosts");
    println!("    --lrs-user <username>               LRS Basic auth username");
    println!("    --lrs-pass <password>               LRS Basic auth password");
    println!("    --session-uuid <hex_uuid>           Current session UUID (32-char hex)");
    println!("    --project-uuid <hex_uuid>           Current project UUID (32-char hex)");
    println!("    --agent-id <u64>                    Task agent identifier (default: 42)");
    println!();
    println!("  s3a-harness package [OPTIONS] <live_db> <out_snapshot.s3a>");
    println!("    Package live research database into portable SRS snapshot");
    println!();
    println!("  s3a-harness intake <snapshot.s3a>     Mount and verify portable SRS snapshot");
    println!(
        "  s3a-harness loop-demo                 Run autonomous inquiry demo with confidence tiers"
    );
}

fn parse_hex_uuid(s: &str) -> Option<[u64; 2]> {
    let clean = s.replace('-', "");
    if clean.len() != 32 {
        return None;
    }
    let high = u64::from_str_radix(&clean[0..16], 16).ok()?;
    let low = u64::from_str_radix(&clean[16..32], 16).ok()?;
    Some([high, low])
}

fn generate_random_uuid_pair() -> [u64; 2] {
    s3a_harness::generate_uuid_v7_pair()
}

fn run_mcp_server(args: &[String]) {
    let mut root_dir = PathBuf::from(".");
    let mut read_only = false;
    let mut db_filename = "scholar_lab.s3a".to_string();
    let mut lrs_endpoint: Option<String> = None;
    let mut lrs_user: Option<String> = None;
    let mut lrs_pass: Option<String> = None;
    let mut allow_insecure_http = false;
    let mut session_uuid = generate_random_uuid_pair();
    let mut project_uuid = generate_random_uuid_pair();
    let mut agent_id = 42u64;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--root" => {
                if i + 1 < args.len() {
                    root_dir = PathBuf::from(&args[i + 1]);
                    i += 1;
                }
            }
            "--read-only" => {
                read_only = true;
            }
            "--allow-insecure-http" => {
                allow_insecure_http = true;
            }
            "--db" => {
                if i + 1 < args.len() {
                    db_filename = args[i + 1].clone();
                    i += 1;
                }
            }
            "--lrs-endpoint" => {
                if i + 1 < args.len() {
                    lrs_endpoint = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "--lrs-user" => {
                if i + 1 < args.len() {
                    lrs_user = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "--lrs-pass" => {
                if i + 1 < args.len() {
                    lrs_pass = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "--session-uuid" => {
                if i + 1 < args.len() {
                    if let Some(pair) = parse_hex_uuid(&args[i + 1]) {
                        session_uuid = pair;
                    }
                    i += 1;
                }
            }
            "--project-uuid" => {
                if i + 1 < args.len() {
                    if let Some(pair) = parse_hex_uuid(&args[i + 1]) {
                        project_uuid = pair;
                    }
                    i += 1;
                }
            }
            "--agent-id" => {
                if i + 1 < args.len() {
                    if let Ok(id) = args[i + 1].parse() {
                        agent_id = id;
                    }
                    i += 1;
                }
            }
            _ => {
                // If it's a positional argument not starting with --, treat as root or db
                if !args[i].starts_with("--") {
                    let path = Path::new(&args[i]);
                    if path.is_dir() {
                        root_dir = path.to_path_buf();
                    } else {
                        db_filename = args[i].clone();
                    }
                }
            }
        }
        i += 1;
    }

    let sandbox = match Sandbox::new(&root_dir, read_only) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[s3a-harness] Sandbox initialization error: {}", e);
            return;
        }
    };

    let store: Arc<dyn Store> = if let Some(ref ep) = lrs_endpoint {
        eprintln!("[s3a-harness] Configured LRS HTTP backend: {}", ep);
        Arc::new(LrsStore::with_options(
            ep,
            lrs_user,
            lrs_pass,
            read_only,
            allow_insecure_http,
        ))
    } else {
        let db_path = sandbox.root().join(&db_filename);
        eprintln!(
            "[s3a-harness] Configured Local S3A backend: {}",
            db_path.display()
        );
        Arc::new(LocalStore::new(&db_path, read_only))
    };

    let dispatcher = S3AMcpDispatcher::new(sandbox, store, project_uuid, session_uuid, agent_id);

    eprintln!(
        "[s3a-harness] MCP Server ready on stdin/stdout (Root: {}, ReadOnly: {}, Project: {:016X}{:016X})",
        root_dir.display(), read_only, project_uuid[0], project_uuid[1]
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
        if let Some(resp) = dispatcher.dispatch(trimmed) {
            let _ = stdout.write_all(resp.as_bytes());
            let _ = stdout.write_all(b"\n");
            let _ = stdout.flush();
        }
    }
}

fn run_package(args: &[String]) {
    let mut live_db: Option<String> = None;
    let mut out_snapshot: Option<String> = None;
    let mut session_uuid = generate_random_uuid_pair();
    let mut project_uuid = generate_random_uuid_pair();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--session-uuid" => {
                if i + 1 < args.len() {
                    if let Some(pair) = parse_hex_uuid(&args[i + 1]) {
                        session_uuid = pair;
                    }
                    i += 1;
                }
            }
            "--project-uuid" => {
                if i + 1 < args.len() {
                    if let Some(pair) = parse_hex_uuid(&args[i + 1]) {
                        project_uuid = pair;
                    }
                    i += 1;
                }
            }
            _ => {
                if !args[i].starts_with("--") {
                    if live_db.is_none() {
                        live_db = Some(args[i].clone());
                    } else if out_snapshot.is_none() {
                        out_snapshot = Some(args[i].clone());
                    }
                }
            }
        }
        i += 1;
    }

    let live_db = match live_db {
        Some(db) => db,
        None => {
            println!("Usage: s3a-harness package [OPTIONS] <live_db> <out_snapshot.s3a>");
            return;
        }
    };
    let snap_file = match out_snapshot {
        Some(f) => f,
        None => "project.snapshot.s3a".to_string(),
    };

    println!(
        "Packaging academic project '{}' into SRS snapshot '{}' (Project: {:016X}{:016X})...",
        live_db, snap_file, project_uuid[0], project_uuid[1]
    );

    let start = Instant::now();
    let ws = SrsWorkspaceRecord::new(project_uuid, 12345, 0b1, [0.5, 0.5, 0.5], 1);
    match SrsContainerManager::package_project(
        &live_db,
        &snap_file,
        project_uuid,
        session_uuid,
        [0, 0],
        Some(&ws),
    ) {
        Ok(bundle) => {
            let elapsed = start.elapsed();
            println!(
                "Packaging complete in {:.2?}: total records = {} (Project: {:016X}{:016X})",
                elapsed, bundle.manifest.total_records, project_uuid[0], project_uuid[1]
            );
        }
        Err(e) => eprintln!("Packaging error: {}", e),
    }
}

fn run_intake(args: &[String]) {
    if args.is_empty() {
        println!("Usage: s3a-harness intake <snapshot.s3a>");
        return;
    }

    let snap_file = &args[0];
    let start = Instant::now();
    match SrsContainerManager::mount_snapshot(snap_file) {
        Ok(b) => {
            let elapsed = start.elapsed();
            println!(
                "Mounted snapshot in {:.3} ms:",
                elapsed.as_secs_f64() * 1000.0
            );
            println!(
                "  Project UUID:     {:016X}{:016X}",
                b.manifest.project_uuid[0], b.manifest.project_uuid[1]
            );
            println!("  Papers:           {}", b.papers.len());
            println!("  Evidence Edges:   {}", b.evidence_edges.len());
            println!("  Human Decisions:  {}", b.human_decisions.len());
            println!("  AI Traces:        {}", b.ai_traces.len());
        }
        Err(e) => eprintln!("Intake error: {}", e),
    }
}

fn run_autonomous_loop_demo() {
    println!("=========================================================================");
    println!("        S3A AUTONOMOUS RESEARCH REASONING LOOP & HUMAN GATE DEMO        ");
    println!("=========================================================================");

    let temp_db = std::env::temp_dir().join(format!("s3a_loop_demo_{}.s3a", std::process::id()));
    let engine = S3ACrudEngine::open_or_create(&temp_db).unwrap();

    let session_uuid = generate_random_uuid_pair();
    let agent_id = 9001;
    let research_loop = ResearchLoopEngine::new(session_uuid, agent_id);

    println!("1. Formulating Research Inquiry...");
    let p_coord = research_loop
        .register_paper(&engine, 0xD0112345, [0.35, 0.42, 0.88], 45, 2025, 12, true)
        .unwrap();
    println!(
        "   ✓ Ingested candidate paper into 3D Hilbert space at {}",
        p_coord
    );

    println!("\n2. Executing AI Extraction Step (Confidence: 0.85 - High Tier)...");
    let tier = EpistemicConfidenceTier::from_score(0.85);
    println!("   Epistemic Phrasing: '{}'", tier.epistemic_phrase());
    let ai_coord = research_loop
        .record_ai_step(
            &engine,
            1,
            0.85,
            s3a_core::S3ACoordinate::new(0, 0, 0),
            1042,
        )
        .unwrap();
    println!("   ✓ AI step recorded at {}", ai_coord);

    println!("\n3. Testing Autonomous Gate Evaluation:");
    let requires_gate_high = research_loop.requires_human_gate(0.85, false);
    println!(
        "   - Gate required for confidence 0.85 (no conflict): {}",
        requires_gate_high
    );

    let requires_gate_low = research_loop.requires_human_gate(0.55, false);
    println!(
        "   - Gate required for confidence 0.55 (moderate):     {} (Human review triggered!)",
        requires_gate_low
    );

    println!("\n4. Human-In-The-Loop Oversight Intervention:");
    let h_coord = research_loop
        .record_human_decision(&engine, 0x00BE_EF42, 1, 1.0, ai_coord, 0xD0112345_u32)
        .unwrap();
    println!("   ✓ Human approval decision linked at {}", h_coord);

    println!("\n5. Knowledge Graph Relationship Construction:");
    let edge_coord = research_loop
        .link_evidence(&engine, 0xD0112345, 0xC1A19999, 1, 0.95)
        .unwrap();
    println!("   ✓ Evidence relationship committed at {}", edge_coord);

    let _ = std::fs::remove_file(&temp_db);
    println!("\n=========================================================================");
    println!("Loop Demo Passed: 100% Deterministic Dual-Actor Provenance Maintained!");
    println!("=========================================================================");
}

fn run_agent_cli(args: &[String]) {
    let mut config_path = PathBuf::from("agents/scholar_reviewer.toml");
    let mut root_dir = PathBuf::from(".");
    let mut task_goal = "Synthesize literature and perform preflight checks".to_string();
    let mut use_mock = false;
    let mut model_url = "http://localhost:11434/v1".to_string();
    let mut db_filename = "scholar_lab.s3a".to_string();
    let mut lrs_endpoint: Option<String> = None;
    let mut lrs_user: Option<String> = None;
    let mut lrs_pass: Option<String> = None;
    let mut session_uuid = generate_random_uuid_pair();
    let mut project_uuid = generate_random_uuid_pair();
    let mut base_snapshot_id: Option<String> = None;
    let mut verbose = false;
    let mut consent_purposes: Vec<String> = Vec::new();

    let mut shadow_model_name: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--config" => {
                if i + 1 < args.len() {
                    config_path = PathBuf::from(&args[i + 1]);
                    i += 1;
                }
            }
            "--root" => {
                if i + 1 < args.len() {
                    root_dir = PathBuf::from(&args[i + 1]);
                    i += 1;
                }
            }
            "--goal" => {
                if i + 1 < args.len() {
                    task_goal = args[i + 1].clone();
                    i += 1;
                }
            }
            "--base-snapshot" => {
                if i + 1 < args.len() {
                    base_snapshot_id = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "--consent" => {
                if i + 1 < args.len() {
                    consent_purposes.push(args[i + 1].clone());
                    i += 1;
                }
            }
            "--verbose" | "-v" => {
                verbose = true;
            }
            "--mock" => {
                use_mock = true;
            }
            "--shadow" => {
                if i + 1 < args.len() {
                    shadow_model_name = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "--model-url" => {
                if i + 1 < args.len() {
                    model_url = args[i + 1].clone();
                    i += 1;
                }
            }
            "--db" => {
                if i + 1 < args.len() {
                    db_filename = args[i + 1].clone();
                    i += 1;
                }
            }
            "--lrs-endpoint" => {
                if i + 1 < args.len() {
                    lrs_endpoint = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "--lrs-user" => {
                if i + 1 < args.len() {
                    lrs_user = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "--lrs-pass" => {
                if i + 1 < args.len() {
                    lrs_pass = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "--session-uuid" => {
                if i + 1 < args.len() {
                    if let Some(pair) = parse_hex_uuid(&args[i + 1]) {
                        session_uuid = pair;
                    }
                    i += 1;
                }
            }
            "--project-uuid" if i + 1 < args.len() => {
                if let Some(pair) = parse_hex_uuid(&args[i + 1]) {
                    project_uuid = pair;
                }
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }

    s3a_harness::init_logging(verbose);

    let config = match s3a_harness::AgentConfig::load_file(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "[s3a-harness] Configuration validation failed for '{}': {}",
                config_path.display(),
                e
            );
            return;
        }
    };

    let sandbox = match Sandbox::new(&root_dir, false) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[s3a-harness] Sandbox error: {}", e);
            return;
        }
    };

    let store: Arc<dyn Store> = if let Some(ref ep) = lrs_endpoint {
        Arc::new(LrsStore::with_options(ep, lrs_user, lrs_pass, false, true))
    } else {
        let db_path = sandbox.root().join(&db_filename);
        Arc::new(LocalStore::new(&db_path, false))
    };

    let (model, escalation_model): (
        Arc<dyn s3a_harness::Model>,
        Option<Arc<dyn s3a_harness::Model>>,
    ) = if use_mock {
        let m = Arc::new(
            s3a_harness::MockModel::new("mock-model")
                .with_default_response("{\"answer\": \"preflight_check\", \"confidence\": 0.95, \"rationale\": \"Proceeding to preflight check\"}")
        );
        (m, None)
    } else {
        let m = Arc::new(s3a_harness::OpenAiHttpAdapter::new(
            &model_url,
            &config.model,
        ));
        let esc = config.escalation_model.as_ref().map(|em| {
            let adapter: Arc<dyn s3a_harness::Model> =
                Arc::new(s3a_harness::OpenAiHttpAdapter::new(&model_url, em));
            adapter
        });
        (m, esc)
    };

    let mut runtime =
        s3a_harness::AgentRuntime::new(config, model, escalation_model, store, sandbox);
    if let Some(ref sm_name) = shadow_model_name {
        let sm: Arc<dyn s3a_harness::Model> = if use_mock {
            Arc::new(
                s3a_harness::MockModel::new(sm_name)
                    .with_default_response("{\"answer\": \"preflight_check\", \"confidence\": 0.90, \"rationale\": \"Shadow model evaluation\"}")
            )
        } else {
            Arc::new(s3a_harness::OpenAiHttpAdapter::new(&model_url, sm_name))
        };
        runtime = runtime.with_shadow_model(sm);
    }
    let mut state = s3a_harness::RunState::with_base_snapshot(
        &runtime.config.id,
        &task_goal,
        session_uuid,
        project_uuid,
        base_snapshot_id,
    );
    // Consent is empty unless the human running the CLI grants purposes explicitly.
    if !consent_purposes.is_empty() {
        state = state.with_consent_from(consent_purposes, "cli --consent");
    }

    println!("=========================================================================");
    println!("Starting Autonomous Agent Run: {}", state.run_id);
    println!("Agent ID: {} ({})", runtime.config.id, runtime.config.name);
    println!("Goal:     {}", task_goal);
    if let Some(ref snap) = state.base_snapshot_id {
        println!("Base Snap: {}", snap);
    }
    if let Some(ref sm) = shadow_model_name {
        println!(
            "Shadow:   {} (Candidate model running alongside live model; logs only, never acts)",
            sm
        );
    }
    println!("=========================================================================");

    match runtime.run(state) {
        Ok(final_state) => {
            println!("\nRun completed with status: {:?}", final_state.status);
            println!(
                "Turns: {}, Tool Calls: {}, Tokens: {}",
                final_state.turn_count, final_state.tool_call_count, final_state.tokens_used
            );
            if let s3a_harness::RunStatus::PausedWaitingHuman {
                ref gate_id,
                ref reason,
            } = final_state.status
            {
                println!("\n[!] RUN PAUSED FOR HUMAN SUPERVISION");
                println!("    Gate ID: {}", gate_id);
                println!("    Reason:  {}", reason);
                println!(
                    "    Resume with: s3a-harness resume {} --decision \"<your response>\"",
                    final_state.run_id
                );
            }
        }
        Err(e) => eprintln!("Agent execution error: {}", e),
    }
}

fn resume_agent_cli(args: &[String]) {
    if args.is_empty() {
        println!("Usage: s3a-harness resume <run_id> [--decision <text>] [--root <dir>]");
        return;
    }

    let run_id = &args[0];
    let mut decision_text = "Approved by human supervisor".to_string();
    let mut root_dir = PathBuf::from(".");
    let mut config_path = PathBuf::from("agents/scholar_reviewer.toml");
    let mut db_filename = "scholar_lab.s3a".to_string();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--decision" => {
                if i + 1 < args.len() {
                    decision_text = args[i + 1].clone();
                    i += 1;
                }
            }
            "--root" => {
                if i + 1 < args.len() {
                    root_dir = PathBuf::from(&args[i + 1]);
                    i += 1;
                }
            }
            "--config" => {
                if i + 1 < args.len() {
                    config_path = PathBuf::from(&args[i + 1]);
                    i += 1;
                }
            }
            "--db" if i + 1 < args.len() => {
                db_filename = args[i + 1].clone();
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }

    let config = s3a_harness::AgentConfig::load_file(&config_path).unwrap_or_else(|_| {
        s3a_harness::AgentConfig {
            id: "default_agent".to_string(),
            name: "Default Agent".to_string(),
            system_prompt: "Default prompt".to_string(),
            model: "mock".to_string(),
            escalation_model: None,
            limits: s3a_harness::RunLimits::default(),
            thresholds: s3a_harness::DecisionThresholds::default(),
            safe_defaults: s3a_harness::SafeDefaults::default(),
            allowed_tools: vec!["preflight_check".to_string(), "ask_human".to_string()],
            require_grant: false,
        }
    });

    let sandbox = match Sandbox::new(&root_dir, false) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[s3a-harness] Sandbox error: {}", e);
            return;
        }
    };

    let db_path = sandbox.root().join(&db_filename);
    let store: Arc<dyn Store> = Arc::new(LocalStore::new(&db_path, false));
    let model: Arc<dyn s3a_harness::Model> = Arc::new(
        s3a_harness::MockModel::new("mock-model")
            .with_default_response("{\"answer\": \"preflight_check\", \"confidence\": 0.95, \"rationale\": \"Resuming preflight check\"}")
    );

    let runtime = s3a_harness::AgentRuntime::new(config, model, None, store, sandbox);
    println!(
        "Resuming Run {} with supervisor decision: '{}'...",
        run_id, decision_text
    );

    match runtime.resume(run_id, Some(decision_text)) {
        Ok(resumed_state) => {
            println!("Run resumed. New status: {:?}", resumed_state.status);
            println!(
                "Turns: {}, Tool Calls: {}",
                resumed_state.turn_count, resumed_state.tool_call_count
            );
        }
        Err(e) => eprintln!("Failed to resume run: {}", e),
    }
}

fn run_shadow_report_cli(args: &[String]) {
    let mut root_dir = PathBuf::from(".");
    let mut run_id: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--root" => {
                if i + 1 < args.len() {
                    root_dir = PathBuf::from(&args[i + 1]);
                    i += 1;
                }
            }
            "--run" => {
                if i + 1 < args.len() {
                    run_id = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            _ => {
                if !args[i].starts_with("--") && run_id.is_none() {
                    run_id = Some(args[i].clone());
                }
            }
        }
        i += 1;
    }

    let sandbox = match Sandbox::new(&root_dir, true) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[s3a-harness] Sandbox error: {}", e);
            return;
        }
    };

    match s3a_harness::AgentRuntime::shadow_report(&sandbox, run_id.as_deref()) {
        Ok(report) => {
            println!("=========================================================================");
            println!("                S3A HARNESS SHADOW MODEL EVALUATION REPORT               ");
            println!("=========================================================================");
            println!(
                "Total Operational Decisions Evaluated: {}",
                report.total_decisions
            );
            println!(
                "Agreed Decisions:                      {} (Agreement: {:.2}%)",
                report.agreements, report.agreement_rate
            );
            println!(
                "Disagreed Decisions:                   {}",
                report.disagreements
            );
            println!(
                "Live Model Avg Confidence:             {:.4}",
                report.avg_live_confidence
            );
            println!(
                "Shadow Model Avg Confidence:           {:.4}",
                report.avg_shadow_confidence
            );
            println!(
                "Mean Confidence Delta (Shadow - Live): {:+.4}",
                report.mean_confidence_delta
            );
            println!("-------------------------------------------------------------------------");
            if report.promote_recommended {
                println!("PROMOTION RECOMMENDATION:              [PASS] Model meets threshold (>= 80.0%)");
            } else {
                println!("PROMOTION RECOMMENDATION:              [REJECT] Model below promotion threshold (>= 80.0%)");
            }
            println!("=========================================================================");
            if !report.details.is_empty() {
                println!("\nDetailed Decision Breakdown:");
                for (idx, d) in report.details.iter().enumerate() {
                    let status = if d.agreement { "[AGREE]" } else { "[DIVERGE]" };
                    println!(
                        "  #{}: {} {} -> Live: '{}' (conf: {:.2}) vs Shadow: '{}' (conf: {:.2})",
                        idx + 1,
                        status,
                        d.decision_type.as_str(),
                        d.live_action,
                        d.live_confidence,
                        d.shadow_action,
                        d.shadow_confidence
                    );
                }
            }
        }
        Err(e) => eprintln!("Failed to generate shadow report: {}", e),
    }
}

fn run_doctor_cli(args: &[String]) {
    let mut config_path: Option<PathBuf> = Some(PathBuf::from("agents/scholar_reviewer.toml"));
    let mut root_dir = PathBuf::from(".");
    let mut use_mock = false;
    let mut model_url = "http://localhost:11434/v1".to_string();
    let mut db_filename = "scholar_lab.s3a".to_string();
    let mut lrs_endpoint: Option<String> = None;
    let mut json_output = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--config" => {
                if i + 1 < args.len() {
                    config_path = Some(PathBuf::from(&args[i + 1]));
                    i += 1;
                }
            }
            "--no-config" => {
                config_path = None;
            }
            "--root" => {
                if i + 1 < args.len() {
                    root_dir = PathBuf::from(&args[i + 1]);
                    i += 1;
                }
            }
            "--mock" => {
                use_mock = true;
            }
            "--model-url" => {
                if i + 1 < args.len() {
                    model_url = args[i + 1].clone();
                    i += 1;
                }
            }
            "--db" => {
                if i + 1 < args.len() {
                    db_filename = args[i + 1].clone();
                    i += 1;
                }
            }
            "--lrs-endpoint" => {
                if i + 1 < args.len() {
                    lrs_endpoint = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "--json" => {
                json_output = true;
            }
            "--help" | "-h" => {
                println!("Usage: s3a-harness doctor [OPTIONS]");
                println!("Options:");
                println!("  --config <path>       Path to agent TOML config (default: agents/scholar_reviewer.toml)");
                println!("  --no-config           Skip config file validation");
                println!("  --root <dir>          Sandbox root directory");
                println!("  --model-url <url>     Model backend URL");
                println!("  --mock                Test against mock model");
                println!("  --db <filename>       Local S3A database filename");
                println!("  --lrs-endpoint <url>  LRS endpoint URL");
                println!("  --json                Emit output as JSON");
                return;
            }
            _ => {}
        }
        i += 1;
    }

    let sandbox = match Sandbox::new(&root_dir, false) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[s3a-harness doctor] Sandbox error: {}", e);
            return;
        }
    };

    let store: Arc<dyn Store> = if let Some(ref ep) = lrs_endpoint {
        Arc::new(LrsStore::with_options(ep, None, None, false, true))
    } else {
        let db_path = sandbox.root().join(&db_filename);
        Arc::new(LocalStore::new(&db_path, false))
    };

    let model: Arc<dyn s3a_harness::Model> = if use_mock {
        Arc::new(s3a_harness::MockModel::new("mock-model").with_default_response("pong"))
    } else {
        Arc::new(s3a_harness::OpenAiHttpAdapter::new(&model_url, "llama3"))
    };

    let report = s3a_harness::run_doctor(
        config_path.as_deref(),
        model.as_ref(),
        store.as_ref(),
        &sandbox,
    );

    if json_output {
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
        return;
    }

    println!("=========================================================================");
    println!("                     S3A HARNESS OPERABILITY DOCTOR                      ");
    println!("=========================================================================");
    for check in &report.checks {
        let mark = if check.passed { "[PASS]" } else { "[FAIL]" };
        println!("{} {}: {}", mark, check.name, check.message);
        if let Some(ref rem) = check.remediation {
            println!("       Remediation: {}", rem);
        }
    }
    println!("-------------------------------------------------------------------------");
    if report.all_healthy {
        println!("OVERALL STATUS: [HEALTHY] All systems operational.");
    } else {
        println!("OVERALL STATUS: [UNHEALTHY] One or more diagnostic checks failed.");
    }
    println!("=========================================================================");
}

fn run_diff_cli(args: &[String]) {
    let mut run_a_id: Option<String> = None;
    let mut run_b_id: Option<String> = None;
    let mut root_dir = PathBuf::from(".");
    let mut json_output = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--root" => {
                if i + 1 < args.len() {
                    root_dir = PathBuf::from(&args[i + 1]);
                    i += 1;
                }
            }
            "--json" => {
                json_output = true;
            }
            "--help" | "-h" => {
                println!("Usage: s3a-harness diff <runA> <runB> [OPTIONS]");
                println!("Options:");
                println!("  --root <dir>   Sandbox root directory");
                println!("  --json         Emit diff report as JSON");
                return;
            }
            _ => {
                if !args[i].starts_with("--") {
                    if run_a_id.is_none() {
                        run_a_id = Some(args[i].clone());
                    } else if run_b_id.is_none() {
                        run_b_id = Some(args[i].clone());
                    }
                }
            }
        }
        i += 1;
    }

    let (run_a, run_b) = match (run_a_id, run_b_id) {
        (Some(a), Some(b)) => (a, b),
        _ => {
            println!("Usage: s3a-harness diff <runA> <runB> [--root <dir>] [--json]");
            return;
        }
    };

    let sandbox = match Sandbox::new(&root_dir, true) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[s3a-harness diff] Sandbox error: {}", e);
            return;
        }
    };

    match s3a_harness::AgentRuntime::diff_saved_runs(&sandbox, &run_a, &run_b) {
        Ok(diff) => {
            if json_output {
                println!("{}", serde_json::to_string_pretty(&diff).unwrap());
                return;
            }

            println!("=========================================================================");
            println!("                  S3A HARNESS RUN & SNAPSHOT BRANCH DIFF                 ");
            println!("=========================================================================");
            println!("Run A:                {}", diff.run_a_id);
            println!("Run B:                {}", diff.run_b_id);
            println!(
                "Base Snapshot Match:  {}",
                if diff.base_snapshot_match {
                    "YES (Identical ancestor)"
                } else {
                    "NO (Divergent branches)"
                }
            );
            println!(
                "Base Snapshot A:      {}",
                diff.base_snapshot_a.as_deref().unwrap_or("None")
            );
            println!(
                "Base Snapshot B:      {}",
                diff.base_snapshot_b.as_deref().unwrap_or("None")
            );
            println!(
                "Branch Snapshot A:    {}",
                diff.forked_snapshot_a.as_deref().unwrap_or("None")
            );
            println!(
                "Branch Snapshot B:    {}",
                diff.forked_snapshot_b.as_deref().unwrap_or("None")
            );
            println!("-------------------------------------------------------------------------");
            println!(
                "Status A vs B:        {:?} vs {:?}",
                diff.status_a, diff.status_b
            );
            println!("Turns A vs B:         {} vs {}", diff.turns_a, diff.turns_b);
            println!(
                "Tool Calls A vs B:    {} vs {}",
                diff.tool_calls_a, diff.tool_calls_b
            );
            println!(
                "Tokens A vs B:        {} vs {}",
                diff.tokens_a, diff.tokens_b
            );
            if let Some(turn) = diff.tool_sequence_divergence_turn {
                println!("Tool Divergence Turn: Turn #{}", turn);
            } else {
                println!("Tool Divergence Turn: None (Identical execution paths)");
            }
            println!("Tools A Sequence:     {:?}", diff.tools_a);
            println!("Tools B Sequence:     {:?}", diff.tools_b);
            println!(
                "Provenance Discrepancies: {}",
                diff.tool_provenance_diff_count
            );
            println!("=========================================================================");
        }
        Err(e) => eprintln!("Failed to diff runs: {}", e),
    }
}

fn run_export_trajectories_cli(args: &[String]) {
    let mut format = s3a_harness::TrajectoryFormat::ChatMl;
    let mut root_dir = PathBuf::from(".");
    let mut out_file = PathBuf::from("trajectories.jsonl");
    let mut required_purpose = "model_training".to_string();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--format" => {
                if i + 1 < args.len() {
                    if let Some(fmt) = s3a_harness::TrajectoryFormat::parse(&args[i + 1]) {
                        format = fmt;
                    } else {
                        eprintln!(
                            "[s3a-harness] Unsupported trajectory format '{}'; choose chatml or sharegpt",
                            args[i + 1]
                        );
                        return;
                    }
                    i += 1;
                }
            }
            "--root" => {
                if i + 1 < args.len() {
                    root_dir = PathBuf::from(&args[i + 1]);
                    i += 1;
                }
            }
            "--out" => {
                if i + 1 < args.len() {
                    out_file = PathBuf::from(&args[i + 1]);
                    i += 1;
                }
            }
            "--purpose" => {
                if i + 1 < args.len() {
                    required_purpose = args[i + 1].clone();
                    i += 1;
                }
            }
            "--help" | "-h" => {
                println!("Usage: s3a-harness export-trajectories [OPTIONS]");
                println!("Options:");
                println!(
                    "  --format <chatml|sharegpt>  Target trajectory schema (default: chatml)"
                );
                println!("  --root <dir>                Sandbox root directory");
                println!("  --out <file>                Output path (default: trajectories.jsonl)");
                println!(
                    "  --purpose <purpose>         Required consent purpose (default: model_training)"
                );
                return;
            }
            _ => {}
        }
        i += 1;
    }

    let sandbox = match Sandbox::new(&root_dir, true) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[s3a-harness] Sandbox error: {}", e);
            return;
        }
    };

    println!(
        "Exporting trajectories from '{}'...",
        sandbox.root().display()
    );
    match s3a_harness::export_trajectories(&sandbox, format, &out_file, &required_purpose) {
        Ok(summary) => {
            println!("=========================================================================");
            println!("             S3A HARNESS TRAJECTORY EXPORT COMPLETE                      ");
            println!("=========================================================================");
            println!("Total Runs Scanned:        {}", summary.total_runs_found);
            println!(
                "Consent Accepted:          {} (Purpose: '{}')",
                summary.consent_accepted, required_purpose
            );
            println!("Consent Filtered (Skipped): {}", summary.consent_filtered);
            println!("Exported Trajectories:     {}", summary.exported_records);
            println!("Format:                    {}", summary.format.as_str());
            println!(
                "Output File:               {}",
                summary.output_file.display()
            );
            println!("=========================================================================");
        }
        Err(e) => eprintln!("Failed to export trajectories: {}", e),
    }
}

fn run_eval_cli(args: &[String]) {
    let mut agent_id = "scholar_reviewer".to_string();
    let mut config_path: Option<PathBuf> = None;
    let mut eval_set_path = PathBuf::from("evals/scholar_evals.jsonl");
    let mut root_dir = PathBuf::from(".");
    let mut use_mock = false;
    let mut model_url = "http://localhost:11434/v1".to_string();
    let mut db_filename = "scholar_lab.s3a".to_string();
    let mut json_output = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--agent" => {
                if i + 1 < args.len() {
                    agent_id = args[i + 1].clone();
                    if agent_id.ends_with(".toml") {
                        config_path = Some(PathBuf::from(&agent_id));
                    }
                    i += 1;
                }
            }
            "--set" => {
                if i + 1 < args.len() {
                    eval_set_path = PathBuf::from(&args[i + 1]);
                    i += 1;
                }
            }
            "--root" => {
                if i + 1 < args.len() {
                    root_dir = PathBuf::from(&args[i + 1]);
                    i += 1;
                }
            }
            "--mock" => {
                use_mock = true;
            }
            "--model-url" => {
                if i + 1 < args.len() {
                    model_url = args[i + 1].clone();
                    i += 1;
                }
            }
            "--db" => {
                if i + 1 < args.len() {
                    db_filename = args[i + 1].clone();
                    i += 1;
                }
            }
            "--json" => {
                json_output = true;
            }
            "--help" | "-h" => {
                println!("Usage: s3a-harness eval [OPTIONS]");
                println!("Options:");
                println!("  --agent <id|path>    Agent ID or config path to evaluate");
                println!("  --set <path>         Path to evals/*.jsonl evaluation suite");
                println!("  --root <dir>         Sandbox root directory");
                println!("  --mock               Use internal mock model");
                println!("  --model-url <url>    Model endpoint URL");
                println!("  --json               Output results as JSON");
                return;
            }
            _ => {}
        }
        i += 1;
    }

    let cfg_file = config_path.unwrap_or_else(|| {
        if Path::new(&agent_id).exists() {
            PathBuf::from(&agent_id)
        } else {
            PathBuf::from(format!("agents/{}.toml", agent_id))
        }
    });

    let config = match s3a_harness::AgentConfig::load_file(&cfg_file) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "[s3a-harness eval] Failed to load agent config '{}': {}",
                cfg_file.display(),
                e
            );
            return;
        }
    };

    let sandbox = match Sandbox::new(&root_dir, false) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[s3a-harness eval] Sandbox error: {}", e);
            return;
        }
    };

    let db_path = sandbox.root().join(&db_filename);
    let store: Arc<dyn Store> = Arc::new(LocalStore::new(&db_path, false));

    let model: Arc<dyn s3a_harness::Model> = if use_mock {
        Arc::new(s3a_harness::MockModel::new("mock-model").with_default_response(
            "{\"answer\": \"preflight_check\", \"confidence\": 0.95, \"rationale\": \"Proceeding to preflight check\"}",
        ))
    } else {
        Arc::new(s3a_harness::OpenAiHttpAdapter::new(
            &model_url,
            &config.model,
        ))
    };

    let runtime = s3a_harness::AgentRuntime::new(config, model, None, store, sandbox);

    println!(
        "Running evaluation suite '{}' for agent '{}'...",
        eval_set_path.display(),
        runtime.config.id
    );
    match s3a_harness::run_eval_set(&runtime, &eval_set_path) {
        Ok(results) => {
            if json_output {
                println!("{}", serde_json::to_string_pretty(&results).unwrap());
                return;
            }

            println!("=========================================================================");
            println!("                S3A HARNESS AGENT EVALUATION BENCHMARK                   ");
            println!("=========================================================================");
            println!("Agent ID:                  {}", results.agent_id);
            println!("Evaluation Set:            {}", results.eval_set_path);
            println!("Total Test Cases:          {}", results.total_cases);
            println!(
                "Passed Cases:              {} (Success Rate: {:.2}%)",
                results.passed_cases, results.success_rate
            );
            println!("Failed Cases:              {}", results.failed_cases);
            println!("Supervisory Gate Hits:     {}", results.gate_hits);
            println!("ask_human Invocations:     {}", results.ask_human_count);
            println!("-------------------------------------------------------------------------");
            println!("Detailed Case Breakdown:");
            for case in &results.cases {
                let status = if case.passed { "[PASS]" } else { "[FAIL]" };
                println!("  {} Case '{}': {}", status, case.id, case.goal);
                println!("     Tools Called:      {:?}", case.tools_called);
                println!(
                    "     Gate Hit:          {}, ask_human: {}",
                    case.gate_hit, case.ask_human_raised
                );
                if let Some(ref reason) = case.failure_reason {
                    println!("     Failure Reason:    {}", reason);
                }
            }
            println!("=========================================================================");
        }
        Err(e) => eprintln!("Evaluation error: {}", e),
    }
}
