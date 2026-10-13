//! Metrics calculation and export to demo/results.json.

use std::fs;
use std::path::Path;
use serde::{Deserialize, Serialize};

/// Repair loop metrics tracking autonomous self-healing.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RepairLoopMetrics {
    pub failures_detected: usize,
    pub auto_repaired: usize,
    pub escalated_to_human: usize,
    pub mean_attempts: f64,
    pub shadow_branches_evaluated: usize,
    pub promoted_after_green: usize,
}

/// High-throughput benchmark metrics for the swarm run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmMetrics {
    pub agents: usize,
    pub commits: usize,
    pub commits_per_sec: f64,
    pub p50_latency_ms: f64,
    pub p99_latency_ms: f64,
    pub auto_merged: usize,
    pub rejected_by_grant: usize,
    #[serde(default)]
    pub rejected_bad_signature: usize,
    pub sent_to_human: usize,
    pub final_seal_hash: String,
    pub verified_signatures: usize,
    pub shadow_mode: bool,
    pub total_duration_secs: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repair_loop: Option<RepairLoopMetrics>,
}

impl SwarmMetrics {
    pub fn compute(
        agents: usize,
        commits: usize,
        total_duration_secs: f64,
        latencies_us: Vec<u64>,
        auto_merged: usize,
        rejected_by_grant: usize,
        sent_to_human: usize,
        final_seal_hash: &str,
        verified_signatures: usize,
        shadow_mode: bool,
    ) -> Self {
        Self::compute_with_bad_sigs(
            agents,
            commits,
            total_duration_secs,
            latencies_us,
            auto_merged,
            rejected_by_grant,
            0,
            sent_to_human,
            final_seal_hash,
            verified_signatures,
            shadow_mode,
        )
    }

    pub fn compute_with_bad_sigs(
        agents: usize,
        commits: usize,
        total_duration_secs: f64,
        mut latencies_us: Vec<u64>,
        auto_merged: usize,
        rejected_by_grant: usize,
        rejected_bad_signature: usize,
        sent_to_human: usize,
        final_seal_hash: &str,
        verified_signatures: usize,
        shadow_mode: bool,
    ) -> Self {
        latencies_us.sort_unstable();
        let count = latencies_us.len();

        let p50_ms = if count > 0 {
            let idx = (count as f64 * 0.50) as usize;
            latencies_us[idx.min(count - 1)] as f64 / 1000.0
        } else {
            0.0
        };

        let p99_ms = if count > 0 {
            let idx = (count as f64 * 0.99) as usize;
            latencies_us[idx.min(count - 1)] as f64 / 1000.0
        } else {
            0.0
        };

        let commits_per_sec = if total_duration_secs > 0.0 {
            commits as f64 / total_duration_secs
        } else {
            0.0
        };

        Self {
            agents,
            commits,
            commits_per_sec,
            p50_latency_ms: p50_ms,
            p99_latency_ms: p99_ms,
            auto_merged,
            rejected_by_grant,
            rejected_bad_signature,
            sent_to_human,
            final_seal_hash: final_seal_hash.to_string(),
            verified_signatures,
            shadow_mode,
            total_duration_secs,
            repair_loop: None,
        }
    }

    /// Prints a clean summary table to stdout.
    pub fn print_summary(&self) {
        println!();
        println!("============================================================");
        println!("       S3A SWARM CONCURRENCY BENCHMARK RESULTS              ");
        println!("============================================================");
        println!("  Simulated Agents:       {:>12}", self.agents);
        println!("  Total Commits:          {:>12}", self.commits);
        println!("  Elapsed Duration:       {:>12.2} s", self.total_duration_secs);
        println!("  Throughput:             {:>12.2} commits/sec", self.commits_per_sec);
        println!("  p50 Push Latency:       {:>12.3} ms", self.p50_latency_ms);
        println!("  p99 Push Latency:       {:>12.3} ms", self.p99_latency_ms);
        println!("  Auto-Merged (Union/LWW):{:>12}", self.auto_merged);
        println!("  Rejected by Grant:      {:>12}", self.rejected_by_grant);
        println!("  Rejected (Bad Sig):     {:>12}", self.rejected_bad_signature);
        println!("  Sent to Human Queue:    {:>12}", self.sent_to_human);
        println!("  Verified Signatures:    {:>12}", self.verified_signatures);
        println!("  Shadow Mode Active:     {:>12}", self.shadow_mode);
        println!("  Final Root Seal Hash:   {}", self.final_seal_hash);
        println!("============================================================");
        println!();
    }

    /// Writes metrics as pretty-printed JSON to the specified destination path.
    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> std::io::Result<()> {
        let p = path.as_ref();
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::Other, e.to_string())
        })?;
        fs::write(p, json)?;
        Ok(())
    }
}
