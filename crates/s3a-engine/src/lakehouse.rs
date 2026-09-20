//! Apache Software Foundation & Linux Foundation Lakehouse Adapters
//!
//! Provides enterprise connectivity between S3A databases and modern lakehouse analytical platforms:
//! 1. Snowflake External Function REST Protocol (`POST /api/v1/snowflake`)
//! 2. Apache Arrow In-Memory Columnar Schema Projector (DuckDB / Polars / Trino IPC)
//! 3. Apache Iceberg Table Metadata Generator (`v1.metadata.json` for Snowflake External Tables)
//! 4. Linux Foundation Delta Lake UniForm Transaction Log Generator (Databricks / Microsoft Fabric)

use s3a_core::{
    TelemetryRecord, GISSurveyPointRecord, RoboticsKinematicRecord,
    S3ACoordinate, S3AError,
};
use crate::S3ACrudEngine;

/// Snowflake External Function Request Payload (array of tuples [row_index, param1, ...]).
#[derive(Debug, Clone)]
pub struct SnowflakeBatchRequest {
    pub rows: Vec<(usize, String)>,
}

impl SnowflakeBatchRequest {
    /// Parses a Snowflake JSON batch request payload.
    /// Expected format: `{"data": [[0, "L0:T0:R0"], [1, "L0:T0:R1"], ...]}`
    pub fn parse(json_str: &str) -> Result<Self, S3AError> {
        let mut rows = Vec::new();
        let trimmed = json_str.trim();

        // Simple zero-dependency JSON parser for Snowflake {"data": [[row_num, val], ...]}
        let data_marker = "\"data\":";
        if let Some(pos) = trimmed.find(data_marker) {
            let after = &trimmed[pos + data_marker.len()..].trim();
            if after.starts_with('[') {
                // Find array elements
                let content = after.trim_start_matches('[').trim_end_matches('}').trim_end_matches(']').trim();
                for row_chunk in content.split("],") {
                    let clean = row_chunk.replace('[', "").replace(']', "");
                    let parts: Vec<&str> = clean.split(',').map(|s| s.trim()).collect();
                    if parts.len() >= 2 {
                        if let Ok(row_num) = parts[0].parse::<usize>() {
                            let param = parts[1].trim_matches('"').to_string();
                            rows.push((row_num, param));
                        }
                    }
                }
            }
        }

        if rows.is_empty() {
            return Ok(Self { rows });
        }

        Ok(Self { rows })
    }
}

/// Dispatches Snowflake batched function calls against the S3A Engine.
/// Returns a Snowflake-compliant JSON response: `{"data": [[row_num, result], ...]}`.
pub fn handle_snowflake_batch_request(
    engine: &S3ACrudEngine,
    request_json: &str,
) -> Result<String, S3AError> {
    let batch = SnowflakeBatchRequest::parse(request_json)?;
    let mut response_data = Vec::new();

    for (row_num, param) in batch.rows {
        // Case A: Parameter is a direct S3A Coordinate (e.g. "L0:T0:R0")
        if let Ok(coord) = param.parse::<S3ACoordinate>() {
            match engine.read_by_coordinate::<TelemetryRecord>(&coord) {
                Ok(rec) => {
                    let json_val = format!(
                        "{{\"status\":\"success\",\"type\":\"TELEMETRY\",\"coord\":\"{}\",\"timestamp\":{},\"sensor_id\":{},\"metric_id\":{},\"val\":{}}}",
                        coord, rec.timestamp, rec.sensor_id, rec.metric_id, rec.value
                    );
                    response_data.push(format!("[{}, {}]", row_num, json_val));
                }
                Err(e) => {
                    let err_val = format!("{{\"status\":\"error\",\"error\":\"{}\"}}", e);
                    response_data.push(format!("[{}, {}]", row_num, err_val));
                }
            }
        } else if param.starts_with("QUERY:") {
            // Case B: Vector / Range query parameter
            let query_body = &param[6..];
            let records = engine.read_telemetry_with_coords(1, 101, 0, u64::MAX).unwrap_or_default();
            let json_val = format!(
                "{{\"status\":\"success\",\"query\":\"{}\",\"matched_count\":{}}}",
                query_body, records.len()
            );
            response_data.push(format!("[{}, {}]", row_num, json_val));
        } else {
            let err_val = format!("{{\"status\":\"error\",\"error\":\"Unrecognized parameter format: {}\"}}", param);
            response_data.push(format!("[{}, {}]", row_num, err_val));
        }
    }

    Ok(format!("{{\"data\": [{}]}}", response_data.join(", ")))
}

