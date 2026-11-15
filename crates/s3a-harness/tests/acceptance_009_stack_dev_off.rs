//! Acceptance 009 follow-up (8): with dev mode OFF the stack runs a real, authenticated LRS.
//!
//! Written before the fix. Do not edit. Real binaries, no stubs.
//!
//! Contract (new at b9daf82): credentials never live in stack.toml or on a command line. The
//! LRS reads them from the file named by env `S3A_LRS_CREDENTIALS_FILE` (one
//! `user:password[:scopes]` per line); `StackManager::start_native` passes that variable through.

use std::io::Write;
use std::net::TcpListener;

use s3a_harness::stack::{find_binary, HealthState, StackConfig, StackManager};

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn ensure(name: &str, pkg: &str) {
    if find_binary(name).is_none() {
        let _ = std::process::Command::new("cargo")
            .args(["build", "-p", pkg])
            .status();
    }
    assert!(find_binary(name).is_some(), "binary {name} must be built");
}

fn b64(s: &str) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let b = s.as_bytes();
    let mut o = String::new();
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                o.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                o.push('=');
            }
        }
    }
    o
}

#[test]
fn dev_mode_off_starts_authenticated_lrs_without_insecure_dev() {
    ensure("s3a-lrs-server", "s3a-lrs-server");
    let tmp = tempfile::tempdir().unwrap();
    let pass = format!(
        "pw{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let cred = tmp.path().join("lrs-credentials.txt");
    writeln!(std::fs::File::create(&cred).unwrap(), "auditor:{pass}:all").unwrap();
    std::env::set_var("S3A_LRS_CREDENTIALS_FILE", &cred);

    let mut config = StackConfig::default();
    config.stack.name = "devoff-stack".into();
    config.stack.dev = false;
    config.stack.data_dir = tmp.path().join("data");
    config.stack.log_dir = tmp.path().join("logs");
    config.services.vault.enabled = false;
    config.services.mcp.enabled = false;
    config.services.harness.enabled = false;
    config.services.sandbox.enabled = false;
    config.services.otel.enabled = false;
    config.services.lrs.enabled = true;
    config.services.lrs.port = free_port();
    assert!(!config.stack.is_dev(), "precondition: dev mode off");
    let toml = config.to_toml();
    assert!(!toml.contains(&pass), "no secret in stack.toml");

    let mgr = StackManager::new(tmp.path());
    let mut handle = mgr
        .start_native(&config)
        .expect("dev-off stack must start (credentials come from the env file)");
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        #[cfg(target_os = "linux")]
        for pid in &handle.pids {
            let cmd = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
            let cmd = String::from_utf8_lossy(&cmd).replace('\0', " ");
            assert!(
                !cmd.contains("--insecure-dev"),
                "dev mode off: no --insecure-dev, got `{cmd}`"
            );
            assert!(
                !cmd.contains(&pass),
                "credentials must not be on the command line"
            );
        }
        let status = mgr.status(&config);
        let lrs = status
            .services
            .iter()
            .find(|s| s.service == "lrs")
            .expect("lrs in status");
        assert_eq!(
            lrs.state,
            HealthState::Up,
            "LRS must be UP with dev mode off: {}",
            lrs.detail
        );
        assert!(!status.to_display().contains("(dev mode)"));

        let url = format!(
            "http://127.0.0.1:{}/xapi/statements",
            config.services.lrs.port
        );
        let anon = ureq::get(&url)
            .set("X-Experience-API-Version", "1.0.3")
            .call();
        match anon {
            Err(ureq::Error::Status(code, _)) => {
                assert_eq!(code, 401, "unauthenticated request must be 401")
            }
            other => panic!(
                "unauthenticated request must be rejected with 401, got {:?}",
                other.map(|r| r.status())
            ),
        }
        let authed = ureq::get(&url)
            .set("X-Experience-API-Version", "1.0.3")
            .set(
                "Authorization",
                &format!("Basic {}", b64(&format!("auditor:{pass}"))),
            )
            .call()
            .expect("credentialed request must succeed");
        assert_eq!(authed.status(), 200);
    }));
    handle.stop();
    std::env::remove_var("S3A_LRS_CREDENTIALS_FILE");
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}
