//! Integration test verifying all QUICKSTART.md workflow steps end-to-end.

use std::fs;
use std::process::Command;

fn get_bin() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_s3a") {
        return std::path::PathBuf::from(p);
    }
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_s3a-cli") {
        return std::path::PathBuf::from(p);
    }
    panic!("Neither CARGO_BIN_EXE_s3a nor CARGO_BIN_EXE_s3a-cli is set");
}

#[test]
fn test_quickstart_full_flow() {
    let bin = get_bin();
    let temp_dir = std::env::temp_dir().join(format!("s3a_quickstart_{}", uuid_simple()));
    fs::create_dir_all(&temp_dir).expect("failed to create temp dir");

    // Step 1: s3a init
    let init_out = Command::new(&bin)
        .arg("init")
        .current_dir(&temp_dir)
        .output()
        .expect("s3a init failed to run");
    assert!(init_out.status.success(), "s3a init must succeed");
    let init_str = String::from_utf8_lossy(&init_out.stdout);
    assert!(
        init_str.contains("Initialized new S3A vault"),
        "init output must mention vault initialization: {}",
        init_str
    );
    assert!(
        temp_dir.join("my.s3a").exists(),
        "default vault ./my.s3a must exist"
    );

    // Step 2: s3a put
    let put_out = Command::new(&bin)
        .args(["put", "user:101", r#"{"name":"Alice","role":"engineer","active":true}"#])
        .current_dir(&temp_dir)
        .output()
        .expect("s3a put failed to run");
    assert!(put_out.status.success(), "s3a put must succeed");
    let put_str = String::from_utf8_lossy(&put_out.stdout);
    assert!(
        put_str.contains("OK: stored 'user:101'"),
        "put output unexpected: {}",
        put_str
    );

    // Step 3: s3a get
    let get_out = Command::new(&bin)
        .args(["get", "user:101"])
        .current_dir(&temp_dir)
        .output()
        .expect("s3a get failed to run");
    assert!(get_out.status.success(), "s3a get must succeed");
    let get_str = String::from_utf8_lossy(&get_out.stdout);
    assert!(
        get_str.contains("Alice") && get_str.contains("engineer"),
        "get output must contain stored JSON content: {}",
        get_str
    );

    // Step 4: s3a import CSV
    let csv_path = temp_dir.join("metrics.csv");
    fs::write(
        &csv_path,
        "timestamp,sensor_id,metric_id,value\n1000,42,1,98.6\n1010,42,1,99.1\n1020,42,1,97.4\n",
    )
    .expect("write metrics.csv");

    let import_out = Command::new(&bin)
        .args(["import", "metrics.csv"])
        .current_dir(&temp_dir)
        .output()
        .expect("s3a import failed to run");
    assert!(import_out.status.success(), "s3a import must succeed");
    let import_str = String::from_utf8_lossy(&import_out.stdout);
    assert!(
        import_str.contains("Imported 3 telemetry records"),
        "import output unexpected: {}",
        import_str
    );

    // Step 5: s3a query table output
    let query_out = Command::new(&bin)
        .args(["query", "SIFT TELEMETRY WHERE SENSOR_ID = 42"])
        .current_dir(&temp_dir)
        .output()
        .expect("s3a query failed to run");
    assert!(query_out.status.success(), "s3a query must succeed");
    let query_str = String::from_utf8_lossy(&query_out.stdout);
    assert!(
        query_str.contains("Timestamp") && query_str.contains("Sensor ID"),
        "query output must contain table headers: {}",
        query_str
    );

    // Step 5b: s3a query --json
    let query_json_out = Command::new(&bin)
        .args(["query", "SIFT TELEMETRY WHERE SENSOR_ID = 42", "--json"])
        .current_dir(&temp_dir)
        .output()
        .expect("s3a query --json failed to run");
    assert!(query_json_out.status.success(), "s3a query --json must succeed");
    let query_json_str = String::from_utf8_lossy(&query_json_out.stdout);
    assert!(
        query_json_str.contains(r#""ok":true"#),
        "query --json must contain valid JSON status: {}",
        query_json_str
    );

    // Step 6: s3a export
    let export_out = Command::new(&bin)
        .args(["export", "backup.csv"])
        .current_dir(&temp_dir)
        .output()
        .expect("s3a export failed to run");
    assert!(export_out.status.success(), "s3a export must succeed");
    assert!(
        temp_dir.join("backup.csv").exists(),
        "exported backup.csv must exist"
    );

    // Step 7: s3a info
    let info_out = Command::new(&bin)
        .arg("info")
        .current_dir(&temp_dir)
        .output()
        .expect("s3a info failed to run");
    assert!(info_out.status.success(), "s3a info must succeed");
    let info_str = String::from_utf8_lossy(&info_out.stdout);
    assert!(
        info_str.contains("S3A VAULT INFORMATION"),
        "info output unexpected: {}",
        info_str
    );

    // Step 8: s3a tour
    let tour_out = Command::new(&bin)
        .arg("tour")
        .current_dir(&temp_dir)
        .output()
        .expect("s3a tour failed to run");
    assert!(tour_out.status.success(), "s3a tour must succeed");
    let tour_str = String::from_utf8_lossy(&tour_out.stdout);
    assert!(
        tour_str.contains("S3A ARCHIVE TOUR"),
        "tour output unexpected: {}",
        tour_str
    );

    // Step 9: s3a demo
    let demo_out = Command::new(&bin)
        .args(["demo", "demo.s3a"])
        .current_dir(&temp_dir)
        .output()
        .expect("s3a demo failed to run");
    assert!(demo_out.status.success(), "s3a demo must succeed");
    assert!(
        temp_dir.join("demo.s3a").exists(),
        "sample vault demo.s3a must exist"
    );

    // Step 10: s3a ls
    let ls_out = Command::new(&bin)
        .args(["ls", "--vault", "demo.s3a"])
        .current_dir(&temp_dir)
        .output()
        .expect("s3a ls failed to run");
    assert!(ls_out.status.success(), "s3a ls must succeed");
    let ls_str = String::from_utf8_lossy(&ls_out.stdout);
    assert!(
        ls_str.contains("book:") || ls_str.contains("KEY"),
        "ls output must list records: {}",
        ls_str
    );

    // Clean up
    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_errors_and_suggestions() {
    let bin = get_bin();
    let temp_dir = std::env::temp_dir().join(format!("s3a_err_{}", uuid_simple()));
    fs::create_dir_all(&temp_dir).expect("failed to create temp dir");

    // Mistyped command: initt -> suggests init
    let typo_out = Command::new(&bin)
        .arg("initt")
        .current_dir(&temp_dir)
        .output()
        .expect("command execution");
    assert!(!typo_out.status.success(), "typo command must exit with failure");
    let typo_err = String::from_utf8_lossy(&typo_out.stderr);
    assert!(
        typo_err.contains("Did you mean 'init'?"),
        "error must suggest 'init': {}",
        typo_err
    );

    // Missing key in get:
    let get_missing_out = Command::new(&bin)
        .arg("get")
        .current_dir(&temp_dir)
        .output()
        .expect("command execution");
    assert!(!get_missing_out.status.success());
    let get_missing_err = String::from_utf8_lossy(&get_missing_out.stderr);
    assert!(
        get_missing_err.contains("requires <key>"),
        "error must explain missing key: {}",
        get_missing_err
    );

    // Key not found error says what went wrong and how to fix:
    let _ = Command::new(&bin).arg("init").current_dir(&temp_dir).output();
    let not_found_out = Command::new(&bin)
        .args(["get", "nonexistent_key_12345"])
        .current_dir(&temp_dir)
        .output()
        .expect("command execution");
    assert!(!not_found_out.status.success());
    let not_found_err = String::from_utf8_lossy(&not_found_out.stderr);
    assert!(
        not_found_err.contains("not found in vault"),
        "error must explain key not found: {}",
        not_found_err
    );
    assert!(
        not_found_err.contains("fix:"),
        "error must provide likely fix: {}",
        not_found_err
    );

    // Help for specific command shows copy-paste examples:
    let help_out = Command::new(&bin)
        .args(["help", "query"])
        .current_dir(&temp_dir)
        .output()
        .expect("command execution");
    assert!(help_out.status.success());
    let help_str = String::from_utf8_lossy(&help_out.stdout);
    assert!(
        help_str.contains("Examples:") && help_str.contains("SIFT TELEMETRY"),
        "help query must show copy-paste examples: {}",
        help_str
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_telemetry_get_split_from_simple_get() {
    let bin = get_bin();
    let temp_dir = std::env::temp_dir().join(format!("s3a_telemetry_split_{}", uuid_simple()));
    fs::create_dir_all(&temp_dir).expect("failed to create temp dir");

    // Seed demo vault
    let demo_out = Command::new(&bin)
        .arg("demo")
        .current_dir(&temp_dir)
        .output()
        .expect("demo run");
    assert!(demo_out.status.success());

    // Simple get retrieves key-value document
    let get_out = Command::new(&bin)
        .args(["get", "book:1", "--vault", "demo.s3a"])
        .current_dir(&temp_dir)
        .output()
        .expect("simple get run");
    assert!(get_out.status.success());
    let get_str = String::from_utf8_lossy(&get_out.stdout);
    assert!(get_str.contains("The Rust Programming Language"));

    // Positional <vault> <key> form
    let get_pos_out = Command::new(&bin)
        .args(["get", "demo.s3a", "book:1"])
        .current_dir(&temp_dir)
        .output()
        .expect("positional vault get run");
    assert!(get_pos_out.status.success());
    let get_pos_str = String::from_utf8_lossy(&get_pos_out.stdout);
    assert!(get_pos_str.contains("The Rust Programming Language"));

    // Dedicated telemetry get retrieves sensor telemetry records
    let tel_out = Command::new(&bin)
        .args(["telemetry", "get", "demo.s3a", "42", "1", "0", "2000000000"])
        .current_dir(&temp_dir)
        .output()
        .expect("telemetry get run");
    assert!(tel_out.status.success());
    let tel_str = String::from_utf8_lossy(&tel_out.stdout);
    assert!(tel_str.contains("sensor=42"));
    assert!(tel_str.contains("metric=1"));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_put_get_hash_agreement_and_lrs_interop() {
    let bin = get_bin();
    let temp_dir = std::env::temp_dir().join(format!("s3a_hash_agree_{}", uuid_simple()));
    fs::create_dir_all(&temp_dir).expect("failed to create temp dir");

    let _ = Command::new(&bin).arg("init").current_dir(&temp_dir).output().expect("init");

    // Put a document
    let put_out = Command::new(&bin)
        .args(["put", "config:main", r#"{"threads":8,"cluster":"prod"}"#])
        .current_dir(&temp_dir)
        .output()
        .expect("put");
    assert!(put_out.status.success());

    // Get the document back via CLI
    let get_out = Command::new(&bin)
        .args(["get", "config:main"])
        .current_dir(&temp_dir)
        .output()
        .expect("get");
    assert!(get_out.status.success());
    let get_str = String::from_utf8_lossy(&get_out.stdout);
    assert!(get_str.contains(r#"{"threads":8,"cluster":"prod"}"#));

    // Verify hash agreement between s3a-core and s3a-engine
    let vault_path = temp_dir.join("my.s3a");
    let key = "config:main";
    let core_hash = s3a_core::document::compute_key_hash(s3a_core::document::DOC_KIND_JSON, key);
    let engine_hash = s3a_engine::LrsStore::document_key_hash(s3a_core::document::DOC_KIND_JSON, key);
    assert_eq!(core_hash, engine_hash, "Core and engine document key hashes must be identical");

    // Verify engine LrsStore can open the vault and retrieve the document using that exact hash
    let store = s3a_engine::LrsStore::open_or_create(&vault_path).expect("engine store open");
    let retrieved = store.get_document(s3a_core::document::DOC_KIND_JSON, key);
    assert!(retrieved.is_some(), "Engine LrsStore must find document written by 's3a put'");
    let (hdr, content, k) = retrieved.unwrap();
    assert_eq!(hdr.key_hash, core_hash);
    assert_eq!(k, key);
    assert_eq!(String::from_utf8_lossy(&content), r#"{"threads":8,"cluster":"prod"}"#);

    let _ = fs::remove_dir_all(&temp_dir);
}

fn uuid_simple() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    ((d.as_secs() as u128) << 64) | (d.subsec_nanos() as u128)
}
