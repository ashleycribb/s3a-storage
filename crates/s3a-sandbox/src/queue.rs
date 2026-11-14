//! Work-queue state engine backed by S3A Vault hyper-tiles.

use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use fs2::FileExt;
use s3a_engine::{MmapReader, TileWriter, HYPER_TILE_PAYLOAD_SIZE};

use crate::records::{
    compute_record_signature, decode_work_queue_records, JobRecord, JobState, ResultRecord,
    StatusRecord, WorkQueueRecord, TILE_SANDBOX_WORK_QUEUE,
};

/// High-level aggregated view of a job in the queue.
#[derive(Debug, Clone, PartialEq)]
pub struct JobView {
    pub job: JobRecord,
    pub state: JobState,
    pub worker_id: [u8; 16],
    pub claimed_at: u64,
    pub lease_expires_at: u64,
    pub retry_count: u32,
    pub error: Option<String>,
    pub result: Option<ResultRecord>,
}

impl JobView {
    /// Returns true if the job is available to be claimed (pending, or claimed with expired lease).
    pub fn is_claimable(&self, now: u64) -> bool {
        match self.state {
            JobState::Pending => true,
            JobState::Claimed | JobState::Running => {
                self.lease_expires_at > 0 && self.lease_expires_at <= now
            }
            JobState::Completed | JobState::Failed => false,
        }
    }
}

/// Persistent work queue embedded in an S3A Vault.
#[derive(Debug, Clone)]
pub struct WorkQueue {
    vault_path: PathBuf,
}

impl WorkQueue {
    pub fn new<P: AsRef<Path>>(vault_path: P) -> Self {
        Self {
            vault_path: vault_path.as_ref().to_path_buf(),
        }
    }

