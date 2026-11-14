use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn get_bin() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_s3a") {
        return std::path::PathBuf::from(p);
    }
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_s3a-cli") {
        return std::path::PathBuf::from(p);
    }
    panic!("Neither CARGO_BIN_EXE_s3a nor CARGO_BIN_EXE_s3a-cli is set");
}

fn temp_dir() -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("s3a_graph_cli_test_{}", nanos));
    fs::create_dir_all(&dir).expect("failed to create temp dir");
    dir
}

#[test]
fn test_graph_cli_commands_and_ql() {
    let bin = get_bin();
    let tmp = temp_dir();
    let vault_path = tmp.join("graph_vault.s3a");
    let vault_str = vault_path.to_str().unwrap();

    // 1. s3a init
    let init_out = Command::new(&bin)
        .args(["init", vault_str])
        .output()
        .expect("s3a init failed");
    assert!(init_out.status.success(), "s3a init must succeed");

    // 2. s3a graph add
    let add1 = Command::new(&bin)
        .args([
            "graph",
            "add",
            vault_str,
            "paper:A",
            "paper:B",
            "cites",
            "--prop",
            "section=intro",
        ])
        .output()
        .expect("s3a graph add 1 failed");
    assert!(
        add1.status.success(),
        "s3a graph add 1 must succeed: {}",
        String::from_utf8_lossy(&add1.stderr)
    );

    let add2 = Command::new(&bin)
        .args(["graph", "add", vault_str, "paper:B", "paper:C", "cites"])
        .output()
        .expect("s3a graph add 2 failed");
    assert!(add2.status.success(), "s3a graph add 2 must succeed");

    let add3 = Command::new(&bin)
        .args(["graph", "add", vault_str, "paper:A", "paper:D", "mentions"])
        .output()
        .expect("s3a graph add 3 failed");
    assert!(add3.status.success(), "s3a graph add 3 must succeed");

    // 3. s3a graph neighbors
    let neighbors_out = Command::new(&bin)
        .args(["graph", "neighbors", vault_str, "paper:A"])
        .output()
        .expect("s3a graph neighbors failed");
    assert!(neighbors_out.status.success());
    let stdout = String::from_utf8_lossy(&neighbors_out.stdout);
    assert!(stdout.contains("paper:B"));
    assert!(stdout.contains("paper:D"));

    // 3b. s3a graph neighbors --json
    let neighbors_json = Command::new(&bin)
        .args(["graph", "neighbors", vault_str, "paper:A", "--json"])
        .output()
        .expect("s3a graph neighbors --json failed");
    assert!(neighbors_json.status.success());
    let json_str = String::from_utf8_lossy(&neighbors_json.stdout);
    let v: serde_json::Value = serde_json::from_str(&json_str).expect("parse json");
    assert_eq!(v["node"], "paper:A");
    assert_eq!(v["neighbors"].as_array().unwrap().len(), 2);

    // 4. s3a graph traverse
    let traverse_out = Command::new(&bin)
        .args([
            "graph", "traverse", vault_str, "paper:A", "--via", "cites", "--depth", "2",
        ])
        .output()
        .expect("s3a graph traverse failed");
    assert!(traverse_out.status.success());
    let stdout = String::from_utf8_lossy(&traverse_out.stdout);
    assert!(stdout.contains("paper:A"));
    assert!(stdout.contains("paper:B"));
    assert!(stdout.contains("paper:C"));
    assert!(!stdout.contains("paper:D")); // "mentions", not "cites"

    // 4b. s3a graph traverse --json
    let traverse_json = Command::new(&bin)
        .args([
            "graph", "traverse", vault_str, "paper:A", "--via", "cites", "--depth", "2", "--json",
        ])
        .output()
        .expect("s3a graph traverse --json failed");
    assert!(traverse_json.status.success());
    let json_str = String::from_utf8_lossy(&traverse_json.stdout);
    let v: serde_json::Value = serde_json::from_str(&json_str).expect("parse json");
    assert_eq!(v["from"], "paper:A");
    assert_eq!(v["depth"], 2);
    assert_eq!(v["visited_nodes"].as_array().unwrap().len(), 3);

    // 5. s3a graph path
    let path_out = Command::new(&bin)
        .args([
            "graph", "path", vault_str, "paper:A", "paper:C", "--via", "cites",
        ])
        .output()
        .expect("s3a graph path failed");
    assert!(path_out.status.success());
    let stdout = String::from_utf8_lossy(&path_out.stdout);
    assert!(stdout.contains("paper:A -> paper:B -> paper:C"));

    // 5b. s3a graph path --json
    let path_json = Command::new(&bin)
        .args([
            "graph", "path", vault_str, "paper:A", "paper:C", "--via", "cites", "--json",
        ])
        .output()
        .expect("s3a graph path --json failed");
    assert!(path_json.status.success());
    let json_str = String::from_utf8_lossy(&path_json.stdout);
    let v: serde_json::Value = serde_json::from_str(&json_str).expect("parse json");
    assert_eq!(v["found"], true);
    assert_eq!(v["nodes"].as_array().unwrap().len(), 3);

    // 6. S3A-QL queries via s3a query
    // 6a. NEIGHBORS
    let ql_neighbors = Command::new(&bin)
        .args(["query", vault_str, "NEIGHBORS \"paper:A\""])
        .output()
        .expect("s3a query NEIGHBORS failed");
    assert!(
        ql_neighbors.status.success(),
        "ql neighbors failed: {}",
        String::from_utf8_lossy(&ql_neighbors.stderr)
    );
    let stdout = String::from_utf8_lossy(&ql_neighbors.stdout);
    assert!(stdout.contains("paper:B"));

    // 6b. TRAVERSE
    let ql_traverse = Command::new(&bin)
        .args([
            "query",
            vault_str,
            "TRAVERSE FROM \"paper:A\" VIA \"cites\" DEPTH 2",
        ])
        .output()
        .expect("s3a query TRAVERSE failed");
    assert!(
        ql_traverse.status.success(),
        "ql traverse failed: {}",
        String::from_utf8_lossy(&ql_traverse.stderr)
    );
    let stdout = String::from_utf8_lossy(&ql_traverse.stdout);
    assert!(stdout.contains("paper:C"));

    // 6c. PATH with --json
    let ql_path_json = Command::new(&bin)
        .args([
            "query",
            vault_str,
            "PATH \"paper:A\" TO \"paper:C\" VIA \"cites\"",
            "--json",
        ])
        .output()
        .expect("s3a query PATH --json failed");
    assert!(
        ql_path_json.status.success(),
        "ql path failed: {}",
        String::from_utf8_lossy(&ql_path_json.stderr)
    );
    let json_str = String::from_utf8_lossy(&ql_path_json.stdout);
    let v: serde_json::Value = serde_json::from_str(&json_str).expect("parse json");
    assert_eq!(v["found"], true);

    // 7. s3a help graph
    let help_graph = Command::new(&bin)
        .args(["help", "graph"])
        .output()
        .expect("s3a help graph failed");
    assert!(help_graph.status.success());
    let stdout = String::from_utf8_lossy(&help_graph.stdout);
    assert!(stdout.contains("s3a graph neighbors"));
    assert!(stdout.contains("s3a graph path"));
    assert!(stdout.contains("s3a graph traverse"));

    let _ = fs::remove_dir_all(&tmp);
}