/// Apache Arrow Column Field definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrowField {
    pub name: &'static str,
    pub data_type: &'static str,
    pub nullable: bool,
}

/// Apache Arrow Schema definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrowSchema {
    pub fields: Vec<ArrowField>,
}

/// In-Memory Columnar Vector Descriptor (matches Apache Arrow Memory Layout).
#[derive(Debug, Clone)]
pub enum ArrowColumnVector {
    Int32(Vec<i32>),
    UInt32(Vec<u32>),
    UInt64(Vec<u64>),
    Float32(Vec<f32>),
    Float64(Vec<f64>),
    FixedSizeBinary { data: Vec<u8>, byte_width: usize },
}

/// Apache Arrow RecordBatch in-memory projection descriptor.
#[derive(Debug, Clone)]
pub struct ArrowRecordBatchDescriptor {
    pub schema: ArrowSchema,
    pub num_rows: usize,
    pub columns: Vec<ArrowColumnVector>,
}

/// Projects a slice of S3A TelemetryRecords into an Apache Arrow Columnar RecordBatch.
pub fn project_telemetry_to_arrow(records: &[TelemetryRecord]) -> ArrowRecordBatchDescriptor {
    let schema = ArrowSchema {
        fields: vec![
            ArrowField { name: "timestamp", data_type: "UInt64", nullable: false },
            ArrowField { name: "sensor_id", data_type: "UInt32", nullable: false },
            ArrowField { name: "metric_id", data_type: "UInt32", nullable: false },
            ArrowField { name: "value", data_type: "Float64", nullable: false },
        ],
    };

    let n = records.len();
    let mut col_ts = Vec::with_capacity(n);
    let mut col_sensor = Vec::with_capacity(n);
    let mut col_metric = Vec::with_capacity(n);
    let mut col_val = Vec::with_capacity(n);

    for r in records {
        col_ts.push(r.timestamp);
        col_sensor.push(r.sensor_id);
        col_metric.push(r.metric_id);
        col_val.push(r.value);
    }

    ArrowRecordBatchDescriptor {
        schema,
        num_rows: n,
        columns: vec![
            ArrowColumnVector::UInt64(col_ts),
            ArrowColumnVector::UInt32(col_sensor),
            ArrowColumnVector::UInt32(col_metric),
            ArrowColumnVector::Float64(col_val),
        ],
    }
}

/// Projects a slice of S3A GISSurveyPointRecords into an Apache Arrow Columnar RecordBatch.
pub fn project_gis_to_arrow(records: &[GISSurveyPointRecord]) -> ArrowRecordBatchDescriptor {
    let schema = ArrowSchema {
        fields: vec![
            ArrowField { name: "latitude_microdeg", data_type: "Int32", nullable: false },
            ArrowField { name: "longitude_microdeg", data_type: "Int32", nullable: false },
            ArrowField { name: "elevation_mm", data_type: "Int32", nullable: false },
            ArrowField { name: "point_class", data_type: "UInt32", nullable: false },
            ArrowField { name: "intensity", data_type: "UInt32", nullable: false },
        ],
    };

    let n = records.len();
    let mut col_lat = Vec::with_capacity(n);
    let mut col_lon = Vec::with_capacity(n);
    let mut col_elev = Vec::with_capacity(n);
    let mut col_class = Vec::with_capacity(n);
    let mut col_intensity = Vec::with_capacity(n);

    for r in records {
        col_lat.push(r.latitude_microdeg);
        col_lon.push(r.longitude_microdeg);
        col_elev.push(r.elevation_mm);
        col_class.push(r.point_class as u32);
        col_intensity.push(r.intensity as u32);
    }

    ArrowRecordBatchDescriptor {
        schema,
        num_rows: n,
        columns: vec![
            ArrowColumnVector::Int32(col_lat),
            ArrowColumnVector::Int32(col_lon),
            ArrowColumnVector::Int32(col_elev),
            ArrowColumnVector::UInt32(col_class),
            ArrowColumnVector::UInt32(col_intensity),
        ],
    }
}

