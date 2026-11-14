use std::process::Command;

#[test]
fn test_trace_export_otlp_replays_vault_records() {
    // 1. Start local mock OTLP receiver
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let port = server.server_addr().to_ip().unwrap().port();
    let endpoint = format!("http://127.0.0.1:{}", port);

    // 2. Create temporary vault with records
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("s3a_trace_test_{}", nonce));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let vault_path = temp_dir.join("trace_vault.s3a");

    let s3a_bin = env!("CARGO_BIN_EXE_s3a");

    // Initialize vault
    let init_status = Command::new(s3a_bin)
        .args(["init", vault_path.to_str().unwrap()])
        .status()
        .expect("Failed to run s3a init");
    assert!(init_status.success());

    // Store a document in vault
    let put_status = Command::new(s3a_bin)
        .args([
            "put",
            vault_path.to_str().unwrap(),
            "experiment_alpha",
            "telemetry_data_run_101",
        ])
        .status()
        .expect("Failed to run s3a put");
    assert!(put_status.success());

    // 3. Run s3a trace export in background thread or run receiver in thread
    let server_thread = std::thread::spawn(move || {
        let mut request = server.recv().unwrap();
        let mut body = String::new();
        request.as_reader().read_to_string(&mut body).unwrap();

        let response = tiny_http::Response::from_string("{\"status\": \"ok\"}")
            .with_status_code(200)
            .with_header(
                tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
                    .unwrap(),
            );
        let _ = request.respond(response);
        body
    });

    // Run s3a trace export --otlp <endpoint> <vault_path>
    let export_output = Command::new(s3a_bin)
        .args([
            "trace",
            "export",
            "--otlp",
            &endpoint,
            vault_path.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute s3a trace export");

    assert!(
        export_output.status.success(),
        "Export failed: {}",
        String::from_utf8_lossy(&export_output.stderr)
    );

    let received_body = server_thread.join().unwrap();
    let json_val: serde_json::Value =
        serde_json::from_str(&received_body).expect("Received body must be valid JSON");

    // 4. Validate OTLP JSON structure
    let resource_spans = json_val
        .get("resourceSpans")
        .and_then(|v| v.as_array())
        .expect("Must have resourceSpans");
    assert!(!resource_spans.is_empty());

    let scope_spans = resource_spans[0]
        .get("scopeSpans")
        .and_then(|v| v.as_array())
        .expect("Must have scopeSpans");
    assert!(!scope_spans.is_empty());

    let spans = scope_spans[0]
        .get("spans")
        .and_then(|v| v.as_array())
        .expect("Must have spans");
    assert!(!spans.is_empty());

    // Find the span for the stored document
    let doc_span = spans
        .iter()
        .find(|s| {
            s.get("name")
                .and_then(|n| n.as_str())
                .map(|n| n.contains("experiment_alpha"))
                .unwrap_or(false)
        })
        .expect("Must find exported span for 'experiment_alpha'");

    let attrs = doc_span
        .get("attributes")
        .and_then(|v| v.as_array())
        .unwrap();

    let hash_attr = attrs
        .iter()
        .find(|a| a.get("key").and_then(|k| k.as_str()) == Some("s3a.record_hash"))
        .expect("Span must have 's3a.record_hash' attribute");

    let record_hash = hash_attr
        .get("value")
        .and_then(|v| v.get("stringValue"))
        .and_then(|s| s.as_str())
        .unwrap();

    // Verify hash matches sha256("telemetry_data_run_101")
    use sha2::{Digest, Sha256};
    let expected_hash = hex::encode(Sha256::digest(b"telemetry_data_run_101"));
    assert_eq!(
        record_hash, expected_hash,
        "Record hash in span must match SHA-256 of stored record"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}