    pub fn vault_path(&self) -> &Path {
        &self.vault_path
    }

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    fn acquire_lock(&self) -> io::Result<File> {
        let lock_path = self.vault_path.with_extension("lock");
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)?;
        file.lock_exclusive()?;
        Ok(file)
    }

    fn open_writer(&self) -> io::Result<TileWriter> {
        if self.vault_path.exists() && File::open(&self.vault_path)?.metadata()?.len() >= 512 {
            TileWriter::open_append(&self.vault_path)
        } else {
            TileWriter::create(&self.vault_path)
        }
    }

    /// Reads all work-queue records from the vault archive and builds aggregate views.
    pub fn read_jobs_map(&self) -> io::Result<HashMap<[u8; 16], JobView>> {
        if !self.vault_path.exists() || File::open(&self.vault_path)?.metadata()?.len() < 512 {
            return Ok(HashMap::new());
        }

        let reader = MmapReader::open(&self.vault_path)?;
        let tile_count = reader.tile_count() as usize;
        let mut jobs: HashMap<[u8; 16], JobView> = HashMap::new();

        for i in 0..tile_count {
            if let Some((hdr, payload)) = reader.get_tile(i) {
                if hdr.tile_type == TILE_SANDBOX_WORK_QUEUE {
                    let records = decode_work_queue_records(payload);
                    for r in records {
                        match r {
                            WorkQueueRecord::Job(job) => {
                                jobs.entry(job.id).or_insert_with(|| JobView {
                                    job: job.clone(),
                                    state: JobState::Pending,
                                    worker_id: [0u8; 16],
                                    claimed_at: 0,
                                    lease_expires_at: 0,
                                    retry_count: 0,
                                    error: None,
                                    result: None,
                                });
                            }
                            WorkQueueRecord::Status(status) => {
                                if let Some(view) = jobs.get_mut(&status.job_id) {
                                    view.state = status.state;
                                    view.worker_id = status.worker_id;
                                    view.claimed_at = status.claimed_at;
                                    view.lease_expires_at = status.lease_expires_at;
                                    view.retry_count = status.retry_count;
                                    if status.error.is_some() {
                                        view.error = status.error;
                                    }
                                }
                            }
                            WorkQueueRecord::Result(res) => {
                                if let Some(view) = jobs.get_mut(&res.job_id) {
                                    view.state = if res.success {
                                        JobState::Completed
                                    } else {
                                        JobState::Failed
                                    };
                                    view.result = Some(res);
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(jobs)
    }

    /// Enqueues a new Job record into the vault.
    pub fn enqueue(&self, job: &JobRecord) -> io::Result<[u8; 16]> {
        let _lock = self.acquire_lock()?;
        let mut writer = self.open_writer()?;
        let encoded = job.encode();
        writer.write_hyper_tile::<u8>(TILE_SANDBOX_WORK_QUEUE, &encoded, None, None)?;
        Ok(job.id)
    }

    /// Fetches the current state and result of a job.
    pub fn get_job(&self, job_id: &[u8; 16]) -> io::Result<Option<JobView>> {
        let jobs = self.read_jobs_map()?;
        Ok(jobs.get(job_id).cloned())
    }

    /// Lists all jobs in insertion/created order.
    pub fn list_jobs(&self) -> io::Result<Vec<JobView>> {
        let mut jobs: Vec<JobView> = self.read_jobs_map()?.into_values().collect();
        jobs.sort_by_key(|j| j.job.created_at);
        Ok(jobs)
    }

    /// Atomically claims the next eligible job under exclusive vault write lock.
    /// Prevents double-claim across competing concurrent workers and handles lease expiry retry.
    pub fn claim_next(
        &self,
        worker_id: &[u8; 16],
        lease_duration_secs: u64,
        supported_kinds: Option<&[&str]>,
    ) -> io::Result<Option<JobRecord>> {
        let now = Self::now_secs();

        // Acquire exclusive write lock by opening TileWriter
        let _lock = self.acquire_lock()?;
        let mut writer = self.open_writer()?;

        // Re-read current state from disk under lock
        let jobs = self.read_jobs_map()?;

        // Sort candidate jobs by created_at ascending (FIFO)
        let mut candidates: Vec<JobView> = jobs
            .into_values()
            .filter(|j| {
                if !j.is_claimable(now) {
                    return false;
                }
                if let Some(kinds) = supported_kinds {
                    kinds.contains(&j.job.kind.as_str())
                } else {
                    true
                }
            })
            .collect();
        candidates.sort_by_key(|j| j.job.created_at);

        if let Some(candidate) = candidates.first() {
            let lease_expires_at = now + lease_duration_secs;
            let retry_count = candidate.retry_count + 1;

            let mut status = StatusRecord {
                job_id: candidate.job.id,
                state: JobState::Claimed,
                worker_id: *worker_id,
                claimed_at: now,
                lease_expires_at,
                retry_count,
                error: None,
                signature: [0u8; 64],
            };
            status.signature = compute_record_signature(
                crate::records::RECORD_TYPE_STATUS,
                &candidate.job.id,
                &[],
                worker_id,
            );

            let encoded = status.encode();
            writer.write_hyper_tile::<u8>(TILE_SANDBOX_WORK_QUEUE, &encoded, None, None)?;

            return Ok(Some(candidate.job.clone()));
        }

        Ok(None)
    }

    /// Updates status to Running and extends lease if needed.
    pub fn heartbeat(
        &self,
        job_id: &[u8; 16],
        worker_id: &[u8; 16],
        lease_duration_secs: u64,
    ) -> io::Result<()> {
        let now = Self::now_secs();
        let _lock = self.acquire_lock()?;
        let mut writer = self.open_writer()?;
        let jobs = self.read_jobs_map()?;
        let prev_retry = jobs.get(job_id).map(|j| j.retry_count).unwrap_or(0);

        let mut status = StatusRecord {
            job_id: *job_id,
            state: JobState::Running,
            worker_id: *worker_id,
            claimed_at: now,
            lease_expires_at: now + lease_duration_secs,
            retry_count: prev_retry,
            error: None,
            signature: [0u8; 64],
        };
        status.signature =
            compute_record_signature(crate::records::RECORD_TYPE_STATUS, job_id, &[], worker_id);

        let encoded = status.encode();
        writer.write_hyper_tile::<u8>(TILE_SANDBOX_WORK_QUEUE, &encoded, None, None)?;
        Ok(())
    }

    /// Records job completion with output result and marks status Completed.
    pub fn complete(
        &self,
        job_id: &[u8; 16],
        worker_id: &[u8; 16],
        output: serde_json::Value,
    ) -> io::Result<()> {
        let now = Self::now_secs();
        let _lock = self.acquire_lock()?;
        let jobs = self.read_jobs_map()?;
        let prev_retry = jobs.get(job_id).map(|j| j.retry_count).unwrap_or(0);
        let mut writer = self.open_writer()?;

        let mut result = ResultRecord {
            job_id: *job_id,
            worker_id: *worker_id,
            completed_at: now,
            success: true,
            output,
            signature: [0u8; 64],
        };
        result.signature = compute_record_signature(
            crate::records::RECORD_TYPE_RESULT,
            job_id,
            &serde_json::to_vec(&result.output).unwrap_or_default(),
            worker_id,
        );

        let res_encoded = result.encode();
        let mut status = StatusRecord {
            job_id: *job_id,
            state: JobState::Completed,
            worker_id: *worker_id,
            claimed_at: now,
            lease_expires_at: 0,
            retry_count: prev_retry,
            error: None,
            signature: [0u8; 64],
        };
        status.signature =
            compute_record_signature(crate::records::RECORD_TYPE_STATUS, job_id, &[], worker_id);
        let stat_encoded = status.encode();

        let mut payload = Vec::new();
        payload.extend_from_slice(&res_encoded);
        payload.extend_from_slice(&stat_encoded);

        if payload.len() <= HYPER_TILE_PAYLOAD_SIZE {
            writer.write_hyper_tile::<u8>(TILE_SANDBOX_WORK_QUEUE, &payload, None, None)?;
        } else {
            writer.write_hyper_tile::<u8>(TILE_SANDBOX_WORK_QUEUE, &res_encoded, None, None)?;
            writer.write_hyper_tile::<u8>(TILE_SANDBOX_WORK_QUEUE, &stat_encoded, None, None)?;
        }

        Ok(())
    }

    /// Records job failure and marks status Failed.
    pub fn fail(&self, job_id: &[u8; 16], worker_id: &[u8; 16], error_msg: &str) -> io::Result<()> {
        let now = Self::now_secs();
        let _lock = self.acquire_lock()?;
        let jobs = self.read_jobs_map()?;
        let prev_retry = jobs.get(job_id).map(|j| j.retry_count).unwrap_or(0);
        let mut writer = self.open_writer()?;

        let mut result = ResultRecord {
            job_id: *job_id,
            worker_id: *worker_id,
            completed_at: now,
            success: false,
            output: serde_json::json!({ "error": error_msg }),
            signature: [0u8; 64],
        };
        result.signature = compute_record_signature(
            crate::records::RECORD_TYPE_RESULT,
            job_id,
            error_msg.as_bytes(),
            worker_id,
        );

        let mut status = StatusRecord {
            job_id: *job_id,
            state: JobState::Failed,
            worker_id: *worker_id,
            claimed_at: now,
            lease_expires_at: 0,
            retry_count: prev_retry,
            error: Some(error_msg.to_string()),
            signature: [0u8; 64],
        };
        status.signature =
            compute_record_signature(crate::records::RECORD_TYPE_STATUS, job_id, &[], worker_id);

        let mut payload = Vec::new();
        payload.extend_from_slice(&result.encode());
        payload.extend_from_slice(&status.encode());

        writer.write_hyper_tile::<u8>(TILE_SANDBOX_WORK_QUEUE, &payload, None, None)?;
        Ok(())
    }
}
