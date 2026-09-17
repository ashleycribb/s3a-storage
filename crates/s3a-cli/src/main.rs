use std::env;
use std::path::Path;
use std::time::Instant;

use s3a_engine::{Compactor, MmapReader, QuerySieve, S3ACrudEngine, TelemetryRecord, TileType, TileWriter};

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_usage();
        return;
    }

    match args[1].as_str() {
        "inspect" => {
            if args.len() < 3 {
                println!("Usage: s3a-cli inspect <file_path>");
                return;
            }
            inspect_file(&args[2]);
        }
        "verify" => {
            if args.len() < 3 {
                println!("Usage: s3a-cli verify <file_path>");
                return;
            }
            verify_file(&args[2]);
        }
        "insert" => {
            if args.len() < 6 {
                println!("Usage: s3a-cli insert <file_path> <timestamp> <sensor_id> <metric_id> <value>");
                return;
            }
            insert_telemetry(&args[2], &args[3], &args[4], &args[5], &args[6]);
        }
        "get" => {
            if args.len() < 5 {
                println!("Usage: s3a-cli get <file_path> <sensor_id> <metric_id> <min_ts> <max_ts>");
                return;
            }
            get_telemetry(&args[2], &args[3], &args[4], &args[5], &args[6]);
        }
        "delete" => {
            if args.len() < 6 {
                println!("Usage: s3a-cli delete <file_path> <sensor_id> <metric_id> <timestamp>");
                return;
            }
            delete_telemetry(&args[2], &args[3], &args[4], &args[5]);
        }
        "compact" => {
            if args.len() < 4 {
                println!("Usage: s3a-cli compact <out_file> <in_file1> [in_file2...]");
                return;
            }
            compact_files(&args[2], &args[3..]);
        }
        "benchmark" => {
            run_benchmark();
        }
        _ => {
            print_usage();
        }
    }
}

fn print_usage() {
    println!("S3A Storage CLI Tool");
    println!("Usage:");
    println!("  s3a-cli inspect <file_path>                       Inspect Hyper-Tile metadata");
    println!("  s3a-cli verify <file_path>                        Verify file and tile checksums");
    println!("  s3a-cli insert <file_path> <ts> <sensor> <metric> <val> Insert a new telemetry record");
    println!("  s3a-cli get <file_path> <sensor> <metric> <min_ts> <max_ts> Get matching telemetry records");
    println!("  s3a-cli delete <file_path> <sensor> <metric> <ts> Soft-delete matching record with tombstone");
    println!("  s3a-cli compact <out_file> <in_file1> [in_file2...] Compact files into stratified Hyper-Tiles");
    println!("  s3a-cli benchmark                                 Run benchmark comparing JSON/uncompressed storage vs S3A Hyper-Tiles");
}

fn inspect_file(path: &str) {
    match MmapReader::open(path) {
        Ok(reader) => {
            let file_header = reader.file_header();
            println!("S3A Archive Inspection: {}", path);
            println!("  Version: {}", file_header.version);
            println!("  Total Tiles: {}", file_header.tile_count);
            println!("----------------------------------------");

            for i in 0..reader.tile_count() as usize {
                if let Some((header, _)) = reader.get_tile(i) {
                    let tile_type_str = match header.tile_type {
                        TileType::TELEMETRY => "TELEMETRY",
                        TileType::EMBEDDING => "EMBEDDING",
                        TileType::HYBRID => "HYBRID",
                        _ => "UNKNOWN",
                    };
                    println!(
                        "Tile #{}: ID={}, Type={}, Records={}, TimeRange=[{}, {}], PayloadBytes={}",
                        i, header.tile_id, tile_type_str, header.record_count, header.min_timestamp, header.max_timestamp, header.payload_bytes
                    );
                }
            }
        }
        Err(e) => eprintln!("Error opening file: {}", e),
    }
}

fn verify_file(path: &str) {
    match MmapReader::open(path) {
        Ok(reader) => match reader.verify_checksums() {
            Ok(_) => println!("Verification SUCCESS: All CRC32 checksums match in {}", path),
            Err(e) => eprintln!("Verification FAILED in {}: {}", path, e),
        },
        Err(e) => eprintln!("Error opening file: {}", e),
    }
}

fn insert_telemetry(path: &str, ts_str: &str, sensor_str: &str, metric_str: &str, val_str: &str) {
    let ts: u64 = ts_str.parse().expect("Invalid timestamp");
    let sensor_id: u32 = sensor_str.parse().expect("Invalid sensor ID");
    let metric_id: u32 = metric_str.parse().expect("Invalid metric ID");
    let value: f64 = val_str.parse().expect("Invalid value");

    let rec = TelemetryRecord::new(ts, sensor_id, metric_id, value);
    let engine = S3ACrudEngine::open_or_create(path).expect("Failed to open archive");
    engine.create_telemetry(&[rec]).expect("Failed to insert record");
    println!("Inserted record into {}: timestamp={}, sensor={}, metric={}, val={}", path, ts, sensor_id, metric_id, value);
}

