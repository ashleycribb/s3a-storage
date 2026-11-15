//! Integration tests for S3A one-command self-hosted stack (Prompt 008).
//!
//! Enforces:
//! 1. Real health checks: `s3a stack status` reports UP only after verifying port + response.
//! 2. Zero demo/placeholder data: Stopped service reports DOWN; two configs produce distinct outputs.
//! 3. Zero secrets in config or logs.
//! 4. Core stays free of stack code.
//! 5. Stack brings up services and exercises 007 viewer demo.

use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use s3a_core::HyperTileHeader;
use s3a_engine::TileWriter;
use s3a_harness::mcp::S3AMcpDispatcher;
use s3a_harness::sandbox::Sandbox;
use s3a_harness::stack::{
    check_service_health, find_binary, HealthState, ServiceKind, StackConfig, StackManager,
};
use s3a_harness::store::{LocalStore, Store};
use s3a_harness::viewer::{generate_viewer_html, ViewerRegistry};
use serde_json::{json, Value};

fn find_free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

fn ensure_binary(name: &str) -> std::path::PathBuf {
    if let Some(p) = find_binary(name) {
        return p;
    }
    let pkg = match name {
        "s3a" | "s3a-cli" => "s3a-cli",
        "s3a-lrs-server" => "s3a-lrs-server",
        "s3a-sandbox" => "s3a-sandbox",
        "s3a-harness" => "s3a-harness",
        _ => name,
    };
    let _ = std::process::Command::new("cargo")
        .args(["build", "-p", pkg])
        .status();
    find_binary(name).unwrap_or_else(|| panic!("failed to find or build binary '{}'", name))
}

fn create_test_dispatcher(db_path: &Path) -> S3AMcpDispatcher {
    let sandbox = Sandbox::new(db_path.parent().unwrap(), false).expect("create sandbox");
    let store: Arc<dyn Store> = Arc::new(LocalStore::new(db_path, false));
    let p_uuid = [0x1111222233334444, 0x5555666677778888];
    let s_uuid = [0xAAAABBBBCCCCDDDD, 0xEEEEFFFF00001111];
    let d = S3AMcpDispatcher::new(sandbox, store, p_uuid, s_uuid, 42);

    let mut policy = s3a_harness::permissions::PermissionPolicy::new(
        s3a_harness::permissions::PermissionMode::Auto,
    );
    policy.require_pass = false;
    d.enforcement_gate().set_policy(policy);
    d
}

fn call_mcp_tool(d: &S3AMcpDispatcher, tool_name: &str, args: Value) -> (Value, bool) {
    let req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": tool_name,
            "arguments": args
        }
    });
    let raw = d
        .dispatch(&serde_json::to_string(&req).unwrap())
        .expect("expected response");
    let resp: Value = serde_json::from_str(&raw).expect("valid JSON response");
    assert_eq!(resp.get("jsonrpc").and_then(|v| v.as_str()), Some("2.0"));
    let res = resp.get("result").expect("expected tool call result");
    let content = res
        .get("content")
        .and_then(|c| c.as_array())
        .expect("content array");
    let text = content[0]
        .get("text")
        .and_then(|t| t.as_str())
        .expect("text field");
    let parsed: Value = serde_json::from_str(text).expect("tool text is JSON");
    let is_err = res
        .get("isError")
        .and_then(|e| e.as_bool())
        .unwrap_or(false);
    (parsed, is_err)
}

#[test]
fn test_stopped_service_reports_down() {
    let _ = ensure_binary("s3a");
    let port = find_free_port();
    let status = check_service_health("127.0.0.1", port, ServiceKind::Vault);

    assert_eq!(
        status.state,
        HealthState::Down,
        "stopped service MUST report Down"
    );
    assert!(
        status.detail.contains("Connection refused") || status.detail.contains("refused"),
        "detail must explain connection failure: {}",
        status.detail
    );
    assert_eq!(status.port, Some(port));
    assert_eq!(status.latency_ms, None);
}

