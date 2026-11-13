//! First-run friendly commands, defaults, ASCII tables, and error suggestions.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use s3a_core::document::{
    compute_key_hash, DocumentRecordHeader, DOC_FLAG_ACTIVE, DOC_FLAG_IS_JSON,
    DOC_KIND_GENERIC_KV, DOC_KIND_JSON,
};
use s3a_core::{HyperTileHeader, TileType, HYPER_TILE_PAYLOAD_SIZE};
use s3a_engine::{execute_query, MmapReader, QueryResult, TelemetryRecord, TileWriter};

/// Canonical list of recognized CLI commands for 'did you mean' suggestions.
pub const KNOWN_COMMANDS: &[&str] = &[
    "init", "put", "get", "import", "export", "query", "ls", "list", "info", "tour", "demo",
    "help", "seal", "verify", "inspect", "status", "doctor", "recover",
    "compact", "fuse", "flatten", "unflatten", "prove", "verify-proof",
    "vcs", "gate", "vault", "mcp-server", "serve", "serve-tcp", "shell", "telemetry"
];

/// Canonical list of core and standard tile type names for 'did you mean' suggestions.
pub const KNOWN_TILE_TYPES: &[&str] = &[
    "telemetry",
    "embedding_128",
    "hybrid_index",
    "quantized_embedding_int8",
    "quantized_embedding_4bit",
    "marks",
    "statement_body",
    "statement_index",
    "document_store",
    "attachment_blob",
    "elr_person",
    "elr_organization",
    "elr_learning_resource",
    "elr_learning_event",
    "schema",
    "manifest",
    "record_root",
];

// =========================================================================
// Error & Suggestion Helpers
// =========================================================================

/// Calculates Levenshtein edit distance between two strings.
pub fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let mut dp = vec![vec![0; b_chars.len() + 1]; a_chars.len() + 1];

    for i in 0..=a_chars.len() {
        dp[i][0] = i;
    }
    for j in 0..=b_chars.len() {
        dp[0][j] = j;
    }

    for i in 1..=a_chars.len() {
        for j in 1..=b_chars.len() {
            let cost = if a_chars[i - 1] == b_chars[j - 1] { 0 } else { 1 };
            dp[i][j] = (dp[i - 1][j] + 1)
                .min(dp[i][j - 1] + 1)
                .min(dp[i - 1][j - 1] + cost);
        }
    }
    dp[a_chars.len()][b_chars.len()]
}

/// Finds the closest matching candidate from a dictionary if within distance threshold.
pub fn find_best_suggestion<'a>(input: &str, candidates: &'a [&'a str], max_distance: usize) -> Option<&'a str> {
    let lower = input.to_lowercase();
    let mut best_match = None;
    let mut min_dist = max_distance + 1;

    for &candidate in candidates {
        let dist = levenshtein_distance(&lower, candidate);
        if dist < min_dist {
            min_dist = dist;
            best_match = Some(candidate);
        }
    }

    if min_dist <= max_distance {
        best_match
    } else {
        None
    }
}

pub fn suggest_command(input: &str) -> Option<&'static str> {
    find_best_suggestion(input, KNOWN_COMMANDS, 2)
}

pub fn suggest_tile_type(input: &str) -> Option<&'static str> {
    find_best_suggestion(input, KNOWN_TILE_TYPES, 3)
}

pub fn print_cli_error(msg: &str, fix: Option<&str>, verbose: bool, debug_info: Option<&str>) -> ! {
    eprintln!("error: {}", msg);
    if let Some(f) = fix {
        eprintln!("  fix: {}", f);
    }
    if verbose {
        if let Some(d) = debug_info {
            eprintln!("\ndebug: {}", d);
        }
    }
    std::process::exit(1);
}

// =========================================================================
// Vault Resolution
// =========================================================================

/// Resolves the target S3A vault file with friendly defaults and zero required flags.
pub fn resolve_vault_path(args: &[String]) -> PathBuf {
    // 1. Explicit --vault flag anywhere in args
    for i in 0..args.len() {
        if args[i] == "--vault" && i + 1 < args.len() {
            return PathBuf::from(&args[i + 1]);
        }
        if let Some(stripped) = args[i].strip_prefix("--vault=") {
            return PathBuf::from(stripped);
        }
    }

    // 2. S3A_VAULT environment variable
    if let Ok(env_path) = std::env::var("S3A_VAULT") {
        let trimmed = env_path.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    // 3. Current directory: if ./my.s3a exists, use it
    let default_path = PathBuf::from("./my.s3a");
    if default_path.exists() {
        return default_path;
    }

    // 4. Current directory: if exactly one .s3a file exists, use it
    if let Ok(entries) = fs::read_dir(".") {
        let mut s3a_files = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("s3a") {
                s3a_files.push(path);
            }
        }
        if s3a_files.len() == 1 {
            return s3a_files.remove(0);
        }
    }

    // 5. Fallback
    default_path
}

pub fn extract_positional_args(args: &[String]) -> Vec<&String> {
    let mut pos = Vec::new();
    let mut i = 2;
    while i < args.len() {
        if args[i] == "--vault" {
            i += 2;
            continue;
        }
        if args[i].starts_with('-') {
            i += 1;
            continue;
        }
        pos.push(&args[i]);
        i += 1;
    }
    pos
}

// =========================================================================
// Table Formatting
// =========================================================================

/// Formats tabular data as a clean ASCII border table.
pub fn format_ascii_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    if headers.is_empty() {
        return String::new();
    }

    let mut col_widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < col_widths.len() && cell.len() > col_widths[i] {
                col_widths[i] = cell.len();
            }
        }
    }

    // Cap column width to avoid terminal wrap explosion
    for w in &mut col_widths {
        *w = (*w).max(4);
    }

    let mut out = String::new();

    // Top border
    out.push('+');
    for w in &col_widths {
        out.push_str(&format!("{}+", "-".repeat(w + 2)));
    }
    out.push('\n');

    // Headers
    out.push('|');
    for (i, h) in headers.iter().enumerate() {
        let width = col_widths[i];
        out.push_str(&format!(" {:<width$} |", h, width = width));
    }
    out.push('\n');

    // Header separator
    out.push('+');
    for w in &col_widths {
        out.push_str(&format!("{}+", "-".repeat(w + 2)));
    }
    out.push('\n');

    // Rows
    for row in rows {
        out.push('|');
        for i in 0..headers.len() {
            let cell = row.get(i).map(|s| s.as_str()).unwrap_or("");
            let width = col_widths[i];
            out.push_str(&format!(" {:<width$} |", cell, width = width));
        }
        out.push('\n');
    }

    // Bottom border
    out.push('+');
    for w in &col_widths {
        out.push_str(&format!("{}+", "-".repeat(w + 2)));
    }
    out.push('\n');

    out
}

// =========================================================================
// Core Document Storage Helpers (Tile 23)
// =========================================================================


pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn format_timestamp_iso(ms: u64) -> String {
    let secs = ms / 1000;
    // Format human-friendly UTC timestamp
    if let Some(dt) = std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_secs(secs)) {
        let dur = dt.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
        let days = dur / 86400;
        let rem = dur % 86400;
        let hours = rem / 3600;
        let mins = (rem % 3600) / 60;
        let s = rem % 60;
        // Approximate year/month/day
        let mut year = 1970;
        let mut day_count = days;
        loop {
            let leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
            let y_days = if leap { 366 } else { 365 };
            if day_count >= y_days {
                day_count -= y_days;
                year += 1;
            } else {
                break;
            }
        }
        let leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
        let month_days = [
            31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31,
        ];
        let mut month = 1;
        for &md in &month_days {
            if day_count >= md {
                day_count -= md;
                month += 1;
            } else {
                break;
            }
        }
        let day = day_count + 1;
        format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC", year, month, day, hours, mins, s)
    } else {
        format!("{} ms", ms)
    }
}

pub fn write_document_record(path: &Path, key: &str, content: &[u8]) -> Result<(), String> {
    let key_bytes = key.as_bytes();
    let content_crc = crc32c::crc32c(content);
    let is_json = serde_json::from_slice::<serde_json::Value>(content).is_ok();
    let doc_kind = if is_json {
        DOC_KIND_JSON
    } else {
        DOC_KIND_GENERIC_KV
    };
    let mut flags = DOC_FLAG_ACTIVE;
    if is_json {
        flags |= DOC_FLAG_IS_JSON;
    }

    let mut doc_hdr = DocumentRecordHeader::default();
    doc_hdr.doc_kind = doc_kind;
    doc_hdr.flags = flags;
    doc_hdr.key_hash = compute_key_hash(doc_kind, key);
    doc_hdr.timestamp_ms = now_ms();
    doc_hdr.key_len = key_bytes.len() as u32;
    doc_hdr.content_len = content.len() as u32;
    doc_hdr.content_crc32 = content_crc;

    let header_size = std::mem::size_of::<DocumentRecordHeader>();
    let total_record_size = header_size + key_bytes.len() + content.len();

    if total_record_size > HYPER_TILE_PAYLOAD_SIZE {
        return Err(format!(
            "Value size ({} bytes) exceeds single tile payload limit ({} bytes)",
            content.len(),
            HYPER_TILE_PAYLOAD_SIZE - header_size - key_bytes.len()
        ));
    }

    let mut tile_payload = Vec::with_capacity(total_record_size);
    tile_payload.extend_from_slice(bytemuck::bytes_of(&doc_hdr));
    tile_payload.extend_from_slice(key_bytes);
    tile_payload.extend_from_slice(content);

    let mut tile_hdr = HyperTileHeader::new(0, TileType::DOCUMENT_STORE);
    tile_hdr.record_count = 1;
    tile_hdr.payload_bytes = tile_payload.len() as u32;
    tile_hdr.data_crc32 = crc32c::crc32c(&tile_payload);

    let mut writer = TileWriter::open_append(path)
        .map_err(|e| format!("Failed to open vault '{}' for writing: {}", path.display(), e))?;

    writer
        .write_raw_hyper_tile(&tile_hdr, &tile_payload)
        .map_err(|e| format!("Failed to commit record: {}", e))?;

    Ok(())
}

