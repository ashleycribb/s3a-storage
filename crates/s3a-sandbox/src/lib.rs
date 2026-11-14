//! S3A Sandbox Pack: Work-queue sandboxes and isolated task execution.
//!
//! Provides vault-backed work queues, isolated subprocess sandboxes with scrubbed environments,
//! lease expiry retries, and high-performance workers requiring no inbound ports.

pub mod queue;
pub mod records;
pub mod runner;
pub mod worker;

pub use queue::{JobView, WorkQueue};
pub use records::{
    compute_record_signature, decode_work_queue_records, JobPayload, JobRecord, JobState,
    ResultPayload, ResultRecord, StatusPayload, StatusRecord, WorkQueueRecord,
    WorkQueueRecordHeader, RECORD_TYPE_JOB, RECORD_TYPE_RESULT, RECORD_TYPE_STATUS,
    TILE_SANDBOX_WORK_QUEUE, WORK_QUEUE_MAGIC,
};
pub use runner::{execute_job_in_sandbox, run_task, TaskRequest, TaskResponse};
pub use worker::{JobOutcome, SandboxWorker};
