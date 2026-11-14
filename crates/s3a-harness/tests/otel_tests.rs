//! Tests for OpenTelemetry (OTel) Export and S3A Signed Log Provenance Synchronization.
//!
//! Required by prompt 006:
//! - Spans must include agent id, job id, and the S3A record hash.
//! - Spans must NEVER include secrets or the proxy's injected credentials.
//! - Signed S3A records stay the immutable source of truth.
//! - Uses an in-memory exporter.
//! - Verifies that EVERY span's record hash matches a signed record in the log.

#[cfg(not(feature = "otel"))]
#[test]
#[allow(clippy::assertions_on_constants)]
fn test_otel_feature_disabled_by_default() {
    assert!(!cfg!(feature = "otel"));
}

#[cfg(feature = "otel")]
mod otel_tests {
    use ed25519_dalek::SigningKey;
    use rand_core::OsRng;
    use s3a_harness::otel::{
        clear_global_exporter, set_global_exporter, InMemorySpanExporter, OtlpHttpExporter,
        SignedAuditLog, Span,
    };
    use s3a_harness::proxy::{
        AuthType, CredentialProxy, HostAllowlist, MemorySecretStore, NamedConnection,
    };
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;

    #[test]
    fn test_every_span_record_hash_matches_signed_log() {
        clear_global_exporter();
        let exporter = Arc::new(InMemorySpanExporter::new());
        set_global_exporter(Arc::clone(&exporter) as _);

        let signed_log = SignedAuditLog::with_exporter(Arc::clone(&exporter) as _);
        let signing_key = SigningKey::generate(&mut OsRng);
        let agent_id = "agent-researcher-007";
        let job_id = "job-42-assembly-render";

        // 1. Record & emit a Tool Call
        let (tool_rec, tool_span) = signed_log.record_and_emit(
            "tool_call",
            agent_id,
            job_id,
            json!({
                "name": "s3a_write",
                "tool_name": "s3a_write",
                "target_tile": 23,
                "document_key": "design_spec_v1",
                "execution_time_ms": 12,
            }),
            &signing_key,
        );
        assert!(tool_rec.verify(), "Tool record signature must verify");
        assert_eq!(tool_span.record_hash(), Some(tool_rec.sha256_hash.as_str()));

        // 2. Record & emit a Job Execution
        let (job_rec, job_span) = signed_log.record_and_emit(
            "job",
            agent_id,
            job_id,
            json!({
                "name": "extract_pdf_text",
                "job_kind": "extract_pdf_text",
                "page_count": 4,
                "status": "success",
            }),
            &signing_key,
        );
        assert!(job_rec.verify(), "Job record signature must verify");
        assert_eq!(job_span.record_hash(), Some(job_rec.sha256_hash.as_str()));

        // 3. Record & emit a Grant Check
        let (grant_rec, grant_span) = signed_log.record_and_emit(
            "grant_check",
            agent_id,
            job_id,
            json!({
                "name": "check_delegation_grant",
                "grant_id": "0102030405060708090a0b0c0d0e0f10",
                "tool_name": "s3a_write",
                "allowed": true,
            }),
            &signing_key,
        );
        assert!(
            grant_rec.verify(),
            "Grant check record signature must verify"
        );
        assert_eq!(
            grant_span.record_hash(),
            Some(grant_rec.sha256_hash.as_str())
        );

        // 4. Record & emit a Proxy Request
        let (proxy_rec, proxy_span) = signed_log.record_and_emit(
            "proxy_request",
            agent_id,
            job_id,
            json!({
                "name": "plm_adapter",
                "connection": "plm_adapter",
                "host": "cad.plm.corp",
                "method": "GET",
                "decision": "Allowed",
            }),
            &signing_key,
        );
        assert!(proxy_rec.verify(), "Proxy record signature must verify");
        assert_eq!(
            proxy_span.record_hash(),
            Some(proxy_rec.sha256_hash.as_str())
        );

        // VERIFY: All spans in InMemorySpanExporter match a signed S3A record
        let exported_spans = exporter.spans();
        assert_eq!(
            exported_spans.len(),
            4,
            "Must have captured all 4 emitted spans"
        );

        for span in &exported_spans {
            // Provenance verification: agent.id and job.id must exist
            assert_eq!(
                span.agent_id(),
                Some(agent_id),
                "Span must contain valid agent id"
            );
            assert_eq!(
                span.job_id(),
                Some(job_id),
                "Span must contain valid job id"
            );

            // Record hash verification: s3a.record_hash must match a signed S3A record
            let record_hash = span
                .record_hash()
                .expect("Span must have 's3a.record_hash' attribute");
            assert!(
                !record_hash.is_empty(),
                "Record hash attribute cannot be empty"
            );

            // Look up the exact record in the signed log
            let matching_record = signed_log
                .find_by_hash(record_hash)
                .expect("Every span's record hash MUST match a record in the signed log!");

            // Verify the cryptographic integrity of the authoritative record
            assert!(
                matching_record.verify(),
                "Signed record in log must pass Ed25519 verification"
            );
            assert_eq!(matching_record.agent_id, agent_id);
            assert_eq!(matching_record.job_id, job_id);

            // Invariant verification: Spans must NEVER contain secrets or injected credentials
            for (k, v) in &span.attributes {
                let k_lower = k.to_lowercase();
                let v_lower = v.to_lowercase();
                assert!(
                    !k_lower.contains("secret")
                        && !k_lower.contains("token")
                        && !k_lower.contains("password")
                        && !k_lower.contains("api_key")
                        && !k_lower.contains("credential"),
                    "Span attribute key '{}' contains forbidden secret term!",
                    k
                );
                assert!(
                    !v_lower.contains("bearer")
                        && !v_lower.contains("super-secret")
                        && !v_lower.contains("private_key"),
                    "Span attribute value for '{}' contains forbidden credential data: {}",
                    k,
                    v
                );
            }
        }
    }

