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
        "package" | "export" => {
            run_package(&args[2..]);
        }
        "intake" | "mount" => {
            run_intake(&args[2..]);
        }
        "loop-demo" => {
            run_autonomous_loop_demo();
        }
        _ => print_usage(),
    }
}

fn print_usage() {
    println!("S3A Standalone Research Agent Harness & MCP Server");
    println!("Usage:");
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
    let s = s3a_engine::lrs_store::generate_uuid_v4();
    s3a_engine::lrs_store::uuid_to_u64_pair(&s).unwrap_or([0x11223344, 0x55667788])
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
        Arc::new(LrsStore::with_options(ep, lrs_user, lrs_pass, read_only, allow_insecure_http))
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