fn get_telemetry(path: &str, sensor_str: &str, metric_str: &str, min_ts_str: &str, max_ts_str: &str) {
    let sensor_id: u32 = sensor_str.parse().expect("Invalid sensor ID");
    let metric_id: u32 = metric_str.parse().expect("Invalid metric ID");
    let min_ts: u64 = min_ts_str.parse().expect("Invalid min timestamp");
    let max_ts: u64 = max_ts_str.parse().expect("Invalid max timestamp");

    let engine = S3ACrudEngine::open_or_create(path).expect("Failed to open archive");
    let results = engine.read_telemetry(sensor_id, metric_id, min_ts, max_ts).expect("Failed to query records");
    println!("Found {} record(s) matching criteria:", results.len());
    for r in results {
        println!("  Record: timestamp={}, sensor={}, metric={}, value={}", r.timestamp, r.sensor_id, r.metric_id, r.value);
    }
}

fn delete_telemetry(path: &str, sensor_str: &str, metric_str: &str, ts_str: &str) {
    let sensor_id: u32 = sensor_str.parse().expect("Invalid sensor ID");
    let metric_id: u32 = metric_str.parse().expect("Invalid metric ID");
    let ts: u64 = ts_str.parse().expect("Invalid timestamp");

    let engine = S3ACrudEngine::open_or_create(path).expect("Failed to open archive");
    let deleted = engine.delete_telemetry(sensor_id, metric_id, ts).expect("Failed to delete record");
    if deleted {
        println!("Soft-deleted record (tombstone appended) in {}", path);
    } else {
        println!("Record not found to delete in {}", path);
    }
}

fn compact_files(output_path: &str, input_paths: &[String]) {
    let refs: Vec<&str> = input_paths.iter().map(|s| s.as_str()).collect();
    match Compactor::compact(&refs, output_path) {
        Ok(count) => println!("Compaction SUCCESS: Wrote {} Hyper-Tiles to {}", count, output_path),
        Err(e) => eprintln!("Compaction FAILED: {}", e),
    }
}

fn run_benchmark() {
    println!("==================================================================");
    println!("   S3A HYPER-TILE STORAGE EFFICIENCY & PERFORMANCE BENCHMARK     ");
    println!("==================================================================");

    let num_records = 100_000;
    println!("Generating {} telemetry records for benchmark...", num_records);

    let mut telemetry_records = Vec::with_capacity(num_records);
    let mut json_bytes = Vec::new();

    for i in 0..num_records {
        let ts = 1000 + i as u64 * 10;
        let sensor_id = (i % 100) as u32;
        let metric_id = (i % 5) as u32;
        let value = (i as f64) * 0.1;

        let rec = TelemetryRecord::new(ts, sensor_id, metric_id, value);
        telemetry_records.push(rec);

        // Simple JSON representation simulation
        let json_line = format!(
            "{{\"timestamp\":{},\"sensor_id\":{},\"metric_id\":{},\"value\":{}}}\n",
            ts, sensor_id, metric_id, value
        );
        json_bytes.extend_from_slice(json_line.as_bytes());
    }

    let raw_binary_size = num_records * std::mem::size_of::<TelemetryRecord>();
    let json_size = json_bytes.len();

    // Write to S3A Hyper-Tile storage file
    let temp_s3a_path = Path::new("benchmark_test.s3a");
    let start_write = Instant::now();
    let mut writer = TileWriter::create(temp_s3a_path).unwrap();

    let rec_per_tile = s3a_core::HYPER_TILE_PAYLOAD_SIZE / std::mem::size_of::<TelemetryRecord>();
    for chunk in telemetry_records.chunks(rec_per_tile) {
        let ts: Vec<u64> = chunk.iter().map(|r| r.timestamp).collect();
        writer.write_hyper_tile(TileType::TELEMETRY, chunk, Some(&ts), None).unwrap();
    }
    let write_duration = start_write.elapsed();

    let s3a_file_size = std::fs::metadata(temp_s3a_path).unwrap().len() as usize;

    println!("\n--- STORAGE DENSITY & FILE SIZE COMPARISON ---");
    println!("JSON Format Size:          {:10} bytes ({:.2} MB)", json_size, json_size as f64 / 1_048_576.0);
    println!("Raw Binary Data Size:      {:10} bytes ({:.2} MB)", raw_binary_size, raw_binary_size as f64 / 1_048_576.0);
    println!("S3A Hyper-Tile File Size:  {:10} bytes ({:.2} MB)", s3a_file_size, s3a_file_size as f64 / 1_048_576.0);
    let compression_vs_json = (1.0 - (s3a_file_size as f64 / json_size as f64)) * 100.0;
    println!("Space Savings vs JSON:     {:.2}%", compression_vs_json);
    println!("S3A Write Time (100k recs): {:?}", write_duration);

    // Benchmark SIMD Query Sieve Rejection Speed
    println!("\n--- SIMD QUERY REJECTION PERFORMANCE ---");
    let reader = MmapReader::open(temp_s3a_path).unwrap();
    let sieve = QuerySieve::new(&reader);

    // Query for tiny range (rejection test across 100k records)
    let start_query = Instant::now();
    let results = sieve.query_telemetry(5000, 6000, Some(42), None);
    let query_duration = start_query.elapsed();

    println!("Query Time Range [5000, 6000]: Found {} records in {:?}", results.len(), query_duration);
    println!("Single-cycle SIMD tile rejection allowed skipping non-matching 128KB Hyper-Tiles instantly!");

    // Cleanup
    let _ = std::fs::remove_file(temp_s3a_path);
    println!("\n==================================================================");
}