    #[test]
    fn test_proxy_egress_span_never_leaks_secrets_or_injected_credentials() {
        clear_global_exporter();
        let exporter = Arc::new(InMemorySpanExporter::new());
        set_global_exporter(Arc::clone(&exporter) as _);

        let allowlist = HostAllowlist::new(vec!["api.plm.corp".to_string()]);
        let secret_store = MemorySecretStore::new();
        secret_store.insert(
            "vault:plm_secret",
            "super-secret-injected-bearer-token-98765",
        );

        let proxy = CredentialProxy::new("agent_proxy_worker", allowlist, Arc::new(secret_store))
            .with_exporter(Arc::clone(&exporter) as _);

        proxy.register_connection(NamedConnection::new(
            "plm_service",
            "api.plm.corp",
            AuthType::Bearer,
            "vault:plm_secret",
        ));

        // Start local mock upstream server
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        // Register loopback override so test runs offline
        proxy.register_connection(
            NamedConnection::new(
                "plm_service",
                "api.plm.corp",
                AuthType::Bearer,
                "vault:plm_secret",
            )
            .with_upstream_url(format!("http://127.0.0.1:{}", port)),
        );

        let server_thread = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                use std::io::{Read, Write};
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let body = "{\"cad\": \"ok\"}";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });

        let req = s3a_harness::proxy::EgressRequest {
            connection: "plm_service".to_string(),
            path: "/cad/model_99".to_string(),
            method: "GET".to_string(),
            body: None,
            headers: HashMap::new(),
        };

        let resp = proxy.handle_request(&req).unwrap();
        assert_eq!(resp.status, 200);
        let _ = server_thread.join();

        // Verify span was emitted through global exporter
        let spans = exporter.spans();
        assert!(
            !spans.is_empty(),
            "Proxy request must have emitted an OpenTelemetry span"
        );

        let proxy_span = spans
            .iter()
            .find(|s| s.name == "proxy_request:plm_service")
            .expect("Must find 'proxy_request:plm_service' span");

        // Verify span attributes
        assert_eq!(proxy_span.agent_id(), Some("agent_proxy_worker"));
        let record_hash = proxy_span
            .record_hash()
            .expect("Proxy span must have s3a.record_hash");

        // Find corresponding signed audit record in proxy
        let audit_records = proxy.audit_records();
        let matching_audit = audit_records
            .iter()
            .find(|r| r.sha256_digest() == record_hash)
            .expect("Proxy span record hash must match an EgressAuditRecord in the proxy's log");

        assert!(
            matching_audit.verify(),
            "EgressAuditRecord must cryptographically verify"
        );
        assert_eq!(matching_audit.connection, "plm_service");
        assert_eq!(matching_audit.host, "api.plm.corp");

        // Assert zero secrets leaked into span
        for (k, v) in &proxy_span.attributes {
            assert!(
                !v.contains("super-secret-injected-bearer-token-98765"),
                "Secret token leaked into span attribute '{}': {}",
                k,
                v
            );
        }
    }

    #[test]
    fn test_blocked_proxy_request_span_matches_signed_record() {
        clear_global_exporter();
        let exporter = Arc::new(InMemorySpanExporter::new());
        set_global_exporter(Arc::clone(&exporter) as _);

        let allowlist = HostAllowlist::new(vec!["allowed.corp".to_string()]);
        let secret_store = MemorySecretStore::new();
        let proxy = CredentialProxy::new("agent_guarded", allowlist, Arc::new(secret_store))
            .with_exporter(Arc::clone(&exporter) as _);

        proxy.register_connection(NamedConnection::new(
            "blocked_conn",
            "forbidden-data-exfil.org",
            AuthType::Bearer,
            "none",
        ));

        let req = s3a_harness::proxy::EgressRequest {
            connection: "blocked_conn".to_string(),
            path: "/exfil".to_string(),
            method: "POST".to_string(),
            body: Some("payload".to_string()),
            headers: HashMap::new(),
        };

        let err = proxy.handle_request(&req);
        assert!(err.is_err(), "Blocked host must return error");

        // Verify that even for blocked hosts, a signed audit record AND span are created
        let spans = exporter.spans();
        let blocked_span = spans
            .iter()
            .find(|s| s.name == "proxy_request:blocked_conn")
            .expect("Must emit span for blocked proxy request");

        let hash = blocked_span
            .record_hash()
            .expect("Span must have record hash");
        let audit_records = proxy.audit_records();
        let matching = audit_records
            .iter()
            .find(|r| r.sha256_digest() == hash)
            .expect("Span record hash must match signed audit record in proxy log");

        assert!(
            matching.verify(),
            "Blocked audit record must have valid signature"
        );
        assert!(matches!(
            matching.decision,
            s3a_harness::proxy::ProxyDecision::HostBlocked { .. }
        ));
    }

    #[test]
    fn test_otlp_http_exporter_json_structure() {
        let exporter = OtlpHttpExporter::new("http://otel-collector.corp:4318", "s3a-test");
        assert_eq!(
            exporter.endpoint(),
            "http://otel-collector.corp:4318/v1/traces"
        );

        let span = Span::new(
            "tool_call:s3a_query",
            "agent_1",
            "job_1",
            "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
        )
        .with_attribute("tool.name", "s3a_query")
        .with_attribute("query.target", "academic_papers");

        let json_payload = exporter.to_otlp_json(&[span]);

        // Validate standard OTLP JSON schema
        let resource_spans = json_payload
            .get("resourceSpans")
            .and_then(|v| v.as_array())
            .expect("Must have resourceSpans array");
        assert_eq!(resource_spans.len(), 1);

        let scope_spans = resource_spans[0]
            .get("scopeSpans")
            .and_then(|v| v.as_array())
            .expect("Must have scopeSpans array");
        assert_eq!(scope_spans.len(), 1);

        let spans = scope_spans[0]
            .get("spans")
            .and_then(|v| v.as_array())
            .expect("Must have spans array");
        assert_eq!(spans.len(), 1);

        let exported_span = &spans[0];
        assert_eq!(
            exported_span.get("name").and_then(|v| v.as_str()),
            Some("tool_call:s3a_query")
        );

        let attrs = exported_span
            .get("attributes")
            .and_then(|v| v.as_array())
            .expect("Must have attributes array");

        let has_hash_attr = attrs.iter().any(|a| {
            a.get("key").and_then(|k| k.as_str()) == Some("s3a.record_hash")
                && a.get("value")
                    .and_then(|v| v.get("stringValue"))
                    .and_then(|s| s.as_str())
                    == Some("abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890")
        });
        assert!(
            has_hash_attr,
            "OTLP JSON must include s3a.record_hash attribute"
        );
    }
}
