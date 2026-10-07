//! Example: Autonomous Research Agent Session with Human-in-the-Loop Supervisory Gates
//!
//! Run with:
//! cargo run --example research_agent_session

use serde_json::json;

use s3a_core::ADDON_EDU;
use s3a_engine::S3ACrudEngine;
use s3a_harness::{LocalStore, ResearchLoopEngine, Sandbox, SrsContainerManager, Store};
use s3a_pack_scholar::SrsWorkspaceRecord;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=========================================================================");
    println!("     S3A RESEARCH AGENT HARNESS: AUTONOMOUS SESSION WITH SUPERVISORY GATE ");
    println!("=========================================================================");

    // 1. Initialize a sandboxed research workspace
    let temp_root = std::env::temp_dir().join(format!("s3a_agent_session_{}", std::process::id()));
    let sandbox = Sandbox::new(&temp_root, false)?;
    println!(
        "[1] Initialized Sandbox Root at: {}",
        sandbox.root().display()
    );
    println!(
        "    Exports directory:           {}",
        sandbox.exports_dir().display()
    );

    // 2. Open research database and enable EDU addon
    let db_path = sandbox.default_db();
    let engine = S3ACrudEngine::open_or_create(db_path)?;
    engine.enable_addon(ADDON_EDU)?;

    // 3. Initialize Research Loop Engine with dynamic session and project UUIDs
    let session_uuid = [0x5555_6666_7777_8888, 0x1111_2222_3333_4444];
    let project_uuid = [0xAAAA_BBBB_CCCC_DDDD, 0x0123_4567_89AB_CDEF];
    let agent_id = 9002;
    let loop_engine = ResearchLoopEngine::new(session_uuid, agent_id);
    let store = LocalStore::new(db_path, false);

    // 4. Register ingested paper into spatial 3D Hilbert topic space
    println!("\n[2] Registering candidate paper into 3D topic space...");
    let paper_id = 0x9876_5432_10FE_DCBA;
    let paper_coord = loop_engine.register_paper(
        &engine,
        paper_id,
        [0.45, 0.72, 0.18], // Topic embedding coordinates
        128,                // Citations
        2025,               // Year
        1,                  // Venue ID
        true,               // Open Access
    )?;
    println!(
        "    Paper 0x{:016X} registered at coordinate: {}",
        paper_id, paper_coord
    );

    // 5. Preflight Gate Check on speculative candidate draft
    println!("\n[3] Performing Preflight Hallucination Check on candidate draft...");
    let candidate_confidence = 0.55; // Below epistemic threshold 0.70
    let preflight_result = loop_engine.preflight_check(candidate_confidence, &[], true);

    match preflight_result {
        Ok(()) => println!("    Preflight passed unconditionally."),
        Err(risk) => {
            println!("    Preflight HALTED: Risk detected: {:?}", risk);
            println!("    Agent raises hand to supervisor and requests Human Gate...");

            // 6. Request Human Gate
            let gate = loop_engine.request_human_gate(
                "Literature Synthesizing Claim",
                "Confidence 0.55 is below threshold 0.70; requires peer review",
            );
            println!(
                "    Gate generated: {} (Status: {:?})",
                gate.gate_id, gate.status
            );

            // Attempting AI step before approval MUST FAIL
            let unapproved_res = loop_engine.record_ai_step_gated(
                &engine,
                1,
                candidate_confidence,
                s3a_core::S3ACoordinate::new(0, 0, 0),
                512,
                Some(&gate.gate_id),
            );
            assert!(unapproved_res.is_err(), "Unapproved step must be rejected!");
            println!(
                "    ✓ Confirmed: unapproved gated AI step was successfully blocked and logged."
            );

            // 7. Human Supervisor Reviews and Approves
            println!("\n[4] Supervisor examines literature and records approval decision...");
            let human_coord = loop_engine.record_human_decision_with_gate(
                &engine,
                0x05E8_7777, // Human Supervisor ID
                3,           // Verb: approved
                1.0,         // Decision score
                s3a_core::S3ACoordinate::new(0, 0, 0),
                paper_id as u32,
                Some(&gate.gate_id),
            )?;
            println!("    Human approval recorded at coordinate: {}", human_coord);

            // 8. Now AI step succeeds with approved gate
            println!("\n[5] Retrying AI step with approved gate clearance...");
            let ai_coord = loop_engine.record_ai_step_gated(
                &engine,
                1,
                candidate_confidence,
                human_coord,
                512,
                Some(&gate.gate_id),
            )?;
            println!(
                "    ✓ AI reasoning step recorded at coordinate: {}",
                ai_coord
            );
        }
    }

    // 9. Record xAPI Statement to Store
    println!("\n[6] Recording xAPI statement to LocalStore...");
    let session_str = format!("{:016X}{:016X}", session_uuid[0], session_uuid[1]);
    let xapi_stmt = json!({
        "actor": {
            "mbox": "mailto:lead-researcher@institute.edu",
            "name": "Dr. Lead Researcher"
        },
        "verb": {
            "id": "https://ashecribb.dev/xapi/verbs/reviewed"
        },
        "object": {
            "id": "https://doi.org/10.1000/182",
            "objectType": "Activity"
        },
        "context": {
            "registration": session_str
        }
    });

    let stmt_id = store.record_xapi_statement(&xapi_stmt)?;
    println!("    xAPI Statement committed: {}", stmt_id);

    // 10. Export immutable SRS Snapshot to Sandbox exports
    println!("\n[7] Exporting Scholar Research Snapshot to <root>/exports/...");
    let snapshot_path =
        sandbox.resolve_export_snapshot_path(Some("final_experiment.snapshot.s3a"))?;
    let ws = SrsWorkspaceRecord::new(project_uuid, 888, 1, [0.45, 0.72, 0.18], 1);
    let bundle = SrsContainerManager::package_project(
        db_path,
        &snapshot_path,
        project_uuid,
        [0x9999, 0x8888],
        session_uuid,
        Some(&ws),
    )?;
    println!(
        "    Exported SRS Snapshot to '{}' ({} total records)",
        snapshot_path.display(),
        bundle.manifest.total_records
    );

    // 11. Intake and verify the snapshot
    println!("\n[8] Zero-copy mounting exported snapshot...");
    let mounted = SrsContainerManager::mount_snapshot(&snapshot_path)?;
    println!("    Mounted Snapshot verified:");
    println!(
        "      Project UUID: {:016X}{:016X}",
        mounted.manifest.project_uuid[0], mounted.manifest.project_uuid[1]
    );
    println!("      Papers:       {}", mounted.papers.len());
    println!("      Decisions:    {}", mounted.human_decisions.len());
    println!("      AI Traces:    {}", mounted.ai_traces.len());

    // Clean up
    let _ = std::fs::remove_dir_all(&temp_root);
    println!("\n=========================================================================");
    println!("                     Session Completed Successfully!                     ");
    println!("=========================================================================");

    Ok(())
}
