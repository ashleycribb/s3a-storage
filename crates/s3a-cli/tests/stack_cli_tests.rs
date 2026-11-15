use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(1);

struct TempDirGuard(PathBuf);

impl TempDirGuard {
    fn new(label: &str) -> Self {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!(
            "s3a_stack_test_{}_{}_{}",
            label,
            std::process::id(),
            id
        ));
        let _ = fs::create_dir_all(&p);
        Self(p)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn test_stack_cli_help() {
    let bin = env!("CARGO_BIN_EXE_s3a-cli");
    let output = Command::new(bin)
        .args(["stack", "help"])
        .output()
        .expect("Failed to execute s3a-cli stack help");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("S3A One-Command Self-Hosted Stack CLI"));
    assert!(stdout.contains("init"));
    assert!(stdout.contains("up"));
    assert!(stdout.contains("status"));
    assert!(stdout.contains("down"));
}

#[test]
fn test_stack_cli_init_and_status() {
    let bin = env!("CARGO_BIN_EXE_s3a-cli");
    let temp_dir = TempDirGuard::new("init_status");
    let config_path = temp_dir.path().join("stack.toml");

    // 1. Run stack init in tempdir
    let init_output = Command::new(bin)
        .current_dir(temp_dir.path())
        .args(["stack", "init", "--provider", "anthropic"])
        .output()
        .expect("Failed to execute s3a-cli stack init");

    assert!(init_output.status.success());
    assert!(config_path.exists());
    assert!(temp_dir.path().join("docker-compose.yml").exists());

    let content = fs::read_to_string(&config_path).expect("read config");
    assert!(content.contains("provider = \"anthropic\""));

    // 2. Run stack status with stopped services
    let status_output = Command::new(bin)
        .current_dir(temp_dir.path())
        .args(["stack", "status", "--json"])
        .output()
        .expect("Failed to execute s3a-cli stack status");

    // Down status causes exit code 1
    let stdout = String::from_utf8_lossy(&status_output.stdout);
    assert!(stdout.contains("\"overall\": \"Down\"") || stdout.contains("\"DOWN\""));
    assert!(stdout.contains("\"service\": \"vault\""));
    assert!(stdout.contains("\"state\": \"Down\""));
}

fn find_free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

#[test]
fn test_stack_cli_up_records_pids_and_down_stops_them() {
    let (tx, rx) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        run_stack_cli_up_down_test();
        let _ = tx.send(());
    });

    let timeout = std::time::Duration::from_secs(50);
    match rx.recv_timeout(timeout) {
        Ok(()) => {
            let _ = thread.join();
        }
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            panic!("test_stack_cli_up_records_pids_and_down_stops_them timed out after 50s");
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            if let Err(e) = thread.join() {
                std::panic::resume_unwind(e);
            }
            panic!("Test thread disconnected without sending completion");
        }
    }
}

fn run_cli_cmd(
    bin: &str,
    dir: &std::path::Path,
    args: &[&str],
    timeout: std::time::Duration,
) -> (std::process::ExitStatus, String) {
    let log_file_path = dir.join(format!(
        "cmd_{}.log",
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let file = fs::File::create(&log_file_path).expect("create log file");
    let err_file = file.try_clone().expect("clone log file handle");

    let mut child = Command::new(bin)
        .current_dir(dir)
        .args(args)
        .stdout(file)
        .stderr(err_file)
        .spawn()
        .expect("spawn cli command");

    let start = std::time::Instant::now();
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(s) => break s,
            None => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    panic!("CLI command {:?} timed out after {:?}", args, timeout);
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
    };

    let output = fs::read_to_string(&log_file_path).unwrap_or_default();
    (status, output)
}

fn run_stack_cli_up_down_test() {
    let bin = env!("CARGO_BIN_EXE_s3a-cli");
    let temp_dir = TempDirGuard::new("up_down");
    let config_path = temp_dir.path().join("stack.toml");

    // 1. Initialize
    let (status, out) = run_cli_cmd(
        bin,
        temp_dir.path(),
        &["stack", "init"],
        std::time::Duration::from_secs(10),
    );
    assert!(status.success(), "stack init failed: {}", out);

    // 2. Configure with ephemeral ports
    let vp = find_free_port();
    let lp = find_free_port();
    let mp = find_free_port();
    let hp = find_free_port();
    let sp = find_free_port();

    let custom_cfg = format!(
        r#"
[stack]
name = "cli-test-stack"
host = "127.0.0.1"
data_dir = "./data"
log_dir = "./logs"

[model]
provider = "local"
model_name = "llama3"
base_url = "http://127.0.0.1:11434/v1"

[services.vault]
enabled = true
port = {}

[services.lrs]
enabled = true
port = {}
archive = "lrs.s3a"

[services.mcp]
enabled = true
port = {}

[services.harness]
enabled = true
port = {}
config_path = "./agents/scholar_reviewer.toml"

[services.sandbox]
enabled = true
port = {}
workers = 1
poll_interval_ms = 500

[services.otel]
enabled = false
port = 4318
grpc_port = 4317
"#,
        vp, lp, mp, hp, sp
    );
    fs::write(&config_path, custom_cfg).expect("write config");

    // 3. Start stack with s3a stack up --native (timeout 30s)
    let (up_status, up_out) = run_cli_cmd(
        bin,
        temp_dir.path(),
        &["stack", "up", "--native"],
        std::time::Duration::from_secs(30),
    );
    assert!(
        up_status.success(),
        "s3a stack up must succeed. output: {}",
        up_out
    );

    // 4. Verify state.json recorded PIDs
    let state_file = temp_dir.path().join(".stack").join("state.json");
    assert!(state_file.exists(), ".stack/state.json must exist");
    let state_content = fs::read_to_string(&state_file).expect("read state");
    assert!(state_content.contains("\"pids\""));
    assert!(state_content.contains("\"native\""));

    let state_json: serde_json::Value =
        serde_json::from_str(&state_content).expect("parse state.json");
    let recorded_pids: Vec<u32> = state_json
        .get("pids")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_u64().map(|n| n as u32))
                .collect()
        })
        .unwrap_or_default();
    assert!(!recorded_pids.is_empty(), "Recorded PIDs must not be empty");

    // 5. Query status -> must be UP
    let (status_code, status_out) = run_cli_cmd(
        bin,
        temp_dir.path(),
        &["stack", "status", "--json"],
        std::time::Duration::from_secs(10),
    );
    assert!(status_code.success(), "status failed: {}", status_out);
    assert!(status_out.contains("\"overall\": \"Up\"") || status_out.contains("\"UP\""));

    // 6. Stop stack with s3a stack down
    let (down_status, down_out) = run_cli_cmd(
        bin,
        temp_dir.path(),
        &["stack", "down"],
        std::time::Duration::from_secs(20),
    );
    assert!(down_status.success(), "stack down failed: {}", down_out);

    // 7. Verify all PIDs are gone
    for pid in recorded_pids {
        assert!(
            !s3a_cli::stack_cli::is_process_alive(pid),
            "PID {} must be gone after stack down",
            pid
        );
    }

    // 8. Verify status -> must be DOWN
    let (status_down_code, status_down_out) = run_cli_cmd(
        bin,
        temp_dir.path(),
        &["stack", "status", "--json"],
        std::time::Duration::from_secs(10),
    );
    assert!(!status_down_code.success());
    assert!(
        status_down_out.contains("\"overall\": \"Down\"") || status_down_out.contains("\"DOWN\"")
    );
}
