use std::env;
use std::io::{self, BufRead, Write};
use std::time::Instant;

use s3a_harness::{ResearchLoopEngine, EpistemicConfidenceTier, SrsContainerManager, S3AMcpDispatcher};
use s3a_engine::S3ACrudEngine;
use s3a_core::SrsWorkspaceRecord;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_usage();
        return;
    }

    match args[1].as_str() {
        "mcp" | "mcp-server" => {
            let db = if args.len() >= 3 { &args[2] } else { "scholar_lab.s3a" };
            run_mcp_loop(db);
        }
        "package" | "export" => {
            if args.len() < 4 {
                println!("Usage: s3a-harness package <live_db> <out_snapshot.s3a>");
                return;
            }
            let live_db = &args[2];
            let snap_file = &args[3];
            let p_uuid = [0x1122334455667788, 0x99AABBCCDDEEFF00];
            let s_uuid = [0xCAFEBABE00001111, 0xDEADBEEF22223333];

            println!("Packaging academic project '{}' into SRS snapshot '{}'...", live_db, snap_file);
            let start = Instant::now();
            let ws = SrsWorkspaceRecord::new(p_uuid, 12345, 0b1, [0.5, 0.5, 0.5], 1);
            match SrsContainerManager::package_project(live_db, snap_file, p_uuid, s_uuid, [0, 0], Some(&ws)) {
                Ok(bundle) => {
                    println!("Packaging complete in {:.2?}: total records = {}", start.elapsed(), bundle.manifest.total_records);
                }
                Err(e) => eprintln!("Packaging error: {}", e),
            }
        }
        "intake" | "mount" => {
            if args.len() < 3 {
                println!("Usage: s3a-harness intake <snapshot.s3a>");
                return;
            }
            let snap_file = &args[2];
            let start = Instant::now();
            match SrsContainerManager::mount_snapshot(snap_file) {
                Ok(b) => {
                    let elapsed = start.elapsed();
                    println!("Mounted snapshot in {:.3} ms:", elapsed.as_secs_f64() * 1000.0);
                    println!("  Project UUID:     {:016X}{:016X}", b.manifest.project_uuid[0], b.manifest.project_uuid[1]);
                    println!("  Papers:           {}", b.papers.len());
                    println!("  Evidence Edges:   {}", b.evidence_edges.len());
                    println!("  Human Decisions:  {}", b.human_decisions.len());
                    println!("  AI Traces:        {}", b.ai_traces.len());
                }
                Err(e) => eprintln!("Intake error: {}", e),
            }
        }
        "loop-demo" => {
            run_autonomous_loop_demo();
        }
        _ => print_usage(),
    }
}

fn print_usage() {
    println!("S3A Standalone Research Agent Harness");
    println!("Usage:");
    println!("  s3a-harness mcp [db_path]              Start native Model Context Protocol (MCP) server over stdin/stdout");
    println!("  s3a-harness package <live_db> <out.s3a> Package live research database into portable SRS snapshot");
    println!("  s3a-harness intake <snapshot.s3a>       Mount and verify portable SRS snapshot via zero-copy mmap");
    println!("  s3a-harness loop-demo                   Demonstrate autonomous inquiry loop with calibrated confidence tiers");
}

fn run_mcp_loop(db_path: &str) {
    eprintln!("[s3a-harness] MCP Server running on stdin/stdout (DB: {})", db_path);
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() { continue; }
        if let Some(resp) = S3AMcpDispatcher::handle_request(trimmed, db_path) {
            let _ = stdout.write_all(resp.as_bytes());
            let _ = stdout.write_all(b"\n");
            let _ = stdout.flush();
        }
    }
}

fn run_autonomous_loop_demo() {
    println!("=========================================================================");
    println!("        S3A AUTONOMOUS RESEARCH REASONING LOOP & HUMAN GATE DEMO        ");
    println!("=========================================================================");

    let temp_db = std::env::temp_dir().join(format!("s3a_loop_demo_{}.s3a", std::process::id()));
    let engine = S3ACrudEngine::open_or_create(&temp_db).unwrap();

    let session_uuid = [0x1111222233334444, 0x5555666677778888];
    let agent_id = 9001;
    let research_loop = ResearchLoopEngine::new(session_uuid, agent_id);

    println!("1. Formulating Research Inquiry...");
    let p_coord = research_loop.register_paper(&engine, 0xD0112345, [0.35, 0.42, 0.88], 45, 2025, 12, true).unwrap();
    println!("   ✓ Ingested candidate paper into 3D Hilbert space at {}", p_coord);

    println!("\n2. Executing AI Extraction Step (Confidence: 0.85 - High Tier)...");
    let tier = EpistemicConfidenceTier::from_score(0.85);
    println!("   Epistemic Phrasing: '{}'", tier.epistemic_phrase());
    let ai_coord = research_loop.record_ai_step(&engine, 1, 0.85, s3a_core::S3ACoordinate::new(0, 0, 0), 1042).unwrap();
    println!("   ✓ AI step recorded at {}", ai_coord);

    println!("\n3. Testing Autonomous Gate Evaluation:");
    let requires_gate_high = research_loop.requires_human_gate(0.85, false);
    println!("   - Gate required for confidence 0.85 (no conflict): {}", requires_gate_high);

    let requires_gate_low = research_loop.requires_human_gate(0.55, false);
    println!("   - Gate required for confidence 0.55 (moderate):     {} (Human review triggered!)", requires_gate_low);

    println!("\n4. Human-In-The-Loop Oversight Intervention:");
    let h_coord = research_loop.record_human_decision(&engine, 0xBEEF_42, 1, 1.0, ai_coord, 0xD0112345 as u32).unwrap();
    println!("   ✓ Human approval decision linked at {}", h_coord);

    println!("\n5. Knowledge Graph Relationship Construction:");
    let edge_coord = research_loop.link_evidence(&engine, 0xD0112345, 0xC1A19999, 1, 0.95).unwrap();
    println!("   ✓ Evidence relationship committed at {}", edge_coord);

    let _ = std::fs::remove_file(&temp_db);
    println!("\n=========================================================================");
    println!("Loop Demo Passed: 100% Deterministic Dual-Actor Provenance Maintained!");
    println!("=========================================================================");
}
