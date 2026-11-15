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