pub fn read_document_record(path: &Path, key: &str) -> Result<Option<Vec<u8>>, String> {
    let reader = MmapReader::open(path)
        .map_err(|e| format!("Could not open vault '{}': {}", path.display(), e))?;

    let tile_count = reader.tile_count() as usize;

    for idx in (0..tile_count).rev() {
        if let Some((hdr, payload)) = reader.get_tile(idx) {
            if hdr.tile_type == TileType::DOCUMENT_STORE {
                let mut offset = 0;
                let header_size = std::mem::size_of::<DocumentRecordHeader>();
                let mut matches = Vec::new();

                while offset + header_size <= payload.len() {
                    let doc_hdr: DocumentRecordHeader =
                        bytemuck::pod_read_unaligned(&payload[offset..offset + header_size]);
                    offset += header_size;

                    let k_len = doc_hdr.key_len as usize;
                    let c_len = doc_hdr.content_len as usize;
                    if offset + k_len + c_len > payload.len() {
                        break;
                    }
                    let k_bytes = &payload[offset..offset + k_len];
                    offset += k_len;
                    let c_bytes = &payload[offset..offset + c_len];
                    offset += c_len;

                    if doc_hdr.key_hash == compute_key_hash(doc_hdr.doc_kind, key) && k_bytes == key.as_bytes() {
                        matches.push((doc_hdr, c_bytes.to_vec()));
                    }
                }

                if let Some((last_hdr, last_content)) = matches.pop() {
                    if last_hdr.is_deleted() {
                        return Ok(None);
                    } else {
                        return Ok(Some(last_content));
                    }
                }
            }
        }
    }

    Ok(None)
}

pub struct StoredDocumentMeta {
    pub key: String,
    pub size_bytes: usize,
    pub timestamp_ms: u64,
}

pub fn list_all_documents(path: &Path) -> Result<Vec<StoredDocumentMeta>, String> {
    let reader = MmapReader::open(path)
        .map_err(|e| format!("Could not open vault '{}': {}", path.display(), e))?;

    let mut map: HashMap<String, StoredDocumentMeta> = HashMap::new();
    let tile_count = reader.tile_count() as usize;

    for idx in 0..tile_count {
        if let Some((hdr, payload)) = reader.get_tile(idx) {
            if hdr.tile_type == TileType::DOCUMENT_STORE {
                let mut offset = 0;
                let header_size = std::mem::size_of::<DocumentRecordHeader>();

                while offset + header_size <= payload.len() {
                    let doc_hdr: DocumentRecordHeader =
                        bytemuck::pod_read_unaligned(&payload[offset..offset + header_size]);
                    offset += header_size;

                    let k_len = doc_hdr.key_len as usize;
                    let c_len = doc_hdr.content_len as usize;
                    if offset + k_len + c_len > payload.len() {
                        break;
                    }
                    let k_bytes = &payload[offset..offset + k_len];
                    offset += k_len;
                    offset += c_len;

                    let key_str = String::from_utf8_lossy(k_bytes).to_string();
                    if doc_hdr.is_deleted() {
                        map.remove(&key_str);
                    } else {
                        map.insert(
                            key_str.clone(),
                            StoredDocumentMeta {
                                key: key_str,
                                size_bytes: c_len,
                                timestamp_ms: doc_hdr.timestamp_ms,
                            },
                        );
                    }
                }
            }
        }
    }

    let mut items: Vec<StoredDocumentMeta> = map.into_values().collect();
    items.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(items)
}

// =========================================================================
// Command Handlers
// =========================================================================

pub fn handle_init(args: &[String], verbose: bool) {
    let pos = extract_positional_args(args);
    let path = if let Some(p) = pos.first() {
        PathBuf::from(p)
    } else {
        resolve_vault_path(args)
    };

    if path.exists() {
        print_cli_error(
            &format!("vault already exists at '{}'", path.display()),
            Some("use 's3a info' or 's3a ls' to inspect it, or specify a new path: 's3a init ./other.s3a'"),
            verbose,
            None,
        );
    }

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            let _ = fs::create_dir_all(parent);
        }
    }

    if let Err(e) = TileWriter::create(&path) {
        print_cli_error(
            &format!("failed to initialize vault at '{}': {}", path.display(), e),
            Some("verify filesystem permissions and available disk space"),
            verbose,
            Some(&e.to_string()),
        );
    }

    // Initialize VCS repo so revision history, commits, and status work immediately
    let _ = s3a_vcs::VcsRepo::init(&path);

    println!("Initialized new S3A vault at {}", path.display());
    println!();
    println!("Next steps:");
    println!("  1. Store a key-value record:");
    println!("     s3a put name \"Alice\"");
    println!("     s3a get name");
    println!();
    println!("  2. Import data:");
    println!("     s3a import data.csv");
    println!("     s3a import data.json");
    println!();
    println!("  3. Explore your vault:");
    println!("     s3a ls");
    println!("     s3a info");
    println!("     s3a tour");
    println!();
    println!("  4. Query your records:");
    println!("     s3a query \"SIFT TELEMETRY WHERE SENSOR_ID = 42\"");
}

pub fn handle_put(args: &[String], verbose: bool) {
    let pos = extract_positional_args(args);
    let has_explicit_vault = args.iter().any(|a| a == "--vault" || a.starts_with("--vault="));

    let (vault_path, key, val_arg) = if !has_explicit_vault && pos.len() >= 3 {
        (PathBuf::from(pos[0]), pos[1].as_str(), pos[2].as_str())
    } else if pos.len() >= 2 {
        (resolve_vault_path(args), pos[0].as_str(), pos[1].as_str())
    } else {
        print_cli_error(
            "missing arguments: 's3a put' requires <key> and <value>",
            Some("usage: s3a put [vault] <key> <value|@file.json> [--vault <path>]"),
            verbose,
            None,
        );
    };

    let content: Vec<u8> = if let Some(file_path) = val_arg.strip_prefix('@') {
        match fs::read(file_path) {
            Ok(bytes) => bytes,
            Err(e) => print_cli_error(
                &format!("could not read file '{}': {}", file_path, e),
                Some("verify that the file exists and is readable"),
                verbose,
                Some(&e.to_string()),
            ),
        }
    } else {
        val_arg.as_bytes().to_vec()
    };

    // Auto-create vault if it does not exist yet (friendly default)
    if !vault_path.exists() {
        if let Err(e) = TileWriter::create(&vault_path) {
            print_cli_error(
                &format!("failed to create vault at '{}': {}", vault_path.display(), e),
                Some("check write permissions in the directory"),
                verbose,
                Some(&e.to_string()),
            );
        }
        let _ = s3a_vcs::VcsRepo::init(&vault_path);
    }

    if let Err(e) = write_document_record(&vault_path, key, &content) {
        print_cli_error(
            &e,
            Some("ensure the vault is not read-only and has sufficient disk space"),
            verbose,
            None,
        );
    }

    println!("OK: stored '{}' ({} bytes) in {}", key, content.len(), vault_path.display());
}

pub fn handle_get(args: &[String], verbose: bool) {
    let pos = extract_positional_args(args);
    if pos.is_empty() {
        print_cli_error(
            "missing argument: 's3a get' requires <key>",
            Some("usage: s3a get [vault] <key> [--vault <path>]"),
            verbose,
            None,
        );
    }

    let has_explicit_vault = args.iter().any(|a| a == "--vault" || a.starts_with("--vault="));
    let (vault_path, key) = if !has_explicit_vault && pos.len() >= 2 {
        (PathBuf::from(pos[0]), pos[1].as_str())
    } else {
        (resolve_vault_path(args), pos[0].as_str())
    };

    if !vault_path.exists() {
        print_cli_error(
            &format!("vault file '{}' does not exist", vault_path.display()),
            Some("run 's3a init' to create a new vault, or specify '--vault <path>'"),
            verbose,
            None,
        );
    }

    match read_document_record(&vault_path, key) {
        Ok(Some(bytes)) => {
            if let Ok(s) = std::str::from_utf8(&bytes) {
                print!("{}", s);
                if !s.ends_with('\n') {
                    println!();
                }
            } else {
                let _ = io::stdout().write_all(&bytes);
            }
        }
        Ok(None) => {
            print_cli_error(
                &format!("key '{}' not found in vault '{}'", key, vault_path.display()),
                Some("use 's3a ls' to list stored keys, or 's3a put <key> <value>' to store it"),
                verbose,
                None,
            );
        }
        Err(e) => {
            print_cli_error(
                &e,
                Some("run 's3a doctor' to diagnose archive integrity"),
                verbose,
                None,
            );
        }
    }
}

