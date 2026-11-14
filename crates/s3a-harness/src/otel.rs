//! OpenTelemetry (OTel) Export and Provenance Trace Synchronization.
//!
//! Exposes an optional OTLP/HTTP 1.0 JSON exporter (feature-gated under `otel`).
//! Emits spans for tool calls, jobs, grant checks, and proxy requests.
//!
//! Invariants:
//! - Spans must include `agent.id`, `job.id`, and `s3a.record_hash`.
//! - Secrets, API keys, passwords, and proxy-injected credentials NEVER enter spans.
//! - Signed S3A records remain the immutable source of truth; OTel is a copy.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum SpanKind {
    Internal = 1,
    Server = 2,
    Client = 3,
    Producer = 4,
    Consumer = 5,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpanStatus {
    Unset,
    Ok,
    Error { message: String },
}

/// Standard OpenTelemetry Span.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Span {
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: Option<String>,
    pub name: String,
    pub kind: SpanKind,
    pub start_time_unix_nano: u64,
    pub end_time_unix_nano: u64,
    pub status: SpanStatus,
    pub attributes: HashMap<String, String>,
}

impl Span {
    /// Creates a new span with required provenance attributes: agent id, job id, and S3A record hash.
    pub fn new(
        name: impl Into<String>,
        agent_id: impl Into<String>,
        job_id: impl Into<String>,
        record_hash: impl Into<String>,
    ) -> Self {
        let now_nano = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;

        let mut attributes = HashMap::new();
        attributes.insert("agent.id".to_string(), agent_id.into());
        attributes.insert("job.id".to_string(), job_id.into());
        attributes.insert("s3a.record_hash".to_string(), record_hash.into());

        Self {
            trace_id: generate_trace_id(),
            span_id: generate_span_id(),
            parent_span_id: None,
            name: name.into(),
            kind: SpanKind::Internal,
            start_time_unix_nano: now_nano,
            end_time_unix_nano: now_nano,
            status: SpanStatus::Ok,
            attributes,
        }
    }

    /// Adds a safe attribute, enforcing that secrets and credentials are never stored.
    pub fn with_attribute(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        let k = key.into();
        let v = value.into();
        let k_lower = k.to_lowercase();
        // Zero secret invariant: never store any secrets
        if !k_lower.contains("secret")
            && !k_lower.contains("token")
            && !k_lower.contains("password")
            && !k_lower.contains("api_key")
            && !k_lower.contains("credential")
            && !k_lower.contains("auth_header")
        {
            self.attributes.insert(k, v);
        }
        self
    }

    pub fn with_parent_span(mut self, parent_span_id: impl Into<String>) -> Self {
        self.parent_span_id = Some(parent_span_id.into());
        self
    }

    pub fn finish(&mut self) {
        let now_nano = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        self.end_time_unix_nano = now_nano;
    }

    pub fn agent_id(&self) -> Option<&str> {
        self.attributes.get("agent.id").map(|s| s.as_str())
    }

    pub fn job_id(&self) -> Option<&str> {
        self.attributes.get("job.id").map(|s| s.as_str())
    }

    pub fn record_hash(&self) -> Option<&str> {
        self.attributes.get("s3a.record_hash").map(|s| s.as_str())
    }
}

pub trait SpanExporter: Send + Sync {
    fn export(&self, spans: &[Span]) -> Result<(), String>;
}

/// In-memory exporter for testing and verification.
#[derive(Clone, Default)]
pub struct InMemorySpanExporter {
    spans: Arc<Mutex<Vec<Span>>>,
}

