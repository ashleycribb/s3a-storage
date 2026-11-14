//! S3A Sandbox CLI and isolated subprocess entry point.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::exit;
use std::time::{SystemTime, UNIX_EPOCH};

use rand_core::{OsRng, RngCore};
use s3a_sandbox::{
    compute_record_signature, run_task, JobRecord, SandboxWorker, TaskRequest, TaskResponse,
    WorkQueue,
};
use serde_json::{json, Value};

fn print_usage() {
    eprintln!(
        "Usage:\n  \
         s3a-sandbox exec --input <file> --output <file>\n  \
         s3a-sandbox worker --vault <path> [--lease <seconds>] [--interval <ms>] [--max-jobs <n>]\n  \
         s3a-sandbox enqueue --vault <path> --kind <kind> --input <json_or_file>\n  \
         s3a-sandbox status --vault <path> [--job-id <hex>]\n"
    );
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_usage();
        exit(1);
    }

    match args[1].as_str() {
        "exec" => handle_exec(&args[2..]),
        "worker" => handle_worker(&args[2..]),
        "enqueue" => handle_enqueue(&args[2..]),
        "status" => handle_status(&args[2..]),
        "-h" | "--help" | "help" => {
            print_usage();
        }
        other => {
            eprintln!("Unknown command: '{}'", other);
            print_usage();
            exit(1);
        }
    }
}

fn handle_exec(args: &[String]) {
    let mut input_path: Option<PathBuf> = None;
    let mut output_path: Option<PathBuf> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--input" if i + 1 < args.len() => {
                input_path = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--output" if i + 1 < args.len() => {
                output_path = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            _ => {
                i += 1;
            }
        }
    }

    let input_path = match input_path {
        Some(p) => p,
        None => {
            eprintln!("Missing required argument: --input <path>");
            exit(1);
        }
    };
    let output_path = match output_path {
        Some(p) => p,
        None => {
            eprintln!("Missing required argument: --output <path>");
            exit(1);
        }
    };

    let req_bytes = match fs::read(&input_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Failed to read input file: {}", e);
            exit(1);
        }
    };

    let req: TaskRequest = match serde_json::from_slice(&req_bytes) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to parse TaskRequest JSON: {}", e);
            exit(1);
        }
    };

    let outcome = run_task(&req.kind, &req.inputs);
    let resp = match outcome {
        Ok(val) => TaskResponse {
            success: true,
            output: val,
            error: None,
        },
        Err(err_msg) => TaskResponse {
            success: false,
            output: Value::Null,
            error: Some(err_msg),
        },
    };

    let resp_bytes = serde_json::to_vec_pretty(&resp).unwrap_or_default();
    if let Err(e) = fs::write(&output_path, resp_bytes) {
        eprintln!("Failed to write output file: {}", e);
        exit(1);
    }
}

fn handle_worker(args: &[String]) {
    let mut vault_path: Option<PathBuf> = None;
    let mut lease_secs: u64 = 30;
    let mut interval_ms: u64 = 250;
    let mut max_jobs: Option<usize> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--vault" if i + 1 < args.len() => {
                vault_path = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--lease" if i + 1 < args.len() => {
                lease_secs = args[i + 1].parse().unwrap_or(30);
                i += 2;
            }
            "--interval" if i + 1 < args.len() => {
                interval_ms = args[i + 1].parse().unwrap_or(250);
                i += 2;
            }
            "--max-jobs" if i + 1 < args.len() => {
                max_jobs = args[i + 1].parse().ok();
                i += 2;
            }
            _ => {
                i += 1;
            }
        }
    }

    let vault_path = match vault_path {
        Some(p) => p,
        None => {
            eprintln!("Missing required argument: --vault <path>");
            exit(1);
        }
    };

    let queue = WorkQueue::new(vault_path);
    let worker = SandboxWorker::new(queue).with_lease_duration(lease_secs);
    eprintln!(
        "Started sandbox worker {} (lease: {}s, interval: {}ms)",
        hex::encode(worker.worker_id),
        lease_secs,
        interval_ms
    );

    match worker.run_loop(interval_ms, max_jobs) {
        Ok(count) => {
            eprintln!("Worker completed {} jobs.", count);
        }
        Err(e) => {
            eprintln!("Worker terminated with error: {}", e);
            exit(1);
        }
    }
}

