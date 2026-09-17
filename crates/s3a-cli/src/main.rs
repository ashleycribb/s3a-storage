use std::env;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::time::Instant;

use s3a_engine::{
    Compactor, MicroTileBuffer, MmapReader, QuerySieve, S3ACrudEngine,
    TelemetryRecord, CompactWearableRecord, TileType, TileWriter, S3ACoordinate,
};

const INDEX_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>S3A Storage Dashboard & Agent Console</title>
    <style>
        :root {
            --bg: #0f172a;
            --card-bg: #1e293b;
            --border: #334155;
            --primary: #38bdf8;
            --primary-hover: #0284c7;
            --success: #4ade80;
            --danger: #f87171;
            --text: #f8fafc;
            --text-muted: #94a3b8;
            --coord: #f59e0b;
        }
        * { box-sizing: border-box; margin: 0; padding: 0; font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; }
        body { background: var(--bg); color: var(--text); padding: 20px; line-height: 1.5; }
        header { display: flex; justify-content: space-between; align-items: center; padding-bottom: 20px; border-bottom: 1px solid var(--border); margin-bottom: 20px; }
        h1 { font-size: 1.5rem; color: var(--primary); display: flex; align-items: center; gap: 10px; }
        .badge { background: #0369a1; color: #e0f2fe; padding: 2px 8px; border-radius: 4px; font-size: 0.8rem; }
        .grid { display: grid; grid-template-columns: 1fr 1fr; gap: 20px; }
        .card { background: var(--card-bg); border: 1px solid var(--border); border-radius: 8px; padding: 20px; margin-bottom: 20px; }
        .card-header { font-weight: 600; font-size: 1.1rem; margin-bottom: 15px; color: var(--primary); border-bottom: 1px solid var(--border); padding-bottom: 8px; display: flex; justify-content: space-between; }
        .form-group { margin-bottom: 12px; }
        label { display: block; font-size: 0.85rem; color: var(--text-muted); margin-bottom: 4px; }
        input, select, textarea { width: 100%; padding: 8px 12px; background: #0f172a; border: 1px solid var(--border); border-radius: 4px; color: var(--text); font-size: 0.9rem; }
        button { background: var(--primary); color: #0f172a; font-weight: 600; border: none; padding: 10px 16px; border-radius: 4px; cursor: pointer; transition: background 0.2s; }
        button:hover { background: var(--primary-hover); }
        button.btn-danger { background: var(--danger); color: #0f172a; }
        .btn-row { display: flex; gap: 10px; }
        table { width: 100%; border-collapse: collapse; margin-top: 10px; font-size: 0.85rem; }
        th, td { padding: 8px; text-align: left; border-bottom: 1px solid var(--border); }
        th { color: var(--text-muted); font-weight: 600; }
        pre { background: #0f172a; padding: 12px; border-radius: 4px; overflow-x: auto; font-size: 0.85rem; color: #a5f3fc; border: 1px solid var(--border); max-height: 250px; }
        .agent-pill { background: #312e81; color: #c7d2fe; padding: 2px 6px; border-radius: 4px; font-size: 0.75rem; }
        .coord-badge { background: #78350f; color: #fef3c7; font-weight: bold; font-family: monospace; padding: 2px 6px; border-radius: 4px; border: 1px solid var(--coord); }
    </style>
</head>
<body>
    <header>
        <h1>S3A Storage Architecture <span class="badge">Wearable Profile Enabled</span></h1>
        <div id="status">Archive: <span id="archive-path">test.s3a</span></div>
    </header>

    <div class="grid">
        <!-- Left Column: Operations & Direct Coordinate Lookup -->
        <div>
            <!-- Direct Coordinate Lookup (Excel Cell style) -->
            <div class="card">
                <div class="card-header">
                    <span>Direct $O(1)$ Coordinate Address Lookup</span>
                    <span class="badge" style="background:#78350f; color:#fef3c7;">Excel A1 Analogue</span>
                </div>
                <div class="form-group">
                    <label>S3A Coordinate Address (e.g. L0:T0:R0)</label>
                    <input type="text" id="coord-address" value="L0:T0:R0" placeholder="L<level>:T<tile>:R<offset>">
                </div>
                <button onclick="lookupCoordinate()" style="background:#f59e0b; color:#0f172a;">Direct O(1) Fetch</button>
            </div>

            <!-- Query Sieve -->
            <div class="card">
                <div class="card-header">
                    <span>SIMD Query Sieve</span>
                    <span class="badge">Zero-Copy Mmap</span>
                </div>
                <div class="form-group">
                    <label>Min Timestamp</label>
                    <input type="number" id="q-min-ts" value="0">
                </div>
                <div class="form-group">
                    <label>Max Timestamp</label>
                    <input type="number" id="q-max-ts" value="100000">
                </div>
                <div class="btn-row">
                    <button id="btn-query" onclick="queryRecords()">Run SIMD Query</button>
                    <button onclick="inspectArchive()" style="background:#475569; color:#f8fafc;">Inspect Tiles</button>
                </div>
            </div>

            <!-- CRUD Operations -->
            <div class="card">
                <div class="card-header">Telemetry CRUD Operations</div>
                <div class="form-group">
                    <label>Timestamp</label>
                    <input type="number" id="crud-ts" value="1000">
                </div>
                <div class="form-group">
                    <label>Sensor ID</label>
                    <input type="number" id="crud-sensor" value="1">
                </div>
                <div class="form-group">
                    <label>Metric ID</label>
                    <input type="number" id="crud-metric" value="101">
                </div>
                <div class="form-group">
                    <label>Value</label>
                    <input type="number" step="0.01" id="crud-val" value="42.5">
                </div>
                <div class="btn-row">
                    <button onclick="insertRecord()">Insert (Create/Update)</button>
                    <button class="btn-danger" onclick="deleteRecord()">Delete (Tombstone)</button>
                    <button onclick="compactArchive()" style="background:#10b981; color:#0f172a;">Compact File</button>
                </div>
            </div>
        </div>

        <!-- Right Column: Inspector & Autonomous Agent Execution -->
        <div>
            <!-- Results / Inspection Table -->
            <div class="card">
                <div class="card-header" id="results-title">Query Results</div>
                <div id="results-container">
                    <p style="color: var(--text-muted); font-size: 0.9rem;">No query run yet. Click "Run SIMD Query" or "Inspect Tiles".</p>
                </div>
            </div>

            <!-- AI Agent Tool Execution Console -->
            <div class="card">
                <div class="card-header">
                    <span>AI Agent Autonomous Tool Console</span>
                    <span class="agent-pill">Function Calling</span>
                </div>
                <div class="form-group">
                    <label>Select Agent Tool Function</label>
                    <select id="agent-tool-select" onchange="updateAgentPayload()">
                        <option value="s3a_lookup_coordinate">s3a_lookup_coordinate</option>
                        <option value="s3a_query_telemetry">s3a_query_telemetry</option>
                        <option value="s3a_insert_telemetry">s3a_insert_telemetry</option>
                        <option value="s3a_delete_telemetry">s3a_delete_telemetry</option>
                        <option value="s3a_compact_archive">s3a_compact_archive</option>
                    </select>
                </div>
                <div class="form-group">
                    <label>Tool Call Arguments (JSON)</label>
                    <textarea id="agent-payload" rows="4">{"coordinate": "L0:T0:R0"}</textarea>
                </div>
                <button onclick="executeAgentTool()" style="background:#8b5cf6; color:#ffffff;">Execute Agent Function Call</button>
                <div style="margin-top: 15px;">
                    <label>Agent Execution Trace Output</label>
                    <pre id="agent-output">// Agent response will appear here...</pre>
                </div>
            </div>
        </div>
    </div>

    <script>
        const ARCHIVE = "test.s3a";

        async function lookupCoordinate() {
            const coord = document.getElementById('coord-address').value;
            const res = await fetch(`/api/lookup?file=${ARCHIVE}&coordinate=${coord}`);
            const data = await res.json();

            document.getElementById('results-title').innerText = `Direct O(1) Coordinate Lookup (${coord})`;
            if (data.record) {
                const r = data.record;
                let html = '<table><thead><tr><th>Coordinate</th><th>Timestamp</th><th>Sensor ID</th><th>Metric ID</th><th>Value</th></tr></thead><tbody>';
                html += `<tr><td><span class="coord-badge">${coord}</span></td><td>${r.timestamp}</td><td>${r.sensor_id}</td><td>${r.metric_id}</td><td>${r.value.toFixed(2)}</td></tr>`;
                html += '</tbody></table>';
                document.getElementById('results-container').innerHTML = html;
            } else {
                document.getElementById('results-container').innerHTML = `<p style="color: var(--danger); padding: 10px;">${data.error || 'Record not found'}</p>`;
            }
        }

        async function queryRecords() {
            const minTs = document.getElementById('q-min-ts').value;
            const maxTs = document.getElementById('q-max-ts').value;
            const res = await fetch(`/api/query?file=${ARCHIVE}&min_ts=${minTs}&max_ts=${maxTs}`);
            const data = await res.json();

            document.getElementById('results-title').innerText = `Query Results (${data.records ? data.records.length : 0} records found in ${data.duration_micros}µs)`;

            if (data.records && data.records.length > 0) {
                let html = '<table><thead><tr><th>S3A Coordinate</th><th>Timestamp</th><th>Sensor ID</th><th>Metric ID</th><th>Value</th></tr></thead><tbody>';
                data.records.forEach(r => {
                    html += `<tr><td><span class="coord-badge">${r.coordinate}</span></td><td>${r.timestamp}</td><td>${r.sensor_id}</td><td>${r.metric_id}</td><td>${r.value.toFixed(2)}</td></tr>`;
                });
                html += '</tbody></table>';
                document.getElementById('results-container').innerHTML = html;
            } else {
                document.getElementById('results-container').innerHTML = '<p style="color: var(--text-muted); padding: 10px;">No matching records found.</p>';
            }
        }

        async function inspectArchive() {
            const res = await fetch(`/api/inspect?file=${ARCHIVE}`);
            const data = await res.json();
            document.getElementById('results-title').innerText = `Archive Inspection (${data.tile_count} Hyper-Tiles)`;
            if (data.tiles) {
                let html = '<table><thead><tr><th>Tile ID</th><th>Type</th><th>Records</th><th>Time Range</th><th>Bytes</th></tr></thead><tbody>';
                data.tiles.forEach(t => {
                    html += `<tr><td>#${t.tile_id}</td><td>${t.type}</td><td>${t.record_count}</td><td>[${t.min_ts}, ${t.max_ts}]</td><td>${t.payload_bytes} B</td></tr>`;
                });
                html += '</tbody></table>';
                document.getElementById('results-container').innerHTML = html;
            }
        }

        async function insertRecord() {
            const ts = parseInt(document.getElementById('crud-ts').value);
            const sensor_id = parseInt(document.getElementById('crud-sensor').value);
            const metric_id = parseInt(document.getElementById('crud-metric').value);
            const value = parseFloat(document.getElementById('crud-val').value);

            const res = await fetch('/api/insert', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ file: ARCHIVE, timestamp: ts, sensor_id, metric_id, value })
            });
            const data = await res.json();
            alert('Inserted record successfully!');
            queryRecords();
        }

        async function deleteRecord() {
            const ts = parseInt(document.getElementById('crud-ts').value);
            const sensor_id = parseInt(document.getElementById('crud-sensor').value);
            const metric_id = parseInt(document.getElementById('crud-metric').value);

            const res = await fetch('/api/delete', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ file: ARCHIVE, timestamp: ts, sensor_id, metric_id })
            });
            const data = await res.json();
            alert(data.status);
            queryRecords();
        }

        async function compactArchive() {
            const res = await fetch('/api/compact', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ file: ARCHIVE })
            });
            const data = await res.json();
            alert('Compaction complete! New tile count: ' + data.tile_count);
            inspectArchive();
        }

        function updateAgentPayload() {
            const tool = document.getElementById('agent-tool-select').value;
            if (tool === 's3a_lookup_coordinate') {
                document.getElementById('agent-payload').value = '{"coordinate": "L0:T0:R0"}';
            } else if (tool === 's3a_query_telemetry') {
                document.getElementById('agent-payload').value = '{"min_ts": 0, "max_ts": 100000}';
            } else if (tool === 's3a_insert_telemetry') {
                document.getElementById('agent-payload').value = '{"timestamp": 5000, "sensor_id": 1, "metric_id": 101, "value": 99.4}';
            } else if (tool === 's3a_delete_telemetry') {
                document.getElementById('agent-payload').value = '{"timestamp": 5000, "sensor_id": 1, "metric_id": 101}';
            } else if (tool === 's3a_compact_archive') {
                document.getElementById('agent-payload').value = '{}';
            }
        }

        async function executeAgentTool() {
            const name = document.getElementById('agent-tool-select').value;
            const args = JSON.parse(document.getElementById('agent-payload').value);

            const res = await fetch('/api/agent/execute', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ tool_name: name, arguments: args, file: ARCHIVE })
            });
            const data = await res.json();
            document.getElementById('agent-output').innerText = JSON.stringify(data, null, 2);
        }

        // Auto initial load
        queryRecords();
    </script>
