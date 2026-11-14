//! OpenTelemetry (OTel) Export for S3A Sandbox.
//!
//! Provides lightweight, standard OTLP/HTTP 1.0 JSON span emission and in-memory test export.
//! Feature-gated under `otel` (off by default).

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
        if !k_lower.contains("secret")
            && !k_lower.contains("token")
            && !k_lower.contains("password")
            && !k_lower.contains("api_key")
            && !k_lower.contains("credential")
        {
            self.attributes.insert(k, v);
        }
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
                                "name": "s3a-sandbox",
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