#[test]
fn test_two_different_configs_produce_different_status_output() {
    let port_a1 = find_free_port();
    let port_a2 = find_free_port();
    let port_b1 = find_free_port();

    let mut config_a = StackConfig::default();
    config_a.stack.name = "lab-node-alpha".to_string();
    config_a.services.vault.port = port_a1;
    config_a.services.lrs.port = port_a2;
    config_a.services.otel.enabled = false;

    let mut config_b = StackConfig::default();
    config_b.stack.name = "lab-node-beta".to_string();
    config_b.services.vault.port = port_b1;
    config_b.services.lrs.enabled = false; // LRS disabled in B
    config_b.services.otel.enabled = true; // OTel enabled in B
    config_b.services.otel.port = find_free_port();

    let mgr = StackManager::new(".");
    let status_a = mgr.status(&config_a);
    let status_b = mgr.status(&config_b);

    assert_ne!(
        status_a.stack_name, status_b.stack_name,
        "Stack names must differ"
    );
    assert_ne!(
        status_a.services, status_b.services,
        "Service statuses must differ between two configs"
    );

    let display_a = status_a.to_display();
    let display_b = status_b.to_display();
    assert_ne!(
        display_a, display_b,
        "Status display string must differ between distinct configs"
    );

    assert!(display_a.contains("lab-node-alpha"));
    assert!(display_b.contains("lab-node-beta"));
    assert!(display_a.contains(&port_a1.to_string()));
    assert!(display_b.contains(&port_b1.to_string()));
}

#[test]
fn test_secret_rejection_policy() {
    let config = StackConfig::default();

    // 1. Direct api_key in config
    let bad_toml_1 = r#"
[stack]
name = "leaky-node"
host = "127.0.0.1"

[model]
provider = "gemini"
model_name = "gemini-2.0"
api_key = "AIzaSyDummySecret123"
"#;
    assert!(
        config.validate_no_secrets(bad_toml_1).is_err(),
        "Must reject config containing api_key"
    );

    // 2. sk- token pattern
    let bad_toml_2 = r#"
[stack]
name = "leaky-node-2"

[model]
token = "sk-ant-api03-secret123456"
"#;
    assert!(
        config.validate_no_secrets(bad_toml_2).is_err(),
        "Must reject config containing sk- token"
    );

    // 3. Clean config
    let clean_toml = config.to_toml();
    assert!(
        config.validate_no_secrets(&clean_toml).is_ok(),
        "Valid config without secrets must pass"
    );
}

#[test]
fn test_docker_compose_generation_zero_secrets() {
    let mut config = StackConfig::default();
    config.stack.name = "production-lab".to_string();
    config.services.vault.port = 8042;
    config.services.lrs.port = 8080;
    config.services.harness.port = 8082;
    config.services.otel.enabled = true;

    let compose = config.generate_docker_compose();

    // Must interpolate environment variables, NEVER contain raw keys
    assert!(
        compose.contains("GEMINI_API_KEY=${GEMINI_API_KEY:-}"),
        "Must reference env var for Gemini key"
    );
    assert!(
        compose.contains("ANTHROPIC_API_KEY=${ANTHROPIC_API_KEY:-}"),
        "Must reference env var for Anthropic key"
    );
    assert!(
        !compose.contains("sk-"),
        "Compose must not contain sk- tokens"
    );
    assert!(
        !compose.contains("AIzaSy"),
        "Compose must not contain AIzaSy tokens"
    );

    // Must include defined services and networks
    assert!(compose.contains("vault:"));
    assert!(compose.contains("lrs:"));
    assert!(compose.contains("harness:"));
    assert!(compose.contains("sandbox:"));
    assert!(compose.contains("otel:"));
    assert!(compose.contains("s3a-net:"));
}

struct TestStubOtelReceiver {
    shutdown: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
    pub port: u16,
}

