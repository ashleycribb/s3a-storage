use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use s3a_harness::{LrsStore, Store};
use serde_json::json;

struct ServerGuard {
    child: Child,
    archive_path: PathBuf,
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.archive_path);
    }
}

fn find_server_exe() -> PathBuf {
    if let Ok(cur) = std::env::current_exe() {
        if let Some(target_dir) = cur.parent().and_then(|p| p.parent()) {
            let candidate = target_dir.join(if cfg!(windows) {
                "s3a-lrs-server.exe"
            } else {
                "s3a-lrs-server"
            });
            if candidate.exists() {
                return candidate;
            }
        }
    }
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidate = manifest_dir
        .join("../../target/debug")
        .join(if cfg!(windows) {
            "s3a-lrs-server.exe"
        } else {
            "s3a-lrs-server"
        });
    if candidate.exists() {
        return candidate;
    }
    PathBuf::from(if cfg!(windows) {
        "target/debug/s3a-lrs-server.exe"
    } else {
        "target/debug/s3a-lrs-server"
    })
}

fn get_available_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind ephemeral port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

fn wait_for_server(port: u16, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if let Ok(mut stream) = TcpStream::connect_timeout(
            &format!("127.0.0.1:{}", port).parse().unwrap(),
            Duration::from_millis(200),
        ) {
            let req = format!(
                "GET /about HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
                port
            );
            if stream.write_all(req.as_bytes()).is_ok() {
                let mut buf = [0u8; 128];
                if let Ok(n) = stream.read(&mut buf) {
                    if n > 0 && String::from_utf8_lossy(&buf[..n]).contains("200 OK") {
                        return true;
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

#[test]
fn test_e2e_harness_lrs_store_statements_and_timeline() {
    let server_exe = find_server_exe();
    assert!(
        server_exe.exists(),
        "s3a-lrs-server executable must exist at {}",
        server_exe.display()
    );

    let port = get_available_port();
    let temp_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let archive_path = std::env::temp_dir().join(format!(
        "s3a_e2e_lrs_{}_{}.s3a",
        std::process::id(),
        temp_ts
    ));

    let child = Command::new(&server_exe)
        .arg("--port")
        .arg(port.to_string())
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--archive")
        .arg(archive_path.to_str().unwrap())
        .arg("--insecure-dev")
        .arg("--admin-api-override")
        .spawn()
        .expect("Failed to start temp s3a-lrs-server");

    let _guard = ServerGuard {
        child,
        archive_path,
    };

    // Wait up to 6 seconds for server to be ready
    let ready = wait_for_server(port, Duration::from_secs(6));
    assert!(
        ready,
        "s3a-lrs-server did not become ready on port {}",
        port
    );

    // Initialize LrsStore
    let endpoint = format!("http://127.0.0.1:{}", port);
    let store = LrsStore::new(&endpoint, None, None, false);

    // 1. List academic verbs
    let verbs = store.list_academic_verbs().unwrap();
    assert!(!verbs.is_empty());
    assert!(verbs.iter().any(|v| v["label"] == "reviewed"));

    // 2. Record xAPI statement with research traceability profile
    let session_uuid = "a1b2c3d4-e5f6-4a5b-8c9d-0e1f2a3b4c5d";
    let stmt_id = "f0e1d2c3-b4a5-4968-8754-3210fedcba98";
    let stmt = json!({
        "id": stmt_id,
        "actor": {
            "mbox": "mailto:agent-checker@lab.org",
            "name": "Dr. Researcher"
        },
        "verb": {
            "id": "https://ashecribb.dev/xapi/verbs/reviewed"
        },
        "object": {
            "id": "https://doi.org/10.1000/182",
            "objectType": "Activity"
        },
        "context": {
            "registration": session_uuid
        }
    });

    let recorded_id = store.record_xapi_statement(&stmt).unwrap();
    assert_eq!(recorded_id, stmt_id);

    // 3. Query researcher timeline back from the server
    let timeline = store
        .get_researcher_timeline(None, Some(session_uuid), None)
        .unwrap();
    assert_eq!(
        timeline.len(),
        1,
        "Timeline must contain the recorded statement"
    );
    assert_eq!(timeline[0]["statement_id"], stmt_id);
    assert_eq!(
        timeline[0]["verb"],
        "https://ashecribb.dev/xapi/verbs/reviewed"
    );

    // 4. Query session context back from the server
    let ctx = store.get_session_context(session_uuid).unwrap();
    assert_eq!(ctx["statements_count"], 1);
    assert_eq!(ctx["total_events"], 1);
}