pub fn handle_import(args: &[String], verbose: bool) {
    let pos = extract_positional_args(args);
    if pos.is_empty() {
        print_cli_error(
            "missing argument: 's3a import' requires a CSV or JSON file",
            Some("usage: s3a import <data.csv|data.json> [--vault <path>]"),
            verbose,
            None,
        );
    }

    let file_path = Path::new(pos[0]);
    if !file_path.exists() {
        print_cli_error(
            &format!("input file '{}' does not exist", file_path.display()),
            Some("check the file path and try again"),
            verbose,
            None,
        );
    }

    let vault_path = resolve_vault_path(args);
    if !vault_path.exists() {
        if let Err(e) = TileWriter::create(&vault_path) {
            print_cli_error(
                &format!("failed to initialize vault at '{}': {}", vault_path.display(), e),
                Some("verify write permissions"),
                verbose,
                Some(&e.to_string()),
            );
        }
        let _ = s3a_vcs::VcsRepo::init(&vault_path);
    }

    let file_bytes = match fs::read(file_path) {
        Ok(b) => b,
        Err(e) => print_cli_error(
            &format!("failed to read input file '{}': {}", file_path.display(), e),
            Some("check file read permissions"),
            verbose,
            Some(&e.to_string()),
        ),
    };

    let filename = file_path.file_name().and_then(|n| n.to_str()).unwrap_or("data");
    let is_csv = filename.ends_with(".csv")
        || (!filename.ends_with(".json") && file_bytes.iter().any(|&b| b == b',') && file_bytes.iter().any(|&b| b == b'\n'));

    if is_csv {
        import_csv(&file_bytes, &vault_path, filename, verbose);
    } else {
        import_json(&file_bytes, &vault_path, filename, verbose);
    }
}

fn import_csv(bytes: &[u8], vault_path: &Path, filename: &str, verbose: bool) {
    let text = match std::str::from_utf8(bytes) {
        Ok(t) => t,
        Err(_) => print_cli_error(
            "CSV file is not valid UTF-8",
            Some("convert the file to UTF-8 before importing"),
            verbose,
            None,
        ),
    };

    let mut lines = text.lines();
    let header_line = match lines.next() {
        Some(h) => h,
        None => print_cli_error(
            "CSV file is empty",
            Some("provide a CSV file with at least a header and data rows"),
            verbose,
            None,
        ),
    };

    let headers: Vec<String> = parse_csv_line(header_line).into_iter().map(|s| s.trim().to_lowercase()).collect();

    // Check if CSV matches Telemetry format (timestamp, sensor_id, metric_id, value)
    let ts_idx = headers.iter().position(|h| h == "timestamp" || h == "time" || h == "ts");
    let sensor_idx = headers.iter().position(|h| h == "sensor_id" || h == "sensor");
    let metric_idx = headers.iter().position(|h| h == "metric_id" || h == "metric");
    let value_idx = headers.iter().position(|h| h == "value" || h == "val");

    if let (Some(ti), Some(si), Some(mi), Some(vi)) = (ts_idx, sensor_idx, metric_idx, value_idx) {
        // Import as Telemetry records
        let mut records = Vec::new();
        let mut timestamps = Vec::new();

        for line in lines {
            if line.trim().is_empty() {
                continue;
            }
            let fields = parse_csv_line(line);
            if fields.len() > ti.max(si).max(mi).max(vi) {
                let ts = fields[ti].trim().parse::<u64>().unwrap_or_else(|_| now_ms() / 1000);
                let sensor = fields[si].trim().parse::<u32>().unwrap_or(0);
                let metric = fields[mi].trim().parse::<u32>().unwrap_or(0);
                let value = fields[vi].trim().parse::<f64>().unwrap_or(0.0);

                records.push(TelemetryRecord::new(ts, sensor, metric, value));
                timestamps.push(ts);
            }
        }

        let count = records.len();
        if count == 0 {
            println!("Imported 0 records from {}", filename);
            return;
        }

        let mut writer = match TileWriter::open_append(vault_path) {
            Ok(w) => w,
            Err(e) => print_cli_error(
                &format!("failed to open vault '{}': {}", vault_path.display(), e),
                Some("check vault permissions"),
                verbose,
                Some(&e.to_string()),
            ),
        };

        let tile_rec_cap = HYPER_TILE_PAYLOAD_SIZE / std::mem::size_of::<TelemetryRecord>();
        let mut tiles_written = 0;
        for (rec_chunk, ts_chunk) in records.chunks(tile_rec_cap).zip(timestamps.chunks(tile_rec_cap)) {
            if let Err(e) = writer.write_hyper_tile(TileType::TELEMETRY, rec_chunk, Some(ts_chunk), None) {
                print_cli_error(
                    &format!("failed to write telemetry tile: {}", e),
                    Some("run 's3a doctor' to check archive health"),
                    verbose,
                    Some(&e.to_string()),
                );
            }
            tiles_written += 1;
        }

        println!(
            "Imported {} telemetry records from {} into {} ({} tile{} written)",
            count,
            filename,
            vault_path.display(),
            tiles_written,
            if tiles_written == 1 { "" } else { "s" }
        );
    } else {
        // Generic CSV: store each row as a structured document
        let stem = Path::new(filename).file_stem().and_then(|s| s.to_str()).unwrap_or("row");
        let id_idx = headers.iter().position(|h| h == "id" || h == "key" || h == "name");
        let mut count = 0;
        let mut tiles_written = 0;

        let mut current_tile_payload = Vec::new();
        let mut current_rec_count = 0;

        let flush_tile = |payload: &mut Vec<u8>, rec_count: &mut u32, tw_count: &mut usize| {
            if payload.is_empty() {
                return;
            }
            let mut tile_hdr = HyperTileHeader::new(0, TileType::DOCUMENT_STORE);
            tile_hdr.record_count = *rec_count;
            tile_hdr.payload_bytes = payload.len() as u32;
            tile_hdr.data_crc32 = crc32c::crc32c(payload);

            let mut writer = TileWriter::open_append(vault_path).unwrap();
            writer.write_raw_hyper_tile(&tile_hdr, payload).unwrap();
            *tw_count += 1;
            payload.clear();
            *rec_count = 0;
        };

        for (row_num, line) in lines.enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let fields = parse_csv_line(line);
            let mut obj = serde_json::Map::new();
            for (i, h) in headers.iter().enumerate() {
                let val = fields.get(i).map(|s| s.trim()).unwrap_or("");
                // Try number parse if possible
                if let Ok(num) = val.parse::<i64>() {
                    obj.insert(h.clone(), serde_json::Value::Number(num.into()));
                } else if let Ok(flt) = val.parse::<f64>() {
                    if let Some(n) = serde_json::Number::from_f64(flt) {
                        obj.insert(h.clone(), serde_json::Value::Number(n));
                    } else {
                        obj.insert(h.clone(), serde_json::Value::String(val.to_string()));
                    }
                } else {
                    obj.insert(h.clone(), serde_json::Value::String(val.to_string()));
                }
            }

            let key = if let Some(idx) = id_idx {
                fields.get(idx).map(|s| s.trim().to_string()).unwrap_or_else(|| format!("{}:{}", stem, row_num + 1))
            } else {
                format!("{}:{}", stem, row_num + 1)
            };

            let json_str = serde_json::to_string(&obj).unwrap_or_default();
            let key_bytes = key.as_bytes();
            let content_bytes = json_str.as_bytes();

            let mut doc_hdr = DocumentRecordHeader::default();
            doc_hdr.doc_kind = DOC_KIND_JSON;
            doc_hdr.flags = DOC_FLAG_ACTIVE | DOC_FLAG_IS_JSON;
            doc_hdr.key_hash = compute_key_hash(DOC_KIND_JSON, &key);
            doc_hdr.timestamp_ms = now_ms();
            doc_hdr.key_len = key_bytes.len() as u32;
            doc_hdr.content_len = content_bytes.len() as u32;
            doc_hdr.content_crc32 = crc32c::crc32c(content_bytes);

            let rec_size = std::mem::size_of::<DocumentRecordHeader>() + key_bytes.len() + content_bytes.len();
            if current_tile_payload.len() + rec_size > HYPER_TILE_PAYLOAD_SIZE {
                flush_tile(&mut current_tile_payload, &mut current_rec_count, &mut tiles_written);
            }

            current_tile_payload.extend_from_slice(bytemuck::bytes_of(&doc_hdr));
            current_tile_payload.extend_from_slice(key_bytes);
            current_tile_payload.extend_from_slice(content_bytes);
            current_rec_count += 1;
            count += 1;
        }

        flush_tile(&mut current_tile_payload, &mut current_rec_count, &mut tiles_written);

        println!(
            "Imported {} records from {} into {} ({} tile{} written)",
            count,
            filename,
            vault_path.display(),
            tiles_written,
            if tiles_written == 1 { "" } else { "s" }
        );
    }
}