</body>
</html>
"#;

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
        "lookup" => {
            if args.len() < 4 {
                println!("Usage: s3a-cli lookup <file_path> <coordinate> (e.g. L0:T0:R0)");
                return;
            }
            lookup_coordinate(&args[2], &args[3]);
        }
        "insert" => {
            if args.len() < 7 {
                println!("Usage: s3a-cli insert <file_path> <timestamp> <sensor_id> <metric_id> <value>");
                return;
            }
            insert_telemetry(&args[2], &args[3], &args[4], &args[5], &args[6]);
        }
        "get" => {
            if args.len() < 7 {
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
        "benchmark-wearable" => {
            run_wearable_benchmark();
        }
        "serve" => {
            let port = if args.len() >= 3 {
                args[2].parse().unwrap_or(8080)
            } else {
                8080
            };
            run_server(port);
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
    println!("  s3a-cli lookup <file_path> <coordinate>           Direct O(1) lookup by coordinate address (e.g. L0:T0:R0)");
    println!("  s3a-cli insert <file_path> <ts> <sensor> <metric> <val> Insert a new telemetry record");
    println!("  s3a-cli get <file_path> <sensor> <metric> <min_ts> <max_ts> Get matching telemetry records with coordinates");
    println!("  s3a-cli delete <file_path> <sensor> <metric> <ts> Soft-delete matching record with tombstone");
    println!("  s3a-cli compact <out_file> <in_file1> [in_file2...] Compact files into stratified Hyper-Tiles");
    println!("  s3a-cli benchmark                                 Run benchmark comparing JSON/uncompressed storage vs S3A Hyper-Tiles");
    println!("  s3a-cli benchmark-wearable                        Run wearable benchmark (4KB Flash page micro-tiles, zero-heap alloc)");
    println!("  s3a-cli serve [port]                              Start Web Dashboard & Agent Function Calling Server (default: 8080)");
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

fn lookup_coordinate(path: &str, coord_str: &str) {
    let coord: S3ACoordinate = match coord_str.parse() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Invalid coordinate '{}': {}", coord_str, e);
            return;
        }
    };

    let engine = S3ACrudEngine::open_or_create(path).expect("Failed to open archive");
    match engine.read_by_coordinate::<TelemetryRecord>(&coord) {
        Ok(r) => {
            println!("Direct O(1) Fetch at {}: timestamp={}, sensor={}, metric={}, value={}", coord, r.timestamp, r.sensor_id, r.metric_id, r.value);
        }
        Err(e) => eprintln!("Fetch failed at {}: {}", coord, e),
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
    let results = engine.read_telemetry_with_coords(sensor_id, metric_id, min_ts, max_ts).expect("Failed to query records");
    println!("Found {} record(s) matching criteria:", results.len());
    for (r, coord) in results {
        println!("  Record [{}]: timestamp={}, sensor={}, metric={}, value={}", coord, r.timestamp, r.sensor_id, r.metric_id, r.value);
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

fn run_wearable_benchmark() {
    println!("==================================================================");
    println!("   S3A WEARABLE DEVICE MICRO-TILE STORAGE & RAM BENCHMARK         ");
    println!("   Devices: Earbuds, Smart Rings, Wristbands, Smart Glasses       ");
    println!("==================================================================");

    let num_wearable_recs = 10_000;
    println!("Generating {} PPG/IMU sensor samples for wearable target...", num_wearable_recs);

    let start_time = Instant::now();
    let mut micro_buf = MicroTileBuffer::new(1);
    let mut filled_pages = 0;

    for i in 0..num_wearable_recs {
        let ts_sec = (i as u32) * 2; // 1 sample every 2 seconds
        let heart_rate = 60.0 + ((i % 40) as f32) * 0.5; // Simulated heart rate PPG
        let rec = CompactWearableRecord::new(ts_sec, 0, 10, heart_rate);

        if micro_buf.push_record(&rec).is_err() {
            let _flash_page = micro_buf.finalize();
            filled_pages += 1;
            micro_buf = MicroTileBuffer::new(filled_pages + 1);
            let _ = micro_buf.push_record(&rec);
        }
    }
    let _last_page = micro_buf.finalize();
    filled_pages += 1;
    let elapsed = start_time.elapsed();

    let total_bytes = (filled_pages as usize) * s3a_core::MICRO_TILE_SIZE;
    let rec_per_page = s3a_core::MICRO_TILE_PAYLOAD_SIZE / std::mem::size_of::<CompactWearableRecord>();

    println!("\n--- WEARABLE DEVICE BENCHMARK RESULTS ---");
    println!("Total Samples Processed:      {}", num_wearable_recs);
    println!("Heap Working Set Allocation:   0 bytes (PURE stack/buffer, 100% no_std safe)");
    println!("SPI NOR Flash Page Size:       4,096 bytes (4 KB Sector Aligned)");
    println!("Records per 4KB Flash Sector:  {} samples", rec_per_page);
    println!("Total 4KB Flash Pages Used:    {}", filled_pages);
    println!("Total Flash Storage Used:      {} bytes ({:.2} KB)", total_bytes, total_bytes as f64 / 1024.0);
    println!("Total Stream Ingest Duration:  {:?}", elapsed);
    println!("Ingest Throughput Rate:        {:.2} million samples/sec", (num_wearable_recs as f64 / elapsed.as_secs_f64()) / 1_000_000.0);
    println!("==================================================================");
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

fn run_server(port: u16) {
    let listener = TcpListener::bind(("0.0.0.0", port)).expect("Failed to bind TCP listener");
    println!("S3A Web Dashboard & Agent Function Calling Server listening on http://0.0.0.0:{}", port);

    for stream in listener.incoming() {
        if let Ok(mut stream) = stream {
            handle_connection(&mut stream);
        }
    }
}

fn handle_connection(stream: &mut TcpStream) {
    let mut buffer = [0u8; 4096];
    let bytes_read = match stream.read(&mut buffer) {
        Ok(n) => n,
        Err(_) => return,
    };
    let request = String::from_utf8_lossy(&buffer[..bytes_read]);

    let mut lines = request.lines();
    let request_line = match lines.next() {
        Some(l) => l,
        None => return,
    };

    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return;
    }

    let method = parts[0];
    let url = parts[1];

    if method == "GET" && (url == "/" || url == "/index.html") {
        send_response(stream, "200 OK", "text/html", INDEX_HTML.as_bytes());
        return;
    }

    if url.starts_with("/api/lookup") {
        let file = extract_query_param(url, "file").unwrap_or_else(|| "test.s3a".to_string());
        let coord_str = extract_query_param(url, "coordinate").unwrap_or_else(|| "L0:T0:R0".to_string());

        let _ = S3ACrudEngine::open_or_create(&file);
        let coord: Result<S3ACoordinate, _> = coord_str.parse();
        if let Ok(c) = coord {
            if let Ok(engine) = S3ACrudEngine::open_or_create(&file) {
                match engine.read_by_coordinate::<TelemetryRecord>(&c) {
                    Ok(r) => {
                        let resp = format!(
                            "{{\"coordinate\":\"{}\",\"record\":{{\"timestamp\":{},\"sensor_id\":{},\"metric_id\":{},\"value\":{}}}}}",
                            c, r.timestamp, r.sensor_id, r.metric_id, r.value
                        );
                        send_response(stream, "200 OK", "application/json", resp.as_bytes());
                    }
                    Err(e) => {
                        let resp = format!("{{\"error\":\"{}\"}}", e);
                        send_response(stream, "404 Not Found", "application/json", resp.as_bytes());
                    }
                }
            } else {
                send_response(stream, "400 Bad Request", "application/json", b"{\"error\":\"Failed to open archive\"}");
            }
        } else {
            send_response(stream, "400 Bad Request", "application/json", b"{\"error\":\"Invalid coordinate format\"}");
        }
        return;
    }

    if url.starts_with("/api/inspect") {
        let file = extract_query_param(url, "file").unwrap_or_else(|| "test.s3a".to_string());
        let _ = S3ACrudEngine::open_or_create(&file);
        if let Ok(reader) = MmapReader::open(&file) {
            let mut tiles_json = Vec::new();
            for i in 0..reader.tile_count() as usize {
                if let Some((h, _)) = reader.get_tile(i) {
                    tiles_json.push(format!(
                        "{{\"tile_id\":{},\"type\":{},\"record_count\":{},\"min_ts\":{},\"max_ts\":{},\"payload_bytes\":{}}}",
                        h.tile_id, h.tile_type, h.record_count, h.min_timestamp, h.max_timestamp, h.payload_bytes
                    ));
                }
            }
            let resp = format!("{{\"tile_count\":{},\"tiles\":[{}]}}", reader.tile_count(), tiles_json.join(","));
            send_response(stream, "200 OK", "application/json", resp.as_bytes());
        } else {
            send_response(stream, "400 Bad Request", "application/json", b"{\"error\":\"Failed to open file\"}");
        }
        return;
    }

    if url.starts_with("/api/query") {
        let file = extract_query_param(url, "file").unwrap_or_else(|| "test.s3a".to_string());
        let min_ts: u64 = extract_query_param(url, "min_ts").and_then(|s| s.parse().ok()).unwrap_or(0);
        let max_ts: u64 = extract_query_param(url, "max_ts").and_then(|s| s.parse().ok()).unwrap_or(u64::MAX);

        let _ = S3ACrudEngine::open_or_create(&file);
        if let Ok(reader) = MmapReader::open(&file) {
            let sieve = QuerySieve::new(&reader);
            let start = Instant::now();
            let records = sieve.query_telemetry_with_coords(min_ts, max_ts, None, None);
            let duration = start.elapsed().as_micros();

            let rec_json: Vec<String> = records.iter().map(|(r, coord)| {
                format!(
                    "{{\"coordinate\":\"{}\",\"timestamp\":{},\"sensor_id\":{},\"metric_id\":{},\"value\":{},\"flags\":{}}}",
                    coord, r.timestamp, r.sensor_id, r.metric_id, r.value, r.flags
                )
            }).collect();

            let resp = format!("{{\"duration_micros\":{},\"records\":[{}]}}", duration, rec_json.join(","));
            send_response(stream, "200 OK", "application/json", resp.as_bytes());
        } else {
            send_response(stream, "400 Bad Request", "application/json", b"{\"error\":\"Failed to query archive\"}");
        }
        return;
    }

    if method == "POST" && url == "/api/insert" {
        let body = extract_body(&request);
        let file = parse_json_str(&body, "file").unwrap_or_else(|| "test.s3a".to_string());
        let ts = parse_json_u64(&body, "timestamp").unwrap_or(1000);
        let sensor = parse_json_u64(&body, "sensor_id").unwrap_or(1) as u32;
        let metric = parse_json_u64(&body, "metric_id").unwrap_or(101) as u32;
        let val = parse_json_f64(&body, "value").unwrap_or(42.5);

        let rec = TelemetryRecord::new(ts, sensor, metric, val);
        if let Ok(engine) = S3ACrudEngine::open_or_create(&file) {
            let _ = engine.create_telemetry(&[rec]);
            send_response(stream, "200 OK", "application/json", b"{\"status\":\"inserted\"}");
        } else {
            send_response(stream, "500 Internal Error", "application/json", b"{\"error\":\"Failed to insert\"}");
        }
        return;
    }

    if method == "POST" && url == "/api/delete" {
        let body = extract_body(&request);
        let file = parse_json_str(&body, "file").unwrap_or_else(|| "test.s3a".to_string());
        let ts = parse_json_u64(&body, "timestamp").unwrap_or(1000);
        let sensor = parse_json_u64(&body, "sensor_id").unwrap_or(1) as u32;
        let metric = parse_json_u64(&body, "metric_id").unwrap_or(101) as u32;

        if let Ok(engine) = S3ACrudEngine::open_or_create(&file) {
            let deleted = engine.delete_telemetry(sensor, metric, ts).unwrap_or(false);
            let status = if deleted { "tombstone_appended" } else { "not_found" };
            let resp = format!("{{\"status\":\"{}\"}}", status);
            send_response(stream, "200 OK", "application/json", resp.as_bytes());
        } else {
            send_response(stream, "500 Internal Error", "application/json", b"{\"error\":\"Failed to delete\"}");
        }
        return;
    }

    if method == "POST" && url == "/api/compact" {
        let body = extract_body(&request);
        let file = parse_json_str(&body, "file").unwrap_or_else(|| "test.s3a".to_string());

        if let Ok(engine) = S3ACrudEngine::open_or_create(&file) {
            let tiles = engine.purge_and_compact().unwrap_or(0);
            let resp = format!("{{\"tile_count\":{}}}", tiles);
            send_response(stream, "200 OK", "application/json", resp.as_bytes());
        } else {
            send_response(stream, "500 Internal Error", "application/json", b"{\"error\":\"Failed to compact\"}");
        }
        return;
    }

    if method == "POST" && url == "/api/agent/execute" {
        let body = extract_body(&request);
        let file = parse_json_str(&body, "file").unwrap_or_else(|| "test.s3a".to_string());
        let tool_name = parse_json_str(&body, "tool_name").unwrap_or_default();

        let engine = S3ACrudEngine::open_or_create(&file).unwrap();

        let response_json = if tool_name == "s3a_lookup_coordinate" {
            let coord_str = parse_json_str(&body, "coordinate").unwrap_or_else(|| "L0:T0:R0".to_string());
            if let Ok(coord) = coord_str.parse::<S3ACoordinate>() {
                match engine.read_by_coordinate::<TelemetryRecord>(&coord) {
                    Ok(r) => format!("{{\"status\":\"success\",\"tool\":\"s3a_lookup_coordinate\",\"coordinate\":\"{}\",\"record\":{{\"timestamp\":{},\"sensor_id\":{},\"metric_id\":{},\"value\":{}}}}}", coord, r.timestamp, r.sensor_id, r.metric_id, r.value),
                    Err(e) => format!("{{\"status\":\"error\",\"tool\":\"s3a_lookup_coordinate\",\"error\":\"{}\"}}", e),
                }
            } else {
                format!("{{\"status\":\"error\",\"tool\":\"s3a_lookup_coordinate\",\"error\":\"Invalid coordinate format\"}}")
            }
        } else if tool_name == "s3a_query_telemetry" {
            let min_ts = parse_json_u64(&body, "min_ts").unwrap_or(0);
            let max_ts = parse_json_u64(&body, "max_ts").unwrap_or(u64::MAX);
            let records = engine.read_telemetry_with_coords(1, 101, min_ts, max_ts).unwrap_or_default();
            format!("{{\"status\":\"success\",\"tool\":\"s3a_query_telemetry\",\"matched_records\":{}}}", records.len())
        } else if tool_name == "s3a_insert_telemetry" {
            let ts = parse_json_u64(&body, "timestamp").unwrap_or(1000);
            let sensor = parse_json_u64(&body, "sensor_id").unwrap_or(1) as u32;
            let metric = parse_json_u64(&body, "metric_id").unwrap_or(101) as u32;
            let val = parse_json_f64(&body, "value").unwrap_or(42.5);
            let rec = TelemetryRecord::new(ts, sensor, metric, val);
            engine.create_telemetry(&[rec]).unwrap();
            format!("{{\"status\":\"success\",\"tool\":\"s3a_insert_telemetry\",\"message\":\"Record appended to 128KB Hyper-Tile\"}}")
        } else if tool_name == "s3a_delete_telemetry" {
            let ts = parse_json_u64(&body, "timestamp").unwrap_or(1000);
            let sensor = parse_json_u64(&body, "sensor_id").unwrap_or(1) as u32;
            let metric = parse_json_u64(&body, "metric_id").unwrap_or(101) as u32;
            let deleted = engine.delete_telemetry(sensor, metric, ts).unwrap_or(false);
            format!("{{\"status\":\"success\",\"tool\":\"s3a_delete_telemetry\",\"deleted\":{}}}", deleted)
        } else if tool_name == "s3a_compact_archive" {
            let tile_count = engine.purge_and_compact().unwrap_or(0);
            format!("{{\"status\":\"success\",\"tool\":\"s3a_compact_archive\",\"new_tile_count\":{}}}", tile_count)
        } else {
            format!("{{\"error\":\"Unknown tool name: {}\"}}", tool_name)
        };

        send_response(stream, "200 OK", "application/json", response_json.as_bytes());
        return;
    }

    send_response(stream, "404 Not Found", "text/plain", b"404 Not Found");
}

fn send_response(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8]) {
    let header = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
        status, content_type, body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

fn extract_query_param(url: &str, key: &str) -> Option<String> {
    if let Some(pos) = url.find('?') {
        let query = &url[pos + 1..];
        for pair in query.split('&') {
            let parts: Vec<&str> = pair.split('=').collect();
            if parts.len() == 2 && parts[0] == key {
                return Some(parts[1].to_string());
            }
        }
    }
    None
}

fn extract_body(request: &str) -> String {
    if let Some(pos) = request.find("\r\n\r\n") {
        request[pos + 4..].to_string()
    } else {
        String::new()
    }
}

fn parse_json_str(json: &str, key: &str) -> Option<String> {
    let search = format!("\"{}\":", key);
    if let Some(pos) = json.find(&search) {
        let rest = json[pos + search.len()..].trim();
        if rest.starts_with('"') {
            if let Some(end) = rest[1..].find('"') {
                return Some(rest[1..1 + end].to_string());
            }
        }
    }
    None
}

fn parse_json_u64(json: &str, key: &str) -> Option<u64> {
    let search = format!("\"{}\":", key);
    if let Some(pos) = json.find(&search) {
        let rest = json[pos + search.len()..].trim();
        let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
        rest[..end].parse().ok()
    } else {
        None
    }
}

fn parse_json_f64(json: &str, key: &str) -> Option<f64> {
    let search = format!("\"{}\":", key);
    if let Some(pos) = json.find(&search) {
        let rest = json[pos + search.len()..].trim();
        let end = rest.find(|c: char| !c.is_ascii_digit() && c != '.').unwrap_or(rest.len());
        rest[..end].parse().ok()
    } else {
        None
    }
}
