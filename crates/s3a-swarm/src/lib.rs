//! `s3a-swarm`: High-concurrency agent simulation and deterministic merge demo
//! for S3A Git-Style Version Control.

pub mod agent;
pub mod coordinator;
pub mod eval;
pub mod grant;
pub mod human_queue;
pub mod merge;
pub mod metrics;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use ed25519_dalek::SigningKey;
use rand_core::{OsRng, RngCore};
use tokio::sync::Mutex;

pub use agent::SimulatedAgent;
pub use coordinator::{PushResult, SharedRepoCoordinator};
pub use human_queue::{AskHumanConflict, AskHumanQueue};
pub use metrics::SwarmMetrics;

/// High-level runner to spawn N simulated agents and execute M concurrent commits each.
pub async fn run_swarm(
    agents_count: usize,
    commits_per_agent: usize,
    shadow_mode: bool,
    out_archive_path: Option<PathBuf>,
) -> Result<(SwarmMetrics, SharedRepoCoordinator), Box<dyn std::error::Error + Send + Sync>> {
    println!(">>> Initializing S3A Swarm: {} agents, {} commits each (total: {} commits)...",
        agents_count, commits_per_agent, agents_count * commits_per_agent);
    if shadow_mode {
        println!(">>> Mode: SHADOW ENABLED (commits quarantined on shadow branches until eval passes)");
    } else {
        println!(">>> Mode: DIRECT CONCURRENT PUSH (disjoint union + deterministic merge)");
    }

    // 1. Root authority keypair for issuing delegation grants
    let mut root_seed = [0u8; 32];
    OsRng.fill_bytes(&mut root_seed);
    let root_key = SigningKey::from_bytes(&root_seed);

    // 2. Initialize shared repository coordinator
    let coordinator = Arc::new(Mutex::new(SharedRepoCoordinator::new(shadow_mode)));

    // 3. Pre-create agents with vault identities and delegation grants
    let mut agents = Vec::with_capacity(agents_count);
    for i in 0..agents_count {
        agents.push(Arc::new(SimulatedAgent::new(i, &root_key)));
    }

    let start_time = Instant::now();

    // 4. Spawn concurrent tokio tasks
    let mut handles = Vec::with_capacity(agents_count);
    for agent in agents {
        let coord = Arc::clone(&coordinator);
        let commits_n = commits_per_agent;

        let handle = tokio::spawn(async move {
            let mut parent_seal = [0u8; 32];
            for seq in 0..commits_n {
                let bundle = agent.prepare_commit(seq, parent_seal);
                let allowed_prefix = agent.allowed_prefix.clone();

                let res = {
                    let mut lock = coord.lock().await;
                    lock.push(bundle, &allowed_prefix, seq as u32)
                };

                if let Ok(PushResult::Merged { seal_hash, .. }) = res {
                    parent_seal = seal_hash;
                }
                // Cooperative yield to maximize interleaving across tokio tasks
                tokio::task::yield_now().await;
            }
        });
        handles.push(handle);
    }

    // 5. Await all agent tasks
    for h in handles {
        h.await?;
    }

    let total_duration = start_time.elapsed().as_secs_f64();

    // 6. Finalize metrics & verify signatures
    let mut coord_final = Arc::try_unwrap(coordinator)
        .map_err(|_| "Failed to unwrap coordinator Arc")?
        .into_inner();

    println!(">>> Verifying all commit cryptographic signatures in repository history...");
    let (verified_sigs, total_commits) = coord_final.verify_all_commit_signatures();
    println!(">>> Verified {} / {} commit signatures successfully.", verified_sigs, total_commits);

    // 7. Finalize archive if path provided
    let final_seal_hash = if let Some(ref path) = out_archive_path {
        println!(">>> Sealing repository archive to {}...", path.display());
        match coord_final.finalize_archive(path) {
            Ok(hash) => hash,
            Err(e) => {
                eprintln!("Warning: could not seal archive to disk: {}", e);
                hex::encode(coord_final.current_seal)
            }
        }
    } else {
        hex::encode(coord_final.current_seal)
    };

    let total_pushes = agents_count * commits_per_agent;
    let latencies = std::mem::take(&mut coord_final.latencies_us);

    let metrics = SwarmMetrics::compute(
        agents_count,
        total_pushes,
        total_duration,
        latencies,
        coord_final.auto_merged,
        coord_final.rejected_by_grant,
        coord_final.sent_to_human,
        &final_seal_hash,
        verified_sigs,
        shadow_mode,
    );

    Ok((metrics, coord_final))
}
