//! Work-queue binary records and signatures for S3A Vault (pack tile 88).

use bytemuck::{Pod, Zeroable};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const TILE_SANDBOX_WORK_QUEUE: u32 = 88;
pub const WORK_QUEUE_MAGIC: [u8; 4] = *b"S3WQ";

pub const RECORD_TYPE_JOB: u16 = 1;
pub const RECORD_TYPE_STATUS: u16 = 2;
pub const RECORD_TYPE_RESULT: u16 = 3;

/// Job lifecycle execution states.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobState {
    Pending = 0,
    Claimed = 1,
    Running = 2,
    Completed = 3,
    Failed = 4,
}

impl JobState {
    pub fn from_u16(val: u16) -> Option<Self> {
        match val {
            0 => Some(Self::Pending),
            1 => Some(Self::Claimed),
            2 => Some(Self::Running),
            3 => Some(Self::Completed),
            4 => Some(Self::Failed),
            _ => None,
        }
    }
}

/// 168-byte 8-byte aligned fixed record header for work queue records in hyper-tiles.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct WorkQueueRecordHeader {
    pub magic: [u8; 4],            // b"S3WQ"
    pub record_type: u16,          // 1 = Job, 2 = Status, 3 = Result
    pub flags: u16,                // Reserved / flags
    pub job_id: [u8; 16],          // Unique job identifier
    pub worker_id: [u8; 16],       // Worker identifier (0 for Job)
    pub requested_by: [u8; 16],    // Requesting identity/key
    pub grant_id: [u8; 16],        // Delegation grant identifier
    pub timestamp: [u8; 8],        // u64 le_bytes (created_at / claimed_at / completed_at)
    pub lease_expires_at: [u8; 8], // u64 le_bytes (for Status)
    pub retry_count: u32,          // Retry counter (for Status)
    pub payload_len: u32,          // Length of payload following header
    pub signature: [u8; 64],       // Ed25519 or HMAC/SHA-256 signature
    pub record_crc32: u32,         // CRC32-C of header (with record_crc32=0) + payload
    pub _reserved: [u8; 4],        // Padding to 8-byte boundary
}

impl WorkQueueRecordHeader {
    pub fn compute_crc(&self, payload: &[u8]) -> u32 {
        let mut copy = *self;
        copy.record_crc32 = 0;
        let mut crc = crc32c::crc32c(bytemuck::bytes_of(&copy));
        crc = crc32c::crc32c_append(crc, payload);
        crc
    }

    pub fn is_valid_crc(&self, payload: &[u8]) -> bool {
        self.compute_crc(payload) == self.record_crc32
    }
}

/// Variable-length Job record payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobPayload {
    pub kind: String,
    pub inputs: serde_json::Value,
}

/// Status record payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StatusPayload {
    pub state: JobState,
    pub error: Option<String>,
}

/// Result record payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResultPayload {
    pub success: bool,
    pub output: serde_json::Value,
}

/// High-level Job definition.
#[derive(Debug, Clone, PartialEq)]
pub struct JobRecord {
    pub id: [u8; 16],
    pub kind: String,
    pub inputs: serde_json::Value,
    pub requested_by: [u8; 16],
    pub grant: [u8; 16],
    pub created_at: u64,
    pub signature: [u8; 64],
}

/// High-level Status update.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusRecord {
    pub job_id: [u8; 16],
    pub state: JobState,
    pub worker_id: [u8; 16],
    pub claimed_at: u64,
    pub lease_expires_at: u64,
    pub retry_count: u32,
    pub error: Option<String>,
    pub signature: [u8; 64],
}

/// High-level Result payload.
#[derive(Debug, Clone, PartialEq)]
pub struct ResultRecord {
    pub job_id: [u8; 16],
    pub worker_id: [u8; 16],
    pub completed_at: u64,
    pub success: bool,
    pub output: serde_json::Value,
    pub signature: [u8; 64],
}

/// Enumeration of all work-queue record variants.
#[derive(Debug, Clone, PartialEq)]
pub enum WorkQueueRecord {
    Job(JobRecord),
    Status(StatusRecord),
    Result(ResultRecord),
}

/// Computes a deterministic ed25519-compatible signature or HMAC digest for testing & verification.
pub fn compute_record_signature(
    record_type: u16,
    job_id: &[u8; 16],
    payload: &[u8],
    key: &[u8],
) -> [u8; 64] {
    let mut sig = [0u8; 64];
    let mut hasher = Sha256::new();
    hasher.update(key);
    hasher.update(record_type.to_le_bytes());
    hasher.update(job_id);
    hasher.update(payload);
    let h1 = hasher.finalize();
    sig[0..32].copy_from_slice(&h1);

    let mut hasher2 = Sha256::new();
    hasher2.update(h1);
    hasher2.update(b"S3A_WORK_QUEUE_SIG_EXT");
    let h2 = hasher2.finalize();
    sig[32..64].copy_from_slice(&h2);
    sig
}

impl JobRecord {
    pub fn encode(&self) -> Vec<u8> {
        let payload = serde_json::to_vec(&JobPayload {
            kind: self.kind.clone(),
            inputs: self.inputs.clone(),
        })
        .unwrap_or_default();

        let mut header = WorkQueueRecordHeader {
            magic: WORK_QUEUE_MAGIC,
            record_type: RECORD_TYPE_JOB,
            flags: 0,
            job_id: self.id,
            worker_id: [0u8; 16],
            requested_by: self.requested_by,
            grant_id: self.grant,
            timestamp: self.created_at.to_le_bytes(),
            lease_expires_at: [0u8; 8],
            retry_count: 0,
            payload_len: payload.len() as u32,
            signature: self.signature,
            record_crc32: 0,
            _reserved: [0u8; 4],
        };
        header.record_crc32 = header.compute_crc(&payload);

        let mut out =
            Vec::with_capacity(std::mem::size_of::<WorkQueueRecordHeader>() + payload.len() + 8);
        out.extend_from_slice(bytemuck::bytes_of(&header));
        out.extend_from_slice(&payload);
        // Pad to 8-byte alignment
        let pad = (8 - (out.len() % 8)) % 8;
        out.resize(out.len() + pad, 0);
        out
    }
}

