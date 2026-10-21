use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use s3a_core::columnar::{ColumnDataType, ColumnarTileBuilder, ColumnarTileView};
use s3a_core::{TelemetryRecord, TileType};
use s3a_engine::lakehouse::{project_columnar_to_arrow, ArrowColumnVector};
use s3a_engine::{
    decode_columnar_telemetry, encode_columnar_telemetry, Compactor, MmapReader, QuerySieve,
    S3ACrudEngine,
};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TestTempFile {
    path: PathBuf,
}

impl TestTempFile {
    fn new(suffix: &str) -> Self {
        let count = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let mut path = std::env::temp_dir();
        path.push(format!(
            "s3a_columnar_test_{}_{}_{}",
            std::process::id(),
            count,
            suffix
        ));
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestTempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[test]
fn test_roundtrip_equality_vs_record_tile() {
    let mut records = Vec::new();
    for i in 0..100 {
        records.push(TelemetryRecord {
            timestamp: 1_000_000 + i * 10,
            sensor_id: (i % 5) as u32,
            metric_id: (i % 2) as u32,
            value: (i as f64) * 1.5,
            flags: 1,
        });
    }

    // Encode to columnar payload
    let columnar_payload = encode_columnar_telemetry(&records);
    assert!(!columnar_payload.is_empty());

    // Decode from columnar payload back to row records
    let decoded = decode_columnar_telemetry(&columnar_payload)
        .expect("decoding columnar telemetry must succeed");
    assert_eq!(decoded.len(), records.len());
    assert_eq!(decoded, records);
}

#[test]
fn test_column_projection_reads_only_requested_columns() {
    let mut records = Vec::new();
    for i in 0..50 {
        records.push(TelemetryRecord {
            timestamp: 1000 + i,
            sensor_id: i as u32,
            metric_id: 1,
            value: 42.0,
            flags: 1,
        });
    }

    let columnar_payload = encode_columnar_telemetry(&records);

    // Project only timestamp and value
    let batch = project_columnar_to_arrow(&columnar_payload, Some(&["timestamp", "value"]))
        .expect("projection must succeed");

    assert_eq!(batch.num_rows, 50);
    assert_eq!(batch.columns.len(), 2);
    assert_eq!(batch.schema.fields[0].name, "timestamp");
    assert_eq!(batch.schema.fields[1].name, "value");

    // Project only sensor_id
    let batch_sensor = project_columnar_to_arrow(&columnar_payload, Some(&["sensor_id"]))
        .expect("projection must succeed");
    assert_eq!(batch_sensor.columns.len(), 1);
    assert_eq!(batch_sensor.schema.fields[0].name, "sensor_id");
}

#[test]
fn test_min_max_pruning_skips_tiles() {
    let mut records = Vec::new();
    for i in 0..20 {
        records.push(TelemetryRecord {
            timestamp: 1000 + i,
            sensor_id: 10, // constant 10
            metric_id: 5,  // constant 5
            value: 50.0 + (i as f64),
            flags: 1,
        });
    }

    let columnar_payload = encode_columnar_telemetry(&records);
    let view = ColumnarTileView::parse(&columnar_payload).expect("view parse must succeed");

    // Query sensor_id == 99 -> disjoint, can prune
    assert!(view.can_prune_column("sensor_id", 99.0, 99.0));
    // Query sensor_id == 10 -> overlaps, cannot prune
    assert!(!view.can_prune_column("sensor_id", 10.0, 10.0));

    // Query value between [0.0, 40.0] -> disjoint (min is 50.0), can prune
    assert!(view.can_prune_column("value", 0.0, 40.0));
    // Query value between [55.0, 60.0] -> overlaps, cannot prune
    assert!(!view.can_prune_column("value", 55.0, 60.0));
}

#[test]
fn test_null_handling() {
    let mut builder = ColumnarTileBuilder::new(10, 0x1234);

    let scores = [1.0f64, 2.0, 0.0, 4.0, 5.0, 0.0, 7.0, 8.0, 9.0, 10.0];
    let ids = [10u32, 20, 30, 40, 50, 60, 70, 80, 90, 100];

    // Validity mask: row 2 and 5 are null (false)
    let mut validity = vec![true; 10];
    validity[2] = false;
    validity[5] = false;

    // Column 0: nullable float64
    builder.add_column(
        "scores",
        ColumnDataType::Float64,
        &scores,
        Some(&validity),
        1.0,
        10.0,
    );
    // Column 1: non-nullable uint32
    builder.add_column("ids", ColumnDataType::UInt32, &ids, None, 10.0, 100.0);

    let payload = builder.build();
    let view = ColumnarTileView::parse(&payload).expect("view parse must succeed");

    let desc0 = view
        .find_column("scores")
        .expect("scores column must exist");
    assert_eq!(desc0.null_count, 2);

    // Row 2 is null
    assert!(!view.is_valid(desc0, 2));
    // Row 5 is null
    assert!(!view.is_valid(desc0, 5));
    // Row 0 is valid
    assert!(view.is_valid(desc0, 0));

    assert!(view.verify_column_hash(desc0));
}

#[test]
fn test_arrow_export_matches() {
    let mut records = Vec::new();
    for i in 0..10 {
        records.push(TelemetryRecord {
            timestamp: 500 + i,
            sensor_id: i as u32,
            metric_id: 1,
            value: (i as f64) * 10.0,
            flags: 1,
        });
    }

    let columnar_payload = encode_columnar_telemetry(&records);
    let batch = project_columnar_to_arrow(
        &columnar_payload,
        Some(&["timestamp", "sensor_id", "value"]),
    )
    .expect("arrow export must succeed");

    assert_eq!(batch.num_rows, 10);
    assert_eq!(batch.columns.len(), 3);

    if let ArrowColumnVector::UInt64(ref ts_vec) = batch.columns[0] {
        for (i, &ts) in ts_vec.iter().enumerate() {
            assert_eq!(ts, 500 + i as u64);
        }
    } else {
        panic!("expected UInt64 for timestamp");
    }

    if let ArrowColumnVector::UInt32(ref s_vec) = batch.columns[1] {
        for (i, &s) in s_vec.iter().enumerate() {
            assert_eq!(s, i as u32);
        }
    } else {
        panic!("expected UInt32 for sensor_id");
    }

    if let ArrowColumnVector::Float64(ref val_vec) = batch.columns[2] {
        for (i, &v) in val_vec.iter().enumerate() {
            assert_eq!(v, (i as f64) * 10.0);
        }
    } else {
        panic!("expected Float64 for value");
    }
}

#[test]
fn test_compaction_opt_in_columnar_roundtrip() {
    let temp_uncompacted = TestTempFile::new("uncompacted.s3a");
    let uncompacted_path = temp_uncompacted.path().to_path_buf();
    let temp_compacted = TestTempFile::new("compacted.s3a");
    let compacted_path = temp_compacted.path().to_path_buf();

    let engine = S3ACrudEngine::open_or_create(&uncompacted_path).unwrap();

    let mut records = Vec::new();
    for i in 0..50 {
        records.push(TelemetryRecord::new(
            10_000 + i * 5,
            42,
            7,
            (i as f64) * 2.5,
        ));
    }

    engine.create_telemetry(&records).unwrap();

    // Verify row query works before compaction
    let pre_query = engine.read_telemetry(42, 7, 0, 100_000).unwrap();
    assert_eq!(pre_query.len(), 50);

    // Compact with opt-in for TileType::TELEMETRY -> converts to TileType::COLUMNAR (Tile 65)
    Compactor::compact_with_columnar_opt_in(
        &[&uncompacted_path],
        &compacted_path,
        &[TileType::TELEMETRY],
    )
    .expect("compaction must succeed");

    // Inspect compacted archive: verify TileType::COLUMNAR is present
    let reader = MmapReader::open(&compacted_path).expect("compacted archive must open");
    let mut found_columnar = false;
    for i in 0..reader.tile_count() as usize {
        if let Some((hdr, _)) = reader.get_tile(i) {
            if hdr.tile_type == TileType::COLUMNAR {
                found_columnar = true;
                assert_eq!(hdr.record_count, 50);
            }
        }
    }
    assert!(
        found_columnar,
        "Compacted archive must contain TileType::COLUMNAR"
    );

    // Query engine reading compacted file: sieve should transparently decode columnar tiles and yield identical data
    let sieve = QuerySieve::new(&reader);
    let post_query = sieve.query_telemetry(0, 100_000, Some(42), Some(7));
    assert_eq!(post_query.len(), 50);
    for (i, rec) in post_query.iter().enumerate() {
        assert_eq!(rec.timestamp, records[i].timestamp);
        assert_eq!(rec.sensor_id, records[i].sensor_id);
        assert_eq!(rec.metric_id, records[i].metric_id);
        assert_eq!(rec.value, records[i].value);
    }
}