impl InMemorySpanExporter {
    pub fn new() -> Self {
        Self {
            spans: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn spans(&self) -> Vec<Span> {
        self.spans.lock().unwrap().clone()
    }

    pub fn clear(&self) {
        self.spans.lock().unwrap().clear();
    }

    pub fn find_by_record_hash(&self, hash: &str) -> Option<Span> {
        self.spans
            .lock()
            .unwrap()
            .iter()
            .find(|s| s.record_hash() == Some(hash))
            .cloned()
    }

    pub fn count(&self) -> usize {
        self.spans.lock().unwrap().len()
    }
}

impl SpanExporter for InMemorySpanExporter {
    fn export(&self, spans: &[Span]) -> Result<(), String> {
        self.spans.lock().unwrap().extend_from_slice(spans);
        Ok(())
    }
}

/// OTLP HTTP/JSON Exporter compliant with standard OpenTelemetry 1.0 specification.
#[derive(Clone)]
pub struct OtlpHttpExporter {
    endpoint: String,
    service_name: String,
    timeout: Duration,
}

impl OtlpHttpExporter {
    pub fn new(endpoint: &str, service_name: &str) -> Self {
        let endpoint = endpoint.trim().trim_end_matches('/').to_string();
        let target_url = if endpoint.ends_with("/v1/traces") {
            endpoint
        } else {
            format!("{}/v1/traces", endpoint)
        };
        Self {
            endpoint: target_url,
            service_name: service_name.to_string(),
            timeout: Duration::from_secs(10),
        }
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn to_otlp_json(&self, spans: &[Span]) -> serde_json::Value {
        let otlp_spans: Vec<serde_json::Value> = spans
            .iter()
            .map(|s| {
                let attrs: Vec<serde_json::Value> = s
                    .attributes
                    .iter()
                    .map(|(k, v)| {
                        json!({
                            "key": k,
                            "value": { "stringValue": v }
                        })
                    })
                    .collect();

                let mut span_val = json!({
                    "traceId": s.trace_id,
                    "spanId": s.span_id,
                    "name": s.name,
                    "kind": s.kind as u8,
                    "startTimeUnixNano": s.start_time_unix_nano.to_string(),
                    "endTimeUnixNano": s.end_time_unix_nano.to_string(),
                    "attributes": attrs,
                    "status": match &s.status {
                        SpanStatus::Ok => json!({ "code": 1 }),
                        SpanStatus::Error { message } => json!({ "code": 2, "message": message }),
                        SpanStatus::Unset => json!({ "code": 0 }),
                    }
                });

                if let Some(parent) = &s.parent_span_id {
                    span_val["parentSpanId"] = json!(parent);
                }

                span_val
            })
            .collect();

        json!({
            "resourceSpans": [
                {
                    "resource": {
                        "attributes": [
                            {
                                "key": "service.name",
                                "value": { "stringValue": self.service_name }
                            }
                        ]
                    },
                    "scopeSpans": [
                        {
                            "scope": {
                                "name": "s3a-harness",
                                "version": "0.1.0"
                            },
                            "spans": otlp_spans
                        }
                    ]
                }
            ]
        })
    }
}

impl SpanExporter for OtlpHttpExporter {
    fn export(&self, spans: &[Span]) -> Result<(), String> {
        if spans.is_empty() {
            return Ok(());
        }
        let payload = self.to_otlp_json(spans);
        let resp = ureq::post(&self.endpoint)
            .set("Content-Type", "application/json")
            .timeout(self.timeout)
            .send_string(&payload.to_string());

        match resp {
            Ok(_) => Ok(()),
            Err(ureq::Error::Status(code, r)) => {
                let err_text = r.into_string().unwrap_or_default();
                Err(format!("OTLP export HTTP {}: {}", code, err_text))
            }
            Err(e) => Err(format!("OTLP export connection failed: {}", e)),
        }
    }
}

static GLOBAL_EXPORTER: OnceLock<RwLock<Option<Arc<dyn SpanExporter>>>> = OnceLock::new();

pub fn set_global_exporter(exporter: Arc<dyn SpanExporter>) {
    let lock = GLOBAL_EXPORTER.get_or_init(|| RwLock::new(None));
    *lock.write().unwrap() = Some(exporter);
}

pub fn clear_global_exporter() {
    if let Some(lock) = GLOBAL_EXPORTER.get() {
        *lock.write().unwrap() = None;
    }
}

pub fn emit_span(span: Span) {
    if let Some(lock) = GLOBAL_EXPORTER.get() {
        if let Some(exporter) = lock.read().unwrap().as_ref() {
            let _ = exporter.export(&[span]);
        }
    }
}

// =============================================================================
// Cryptographically Signed S3A Audit Log (Source of Truth)
// =============================================================================

/// Canonical, Ed25519-signed record stored in S3A.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedRecord {
    pub record_id: [u8; 16],
    pub record_type: String, // "tool_call", "job", "grant_check", "proxy_request"
    pub agent_id: String,
    pub job_id: String,
    pub data: serde_json::Value,
    pub timestamp: u64,
    pub sha256_hash: String,
    #[serde(with = "hex_64")]
    pub signature: [u8; 64],
    #[serde(with = "hex_32")]
    pub verifying_key: [u8; 32],
}

impl SignedRecord {
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.record_id);
        out.extend_from_slice(self.record_type.as_bytes());
        out.extend_from_slice(self.agent_id.as_bytes());
        out.extend_from_slice(self.job_id.as_bytes());
        let data_bytes = serde_json::to_vec(&self.data).unwrap_or_default();
        out.extend_from_slice(&data_bytes);
        out.extend_from_slice(&self.timestamp.to_le_bytes());
        out
    }

    pub fn compute_hash(&self) -> String {
        let bytes = self.canonical_bytes();
        let digest = Sha256::digest(&bytes);
        hex::encode(digest)
    }

    pub fn verify(&self) -> bool {
        let Ok(vk) = VerifyingKey::from_bytes(&self.verifying_key) else {
            return false;
        };
        let sig = Signature::from_bytes(&self.signature);
        let bytes = self.canonical_bytes();
        vk.verify(&bytes, &sig).is_ok()
    }
}

