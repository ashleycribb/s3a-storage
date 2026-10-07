use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::path::PathBuf;

use s3a_engine::ql::{
    execute_query, register_pack_sift_handler, Lexer, Parser, QueryResult, SiftDomain,
};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(100);

fn temp_archive_path(label: &str) -> PathBuf {
    let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut p = std::env::temp_dir();
    p.push(format!("s3a_ql_test_{}_{}_{}.s3a", label, std::process::id(), id));
    p
}

struct TempArchiveGuard(PathBuf);
impl Drop for TempArchiveGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn test_version_preamble_and_ast() {
    let p = temp_archive_path("version_preamble");
    let guard = TempArchiveGuard(p.clone());
    let path_str = p.to_str().unwrap();

    // 1. Plain VERSION preamble
    let query1 = format!("VERSION 1; FETCH RECORD AT L0:T0:R0 FROM \"{}\"", path_str);
    let mut lexer = Lexer::new(&query1);
    let tokens = lexer.tokenize_all().unwrap();
    let mut parser = Parser::new(tokens);
    let stmt = parser.parse_statement().unwrap();
    assert_eq!(stmt.version(), Some(1));

    // 2. PRAGMA VERSION preamble
    let query2 = format!("PRAGMA VERSION 1; SIFT TELEMETRY FROM \"{}\" WHERE TIME BETWEEN 100 AND 200", path_str);
    let mut lexer2 = Lexer::new(&query2);
    let tokens2 = lexer2.tokenize_all().unwrap();
    let mut parser2 = Parser::new(tokens2);
    let stmt2 = parser2.parse_statement().unwrap();
    assert_eq!(stmt2.version(), Some(1));

    drop(guard);
}

#[test]
fn test_ql_error_codes_and_json_serialization() {
    // 1. Parse error
    let bad_query = "SIFT NOT_A_DOMAIN FROM \"test.s3a\"";
    let err = execute_query(bad_query).unwrap_err();
    assert_eq!(err.code(), "parse_error");
    let json = err.to_json();
    assert!(json.contains("\"code\":\"parse_error\""));
    assert!(json.contains("\"ok\":false"));

    // 2. Pack required error for unhandled pack domain
    let kinematics_query = "SIFT KINEMATICS FROM \"non_existent.s3a\"";
    // Without registered handler, returns PackRequired
    let err_kinematics = execute_query(kinematics_query).unwrap_err();
    assert_eq!(err_kinematics.code(), "pack_required");
    let json_k = err_kinematics.to_json();
    assert!(json_k.contains("\"code\":\"pack_required\""));
    assert!(json_k.contains("\"pack\":\"s3a-pack-robotics\""));
    assert!(json_k.contains("\"feature\":\"kinematics\""));

    // 3. Not found error
    let p = temp_archive_path("not_found");
    let guard = TempArchiveGuard(p.clone());
    let path_str = p.to_str().unwrap();

    // Insert 1 record to make a valid archive
    let ins = format!("INSERT TELEMETRY (1000, 1, 1, 10.0) INTO \"{}\"", path_str);
    let _ = execute_query(&ins).unwrap();

    let fetch_out_of_bounds = format!("FETCH RECORD AT L0:T999:R0 FROM \"{}\"", path_str);
    let err_fetch = execute_query(&fetch_out_of_bounds).unwrap_err();
    assert_eq!(err_fetch.code(), "not_found");
    let json_f = err_fetch.to_json();
    assert!(json_f.contains("\"code\":\"not_found\""));
    assert!(json_f.contains("L0:T999:R0"));

    drop(guard);
}

#[test]
fn test_core_query_execution_lifecycle() {
    let p = temp_archive_path("lifecycle");
    let guard = TempArchiveGuard(p.clone());
    let path_str = p.to_str().unwrap();

    // 1. Insert
    let insert_ql = format!("INSERT TELEMETRY (1000, 42, 99, 123.45) INTO \"{}\"", path_str);
    let res_insert = execute_query(&insert_ql).unwrap();
    match res_insert {
        QueryResult::Inserted { coordinate } => {
            assert_eq!(coordinate.tile_id, 0);
        }
        other => panic!("Expected Inserted, got {:?}", other),
    }

    // 2. Fetch
    let fetch_ql = format!("FETCH RECORD AT L0:T0:R0 FROM \"{}\"", path_str);
    let res_fetch = execute_query(&fetch_ql).unwrap();
    match res_fetch {
        QueryResult::Telemetry(rec) => {
            assert_eq!(rec.timestamp, 1000);
            assert_eq!(rec.sensor_id, 42);
            assert_eq!(rec.metric_id, 99);
            assert!((rec.value - 123.45).abs() < 1e-4);
        }
        other => panic!("Expected Telemetry, got {:?}", other),
    }

    // 3. Sift Telemetry with Limit
    let sift_ql = format!(
        "SIFT TELEMETRY FROM \"{}\" WHERE TIME BETWEEN 500 AND 1500 AND SENSOR_ID = 42 LIMIT 5",
        path_str
    );
    let res_sift = execute_query(&sift_ql).unwrap();
    match res_sift {
        QueryResult::TelemetryList(list) => {
            assert_eq!(list.len(), 1);
            assert_eq!(list[0].0.sensor_id, 42);
        }
        other => panic!("Expected TelemetryList, got {:?}", other),
    }

    // 4. Delete
    let delete_ql = format!(
        "DELETE TELEMETRY WHERE SENSOR_ID = 42 AND METRIC_ID = 99 AND TIME = 1000 FROM \"{}\"",
        path_str
    );
    let res_del = execute_query(&delete_ql).unwrap();
    match res_del {
        QueryResult::Deleted { success } => assert!(success),
        other => panic!("Expected Deleted, got {:?}", other),
    }

    // 5. Compact
    let compact_ql = format!("COMPACT ARCHIVE \"{}\"", path_str);
    let res_comp = execute_query(&compact_ql).unwrap();
    match res_comp {
        QueryResult::Compacted { new_tile_count } => {
            assert_eq!(new_tile_count, 0); // Deleted record was purged
        }
        other => panic!("Expected Compacted, got {:?}", other),
    }

    drop(guard);
}

#[test]
fn test_pack_routing_with_registered_handler() {
    let p = temp_archive_path("pack_routing");
    let guard = TempArchiveGuard(p.clone());
    let path_str = p.to_str().unwrap();

    // Register a mock handler for DA Commitments
    register_pack_sift_handler(
        SiftDomain::DaCommitments,
        Arc::new(|sift| {
            Ok(QueryResult::PackData {
                domain: sift.domain,
                count: 7,
                summary: format!("Mock DA handler executed for '{}'", sift.from_path),
                json_payload: Some(r#"{"mock":true,"block_count":7}"#.to_string()),
            })
        }),
    );

    let da_query = format!("SIFT DA_COMMITMENTS FROM \"{}\"", path_str);
    let res = execute_query(&da_query).unwrap();
    match res {
        QueryResult::PackData { domain, count, summary, json_payload } => {
            assert_eq!(domain, SiftDomain::DaCommitments);
            assert_eq!(count, 7);
            assert!(summary.contains("Mock DA handler executed"));
            assert_eq!(json_payload, Some(r#"{"mock":true,"block_count":7}"#.to_string()));
        }
        other => panic!("Expected PackData, got {:?}", other),
    }

    drop(guard);
}