fn handle_enqueue(args: &[String]) {
    let mut vault_path: Option<PathBuf> = None;
    let mut kind: Option<String> = None;
    let mut input_str: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--vault" if i + 1 < args.len() => {
                vault_path = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--kind" if i + 1 < args.len() => {
                kind = Some(args[i + 1].clone());
                i += 2;
            }
            "--input" if i + 1 < args.len() => {
                input_str = Some(args[i + 1].clone());
                i += 2;
            }
            _ => {
                i += 1;
            }
        }
    }

    let vault_path = match vault_path {
        Some(p) => p,
        None => {
            eprintln!("Missing required argument: --vault <path>");
            exit(1);
        }
    };
    let kind = match kind {
        Some(k) => k,
        None => {
            eprintln!("Missing required argument: --kind <kind>");
            exit(1);
        }
    };

    let inputs: Value = if let Some(raw) = input_str {
        if Path::new(&raw).exists() {
            let bytes = fs::read(&raw).unwrap_or_default();
            serde_json::from_slice(&bytes).unwrap_or(json!({ "file": raw }))
        } else {
            serde_json::from_str(&raw).unwrap_or(json!({ "raw": raw }))
        }
    } else {
        json!({})
    };

    let mut job_id = [0u8; 16];
    OsRng.fill_bytes(&mut job_id);

    let mut requested_by = [0u8; 16];
    OsRng.fill_bytes(&mut requested_by);

    let mut grant = [0u8; 16];
    OsRng.fill_bytes(&mut grant);

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut job = JobRecord {
        id: job_id,
        kind,
        inputs,
        requested_by,
        grant,
        created_at: now,
        signature: [0u8; 64],
    };
    job.signature = compute_record_signature(
        s3a_sandbox::RECORD_TYPE_JOB,
        &job_id,
        &serde_json::to_vec(&job.inputs).unwrap_or_default(),
        &requested_by,
    );

    let queue = WorkQueue::new(vault_path);
    match queue.enqueue(&job) {
        Ok(id) => {
            println!("{}", hex::encode(id));
        }
        Err(e) => {
            eprintln!("Failed to enqueue job: {}", e);
            exit(1);
        }
    }
}

fn handle_status(args: &[String]) {
    let mut vault_path: Option<PathBuf> = None;
    let mut target_job_id: Option<[u8; 16]> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--vault" if i + 1 < args.len() => {
                vault_path = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--job-id" if i + 1 < args.len() => {
                if let Ok(bytes) = hex::decode(&args[i + 1]) {
                    if bytes.len() == 16 {
                        let mut arr = [0u8; 16];
                        arr.copy_from_slice(&bytes);
                        target_job_id = Some(arr);
                    }
                }
                i += 2;
            }
            _ => {
                i += 1;
            }
        }
    }

    let vault_path = match vault_path {
        Some(p) => p,
        None => {
            eprintln!("Missing required argument: --vault <path>");
            exit(1);
        }
    };

    let queue = WorkQueue::new(vault_path);

    if let Some(jid) = target_job_id {
        match queue.get_job(&jid) {
            Ok(Some(view)) => {
                let out = json!({
                    "job_id": hex::encode(view.job.id),
                    "kind": view.job.kind,
                    "state": format!("{:?}", view.state),
                    "worker_id": hex::encode(view.worker_id),
                    "claimed_at": view.claimed_at,
                    "lease_expires_at": view.lease_expires_at,
                    "retry_count": view.retry_count,
                    "error": view.error,
                    "result": view.result.map(|r| r.output),
                });
                println!("{}", serde_json::to_string_pretty(&out).unwrap());
            }
            Ok(None) => {
                eprintln!("Job {} not found in vault", hex::encode(jid));
                exit(1);
            }
            Err(e) => {
                eprintln!("Failed to read vault: {}", e);
                exit(1);
            }
        }
    } else {
        match queue.list_jobs() {
            Ok(jobs) => {
                let list: Vec<Value> = jobs
                    .into_iter()
                    .map(|v| {
                        json!({
                            "job_id": hex::encode(v.job.id),
                            "kind": v.job.kind,
                            "state": format!("{:?}", v.state),
                            "worker_id": hex::encode(v.worker_id),
                            "retry_count": v.retry_count,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&list).unwrap());
            }
            Err(e) => {
                eprintln!("Failed to list vault jobs: {}", e);
                exit(1);
            }
        }
    }
}