impl StatusRecord {
    pub fn encode(&self) -> Vec<u8> {
        let payload = serde_json::to_vec(&StatusPayload {
            state: self.state,
            error: self.error.clone(),
        })
        .unwrap_or_default();

        let mut header = WorkQueueRecordHeader {
            magic: WORK_QUEUE_MAGIC,
            record_type: RECORD_TYPE_STATUS,
            flags: 0,
            job_id: self.job_id,
            worker_id: self.worker_id,
            requested_by: [0u8; 16],
            grant_id: [0u8; 16],
            timestamp: self.claimed_at.to_le_bytes(),
            lease_expires_at: self.lease_expires_at.to_le_bytes(),
            retry_count: self.retry_count,
            payload_len: payload.len() as u32,
            signature: self.signature,
            record_crc32: 0,
            _reserved: [0u8; 4],
        };
        header.record_crc32 = header.compute_crc(&payload);

        let mut out =
            Vec::with_capacity(std::mem::size_of::<WorkQueueRecordHeader>() + payload.len() + 8);
        out.extend_from_slice(bytemuck::bytes_of(&header));
        out.extend_from_slice(&payload);
        let pad = (8 - (out.len() % 8)) % 8;
        out.resize(out.len() + pad, 0);
        out
    }
}

impl ResultRecord {
    pub fn encode(&self) -> Vec<u8> {
        let payload = serde_json::to_vec(&ResultPayload {
            success: self.success,
            output: self.output.clone(),
        })
        .unwrap_or_default();

        let mut header = WorkQueueRecordHeader {
            magic: WORK_QUEUE_MAGIC,
            record_type: RECORD_TYPE_RESULT,
            flags: 0,
            job_id: self.job_id,
            worker_id: self.worker_id,
            requested_by: [0u8; 16],
            grant_id: [0u8; 16],
            timestamp: self.completed_at.to_le_bytes(),
            lease_expires_at: [0u8; 8],
            retry_count: 0,
            payload_len: payload.len() as u32,
            signature: self.signature,
            record_crc32: 0,
            _reserved: [0u8; 4],
        };
        header.record_crc32 = header.compute_crc(&payload);

        let mut out =
            Vec::with_capacity(std::mem::size_of::<WorkQueueRecordHeader>() + payload.len() + 8);
        out.extend_from_slice(bytemuck::bytes_of(&header));
        out.extend_from_slice(&payload);
        let pad = (8 - (out.len() % 8)) % 8;
        out.resize(out.len() + pad, 0);
        out
    }
}

/// Decodes all work-queue records from raw tile payload bytes.
pub fn decode_work_queue_records(bytes: &[u8]) -> Vec<WorkQueueRecord> {
    let mut records = Vec::new();
    let mut offset = 0;
    let hdr_size = std::mem::size_of::<WorkQueueRecordHeader>();

    while offset + hdr_size <= bytes.len() {
        let hdr_bytes = &bytes[offset..offset + hdr_size];
        let header: WorkQueueRecordHeader = *bytemuck::from_bytes(hdr_bytes);

        if header.magic != WORK_QUEUE_MAGIC {
            break;
        }

        let payload_len = header.payload_len as usize;
        let payload_start = offset + hdr_size;
        let payload_end = payload_start + payload_len;

        if payload_end > bytes.len() {
            break;
        }

        let payload = &bytes[payload_start..payload_end];
        if !header.is_valid_crc(payload) {
            // CRC mismatch, stop parsing corrupted region
            break;
        }

        match header.record_type {
            RECORD_TYPE_JOB => {
                if let Ok(jp) = serde_json::from_slice::<JobPayload>(payload) {
                    records.push(WorkQueueRecord::Job(JobRecord {
                        id: header.job_id,
                        kind: jp.kind,
                        inputs: jp.inputs,
                        requested_by: header.requested_by,
                        grant: header.grant_id,
                        created_at: u64::from_le_bytes(header.timestamp),
                        signature: header.signature,
                    }));
                }
            }
            RECORD_TYPE_STATUS => {
                if let Ok(sp) = serde_json::from_slice::<StatusPayload>(payload) {
                    records.push(WorkQueueRecord::Status(StatusRecord {
                        job_id: header.job_id,
                        state: sp.state,
                        worker_id: header.worker_id,
                        claimed_at: u64::from_le_bytes(header.timestamp),
                        lease_expires_at: u64::from_le_bytes(header.lease_expires_at),
                        retry_count: header.retry_count,
                        error: sp.error,
                        signature: header.signature,
                    }));
                }
            }
            RECORD_TYPE_RESULT => {
                if let Ok(rp) = serde_json::from_slice::<ResultPayload>(payload) {
                    records.push(WorkQueueRecord::Result(ResultRecord {
                        job_id: header.job_id,
                        worker_id: header.worker_id,
                        completed_at: u64::from_le_bytes(header.timestamp),
                        success: rp.success,
                        output: rp.output,
                        signature: header.signature,
                    }));
                }
            }
            _ => {}
        }

        let raw_len = hdr_size + payload_len;
        let pad = (8 - (raw_len % 8)) % 8;
        offset += raw_len + pad;
    }

    records
}
