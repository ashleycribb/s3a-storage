//! Work-queue polling worker daemon that claims and executes jobs without inbound ports.

use std::io;
use std::thread::sleep;
use std::time::Duration;

use rand_core::{OsRng, RngCore};
use serde_json::Value;

use crate::queue::WorkQueue;
use crate::runner::execute_job_in_sandbox;

/// Outcome of a single job poll and execution cycle.
#[derive(Debug, Clone, PartialEq)]
pub enum JobOutcome {
    Completed { job_id: [u8; 16], output: Value },
    Failed { job_id: [u8; 16], error: String },
}

/// Autonomous sandbox worker that polls the vault work queue, claims jobs with leases,
/// executes them in isolated subprocess sandboxes, and writes signed results back.
pub struct SandboxWorker {
    pub worker_id: [u8; 16],
    pub queue: WorkQueue,
    pub lease_duration_secs: u64,
    pub supported_kinds: Option<Vec<String>>,
}

impl SandboxWorker {
    /// Creates a new worker with a fresh random worker identity and default 30-second lease.
    pub fn new(queue: WorkQueue) -> Self {
        let mut worker_id = [0u8; 16];
        OsRng.fill_bytes(&mut worker_id);

        Self {
            worker_id,
            queue,
            lease_duration_secs: 30,
            supported_kinds: None,
        }
    }

    pub fn with_worker_id(mut self, worker_id: [u8; 16]) -> Self {
        self.worker_id = worker_id;
        self
    }

    pub fn with_lease_duration(mut self, lease_duration_secs: u64) -> Self {
        self.lease_duration_secs = lease_duration_secs;
        self
    }

    pub fn with_supported_kinds(mut self, kinds: Vec<String>) -> Self {
        self.supported_kinds = Some(kinds);
        self
    }

    /// Performs one poll cycle: checks for claimable jobs, claims with lease, executes in sandbox,
    /// and records final status. Returns `None` if no eligible jobs are pending.
    pub fn poll_and_run_once(&self) -> io::Result<Option<JobOutcome>> {
        let supported_refs: Option<Vec<&str>> = self
            .supported_kinds
            .as_ref()
            .map(|kinds| kinds.iter().map(|s| s.as_str()).collect());

        let claimed_job = self.queue.claim_next(
            &self.worker_id,
            self.lease_duration_secs,
            supported_refs.as_deref(),
        )?;

        let job = match claimed_job {
            Some(j) => j,
            None => return Ok(None),
        };

        // Run claimed job in isolated sandbox
        match execute_job_in_sandbox(&job) {
            Ok(output) => {
                self.queue
                    .complete(&job.id, &self.worker_id, output.clone())?;
                Ok(Some(JobOutcome::Completed {
                    job_id: job.id,
                    output,
                }))
            }
            Err(e) => {
                self.queue.fail(&job.id, &self.worker_id, &e)?;
                Ok(Some(JobOutcome::Failed {
                    job_id: job.id,
                    error: e,
                }))
            }
        }
    }

    /// Continuous polling loop. Runs until `max_jobs` completed (if Some) or indefinitely.
    pub fn run_loop(&self, poll_interval_ms: u64, max_jobs: Option<usize>) -> io::Result<usize> {
        let mut completed_count = 0;
        let sleep_dur = Duration::from_millis(poll_interval_ms);

        loop {
            if let Some(limit) = max_jobs {
                if completed_count >= limit {
                    break;
                }
            }

            match self.poll_and_run_once() {
                Ok(Some(_outcome)) => {
                    completed_count += 1;
                }
                Ok(None) => {
                    sleep(sleep_dur);
                }
                Err(e) => {
                    eprintln!("Worker error during poll cycle: {}", e);
                    sleep(sleep_dur);
                }
            }
        }

        Ok(completed_count)
    }
}