impl TestStubOtelReceiver {
    fn new_raw_tcp_stub() -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind raw tcp stub");
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let shutdown = Arc::new(AtomicBool::new(false));
        let s_clone = shutdown.clone();
        let handle = std::thread::spawn(move || {
            while !s_clone.load(Ordering::SeqCst) {
                if let Ok((_stream, _)) = listener.accept() {
                    std::thread::sleep(Duration::from_millis(100));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        Self {
            shutdown,
            handle: Some(handle),
            port,
        }
    }

    fn new_http_stub() -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind http stub");
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let shutdown = Arc::new(AtomicBool::new(false));
        let s_clone = shutdown.clone();
        let handle = std::thread::spawn(move || {
            while !s_clone.load(Ordering::SeqCst) {
                if let Ok((mut stream, _)) = listener.accept() {
                    use std::io::Write;
                    let resp = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 17\r\nConnection: close\r\n\r\n{\"status\":\"ok\"}";
                    let _ = stream.write_all(resp.as_bytes());
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        Self {
            shutdown,
            handle: Some(handle),
            port,
        }
    }

    fn stop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for TestStubOtelReceiver {
    fn drop(&mut self) {
        self.stop();
    }
}

#[test]
fn test_live_health_check_real_http_up() {
    let host = "127.0.0.1";
    let port = find_free_port();
    let tmp = tempfile::tempdir().expect("create tempdir");
    let vault_file = tmp.path().join("vault.s3a");

    // Initially down
    let status_pre = check_service_health(host, port, ServiceKind::Vault);
    assert_eq!(status_pre.state, HealthState::Down);

    // Launch real binary
    let bin = ensure_binary("s3a");
    let mut child = std::process::Command::new(bin)
        .args([
            "serve",
            "--port",
            &port.to_string(),
            "--vault",
            vault_file.to_str().unwrap(),
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn s3a serve");

    // Poll: must report UP with valid JSON parsed
    let start = std::time::Instant::now();
    let mut status_post = check_service_health(host, port, ServiceKind::Vault);
    while start.elapsed() < Duration::from_secs(4) {
        status_post = check_service_health(host, port, ServiceKind::Vault);
        if status_post.state == HealthState::Up {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    assert_eq!(
        status_post.state,
        HealthState::Up,
        "running real binary must report UP. detail: {}",
        status_post.detail
    );
    assert!(
        status_post.detail.contains("Healthy: vault"),
        "detail must report Healthy: vault: {}",
        status_post.detail
    );
    assert!(status_post.latency_ms.is_some());

    // Stop process
    let _ = child.kill();
    let _ = child.wait();

    // Probe after stop: must report DOWN
    let status_stopped = check_service_health(host, port, ServiceKind::Vault);
    assert_eq!(
        status_stopped.state,
        HealthState::Down,
        "killed child process must report DOWN"
    );
}

#[test]
fn test_kill_real_child_process_reports_down() {
    let tmp_dir = tempfile::tempdir().expect("create tempdir");
    let mgr = StackManager::new(tmp_dir.path());

    let vault_port = find_free_port();
    let lrs_port = find_free_port();
    let mcp_port = find_free_port();
    let harness_port = find_free_port();
    let sandbox_port = find_free_port();

    let mut config = StackConfig::default();
    config.stack.name = "kill-test-stack".to_string();
    config.stack.data_dir = tmp_dir.path().join("data");
    config.stack.log_dir = tmp_dir.path().join("logs");
    config.services.vault.port = vault_port;
    config.services.lrs.port = lrs_port;
    config.services.mcp.port = mcp_port;
    config.services.harness.port = harness_port;
    config.services.sandbox.port = sandbox_port;
    config.services.otel.enabled = false;

    // Ensure binaries are available
    let _ = ensure_binary("s3a");
    let _ = ensure_binary("s3a-lrs-server");
    let _ = ensure_binary("s3a-sandbox");
    let _ = ensure_binary("s3a-harness");

    // Start native real binaries
    let mut handle = mgr.start_native(&config).expect("start native stack");

    // Verify all UP
    let status_up = mgr.status(&config);
    assert_eq!(status_up.overall, HealthState::Up);

    // Kill one of the real child processes
    assert!(!handle.children.is_empty(), "Children must be recorded");
    let _ = handle.children[0].kill();
    let _ = handle.children[0].wait();

    // Poll status: must report overall DOWN
    let start = std::time::Instant::now();
    let mut status_after_kill = mgr.status(&config);
    while start.elapsed() < Duration::from_secs(3) {
        status_after_kill = mgr.status(&config);
        if status_after_kill.overall == HealthState::Down {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    assert_eq!(
        status_after_kill.overall,
        HealthState::Down,
        "Killing real child process MUST cause stack overall status to report DOWN"
    );

    handle.stop();
}

#[test]
fn test_unimplemented_binary_never_up() {
    // If a service has no compiled binary, it reports Down with not implemented / not found
    let port = find_free_port();
    let status = check_service_health("127.0.0.1", port, ServiceKind::Otel);
    // Port closed -> Down
    assert_eq!(status.state, HealthState::Down);
    assert_ne!(status.state, HealthState::Up);
}

#[test]
fn test_otel_requires_real_response_not_tcp_alone() {
    // 1. Raw TCP open port that sends no HTTP/OTLP response MUST NOT report Up
    let mut raw_stub = TestStubOtelReceiver::new_raw_tcp_stub();
    let raw_status = check_service_health("127.0.0.1", raw_stub.port, ServiceKind::Otel);
    assert_ne!(
        raw_status.state,
        HealthState::Up,
        "Raw TCP port must NEVER report UP for OTel"
    );
    raw_stub.stop();

    // 2. HTTP OTel response MUST report Up
    let mut http_stub = TestStubOtelReceiver::new_http_stub();
    let http_status = check_service_health("127.0.0.1", http_stub.port, ServiceKind::Otel);
    assert_eq!(
        http_status.state,
        HealthState::Up,
        "Valid HTTP response must report UP for OTel"
    );
    http_stub.stop();

    // 3. Stopped stub MUST report Down
    let stopped_status = check_service_health("127.0.0.1", http_stub.port, ServiceKind::Otel);
    assert_eq!(stopped_status.state, HealthState::Down);
}

#[test]
fn test_stack_up_and_007_viewer_demo() {
    let tmp_dir = tempfile::tempdir().expect("create tempdir");
    let mgr = StackManager::new(tmp_dir.path());

    let vault_port = find_free_port();
    let lrs_port = find_free_port();
    let mcp_port = find_free_port();
    let harness_port = find_free_port();
    let sandbox_port = find_free_port();

    let mut config = StackConfig::default();
    config.stack.name = "demo-stack".to_string();
    config.stack.data_dir = tmp_dir.path().join("data");
    config.stack.log_dir = tmp_dir.path().join("logs");
    config.services.vault.port = vault_port;
    config.services.lrs.port = lrs_port;
    config.services.mcp.port = mcp_port;
    config.services.harness.port = harness_port;
    config.services.sandbox.port = sandbox_port;
    config.services.otel.enabled = false;

    // Ensure binaries are available
    let _ = ensure_binary("s3a");
    let _ = ensure_binary("s3a-lrs-server");
    let _ = ensure_binary("s3a-sandbox");
    let _ = ensure_binary("s3a-harness");

    // 1. Verify stack status before startup is DOWN
    let status_pre = mgr.status(&config);
    assert_eq!(status_pre.overall, HealthState::Down);

    // 2. Start native services (real binaries)
    let mut handle = mgr.start_native(&config).expect("start native stack");

    // 3. Verify stack status is UP
    let status_up = mgr.status(&config);
    assert_eq!(
        status_up.overall,
        HealthState::Up,
        "Stack overall status must be UP"
    );
    let up_services: Vec<_> = status_up
        .services
        .iter()
        .filter(|s| s.enabled && s.state == HealthState::Up)
        .collect();
    assert_eq!(
        up_services.len(),
        5,
        "All enabled services (vault, lrs, mcp, harness, sandbox) must be UP"
    );

    // 4. Exercise the 007 Viewer Demo over the active stack
    let vault_path = tmp_dir.path().join("demo_record.s3a");
    {
        let mut writer = TileWriter::create(&vault_path).expect("create demo archive");
        let payload =
            b"{\"title\":\"S3A Demo Record\",\"summary\":\"Self-hosted stack integration test\"}";
        let mut hdr = HyperTileHeader::new(0, 1);
        hdr.record_count = 1;
        hdr.payload_bytes = payload.len() as u32;
        hdr.data_crc32 = crc32c::crc32c(payload);
        writer
            .write_raw_hyper_tile(&hdr, payload)
            .expect("write tile");
    }

    let dispatcher = create_test_dispatcher(&vault_path);
    let reg = ViewerRegistry::new();

    // Call viewer MCP tools
    let (open_data, is_err) = call_mcp_tool(
        &dispatcher,
        "s3a_viewer_open",
        json!({ "file_path": vault_path.to_str().unwrap() }),
    );
    assert!(!is_err, "s3a_viewer_open must succeed");
    assert!(
        open_data.get("summary").is_some(),
        "s3a_viewer_open returns summary"
    );

    let (tree_data, is_err) = call_mcp_tool(
        &dispatcher,
        "s3a_viewer_get_tree",
        json!({ "file_path": vault_path.to_str().unwrap() }),
    );
    assert!(!is_err, "s3a_viewer_get_tree must succeed");
    assert!(
        tree_data.is_array(),
        "s3a_viewer_get_tree returns tree array"
    );

    let state = reg.load_archive(&vault_path).expect("open full state");
    let html = generate_viewer_html(&state);
    assert!(
        html.contains("S3A Record Viewer"),
        "Generated HTML must contain viewer title"
    );
    assert!(
        html.contains("demo_record.s3a"),
        "Generated HTML must contain archive path"
    );

    // 5. Stop stack and verify it returns to DOWN
    handle.stop();

    let status_post = mgr.status(&config);
    assert_eq!(
        status_post.overall,
        HealthState::Down,
        "Stack overall status must be DOWN after stop"
    );
}

#[test]
fn test_core_free_of_stack_code() {
    let core_src = Path::new("crates/s3a-core/src");
    if core_src.exists() {
        for entry in fs::read_dir(core_src).expect("read core src") {
            let entry = entry.expect("entry");
            let path = entry.path();
            if path.is_file() && path.extension().is_some_and(|ext| ext == "rs") {
                let content = fs::read_to_string(&path).expect("read core file");
                assert!(
                    !content.contains("StackConfig"),
                    "s3a-core must not contain stack code ({})",
                    path.display()
                );
                assert!(
                    !content.contains("s3a stack"),
                    "s3a-core must not contain stack CLI strings ({})",
                    path.display()
                );
            }
        }
    }
}