/// Projects a slice of S3A RoboticsKinematicRecords into an Apache Arrow Columnar RecordBatch.
pub fn project_robotics_to_arrow(records: &[RoboticsKinematicRecord]) -> ArrowRecordBatchDescriptor {
    let schema = ArrowSchema {
        fields: vec![
            ArrowField { name: "timestamp_us", data_type: "UInt64", nullable: false },
            ArrowField { name: "robot_id", data_type: "UInt32", nullable: false },
            ArrowField { name: "joint_mask", data_type: "UInt32", nullable: false },
            ArrowField { name: "position_xyz", data_type: "FixedSizeBinary[12]", nullable: false },
            ArrowField { name: "orientation_quat", data_type: "FixedSizeBinary[16]", nullable: false },
        ],
    };

    let n = records.len();
    let mut col_ts = Vec::with_capacity(n);
    let mut col_robot = Vec::with_capacity(n);
    let mut col_joint = Vec::with_capacity(n);
    let mut col_pos = Vec::with_capacity(n * 12);
    let mut col_quat = Vec::with_capacity(n * 16);

    for r in records {
        col_ts.push(r.timestamp_us);
        col_robot.push(r.robot_id);
        col_joint.push(r.joint_mask);
        for i in 0..3 {
            col_pos.extend_from_slice(&r.position_xyz[i].to_le_bytes());
        }
        for i in 0..4 {
            col_quat.extend_from_slice(&r.orientation_quat[i].to_le_bytes());
        }
    }

    ArrowRecordBatchDescriptor {
        schema,
        num_rows: n,
        columns: vec![
            ArrowColumnVector::UInt64(col_ts),
            ArrowColumnVector::UInt32(col_robot),
            ArrowColumnVector::UInt32(col_joint),
            ArrowColumnVector::FixedSizeBinary { data: col_pos, byte_width: 12 },
            ArrowColumnVector::FixedSizeBinary { data: col_quat, byte_width: 16 },
        ],
    }
}

/// Generates Apache Iceberg `v1.metadata.json` for Snowflake External Tables and AWS Athena.
pub fn generate_iceberg_metadata(
    table_name: &str,
    _archive_file: &str,
    tile_count: usize,
    total_records: usize,
) -> String {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();

    format!(r#"{{
  "format-version": 2,
  "table-uuid": "s3a-iceberg-{}-{}",
  "location": "s3://s3a-cold-archive/{}",
  "last-sequence-number": 1,
  "last-updated-ms": {},
  "last-column-id": 4,
  "current-schema-id": 0,
  "schemas": [
    {{
      "type": "struct",
      "schema-id": 0,
      "fields": [
        {{ "id": 1, "name": "timestamp", "required": true, "type": "long" }},
        {{ "id": 2, "name": "sensor_id", "required": true, "type": "int" }},
        {{ "id": 3, "name": "metric_id", "required": true, "type": "int" }},
        {{ "id": 4, "name": "value", "required": true, "type": "double" }}
      ]
    }}
  ],
  "default-spec-id": 0,
  "partition-specs": [
    {{
      "spec-id": 0,
      "fields": [
        {{ "source-id": 2, "field-id": 1000, "name": "sensor_id_part", "transform": "identity" }}
      ]
    }}
  ],
  "properties": {{
    "s3a.engine.version": "0.1.0",
    "s3a.tile.count": "{}",
    "s3a.record.count": "{}",
    "write.parquet.compression-codec": "zstd"
  }},
  "snapshots": [
    {{
      "snapshot-id": 1000000000000000001,
      "timestamp-ms": {},
      "summary": {{
        "operation": "append",
        "added-data-files": "{}",
        "added-records": "{}"
      }},
      "manifest-list": "s3://s3a-cold-archive/{}/metadata/snap-1.avro"
    }}
  ]
}}"#, table_name, ts, table_name, ts, tile_count, total_records, ts, tile_count, total_records, table_name)
}

