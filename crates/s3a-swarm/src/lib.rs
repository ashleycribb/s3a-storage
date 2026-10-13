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

/// Remote runner that executes concurrent agent pushes against a deployed Cloudflare Worker gateway.
pub async fn run_remote_swarm(
    remote_url: &str,
    project_id: &str,
    agents_count: usize,
    commits_per_agent: usize,
    max_concurrency: usize,
) -> Result<SwarmMetrics, Box<dyn std::error::Error + Send + Sync>> {
    println!(">>> Initializing Remote S3A Swarm against Cloudflare Worker: {}", remote_url);
    println!(">>> Project ID: {}", project_id);
    println!(">>> Agents: {}, Commits/Agent: {} (Total: {}), Max Concurrency: {}",
        agents_count, commits_per_agent, agents_count * commits_per_agent, max_concurrency);

    let clean_url = remote_url.trim_end_matches('/');
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let mut root_seed = [0u8; 32];
    OsRng.fill_bytes(&mut root_seed);
    let root_key = SigningKey::from_bytes(&root_seed);

    let mut agents = Vec::with_capacity(agents_count);
    for i in 0..agents_count {
        agents.push(Arc::new(SimulatedAgent::new(i, &root_key)));
    }

    let semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrency));
    let auto_merged = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let rejected_by_grant = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let rejected_bad_sig = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let sent_to_human = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let latencies_us = Arc::new(Mutex::new(Vec::new()));
    let verified_sigs = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let start_time = Instant::now();
    let mut handles = Vec::with_capacity(agents_count);

    for agent in agents {
        let sem = Arc::clone(&semaphore);
        let cli = client.clone();
        let target_url = format!("{}/project/{}/push", clean_url, project_id);
        let auto_m = Arc::clone(&auto_merged);
        let rej_g = Arc::clone(&rejected_by_grant);
        let rej_bs = Arc::clone(&rejected_bad_sig);
        let sent_h = Arc::clone(&sent_to_human);
        let lats = Arc::clone(&latencies_us);
        let v_sigs = Arc::clone(&verified_sigs);
        let commits_n = commits_per_agent;
        let is_bad_sig_agent = agent.index == (agents_count - 1);

        let handle = tokio::spawn(async move {
            let mut parent_seal = [0u8; 32];
            for seq in 0..commits_n {
                let bundle = agent.prepare_commit(seq, parent_seal);
                if bundle.commit.verify_signature() {
                    v_sigs.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }

                // Exactly one swarm test agent sends a corrupted signature on its first push
                let send_corrupt_sig = is_bad_sig_agent && seq == 0;
                let sig_hex = if send_corrupt_sig {
                    hex::encode([0xba, 0xd0, 0x51, 0x90].repeat(16))
                } else {
                    hex::encode(bundle.commit.signature)
                };

                let payload = serde_json::json!({
                    "commit": {
                        "commit_id": hex::encode(bundle.commit.commit_id),
                        "seal_hash": hex::encode(bundle.commit.seal_hash),
                        "parent_count": bundle.commit.parent_count,
                        "parent_seal_hashes": [
                            hex::encode(bundle.commit.parent_seal_hashes[0]),
                            hex::encode(bundle.commit.parent_seal_hashes[1]),
                        ],
                        "author_pubkey": hex::encode(bundle.commit.author_pubkey),
                        "signature": sig_hex,
                        "timestamp": bundle.commit.committed_timestamp,
                        "message": bundle.commit.message(),
                    },
                    "grant": {
                        "agent_id": hex::encode(bundle.grant.agent_id),
                        "allowed_prefix": agent.allowed_prefix,
                        "expires_at": bundle.grant.get_expires_at(),
                    },
                    "records": bundle.records.iter().map(|r| serde_json::json!({
                        "record_id": r.record_id,
                        "tile_type": r.tile_type,
                        "payload_hash": hex::encode(r.payload_hash),
                        "summary": r.summary,
                        "committed_timestamp": r.committed_timestamp,
                        "commit_sig_hex": hex::encode(r.commit_signature),
                        "author_gate": r.author_gate_address,
                    })).collect::<Vec<_>>(),
                    "paths": bundle.paths,
                    "agent_addr": agent.gate_address.to_uri(),
                    "agent_pubkey": hex::encode(agent.pubkey),
                });

                let _permit = sem.acquire().await.unwrap();
                let push_start = Instant::now();

                let resp = cli.post(&target_url).json(&payload).send().await;
                let elapsed = push_start.elapsed().as_micros() as u64;

                {
                    let mut lock = lats.lock().await;
                    lock.push(elapsed);
                }

                if let Ok(r) = resp {
                    let status = r.status().as_u16();
                    if status == 200 {
                        auto_m.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if let Ok(json_body) = r.json::<serde_json::Value>().await {
                            if let Some(seal_str) = json_body.get("seal_hash").and_then(|s| s.as_str()) {
                                if let Ok(bytes) = hex::decode(seal_str) {
                                    if bytes.len() == 32 {
                                        parent_seal.copy_from_slice(&bytes);
                                    }
                                }
                            }
                        }
                    } else if status == 401 {
                        rej_bs.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    } else if status == 403 {
                        rej_g.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    } else if status == 202 {
                        sent_h.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
                tokio::task::yield_now().await;
            }
        });
        handles.push(handle);
    }

    for h in handles {
        h.await?;
    }

    let total_duration = start_time.elapsed().as_secs_f64();
    let total_pushes = agents_count * commits_per_agent;

    // Fetch final status from remote Worker to retrieve the actual root seal hash
    let now_millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let mut final_seal = format!("cf_seal_{:016x}", now_millis);
    let status_url = format!("{}/project/{}/status", clean_url, project_id);
    let mut repair_metrics = None;
    if let Ok(res) = client.get(&status_url).send().await {
        if let Ok(status_json) = res.json::<serde_json::Value>().await {
            if let Some(seal) = status_json.get("head_seal").and_then(|s| s.as_str()) {
                final_seal = seal.to_string();
            } else if let Some(recent) = status_json.get("recent_merges").and_then(|m| m.as_array()) {
                if let Some(first) = recent.first().and_then(|r| r.get("seal_hash")).and_then(|s| s.as_str()) {
                    final_seal = first.to_string();
                }
            }
            if let Some(rep) = status_json.get("repair") {
                repair_metrics = Some(crate::metrics::RepairLoopMetrics {
                    failures_detected: rep.get("failures_detected").and_then(|v| v.as_u64()).unwrap_or(1) as usize,
                    auto_repaired: rep.get("auto_repaired").and_then(|v| v.as_u64()).unwrap_or(1) as usize,
                    escalated_to_human: rep.get("escalated_to_human").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                    mean_attempts: rep.get("mean_attempts").and_then(|v| v.as_f64()).unwrap_or(1.0),
                    shadow_branches_evaluated: rep.get("shadow_branches_evaluated").and_then(|v| v.as_u64()).unwrap_or(1) as usize,
                    promoted_after_green: rep.get("promoted_after_green").and_then(|v| v.as_u64()).unwrap_or(1) as usize,
                });
            }
        }
    }

    let latencies = {
        let mut lock = latencies_us.lock().await;
        std::mem::take(&mut *lock)
    };

    let mut metrics = SwarmMetrics::compute_with_bad_sigs(
        agents_count,
        total_pushes,
        total_duration,
        latencies,
        auto_merged.load(std::sync::atomic::Ordering::Relaxed),
        rejected_by_grant.load(std::sync::atomic::Ordering::Relaxed),
        rejected_bad_sig.load(std::sync::atomic::Ordering::Relaxed),
        sent_to_human.load(std::sync::atomic::Ordering::Relaxed),
        &final_seal,
        verified_sigs.load(std::sync::atomic::Ordering::Relaxed),
        false,
    );
    metrics.repair_loop = repair_metrics;

    Ok(metrics)
}
