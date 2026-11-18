//! Regression tests for the S3A usability fixes (Nov 18, 2026):
//! - B1: non-zero exit codes on every CLI failure path (with `--json` error
//!   objects preserved).
//! - B4: tile counts agree across `status` / `inspect` / `doctor`; `doctor`
//!   no longer counts a sealed archive's footer as torn bytes.
//! - B7: `serve` binds loopback by default and refuses a non-loopback bind
//!   without the explicit `--public` flag.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(900);

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_s3a-cli")
}

fn temp_path(label: &str) -> PathBuf {
    let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "s3a_exit_{}_{}_{}.s3a",
        label,
        std::process::id(),
        id
    ));
    p
}

struct TempGuard(PathBuf);
impl Drop for TempGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new(bin()).args(args).output().expect("run s3a-cli")
}

fn seed_demo(path: &PathBuf) -> TempGuard {
    let guard = TempGuard(path.clone());
    let out = run(&["scholar", "demo-seed", path.to_str().unwrap()]);
    assert!(out.status.success(), "demo-seed failed: {:?}", out);
    guard
}

fn json_tile_count(stdout: &[u8]) -> u64 {
    let v: serde_json::Value =
        serde_json::from_slice(stdout).expect("stdout is valid JSON");
    v["tile_count"]
        .as_u64()
        .expect("tile_count present in JSON output")
}

#[test]
fn test_failure_paths_exit_nonzero_and_keep_json_errors() {
    // tour on a missing file (plain + --json).
    let out = run(&["tour", "/nonexistent-s3a-test.s3a"]);
    assert!(!out.status.success(), "tour on missing file must fail");
    let out = run(&["tour", "/nonexistent-s3a-test.s3a", "--json"]);
    assert!(!out.status.success(), "tour --json on missing file must fail");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("\"ok\":false"),
        "tour --json must keep its error object"
    );

    // verify on a missing file (plain + --json).
    let out = run(&["verify", "/nonexistent-s3a-test.s3a"]);
    assert!(!out.status.success(), "verify on missing file must fail");
    let out = run(&["verify", "/nonexistent-s3a-test.s3a", "--json"]);
    assert!(!out.status.success(), "verify --json on missing file must fail");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("\"ok\":false"),
        "verify --json must keep its error object"
    );

    // inspect / status / doctor on a missing file.
    for cmd in ["inspect", "status", "doctor"] {
        let out = run(&[cmd, "/nonexistent-s3a-test.s3a"]);
        assert!(
            !out.status.success(),
            "{} on missing file must fail",
            cmd
        );
        let out = run(&[cmd, "--json", "/nonexistent-s3a-test.s3a"]);
        assert!(
            !out.status.success(),
            "{} --json on missing file must fail",
            cmd
        );
        assert!(
            String::from_utf8_lossy(&out.stdout).contains("\"ok\":false"),
            "{} --json must keep its error object",
            cmd
        );
    }

    // Failed scholar ingest (missing directory).
    let out = run(&["scholar", "ingest", "/nonexistent-dir", "out.s3a"]);
    assert!(
        !out.status.success(),
        "scholar ingest of a missing dir must fail"
    );
}

#[test]
fn test_unknown_command_and_usage_errors_exit_nonzero() {
    let out = run(&["frobnicate"]);
    assert!(!out.status.success(), "unknown command must fail");

    let out = run(&["scholar", "frobnicate"]);
    assert!(!out.status.success(), "unknown scholar action must fail");

    let out = run(&["tour"]);
    assert!(!out.status.success(), "tour with no file must fail");

    // Bare invocation still prints help and exits 0 (existing contract).
    let out = run(&[]);
    assert!(out.status.success(), "bare invocation must still exit 0");
    assert!(String::from_utf8_lossy(&out.stdout).contains("STABLE CORE OPS"));
}

#[test]
fn test_success_paths_still_exit_zero() {
    let p = temp_path("success");
    let _guard = seed_demo(&p);
    let path = p.to_str().unwrap();
    assert!(run(&["status", path]).status.success());
    assert!(run(&["status", "--json", path]).status.success());
    assert!(run(&["verify", path]).status.success());
    assert!(run(&["doctor", path]).status.success());
    assert!(run(&["inspect", path]).status.success());
}

