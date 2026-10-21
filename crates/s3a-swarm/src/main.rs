//! CLI Binary for `s3a-swarm` concurrency benchmark and human dispute queue.

use ed25519_dalek::SigningKey;
use rand_core::{OsRng, RngCore};
use s3a_swarm::run_swarm;
use s3a_vcs::records::{RESOLUTION_CHOICE_SOURCE, RESOLUTION_CHOICE_TARGET};
use s3a_vcs::VcsRepo;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print_usage();
        return Ok(());
    }

    match args[1].as_str() {
        "run" => {
            let mut agents: Option<usize> = None;
            let mut commits = 5;
            let mut shadow = false;
            let mut out_path = Some(PathBuf::from("demo/swarm_repo.s3a"));
            let mut results_path: Option<PathBuf> = None;
            let mut remote_url: Option<String> = None;

            let mut i = 2;
            while i < args.len() {
                if args[i] == "--remote" && i + 1 < args.len() {
                    remote_url = Some(args[i + 1].clone());
                    i += 2;
                } else if args[i] == "--agents" && i + 1 < args.len() {
                    agents = Some(args[i + 1].parse().unwrap_or(1000));
                    i += 2;
                } else if args[i] == "--commits" && i + 1 < args.len() {
                    commits = args[i + 1].parse().unwrap_or(5);
                    i += 2;
                } else if args[i] == "--shadow" {
                    shadow = true;
                    i += 1;
                } else if args[i] == "--out" && i + 1 < args.len() {
                    out_path = Some(PathBuf::from(&args[i + 1]));
                    i += 2;
                } else if args[i] == "--results" && i + 1 < args.len() {
                    results_path = Some(PathBuf::from(&args[i + 1]));
                    i += 2;
                } else {
                    i += 1;
                }
            }

            if let Some(remote) = remote_url {
                let actual_agents = agents.unwrap_or(200);
                let actual_results =
                    results_path.unwrap_or_else(|| PathBuf::from("demo/results-cloudflare.json"));
                println!(
                    ">>> Running remote swarm benchmark against Cloudflare Worker: {}",
                    remote
                );
                println!(
                    ">>> Agents: {}, Commits/agent: {}, Target results: {}",
                    actual_agents,
                    commits,
                    actual_results.display()
                );

                let metrics = s3a_swarm::run_remote_swarm(
                    &remote,
                    "s3a-swarm-main",
                    actual_agents,
                    commits,
                    50,
                )
                .await?;
                println!("\n=== CLOUDFLARE WORKER REMOTE BENCHMARK RESULTS ===");
                metrics.print_summary();
                metrics.save_to_file(&actual_results)?;
                println!(">>> Remote metrics saved to: {}", actual_results.display());
            } else {
                let actual_agents = agents.unwrap_or(1000);
                let actual_results =
                    results_path.unwrap_or_else(|| PathBuf::from("demo/results.json"));

                let (metrics, coordinator) =
                    run_swarm(actual_agents, commits, shadow, out_path).await?;
                println!("\n=== LOCAL IN-PROCESS BENCHMARK RESULTS ===");
                metrics.print_summary();
                metrics.save_to_file(&actual_results)?;
                println!(">>> Local metrics saved to: {}", actual_results.display());

                // Save any pending human conflicts to demo/pending_conflicts.json for CLI interaction
                let pending = coordinator.human_queue.list_pending();
                if !pending.is_empty() {
                    let conflicts_path = PathBuf::from("demo/pending_conflicts.json");
                    if let Some(parent) = conflicts_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let json = serde_json::to_string_pretty(&pending)?;
                    std::fs::write(&conflicts_path, json)?;
                    println!(
                        ">>> {} claim conflicts pending in human queue. Saved to: {}",
                        pending.len(),
                        conflicts_path.display()
                    );
                    println!(">>> Inspect with: `s3a-swarm human list`");
                }
            }
        }
        "human" => {
            if args.len() < 3 {
                println!("Usage: s3a-swarm human <list|resolve> ...");
                return Ok(());
            }
            match args[2].as_str() {
                "list" => {
                    let conflicts_path = PathBuf::from("demo/pending_conflicts.json");
                    if !conflicts_path.exists() {
                        println!(
                            "No pending conflicts file found at {}",
                            conflicts_path.display()
                        );
                        return Ok(());
                    }
                    let content = std::fs::read_to_string(&conflicts_path)?;
                    let list: Vec<s3a_swarm::AskHumanConflict> = serde_json::from_str(&content)?;
                    println!(
                        "=== PENDING HUMAN SUPERVISOR CONFLICTS ({}) ===",
                        list.len()
                    );
                    for c in &list {
                        println!("  Conflict ID:     {}", c.conflict_id);
                        println!("  Record ID:       {}", c.record_id);
                        println!("  Tile Type:       {} ({})", c.tile_type, c.type_name);
                        println!("  Version A (Tgt): {}", c.target_version);
                        println!("  Version B (Src): {}", c.source_version);
                        println!("  Author A:        {}", c.target_author);
                        println!("  Author B:        {}", c.source_author);
                        println!(
                            "  Status:          {}",
                            if c.resolved { "RESOLVED" } else { "PENDING" }
                        );
                        println!("------------------------------------------------------------");
                    }
                }
                "resolve" => {
                    if args.len() < 4 {
                        println!("Usage: s3a-swarm human resolve <conflict_id> --choice <target|source> [--rationale \"...\"]");
                        return Ok(());
                    }
                    let conflict_id = &args[3];
                    let mut choice_side = RESOLUTION_CHOICE_TARGET;
                    let mut rationale =
                        "Human supervisor accepted target version after review".to_string();

                    let mut i = 4;
                    while i < args.len() {
                        if args[i] == "--choice" && i + 1 < args.len() {
                            let val = args[i + 1].to_lowercase();
                            if val == "source" || val == "2" {
                                choice_side = RESOLUTION_CHOICE_SOURCE;
                            } else {
                                choice_side = RESOLUTION_CHOICE_TARGET;
                            }
                            i += 2;
                        } else if args[i] == "--rationale" && i + 1 < args.len() {
                            rationale = args[i + 1].clone();
                            i += 2;
                        } else {
                            i += 1;
                        }
                    }

                    let conflicts_path = PathBuf::from("demo/pending_conflicts.json");
                    if !conflicts_path.exists() {
                        eprintln!("No pending conflicts found at {}", conflicts_path.display());
                        return Ok(());
                    }
                    let content = std::fs::read_to_string(&conflicts_path)?;
                    let mut list: Vec<s3a_swarm::AskHumanConflict> =
                        serde_json::from_str(&content)?;

                    let mut seed = [0u8; 32];
                    OsRng.fill_bytes(&mut seed);
                    let human_key = SigningKey::from_bytes(&seed);

                    let mut found = false;
                    for c in &mut list {
                        if c.conflict_id == *conflict_id {
                            let resolution = s3a_vcs::records::VcsResolutionRecord::new_signed(
                                &c.record_id,
                                choice_side,
                                &rationale,
                                &human_key,
                            );
                            c.resolved = true;
                            c.choice = Some(choice_side);
                            c.rationale = Some(rationale.clone());
                            found = true;
                            println!(
                                "Conflict '{}' resolved with signed VcsResolutionRecord:",
                                conflict_id
                            );
                            println!("  Resolution ID: {}", hex::encode(resolution.resolution_id));
                            println!(
                                "  Chosen Side:   {}",
                                if choice_side == RESOLUTION_CHOICE_TARGET {
                                    "Target (1)"
                                } else {
                                    "Source (2)"
                                }
                            );
                            println!("  Rationale:     {}", rationale);
                            println!("  Signature Verified: {}", resolution.verify_signature());
                            break;
                        }
                    }

                    if !found {
                        eprintln!("Conflict ID '{}' not found in pending list", conflict_id);
                        return Ok(());
                    }

                    let updated_json = serde_json::to_string_pretty(&list)?;
                    std::fs::write(&conflicts_path, updated_json)?;
                    println!("Updated {}", conflicts_path.display());
                }
                _ => {
                    println!("Unknown human subcommand. Use 'list' or 'resolve'.");
                }
            }
        }
        "verify" => {
            if args.len() < 3 {
                println!("Usage: s3a-swarm verify <archive_path>");
                return Ok(());
            }
            let path = &args[2];
            let repo = VcsRepo::open(path)?;
            let commits = repo.list_commits()?;
            println!("Verifying {} commits in {}...", commits.len(), path);
            let mut valid = 0;
            for c in &commits {
                if c.verify_signature() {
                    valid += 1;
                } else {
                    eprintln!("Corrupt or forged commit: {}", hex::encode(c.commit_id));
                }
            }
            println!(
                "Verification result: {} / {} commits verified valid.",
                valid,
                commits.len()
            );
        }
        _ => print_usage(),
    }

    Ok(())
}

fn print_usage() {
    println!("S3A Swarm - Concurrency Benchmark & Human Supervisor CLI");
    println!("Usage:");
    println!("  s3a-swarm run [--remote <URL>] [--agents N] [--commits M] [--shadow] [--out <PATH>] [--results <PATH>]");
    println!("  s3a-swarm human list");
    println!(
        "  s3a-swarm human resolve <conflict_id> --choice <target|source> [--rationale \"...\"]"
    );
    println!("  s3a-swarm verify <archive_path>");
}