/// Append-only signed audit log acting as the source of truth for all agent operations.
#[derive(Clone, Default)]
pub struct SignedAuditLog {
    records: Arc<Mutex<Vec<SignedRecord>>>,
    exporter: Option<Arc<dyn SpanExporter>>,
}

impl SignedAuditLog {
    pub fn new() -> Self {
        Self {
            records: Arc::new(Mutex::new(Vec::new())),
            exporter: None,
        }
    }

    pub fn with_exporter(exporter: Arc<dyn SpanExporter>) -> Self {
        Self {
            records: Arc::new(Mutex::new(Vec::new())),
            exporter: Some(exporter),
        }
    }

    pub fn records(&self) -> Vec<SignedRecord> {
        self.records.lock().unwrap().clone()
    }

    pub fn len(&self) -> usize {
        self.records.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.lock().unwrap().is_empty()
    }

    pub fn find_by_hash(&self, hash: &str) -> Option<SignedRecord> {
        self.records
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.sha256_hash == hash)
            .cloned()
    }

    pub fn contains_hash(&self, hash: &str) -> bool {
        self.records
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.sha256_hash == hash)
    }

    /// Records an authoritative operation into the signed log and emits the corresponding OTel span.
    pub fn record_and_emit(
        &self,
        record_type: &str,
        agent_id: &str,
        job_id: &str,
        data: serde_json::Value,
        signing_key: &SigningKey,
    ) -> (SignedRecord, Span) {
        let mut record_id = [0u8; 16];
        let now_nano = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        record_id[..8].copy_from_slice(&(now_nano as u64).to_be_bytes());
        record_id[8..].copy_from_slice(&((now_nano >> 64) as u64).to_be_bytes());

        let timestamp = (now_nano / 1_000_000_000) as u64;

        let mut record = SignedRecord {
            record_id,
            record_type: record_type.to_string(),
            agent_id: agent_id.to_string(),
            job_id: job_id.to_string(),
            data: data.clone(),
            timestamp,
            sha256_hash: String::new(),
            signature: [0u8; 64],
            verifying_key: signing_key.verifying_key().to_bytes(),
        };

        // Canonical hash
        let hash = record.compute_hash();
        record.sha256_hash = hash.clone();

        // Sign canonical bytes
        let bytes_to_sign = record.canonical_bytes();
        let sig = signing_key.sign(&bytes_to_sign);
        record.signature = sig.to_bytes();

        // Store authoritative signed record
        self.records.lock().unwrap().push(record.clone());

        // Emit corresponding OpenTelemetry Span (OTel is a copy, signed record is source of truth)
        let span_name = format!(
            "{}:{}",
            record_type,
            data.get("name").and_then(|v| v.as_str()).unwrap_or("op")
        );
        let mut span = Span::new(span_name, agent_id, job_id, &hash);

        // Safe attributes extraction (secrets filtered)
        if let Some(obj) = data.as_object() {
            for (k, v) in obj {
                if let Some(v_str) = v.as_str() {
                    span = span.with_attribute(k.as_str(), v_str);
                }
            }
        }

        if let Some(ref exp) = self.exporter {
            let _ = exp.export(&[span.clone()]);
        } else {
            emit_span(span.clone());
        }

        (record, span)
    }

    /// Verifies that every record in the signed log has a valid cryptographic signature.
    pub fn verify_all(&self) -> bool {
        let guard = self.records.lock().unwrap();
        guard.iter().all(|r| r.verify())
    }
}

fn generate_trace_id() -> String {
    let mut bytes = [0u8; 16];
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    bytes[..8].copy_from_slice(&(now as u64).to_be_bytes());
    bytes[8..].copy_from_slice(&((now >> 64) as u64).to_be_bytes());
    hex::encode(bytes)
}

fn generate_span_id() -> String {
    let mut bytes = [0u8; 8];
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let hash = Sha256::digest((now as u64).to_be_bytes());
    bytes.copy_from_slice(&hash[..8]);
    hex::encode(bytes)
}

mod hex_64 {
    use serde::{self, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &[u8; 64], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 64], D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let vec = hex::decode(&s).map_err(serde::de::Error::custom)?;
        if vec.len() != 64 {
            return Err(serde::de::Error::custom("expected 64 bytes for signature"));
        }
        let mut arr = [0u8; 64];
        arr.copy_from_slice(&vec);
        Ok(arr)
    }
}

mod hex_32 {
    use serde::{self, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &[u8; 32], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 32], D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let vec = hex::decode(&s).map_err(serde::de::Error::custom)?;
        if vec.len() != 32 {
            return Err(serde::de::Error::custom(
                "expected 32 bytes for verifying key",
            ));
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&vec);
        Ok(arr)
    }
}