#[test]
fn test_counts_agree_across_status_inspect_doctor() {
    let p = temp_path("counts");
    let _guard = seed_demo(&p);
    let path = p.to_str().unwrap();

    let status = run(&["status", "--json", path]);
    let inspect = run(&["inspect", "--json", path]);
    let doctor = run(&["doctor", "--json", path]);
    assert!(status.status.success());
    assert!(inspect.status.success());
    assert!(doctor.status.success());

    let s = json_tile_count(&status.stdout);
    let i = json_tile_count(&inspect.stdout);
    let d = json_tile_count(&doctor.stdout);
    assert_eq!(s, i, "status vs inspect tile_count");
    assert_eq!(s, d, "status vs doctor tile_count");

    // The inspect tile list itself must have exactly that many entries.
    let v: serde_json::Value =
        serde_json::from_slice(&inspect.stdout).expect("inspect JSON");
    assert_eq!(v["tiles"].as_array().unwrap().len() as u64, s);

    // Honest labels: "Committed Tiles" (same defined meaning as status and
    // doctor), and no raw u64::MAX sentinel in the text output.
    let text = run(&["inspect", path]);
    let text = String::from_utf8_lossy(&text.stdout);
    assert!(text.contains("Committed Tiles:"), "inspect label");
    assert!(
        !text.contains("18446744073709551615"),
        "no raw u64::MAX sentinel in inspect text"
    );
    // Note: a freshly seeded demo has no empty tile, so the "TimeRange=
    // [empty]" rendering is covered by
    // test_doctor_sealed_archive_is_healthy_not_degraded, where the sealed
    // copy carries the empty manifest + SCHEMA tiles.
}

#[test]
fn test_doctor_sealed_archive_is_healthy_not_degraded() {
    let p = temp_path("sealed");
    let _guard = seed_demo(&p);
    let sealed = temp_path("sealed-out");
    let _guard2 = TempGuard(sealed.clone());
    let path = p.to_str().unwrap();
    let sealed_str = sealed.to_str().unwrap();

    let out = run(&["seal", path, sealed_str]);
    assert!(out.status.success(), "seal failed: {:?}", out);

    let out = run(&["doctor", "--json", sealed_str]);
    assert!(out.status.success(), "doctor on sealed archive must succeed");
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("doctor JSON");
    assert_eq!(v["health"], "HEALTHY", "pristine sealed archive is HEALTHY");
    assert_eq!(v["torn_bytes"], 0, "sealed footer is not torn bytes");
    assert!(
        v["sealed_footer_bytes"].as_u64().unwrap() > 0,
        "sealed footer is reported"
    );

    // And the counts still agree on the sealed copy.
    let s = json_tile_count(&run(&["status", "--json", sealed_str]).stdout);
    let i = json_tile_count(&run(&["inspect", "--json", sealed_str]).stdout);
    let d = json_tile_count(&run(&["doctor", "--json", sealed_str]).stdout);
    assert_eq!(s, i);
    assert_eq!(s, d);

    // `seal` never writes its input (upstream cf61930dc: schema tile goes
    // to a scratch copy, only the sealed output gains the empty tiles), so
    // the source keeps its 3 tiles and has no empty tile — while the sealed
    // copy carries the empty manifest + SCHEMA tiles, whose ranges must
    // render as "empty", never the raw u64::MAX..0 sentinel.
    let text = run(&["inspect", path]);
    let text = String::from_utf8_lossy(&text.stdout);
    assert!(
        !text.contains("18446744073709551615"),
        "no raw u64::MAX sentinel in inspect text"
    );
    assert!(
        !text.contains("TimeRange=[empty]"),
        "seal must not mutate its input: source gains no empty tile"
    );
    let sealed_text = run(&["inspect", sealed_str]);
    let sealed_text = String::from_utf8_lossy(&sealed_text.stdout);
    assert!(
        !sealed_text.contains("18446744073709551615"),
        "no raw u64::MAX sentinel in sealed inspect text"
    );
    assert!(
        sealed_text.contains("TimeRange=[empty]"),
        "empty tile range label: {}",
        sealed_text
    );
}