fn import_json(bytes: &[u8], vault_path: &Path, filename: &str, verbose: bool) {
    let json_val: serde_json::Value = match serde_json::from_slice(bytes) {
        Ok(v) => v,
        Err(e) => print_cli_error(
            &format!("failed to parse JSON from '{}': {}", filename, e),
            Some("verify that the file contains valid JSON or NDJSON"),
            verbose,
            Some(&e.to_string()),
        ),
    };

    let stem = Path::new(filename).file_stem().and_then(|s| s.to_str()).unwrap_or("record");
    let mut count = 0;
    let mut tiles_written = 0;
    let mut current_tile_payload = Vec::new();
    let mut current_rec_count = 0;

    let flush_tile = |payload: &mut Vec<u8>, rec_count: &mut u32, tw_count: &mut usize| {
        if payload.is_empty() {
            return;
        }
        let mut tile_hdr = HyperTileHeader::new(0, TileType::DOCUMENT_STORE);
        tile_hdr.record_count = *rec_count;
        tile_hdr.payload_bytes = payload.len() as u32;
        tile_hdr.data_crc32 = crc32c::crc32c(payload);

        let mut writer = TileWriter::open_append(vault_path).unwrap();
        writer.write_raw_hyper_tile(&tile_hdr, payload).unwrap();
        *tw_count += 1;
        payload.clear();
        *rec_count = 0;
    };

    match json_val {
        serde_json::Value::Array(arr) => {
            for (idx, item) in arr.into_iter().enumerate() {
                let key = item
                    .get("id")
                    .or_else(|| item.get("key"))
                    .and_then(|v| v.as_str().or_else(|| v.as_i64().map(|n| Box::leak(n.to_string().into_boxed_str()) as &str)))
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("{}:{}", stem, idx + 1));

                let val_str = item.to_string();
                let key_bytes = key.as_bytes();
                let content_bytes = val_str.as_bytes();

                let mut doc_hdr = DocumentRecordHeader::default();
                doc_hdr.doc_kind = DOC_KIND_JSON;
                doc_hdr.flags = DOC_FLAG_ACTIVE | DOC_FLAG_IS_JSON;
                doc_hdr.key_hash = compute_key_hash(DOC_KIND_JSON, &key);
                doc_hdr.timestamp_ms = now_ms();
                doc_hdr.key_len = key_bytes.len() as u32;
                doc_hdr.content_len = content_bytes.len() as u32;
                doc_hdr.content_crc32 = crc32c::crc32c(content_bytes);

                let rec_size = std::mem::size_of::<DocumentRecordHeader>() + key_bytes.len() + content_bytes.len();
                if current_tile_payload.len() + rec_size > HYPER_TILE_PAYLOAD_SIZE {
                    flush_tile(&mut current_tile_payload, &mut current_rec_count, &mut tiles_written);
                }

                current_tile_payload.extend_from_slice(bytemuck::bytes_of(&doc_hdr));
                current_tile_payload.extend_from_slice(key_bytes);
                current_tile_payload.extend_from_slice(content_bytes);
                current_rec_count += 1;
                count += 1;
            }
        }
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                let val_str = if let serde_json::Value::String(s) = v {
                    s
                } else {
                    v.to_string()
                };

                let key_bytes = k.as_bytes();
                let content_bytes = val_str.as_bytes();

                let mut doc_hdr = DocumentRecordHeader::default();
                doc_hdr.doc_kind = DOC_KIND_JSON;
                doc_hdr.flags = DOC_FLAG_ACTIVE | DOC_FLAG_IS_JSON;
                doc_hdr.key_hash = compute_key_hash(DOC_KIND_JSON, &k);
                doc_hdr.timestamp_ms = now_ms();
                doc_hdr.key_len = key_bytes.len() as u32;
                doc_hdr.content_len = content_bytes.len() as u32;
                doc_hdr.content_crc32 = crc32c::crc32c(content_bytes);

                let rec_size = std::mem::size_of::<DocumentRecordHeader>() + key_bytes.len() + content_bytes.len();
                if current_tile_payload.len() + rec_size > HYPER_TILE_PAYLOAD_SIZE {
                    flush_tile(&mut current_tile_payload, &mut current_rec_count, &mut tiles_written);
                }

                current_tile_payload.extend_from_slice(bytemuck::bytes_of(&doc_hdr));
                current_tile_payload.extend_from_slice(key_bytes);
                current_tile_payload.extend_from_slice(content_bytes);
                current_rec_count += 1;
                count += 1;
            }
        }
        _ => {
            print_cli_error(
                "JSON root must be an array of objects or an object map",
                Some("format input data as a JSON array or JSON object"),
                verbose,
                None,
            );
        }
    }

    flush_tile(&mut current_tile_payload, &mut current_rec_count, &mut tiles_written);

    println!(
        "Imported {} records from {} into {} ({} tile{} written)",
        count,
        filename,
        vault_path.display(),
        tiles_written,
        if tiles_written == 1 { "" } else { "s" }
    );
}

pub fn handle_export(args: &[String], verbose: bool) {
    let pos = extract_positional_args(args);
    if pos.is_empty() {
        print_cli_error(
            "missing argument: 's3a export' requires an output file path",
            Some("usage: s3a export <out.csv|out.json> [--vault <path>]"),
            verbose,
            None,
        );
    }

    let out_path = Path::new(pos[0]);
    let vault_path = resolve_vault_path(args);

    if !vault_path.exists() {
        print_cli_error(
            &format!("vault file '{}' does not exist", vault_path.display()),
            Some("run 's3a init' to create a new vault, or specify '--vault <path>'"),
            verbose,
            None,
        );
    }

    let is_json = out_path.extension().and_then(|e| e.to_str()) == Some("json")
        || args.iter().any(|a| a == "--json");

    // 1. Check for stored documents (Tile 23)
    let docs = list_all_documents(&vault_path).unwrap_or_default();
    if !docs.is_empty() {
        let mut doc_items = Vec::new();
        for d in &docs {
            if let Ok(Some(bytes)) = read_document_record(&vault_path, &d.key) {
                if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                    doc_items.push((d.key.clone(), val));
                } else {
                    let s = String::from_utf8_lossy(&bytes).to_string();
                    doc_items.push((d.key.clone(), serde_json::Value::String(s)));
                }
            }
        }

        if is_json {
            let json_array: Vec<serde_json::Value> = doc_items
                .into_iter()
                .map(|(k, v)| match v {
                    serde_json::Value::Object(mut map) => {
                        if !map.contains_key("key") && !map.contains_key("id") {
                            map.insert("key".to_string(), serde_json::Value::String(k));
                        }
                        serde_json::Value::Object(map)
                    }
                    other => {
                        let mut map = serde_json::Map::new();
                        map.insert("key".to_string(), serde_json::Value::String(k));
                        map.insert("value".to_string(), other);
                        serde_json::Value::Object(map)
                    }
                })
                .collect();

            let out_str = serde_json::to_string_pretty(&json_array).unwrap_or_default();
            if let Err(e) = fs::write(out_path, out_str) {
                print_cli_error(
                    &format!("failed to write export to '{}': {}", out_path.display(), e),
                    Some("check destination write permissions"),
                    verbose,
                    Some(&e.to_string()),
                );
            }
            println!("Exported {} records from {} to {}", json_array.len(), vault_path.display(), out_path.display());
            return;
        } else {
            // CSV export
            let mut all_headers = Vec::new();
            for (_, v) in &doc_items {
                if let serde_json::Value::Object(map) = v {
                    for k in map.keys() {
                        if !all_headers.contains(k) {
                            all_headers.push(k.clone());
                        }
                    }
                }
            }
            if all_headers.is_empty() {
                all_headers.push("key".to_string());
                all_headers.push("value".to_string());
            }

            let mut csv_out = String::new();
            csv_out.push_str(&all_headers.join(","));
            csv_out.push('\n');

            let count = doc_items.len();
            for (k, v) in doc_items {
                match v {
                    serde_json::Value::Object(map) => {
                        let row: Vec<String> = all_headers
                            .iter()
                            .map(|h| {
                                map.get(h)
                                    .map(|val| match val {
                                        serde_json::Value::String(s) => {
                                            if s.contains(',') || s.contains('"') || s.contains('\n') {
                                                format!("\"{}\"", s.replace('"', "\"\""))
                                            } else {
                                                s.clone()
                                            }
                                        }
                                        serde_json::Value::Number(n) => n.to_string(),
                                        serde_json::Value::Bool(b) => b.to_string(),
                                        serde_json::Value::Null => String::new(),
                                        other => format!("\"{}\"", other.to_string().replace('"', "\"\"")),
                                    })
                                    .unwrap_or_default()
                            })
                            .collect();
                        csv_out.push_str(&row.join(","));
                        csv_out.push('\n');
                    }
                    other => {
                        let val_str = other.to_string();
                        csv_out.push_str(&format!("{},{}\n", k, val_str));
                    }
                }
            }

            if let Err(e) = fs::write(out_path, csv_out) {
                print_cli_error(
                    &format!("failed to write export to '{}': {}", out_path.display(), e),
                    Some("check destination write permissions"),
                    verbose,
                    Some(&e.to_string()),
                );
            }
            println!("Exported {} records from {} to {}", count, vault_path.display(), out_path.display());
            return;
        }
    }

    // 2. Check for telemetry records (Tile 0)
    if let Ok(reader) = MmapReader::open(&vault_path) {
        let mut telemetry_records = Vec::new();
        for floor_idx in 0..reader.tile_count() as usize {
            if let Some((hdr, payload)) = reader.get_tile(floor_idx) {
                if hdr.tile_type == TileType::TELEMETRY {
                    let rec_size = std::mem::size_of::<TelemetryRecord>();
                    let count = hdr.record_count as usize;
                    for i in 0..count {
                        if (i + 1) * rec_size <= payload.len() {
                            let rec: TelemetryRecord =
                                bytemuck::pod_read_unaligned(&payload[i * rec_size..(i + 1) * rec_size]);
                            telemetry_records.push(rec);
                        }
                    }
                }
            }
        }

        if !telemetry_records.is_empty() {
            let count = telemetry_records.len();
            if is_json {
                let json_items: Vec<serde_json::Value> = telemetry_records
                    .iter()
                    .map(|r| {
                        serde_json::json!({
                            "timestamp": r.timestamp,
                            "sensor_id": r.sensor_id,
                            "metric_id": r.metric_id,
                            "value": r.value
                        })
                    })
                    .collect();
                let out_str = serde_json::to_string_pretty(&json_items).unwrap_or_default();
                if let Err(e) = fs::write(out_path, out_str) {
                    print_cli_error(
                        &format!("failed to write export to '{}': {}", out_path.display(), e),
                        Some("check destination write permissions"),
                        verbose,
                        Some(&e.to_string()),
                    );
                }
            } else {
                let mut csv_out = String::from("timestamp,sensor_id,metric_id,value\n");
                for r in &telemetry_records {
                    csv_out.push_str(&format!("{},{},{},{}\n", r.timestamp, r.sensor_id, r.metric_id, r.value));
                }
                if let Err(e) = fs::write(out_path, csv_out) {
                    print_cli_error(
                        &format!("failed to write export to '{}': {}", out_path.display(), e),
                        Some("check destination write permissions"),
                        verbose,
                        Some(&e.to_string()),
                    );
                }
            }
            println!("Exported {} telemetry records from {} to {}", count, vault_path.display(), out_path.display());
            return;
        }
    }

    println!("Exported 0 records from {} to {}", vault_path.display(), out_path.display());
}