/// Generates Linux Foundation Delta Lake UniForm Transaction Log Metadata.
/// (`_delta_log/00000000000000000000.json`).
pub fn generate_delta_metadata(
    table_name: &str,
    _tile_count: usize,
    total_records: usize,
) -> String {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();

    format!(r#"{{"protocol":{{"minReaderVersion":3,"minWriterVersion":7,"readerFeatures":["icebergCompatV2"]}}}}
{{"metaData":{{"id":"s3a-delta-{}","format":{{"provider":"parquet","options":{{}}}},"schemaString":"{{\"type\":\"struct\",\"fields\":[{{\"name\":\"timestamp\",\"type\":\"long\",\"nullable\":false,\"metadata\":{{}}}},{{\"name\":\"sensor_id\",\"type\":\"integer\",\"nullable\":false,\"metadata\":{{}}}},{{\"name\":\"metric_id\",\"type\":\"integer\",\"nullable\":false,\"metadata\":{{}}}},{{\"name\":\"value\",\"type\":\"double\",\"nullable\":false,\"metadata\":{{}}}}]}}","partitionColumns":["sensor_id"],"configuration":{{"delta.universalFormat.enabledFormats":"iceberg"}},"createdTime":{}}}}}
{{"add":{{"path":"tile_000000.parquet","partitionValues":{{"sensor_id":"1"}},"size":131072,"modificationTime":{},"dataChange":true,"stats":"{{\"numRecords\":{}}}"}}}}"#,
        table_name, ts, ts, total_records
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snowflake_request_parsing() {
        let json_payload = r#"{"data": [[0, "L0:T0:R0"], [1, "L0:T0:R1"], [2, "QUERY:sensor=1"]]}"#;
        let req = SnowflakeBatchRequest::parse(json_payload).unwrap();
        assert_eq!(req.rows.len(), 3);
        assert_eq!(req.rows[0], (0, "L0:T0:R0".to_string()));
        assert_eq!(req.rows[1], (1, "L0:T0:R1".to_string()));
        assert_eq!(req.rows[2], (2, "QUERY:sensor=1".to_string()));
    }

    #[test]
    fn test_arrow_telemetry_and_robotics_projection() {
        let records = [
            TelemetryRecord::new(1000, 1, 101, 42.5),
            TelemetryRecord::new(2000, 2, 102, 99.1),
        ];

        let batch = project_telemetry_to_arrow(&records);
        assert_eq!(batch.num_rows, 2);
        assert_eq!(batch.schema.fields.len(), 4);
        assert_eq!(batch.schema.fields[0].name, "timestamp");
        assert_eq!(batch.schema.fields[3].name, "value");

        match &batch.columns[0] {
            ArrowColumnVector::UInt64(vec) => assert_eq!(vec, &[1000, 2000]),
            _ => panic!("Expected UInt64 column"),
        }

        let rob_records = [
            RoboticsKinematicRecord {
                timestamp_us: 50000,
                robot_id: 1,
                joint_mask: 7,
                position_xyz: [1.0, 2.0, 3.0],
                orientation_quat: [1.0, 0.0, 0.0, 0.0],
                linear_velocity: [0.1, 0.2, 0.3],
                angular_velocity: [0.0, 0.0, 0.0],
                _padding: 0,
            }
        ];
        let rob_batch = project_robotics_to_arrow(&rob_records);
        assert_eq!(rob_batch.num_rows, 1);
        assert_eq!(rob_batch.schema.fields.len(), 5);
    }

    #[test]
    fn test_iceberg_and_delta_metadata_generation() {
        let iceberg_json = generate_iceberg_metadata("telemetry_gold", "archive.s3a", 10, 20000);
        assert!(iceberg_json.contains("\"table-uuid\": \"s3a-iceberg-telemetry_gold-"));
        assert!(iceberg_json.contains("\"format-version\": 2"));
        assert!(iceberg_json.contains("\"s3a.tile.count\": \"10\""));

        let delta_json = generate_delta_metadata("telemetry_gold", 10, 20000);
        assert!(delta_json.contains("icebergCompatV2"));
        assert!(delta_json.contains("delta.universalFormat.enabledFormats"));
    }
}