#[test]
fn test_verify_failure_on_corrupted_record_exits_nonzero() {
    let p = temp_path("corrupt");
    let _guard = seed_demo(&p);
    let sealed = temp_path("corrupt-sealed");
    let _guard2 = TempGuard(sealed.clone());
    let corrupt = temp_path("corrupt-out");
    let _guard3 = TempGuard(corrupt.clone());
    let path = p.to_str().unwrap();
    let sealed_str = sealed.to_str().unwrap();

    assert!(run(&["seal", path, sealed_str]).status.success());

    // Flip one byte inside a record payload: tile 1 (ACADEMIC_PAPERS) starts
    // at FILE_HEADER_SIZE (512) + 1 * HYPER_TILE_SIZE (131072); its payload
    // starts 512 bytes later (tile header) and the record is 64 bytes.
    let mut bytes = std::fs::read(sealed_str).expect("read sealed archive");
    let record_byte = 512 + 131072 + 512 + 10;
    bytes[record_byte] ^= 0xFF;
    std::fs::write(&corrupt, &bytes).expect("write corrupted archive");

    let corrupt_str = corrupt.to_str().unwrap();
    let out = run(&["verify", corrupt_str]);
    assert!(
        !out.status.success(),
        "verify of a corrupted sealed archive must fail"
    );
    let out = run(&["verify", "--json", corrupt_str]);
    assert!(
        !out.status.success(),
        "verify --json of a corrupted sealed archive must fail"
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("\"ok\":false"),
        "verify --json must keep its error object on failure"
    );
}

fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    listener.local_addr().unwrap().port()
}

fn wait_for_tcp(port: u16, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn test_serve_binds_loopback_by_default() {
    // The merged serve health endpoint reports the vault's health: 200 only
    // when the vault file (S3A_VAULT_PATH, default "vault.s3a") exists and
    // opens, 503 otherwise. So seed a vault and point the server at it;
    // the loopback-bind contract is what this test pins (B7).
    let vault = temp_path("serve-vault");
    let _vault_guard = seed_demo(&vault);
    let vault_str = vault.to_str().unwrap().to_string();

    // Retry on a fresh port: the ephemeral port picked by free_port() can be
    // claimed by another parallel test process before `serve` binds it.
    let mut last_err = String::new();
    for _attempt in 0..5 {
        let port = free_port();
        let mut child = ChildGuard(
            Command::new(bin())
                .args(["serve", &port.to_string()])
                .env("S3A_VAULT_PATH", &vault_str)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("spawn serve"),
        );
        if !wait_for_tcp(port, Duration::from_secs(5)) {
            last_err = format!(
                "serve did not start listening on loopback port {} (child exited: {:?})",
                port,
                child.0.try_wait().ok().flatten().map(|s| s.code())
            );
            continue;
        }

        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect loopback");
        stream
            .write_all(b"GET /api/health HTTP/1.0\r\n\r\n")
            .expect("write health request");
        let mut resp = String::new();
        stream.read_to_string(&mut resp).expect("read health response");
        assert!(resp.contains("200 OK"), "health endpoint must answer: {}", resp);
        assert!(resp.contains("\"status\":\"ok\""), "health body: {}", resp);

        // The process must still be running (it is a server, not a one-shot).
        assert!(child.0.try_wait().expect("poll serve").is_none());
        return;
    }
    panic!("serve never became reachable on loopback: {}", last_err);
}

#[test]
fn test_serve_refuses_public_bind_without_flag() {
    let port = free_port();
    let out = run(&["serve", &port.to_string(), "--bind", "0.0.0.0"]);
    assert!(
        !out.status.success(),
        "serve --bind 0.0.0.0 without --public must refuse to start"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--public"), "refusal must name --public: {}", stderr);
}