fn parse_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut in_quotes = false;
    let mut current = String::new();
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '"' => {
                if in_quotes && chars.peek() == Some(&'"') {
                    current.push('"');
                    chars.next();
                } else {
                    in_quotes = !in_quotes;
                }
            }
            ',' if !in_quotes => {
                fields.push(current.trim().to_string());
                current.clear();
            }
            _ => {
                current.push(c);
            }
        }
    }
    fields.push(current.trim().to_string());
    fields
}

pub fn handle_query(args: &[String], verbose: bool) {
    let json_mode = args.iter().any(|a| a == "--json");
    let pos = extract_positional_args(args);
    let query_opt = pos.first().map(|s| s.as_str());

    let raw_query = match query_opt {
        Some(q) => q,
        None => {
            print_cli_error(
                "missing query string",
                Some("usage: s3a query \"<S3A_QL_STATEMENT>\" [--json] [--vault <path>]"),
                verbose,
                None,
            );
        }
    };

    let vault_path = resolve_vault_path(args);

    // If query does not contain FROM, inject default vault path
    let effective_query = if !contains_case_insensitive_word(raw_query, "FROM")
        && !contains_case_insensitive_word(raw_query, "INTO")
    {
        let path_str = vault_path.to_string_lossy();
        if let Some(pos) = raw_query.to_uppercase().find("WHERE") {
            format!(
                "{} FROM \"{}\" {}",
                &raw_query[..pos].trim_end(),
                path_str,
                &raw_query[pos..]
            )
        } else if let Some(pos) = raw_query.to_uppercase().find("LIMIT") {
            format!(
                "{} FROM \"{}\" {}",
                &raw_query[..pos].trim_end(),
                path_str,
                &raw_query[pos..]
            )
        } else {
            format!("{} FROM \"{}\"", raw_query.trim_end(), path_str)
        }
    } else {
        raw_query.to_string()
    };

    let start = Instant::now();
    match execute_query(&effective_query) {
        Ok(result) => {
            let elapsed = start.elapsed();
            match result {
                QueryResult::Telemetry(rec) => {
                    if json_mode {
                        println!(
                            r#"{{"ok":true,"type":"fetch_telemetry","elapsed_us":{},"timestamp":{},"sensor_id":{},"metric_id":{},"value":{}}}"#,
                            elapsed.as_micros(),
                            rec.timestamp,
                            rec.sensor_id,
                            rec.metric_id,
                            rec.value
                        );
                    } else {
                        let headers = ["Coordinate", "Timestamp", "Sensor ID", "Metric ID", "Value"];
                        let rows = vec![vec![
                            "L0:T0:R0".to_string(),
                            rec.timestamp.to_string(),
                            rec.sensor_id.to_string(),
                            rec.metric_id.to_string(),
                            format!("{:.2}", rec.value),
                        ]];
                        print!("{}", format_ascii_table(&headers, &rows));
                        println!("(1 row in {:?})", elapsed);
                    }
                }
                QueryResult::TelemetryList(list) => {
                    if json_mode {
                        let records_json = list
                            .iter()
                            .map(|(r, c)| {
                                format!(
                                    r#"{{"coord":"{}","timestamp":{},"sensor_id":{},"metric_id":{},"value":{}}}"#,
                                    c, r.timestamp, r.sensor_id, r.metric_id, r.value
                                )
                            })
                            .collect::<Vec<_>>()
                            .join(",");
                        println!(
                            r#"{{"ok":true,"type":"sift_telemetry","elapsed_us":{},"count":{},"records":[{}]}}"#,
                            elapsed.as_micros(),
                            list.len(),
                            records_json
                        );
                    } else {
                        if list.is_empty() {
                            println!("(0 rows returned in {:?})", elapsed);
                        } else {
                            let headers = ["Coordinate", "Timestamp", "Sensor ID", "Metric ID", "Value"];
                            let rows: Vec<Vec<String>> = list
                                .iter()
                                .map(|(r, c)| {
                                    vec![
                                        c.to_string(),
                                        r.timestamp.to_string(),
                                        r.sensor_id.to_string(),
                                        r.metric_id.to_string(),
                                        format!("{:.2}", r.value),
                                    ]
                                })
                                .collect();
                            print!("{}", format_ascii_table(&headers, &rows));
                            println!("({} row{} in {:?})", list.len(), if list.len() == 1 { "" } else { "s" }, elapsed);
                        }
                    }
                }
                QueryResult::EmbeddingList(list) => {
                    if json_mode {
                        let records_json = list
                            .iter()
                            .map(|(r, score, c)| {
                                format!(
                                    r#"{{"coord":"{}","id":{},"timestamp":{},"score":{}}}"#,
                                    c, r.id, r.timestamp, score
                                )
                            })
                            .collect::<Vec<_>>()
                            .join(",");
                        println!(
                            r#"{{"ok":true,"type":"sift_embeddings","elapsed_us":{},"count":{},"records":[{}]}}"#,
                            elapsed.as_micros(),
                            list.len(),
                            records_json
                        );
                    } else {
                        if list.is_empty() {
                            println!("(0 matches returned in {:?})", elapsed);
                        } else {
                            let headers = ["Coordinate", "Record ID", "Timestamp", "Similarity Score"];
                            let rows: Vec<Vec<String>> = list
                                .iter()
                                .map(|(r, score, c)| {
                                    vec![
                                        c.to_string(),
                                        r.id.to_string(),
                                        r.timestamp.to_string(),
                                        format!("{:.4}", score),
                                    ]
                                })
                                .collect();
                            print!("{}", format_ascii_table(&headers, &rows));
                            println!("({} match{} in {:?})", list.len(), if list.len() == 1 { "" } else { "es" }, elapsed);
                        }
                    }
                }
                QueryResult::LearningActivityList(list) => {
                    if json_mode {
                        let records_json = list
                            .iter()
                            .map(|(r, c)| {
                                format!(
                                    r#"{{"coord":"{}","actor_id":{},"verb_id":{},"object_id":{},"score":{},"duration_ms":{}}}"#,
                                    c, r.actor_id, r.verb_id, r.object_id, r.result_score, r.duration_ms
                                )
                            })
                            .collect::<Vec<_>>()
                            .join(",");
                        println!(
                            r#"{{"ok":true,"type":"sift_learning_activities","elapsed_us":{},"count":{},"records":[{}]}}"#,
                            elapsed.as_micros(),
                            list.len(),
                            records_json
                        );
                    } else {
                        if list.is_empty() {
                            println!("(0 activities returned in {:?})", elapsed);
                        } else {
                            let headers = ["Coordinate", "Actor ID", "Verb ID", "Object ID", "Score", "Duration"];
                            let rows: Vec<Vec<String>> = list
                                .iter()
                                .map(|(r, c)| {
                                    vec![
                                        c.to_string(),
                                        r.actor_id.to_string(),
                                        r.verb_id.to_string(),
                                        r.object_id.to_string(),
                                        format!("{:.2}", r.result_score),
                                        format!("{} ms", r.duration_ms),
                                    ]
                                })
                                .collect();
                            print!("{}", format_ascii_table(&headers, &rows));
                            println!("({} row{} in {:?})", list.len(), if list.len() == 1 { "" } else { "s" }, elapsed);
                        }
                    }
                }
                QueryResult::PackData {
                    domain,
                    count,
                    summary,
                    json_payload,
                } => {
                    if json_mode {
                        let payload_frag = match json_payload {
                            Some(p) => format!(r#","payload":{}"#, p),
                            None => String::new(),
                        };
                        println!(
                            r#"{{"ok":true,"type":"pack_data","elapsed_us":{},"domain":"{}","count":{},"summary":"{}"{}}}"#,
                            elapsed.as_micros(),
                            domain.as_str(),
                            count,
                            summary.replace('"', "\\\""),
                            payload_frag
                        );
                    } else {
                        println!("--- S3A-QL SIFT PACK [{}] ({} records in {:?}) ---", domain.as_str(), count, elapsed);
                        println!("{}", summary);
                    }
                }
                QueryResult::Inserted { coordinate } => {
                    if json_mode {
                        println!(
                            r#"{{"ok":true,"type":"insert","elapsed_us":{},"coordinate":"{}"}}"#,
                            elapsed.as_micros(),
                            coordinate
                        );
                    } else {
                        println!("Record stored at coordinate: {} (in {:?})", coordinate, elapsed);
                    }
                }
                QueryResult::Deleted { success } => {
                    if json_mode {
                        println!(
                            r#"{{"ok":true,"type":"delete","elapsed_us":{},"success":{}}}"#,
                            elapsed.as_micros(),
                            success
                        );
                    } else {
                        println!("Delete operation status: {} (in {:?})", if success { "success" } else { "failed" }, elapsed);
                    }
                }
                QueryResult::Compacted { new_tile_count } => {
                    if json_mode {
                        println!(
                            r#"{{"ok":true,"type":"compact","elapsed_us":{},"new_tile_count":{}}}"#,
                            elapsed.as_micros(),
                            new_tile_count
                        );
                    } else {
                        println!("Compaction finished: {} active tiles in vault (in {:?})", new_tile_count, elapsed);
                    }
                }
                QueryResult::Fused { new_tile_count } => {
                    if json_mode {
                        println!(
                            r#"{{"ok":true,"type":"fuse","elapsed_us":{},"new_tile_count":{}}}"#,
                            elapsed.as_micros(),
                            new_tile_count
                        );
                    } else {
                        println!("Fusion finished: {} tiles consolidated (in {:?})", new_tile_count, elapsed);
                    }
                }
            }
        }
        Err(e) => {
            let err_msg = format!("{}", e);
            let mut suggestion = None;
            // Check for unrecognized domain or tile type
            for word in effective_query.split_whitespace() {
                let clean = word.trim_matches(|c: char| !c.is_alphanumeric());
                if let Some(s) = suggest_tile_type(clean) {
                    if s != clean.to_lowercase() {
                        suggestion = Some(s);
                        break;
                    }
                }
            }

            if let Some(sug) = suggestion {
                print_cli_error(
                    &err_msg,
                    Some(&format!("did you mean tile type '{}'? Supported: telemetry, embedding_128, document_store", sug)),
                    verbose,
                    Some(&err_msg),
                );
            } else {
                print_cli_error(
                    &err_msg,
                    Some("check S3A-QL syntax with 's3a help query' or verify file path in FROM clause"),
                    verbose,
                    Some(&err_msg),
                );
            }
        }
    }
}

fn contains_case_insensitive_word(text: &str, word: &str) -> bool {
    let target = word.to_uppercase();
    for token in text.split_whitespace() {
        if token.trim_matches(|c: char| !c.is_alphanumeric()).to_uppercase() == target {
            return true;
        }
    }
    false
}

pub fn handle_ls(args: &[String], verbose: bool) {
    let json_mode = args.iter().any(|a| a == "--json");
    let vault_path = resolve_vault_path(args);

    if !vault_path.exists() {
        print_cli_error(
            &format!("vault file '{}' does not exist", vault_path.display()),
            Some("run 's3a init' to create a new vault, or specify '--vault <path>'"),
            verbose,
            None,
        );
    }

    let docs = match list_all_documents(&vault_path) {
        Ok(d) => d,
        Err(e) => print_cli_error(&e, Some("run 's3a doctor' to inspect archive integrity"), verbose, None),
    };

    // Also inspect telemetry tiles for summary
    let mut telemetry_count = 0;
    if let Ok(reader) = MmapReader::open(&vault_path) {
        for i in 0..reader.tile_count() as usize {
            if let Some((hdr, _)) = reader.get_tile(i) {
                if hdr.tile_type == TileType::TELEMETRY {
                    telemetry_count += hdr.record_count as usize;
                }
            }
        }
    }

    if json_mode {
        let items_json = docs
            .iter()
            .map(|d| {
                format!(
                    r#"{{"key":"{}","size_bytes":{},"timestamp_ms":{}}}"#,
                    d.key.replace('"', "\\\""),
                    d.size_bytes,
                    d.timestamp_ms
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        println!(
            r#"{{"ok":true,"vault":"{}","count":{},"telemetry_records":{},"documents":[{}]}}"#,
            vault_path.display(),
            docs.len(),
            telemetry_count,
            items_json
        );
    } else {
        println!("Vault: {}", vault_path.display());
        if docs.is_empty() && telemetry_count == 0 {
            println!("(vault is empty: use 's3a put' or 's3a import' to add records)");
            return;
        }

        if !docs.is_empty() {
            let headers = ["KEY", "KIND", "SIZE", "LAST UPDATED"];
            let rows: Vec<Vec<String>> = docs
                .iter()
                .map(|d| {
                    vec![
                        d.key.clone(),
                        "document".to_string(),
                        format!("{} B", d.size_bytes),
                        format_timestamp_iso(d.timestamp_ms),
                    ]
                })
                .collect();
            print!("{}", format_ascii_table(&headers, &rows));
        }

        let mut summary_parts = Vec::new();
        if !docs.is_empty() {
            summary_parts.push(format!("{} document{}", docs.len(), if docs.len() == 1 { "" } else { "s" }));
        }
        if telemetry_count > 0 {
            summary_parts.push(format!("{} telemetry record{}", telemetry_count, if telemetry_count == 1 { "" } else { "s" }));
        }
        println!("Total: {}", summary_parts.join(", "));
    }
}

pub fn handle_info(args: &[String], verbose: bool) {
    let json_mode = args.iter().any(|a| a == "--json");
    let vault_path = resolve_vault_path(args);

    if !vault_path.exists() {
        print_cli_error(
            &format!("vault file '{}' does not exist", vault_path.display()),
            Some("run 's3a init' to create a new vault, or specify '--vault <path>'"),
            verbose,
            None,
        );
    }

    let file_size = fs::metadata(&vault_path).map(|m| m.len()).unwrap_or(0);
    let reader = match MmapReader::open(&vault_path) {
        Ok(r) => r,
        Err(e) => print_cli_error(
            &format!("failed to read vault '{}': {}", vault_path.display(), e),
            Some("run 's3a doctor' to diagnose corrupted archive bytes"),
            verbose,
            Some(&e.to_string()),
        ),
    };

    let total_tiles = reader.tile_count();

    // Group tiles by type
    let mut type_counts: BTreeMap<u32, (usize, usize)> = BTreeMap::new(); // tile_type -> (tile_count, record_count)
    for i in 0..total_tiles as usize {
        if let Some((hdr, _)) = reader.get_tile(i) {
            let entry = type_counts.entry(hdr.tile_type).or_insert((0, 0));
            entry.0 += 1;
            entry.1 += hdr.record_count as usize;
        }
    }

    // Check VCS repo for last commit
    let mut last_commit_info = None;
    if let Ok(vcs_repo) = s3a_vcs::VcsRepo::open(&vault_path) {
        if let Ok(commits) = vcs_repo.list_commits() {
            if let Some(latest) = commits.last() {
                last_commit_info = Some((
                    hex::encode(latest.commit_id),
                    latest.message().to_string(),
                    latest.committed_timestamp,
                ));
            }
        }
    }

    if json_mode {
        let types_json = type_counts
            .iter()
            .map(|(t, (tiles, recs))| {
                let name = s3a_core::registry::global_registry()
                    .get(*t)
                    .map(|d| d.name)
                    .unwrap_or("unknown");
                format!(r#"{{"type_id":{},"name":"{}","tiles":{},"records":{}}}"#, t, name, tiles, recs)
            })
            .collect::<Vec<_>>()
            .join(",");

        let commit_json = match &last_commit_info {
            Some((cid, msg, ts)) => format!(
                r#","last_commit":{{"commit_id":"{}","message":"{}","timestamp":{}}}"#,
                cid,
                msg.replace('"', "\\\""),
                ts
            ),
            None => String::new(),
        };

        println!(
            r#"{{"ok":true,"vault":"{}","file_size":{},"total_tiles":{},"types":[{}]{}}}"#,
            vault_path.display(),
            file_size,
            total_tiles,
            types_json,
            commit_json
        );
    } else {
        println!("=========================================================================");
        println!("                          S3A VAULT INFORMATION                          ");
        println!("=========================================================================");
        println!("Vault File:         {}", vault_path.display());
        println!(
            "File Size:          {} bytes ({:.2} KB)",
            file_size,
            file_size as f64 / 1024.0
        );
        println!("Storage Version:    1");
        println!("Committed Tiles:    {}", total_tiles);
        println!();
        println!("Tile Types Breakdown:");
        if type_counts.is_empty() {
            println!("  (no tiles committed yet)");
        } else {
            for (t, (tiles, recs)) in &type_counts {
                let name = s3a_core::registry::global_registry()
                    .get(*t)
                    .map(|d| d.name)
                    .unwrap_or("custom");
                println!(
                    "  - Tile {:<2} ({:<18}): {} tile{}, {} record{}",
                    t,
                    name,
                    tiles,
                    if *tiles == 1 { "" } else { "s" },
                    recs,
                    if *recs == 1 { "" } else { "s" }
                );
            }
        }
        println!();
        println!("Version Control (VCS):");
        if let Some((cid, msg, ts)) = last_commit_info {
            println!("  - Last Commit:    {} ({} UTC)", &cid[..8.min(cid.len())], format_timestamp_iso(ts * 1000));
            println!("  - Message:        {}", msg);
        } else {
            println!("  - Last Commit:    none (uncommitted snapshot)");
        }
        println!("=========================================================================");
    }
}

pub fn handle_demo(args: &[String], verbose: bool) {
    let pos = extract_positional_args(args);
    let demo_path = if let Some(p) = pos.first() {
        PathBuf::from(p)
    } else {
        PathBuf::from("./demo.s3a")
    };

    if demo_path.exists() {
        let _ = fs::remove_file(&demo_path);
    }

    if let Err(e) = TileWriter::create(&demo_path) {
        print_cli_error(
            &format!("failed to create demo vault at '{}': {}", demo_path.display(), e),
            Some("verify write permissions in the directory"),
            verbose,
            Some(&e.to_string()),
        );
    }

    // 1. Add Books (Documents)
    let books = [
        ("book:1", r#"{"id":1,"title":"The Rust Programming Language","author":"Steve Klabnik & Carol Nichols","year":2018,"rating":4.9}"#),
        ("book:2", r#"{"id":2,"title":"Designing Data-Intensive Applications","author":"Martin Kleppmann","year":2017,"rating":4.9}"#),
        ("book:3", r#"{"id":3,"title":"Structure and Interpretation of Computer Programs","author":"Harold Abelson","year":1985,"rating":4.8}"#),
    ];
    for (k, v) in &books {
        let _ = write_document_record(&demo_path, k, v.as_bytes());
    }

    // 2. Add Devices (Telemetry)
    let mut writer = TileWriter::open_append(&demo_path).unwrap();
    let now = now_ms() / 1000;
    let telemetry_recs = vec![
        TelemetryRecord::new(now - 100, 1, 10, 21.5),
        TelemetryRecord::new(now - 80, 1, 10, 21.8),
        TelemetryRecord::new(now - 60, 1, 10, 22.1),
        TelemetryRecord::new(now - 100, 2, 11, 45.2),
        TelemetryRecord::new(now - 80, 2, 11, 45.0),
        TelemetryRecord::new(now - 60, 2, 11, 44.8),
        TelemetryRecord::new(now - 100, 42, 1, 1013.25),
        TelemetryRecord::new(now - 80, 42, 1, 1013.30),
        TelemetryRecord::new(now - 60, 42, 1, 1013.35),
        TelemetryRecord::new(now - 40, 42, 1, 1013.40),
    ];
    let timestamps: Vec<u64> = telemetry_recs.iter().map(|r| r.timestamp).collect();
    let _ = writer.write_hyper_tile(TileType::TELEMETRY, &telemetry_recs, Some(&timestamps), None);

    // 3. Add Events (Documents)
    let events = [
        ("event:boot", r#"{"event":"system_boot","node":"node-alpha","status":"ok","timestamp":1700000000}"#),
        ("event:backup", r#"{"event":"snapshot_sealed","status":"verified","tiles":2,"timestamp":1700003600}"#),
    ];
    for (k, v) in &events {
        let _ = write_document_record(&demo_path, k, v.as_bytes());
    }

    // 4. VCS commit
    let _ = s3a_vcs::VcsRepo::init(&demo_path);
    if let Ok(repo) = s3a_vcs::VcsRepo::open(&demo_path) {
        let signing_key = ed25519_dalek::SigningKey::generate(&mut rand_core::OsRng);
        let _ = repo.commit("Initial demo dataset (books, devices, events)", "main", &signing_key, None);
    }

    println!("Created demo vault at {} (sample data loaded)", demo_path.display());
    println!();
    println!("Sample records created:");
    println!("  - Books:   3 documents (book:1, book:2, book:3)");
    println!("  - Devices: 10 telemetry sensor readings (sensor_id 1, 2, 42)");
    println!("  - Events:  2 system event documents (event:boot, event:backup)");
    println!();
    println!("Try these commands:");
    println!("  s3a get book:1 --vault {}", demo_path.display());
    println!("  s3a ls --vault {}", demo_path.display());
    println!("  s3a info --vault {}", demo_path.display());
    println!("  s3a tour {}", demo_path.display());
    println!("  s3a query \"SIFT TELEMETRY FROM \\\"{}\\\" WHERE SENSOR_ID = 42\"", demo_path.display());
}

// =========================================================================
// Command-Specific Help with Copy-Paste Examples
// =========================================================================

pub fn handle_help(args: &[String]) {
    let subcmd = args.iter().skip(2).find(|a| !a.starts_with('-')).map(|s| s.as_str());

    match subcmd {
        Some("init") => {
            println!("s3a init - Create a new S3A vault");
            println!();
            println!("Usage:");
            println!("  s3a init [path]");
            println!();
            println!("Description:");
            println!("  Initializes a new sector-aligned S3A archive with generation 1 dual A/B headers.");
            println!("  Defaults to './my.s3a' if no path is provided.");
            println!();
            println!("Examples:");
            println!("  s3a init");
            println!("  s3a init ./production.s3a");
            println!("  s3a init /var/data/telemetry.s3a");
        }
        Some("put") => {
            println!("s3a put - Store a key-value record in the vault");
            println!();
            println!("Usage:");
            println!("  s3a put <key> <value|@file.json> [--vault <path>]");
            println!();
            println!("Description:");
            println!("  Stores a key-value document in the vault's Document Store (Tile 23).");
            println!("  If the value starts with '@', it reads content directly from the specified file.");
            println!();
            println!("Examples:");
            println!("  s3a put greeting \"Hello, S3A!\"");
            println!("  s3a put app:config '{{\"theme\":\"dark\",\"max_workers\":4}}'");
            println!("  s3a put schema:v1 @schema.json --vault production.s3a");
        }
        Some("get") => {
            println!("s3a get - Retrieve a value by key from the vault");
            println!();
            println!("Usage:");
            println!("  s3a get <key> [--vault <path>]");
            println!("  s3a get <vault> <key>");
            println!();
            println!("Description:");
            println!("  Retrieves the latest stored document payload for <key> from Document Store (Tile 23).");
            println!();
            println!("Examples:");
            println!("  s3a get greeting");
            println!("  s3a get my.s3a greeting");
            println!("  s3a get production.s3a schema:v1 > exported_schema.json");
        }
        Some("import") => {
            println!("s3a import - Import records from CSV or JSON into the vault");
            println!();
            println!("Usage:");
            println!("  s3a import <data.csv|data.json> [--vault <path>]");
            println!();
            println!("Description:");
            println!("  Auto-detects format (CSV or JSON). Columns with timestamp, sensor_id,");
            println!("  metric_id, value are stored as high-speed SIMD Telemetry tiles; general");
            println!("  rows and JSON objects are stored as Document Store records.");
            println!();
            println!("Examples:");
            println!("  s3a import sensors.csv");
            println!("  s3a import dataset.json");
            println!("  s3a import telemetry_batch.csv --vault production.s3a");
        }
        Some("export") => {
            println!("s3a export - Export vault records to CSV or JSON");
            println!();
            println!("Usage:");
            println!("  s3a export <out.csv|out.json> [--json] [--vault <path>]");
            println!();
            println!("Description:");
            println!("  Exports stored documents or telemetry records from the vault to a CSV or JSON file.");
            println!();
            println!("Examples:");
            println!("  s3a export data.csv");
            println!("  s3a export backup.json");
            println!("  s3a export telemetry.csv --vault production.s3a");
        }
        Some("query") => {
            println!("s3a query - Execute native S3A-QL query statement");
            println!();
            println!("Usage:");
            println!("  s3a query \"<S3A_QL_STATEMENT>\" [--json] [--vault <path>]");
            println!();
            println!("Description:");
            println!("  Executes a native S3A-QL statement with readable table formatting or machine-readable JSON.");
            println!("  If FROM is omitted, the default or specified vault path is automatically targeted.");
            println!();
            println!("Examples:");
            println!("  s3a query \"SIFT TELEMETRY WHERE SENSOR_ID = 42\"");
            println!("  s3a query \"SIFT TELEMETRY FROM \\\"my.s3a\\\" WHERE TIME BETWEEN 1000 AND 2000\"");
            println!("  s3a query \"SIFT TELEMETRY WHERE VALUE > 90.0\" --json");
        }
        Some("ls") | Some("list") => {
            println!("s3a ls - List stored keys and records in the vault");
            println!();
            println!("Usage:");
            println!("  s3a ls [--json] [--vault <path>]");
            println!();
            println!("Description:");
            println!("  Lists all active document keys, sizes, modification timestamps, and record counts.");
            println!();
            println!("Examples:");
            println!("  s3a ls");
            println!("  s3a ls --json");
            println!("  s3a ls --vault ./production.s3a");
        }
        Some("info") => {
            println!("s3a info - Display vault metadata, tile breakdown, and commit history");
            println!();
            println!("Usage:");
            println!("  s3a info [--json] [--vault <path>]");
            println!();
            println!("Description:");
            println!("  Displays file size, tile counts by type, generation, active slot, and VCS commits.");
            println!();
            println!("Examples:");
            println!("  s3a info");
            println!("  s3a info --json");
            println!("  s3a info --vault /var/data/analytics.s3a");
        }
        Some("tour") => {
            println!("s3a tour - Tour archive building floors and inspect tile headers");
            println!();
            println!("Usage:");
            println!("  s3a tour [--json] [--floor N] [archive]");
            println!();
            println!("Description:");
            println!("  Provides an architectural tour of the archive lobby, floors, schemas, and Merkle seals.");
            println!();
            println!("Examples:");
            println!("  s3a tour");
            println!("  s3a tour my.s3a");
            println!("  s3a tour --floor 0 my.s3a");
        }
        Some("demo") => {
            println!("s3a demo - Create a sample vault with preloaded data");
            println!();
            println!("Usage:");
            println!("  s3a demo [path]");
            println!();
            println!("Description:");
            println!("  Creates a sample vault with books, telemetry sensor devices, and system audit events.");
            println!();
            println!("Examples:");
            println!("  s3a demo");
            println!("  s3a demo ./sample_store.s3a");
        }
        Some("seal") => {
            println!("s3a seal - Seal cold archive with Reed-Solomon parity and Merkle roots");
            println!();
            println!("Usage:");
            println!("  s3a seal <in_file> <out_file> [--about] [--record-roots]");
            println!();
            println!("Examples:");
            println!("  s3a seal my.s3a my.sealed.s3a");
            println!("  s3a seal my.s3a my.sealed.s3a --about --record-roots");
        }
        Some("verify") => {
            println!("s3a verify - Verify sealed parity, self-addressing, or anchors");
            println!();
            println!("Usage:");
            println!("  s3a verify [--json] <sealed_archive>");
            println!();
            println!("Examples:");
            println!("  s3a verify my.sealed.s3a");
            println!("  s3a verify --json my.sealed.s3a");
        }
        Some("telemetry") => {
            println!("s3a telemetry - Query, insert, or delete raw telemetry time-series records");
            println!();
            println!("Usage:");
            println!("  s3a telemetry get <file_path> <sensor_id> <metric_id> <min_ts> <max_ts>");
            println!("  s3a telemetry insert <file_path> <timestamp> <sensor_id> <metric_id> <value>");
            println!("  s3a telemetry delete <file_path> <sensor_id> <metric_id> <timestamp>");
            println!();
            println!("Examples:");
            println!("  s3a telemetry get my.s3a 1 10 0 2000000000");
            println!("  s3a telemetry insert my.s3a 1700000000 1 10 23.5");
            println!("  s3a telemetry delete my.s3a 1 10 1700000000");
        }
        Some(other) => {
            if let Some(sug) = suggest_command(other) {
                println!("Unknown command '{}'. Did you mean '{}'?", other, sug);
            } else {
                println!("Unknown command '{}'.", other);
            }
            println!("Run 's3a help' to see all available commands.");
        }
        None => {
            print_general_help();
        }
    }
}

pub fn print_general_help() {
    println!("S3A Storage Architecture (v{})", env!("CARGO_PKG_VERSION"));
    println!("High-performance, crash-resilient columnar database and storage engine.");
    println!();
    println!("FIRST-RUN COMMANDS:");
    println!("  s3a init [path]                  Create a new vault (default: ./my.s3a)");
    println!("  s3a put <key> <value|@file>      Store a key-value record in the vault");
    println!("  s3a get <key>                    Retrieve a value by key");
    println!("  s3a import <data.csv|data.json>  Import records from CSV or JSON");
    println!("  s3a export <data.csv|data.json>  Export records to CSV or JSON");
    println!("  s3a query \"<QL>\" [--json]        Execute native S3A-QL statement with table output");
    println!("  s3a ls [--json]                  List stored keys and records");
    println!("  s3a info [--json]                Vault summary (tiles, types, size, last commit)");
    println!("  s3a tour [--json] [archive]      Interactive building & floor tour");
    println!("  s3a demo [path]                  Create sample vault (books, devices, events)");
    println!("  s3a help [command]               Show detailed help and copy-paste examples");
    println!();
    println!("STABLE CORE OPS & MAINTENANCE:");
    println!("  s3a-cli flatten [--zstd] <in.s3a> <out.s3af>    Zero-run-elided flat transport copy");
    println!("  s3a-cli unflatten <in.s3af> <out.s3a>           Rebuild byte-identical original");
    println!("  s3a-cli inspect [--json] <archive>              Inspect Hyper-Tile layout, headers, and metadata");
    println!("  s3a-cli seal <in_file> <out_file>               Seal cold archive with Reed-Solomon parity");
    println!("  s3a-cli verify [--json] <archive>               Verify sealed parity, self-addressing, or anchors");
    println!("  s3a-cli status [--json] <archive>               Quick archive state, generation, slot, and addon mask");
    println!("  s3a-cli doctor [--json] <archive>               Deep integrity diagnosis (A/B slots, torn bytes)");
    println!("  s3a-cli recover <archive>                       Crash recovery: truncate torn bytes, harden A/B slots");
    println!("  s3a-cli compact <out_file> <in1> [in2...]       Compact files into stratified Hyper-Tiles");
    println!("  s3a-cli fuse <out_file> <in1> [in2...]          Consolidate multi-tile archives");
    println!("  s3a-cli query [--json] \"<S3A_QL_STATEMENT>\"     Execute versioned S3A-QL v1 statement");
    println!("  s3a telemetry get <file> <sensor> <metric> <min> <max>  Query raw telemetry records");
    println!();
    println!("DEVELOPER REPL, WIRE SERVER & PROTOCOLS:");
    println!("  s3a-cli shell                                   Interactive SQL-style REPL shell");
    println!("  s3a-cli serve [port]                            Web dashboard & function calling server (default: 8080)");
    println!("  s3a-cli serve-tcp [host:port]                   Low-latency binary wire protocol server (default: 9333)");
    println!("  s3a-cli mcp-server [archive] [--allow-writes]   Model Context Protocol (MCP) server");
    println!();
    println!("DOMAIN PACK & LAKEHOUSE INTEGRATIONS:");
    println!("  s3a-cli scholar ingest <json_dir> <out.s3a>     Ingest academic literature into Scholar Hyper-Tiles");
    println!("  s3a-cli scholar recheck <file> [--online]       Recheck citation fingerprints against sources & Crossref");
    println!("  s3a-cli scholar proof <file> --claim <id>       Print/export claim evidence chain (PROV-O JSON-LD)");
    println!("  s3a-cli scholar export-bib <file> --out <path>  Export BibTeX/RIS bibliography with companion .proof.json");
    println!("  s3a-cli export-iceberg <file> <out_dir> [name]  Export Apache Iceberg v1 metadata & Delta Lake UniForm log");
    println!("  s3a-cli export-orange <snapshot> <out_dir>      Export papers.tab + sessions.tab for Orange");
    println!("  s3a-cli snowflake-serve [port]                  Start Snowflake External Function REST API gateway");
    println!();
    println!("BENCHMARKS & STRESS HARNESSES (EXPERIMENTAL):");
    println!("  s3a-cli benchmark                               Compare raw JSON/uncompressed storage vs S3A Hyper-Tiles");
    println!("  s3a-cli benchmark-wearable                      Run wearable benchmark (4KB Flash page micro-tiles)");
    println!("  s3a-cli benchmark-robotics                      Run robotics platform benchmark (1 kHz stream ring-buffer)");
    println!("  s3a-cli benchmark-gis                           Run GIS Earth surface/subsurface 3D point cloud benchmark");
    println!("  s3a-cli benchmark-da                            Run L2 Rollup & Decentralized AI Data Availability benchmark");
    println!("  s3a-cli benchmark-bvh                           Run Hierarchical Tree of Hulls (BVH) O(log N) pruning benchmark");
    println!("  s3a-cli benchmark-3d                            Run 3D Hilbert curve locality & 14-DOP bounding volume benchmark");
    println!();
    println!("Run 's3a help <command>' for copy-paste examples of any command.");
}
