//! CLI commands for S3A one-command self-hosted stack (`s3a stack <init|up|down|status>`).
//!
//! Enforces:
//! 1. Real health checks: `s3a stack status` reports UP only after verifying port + response.
//! 2. Zero demo/placeholder data: Stopped services report DOWN.
//! 3. Zero secrets in config or logs.
//! 4. Cross-platform execution (Windows and Linux).

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use s3a_harness::stack::{HealthState, StackConfig, StackManager};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct StackState {
    pid: u32,
    mode: String, // "native" or "docker"
    config_path: String,
}

pub fn handle_stack_command(args: &[String], verbose: bool) {
    if args.len() < 3 {
        print_stack_help();
        return;
    }

    match args[2].as_str() {
        "init" => handle_init(&args[3..], verbose),
        "up" => handle_up(&args[3..], verbose),
        "status" => handle_status(&args[3..], verbose),
        "down" => handle_down(&args[3..], verbose),
        "_serve-native" => handle_serve_native(&args[3..], verbose),
        "--help" | "-h" | "help" => print_stack_help(),
        other => {
            eprintln!("Unknown stack subcommand: '{}'", other);
            print_stack_help();
            std::process::exit(1);
        }
    }
}

pub fn print_stack_help() {
    println!("S3A One-Command Self-Hosted Stack CLI");
    println!();
    println!("USAGE:");
    println!("  s3a stack <COMMAND> [OPTIONS]");
    println!();
    println!("COMMANDS:");
    println!("  init     Initialize stack.toml and docker-compose.yml");
    println!("           [--provider <local|gemini|anthropic|gateway>]");
    println!("  up       Start all stack services and verify live health");
    println!("           [--docker] [--native] [--foreground] [--config <path>]");
    println!(
        "  status   Inspect real-time health of all stack services (TCP probe + HTTP response)"
    );
    println!("           [--config <path>] [--json]");
    println!("  down     Stop all stack services and verify shutdown");
    println!("           [--docker] [--config <path>]");
    println!();
    println!("GUARANTEES:");
    println!("  - Services are reported UP only after an actual network health check.");
    println!(
        "  - Secrets (API keys, tokens) stay strictly in OS SecretStore or environment variables."
    );
    println!("  - Stopped services report DOWN with zero fabricated states.");
}

fn handle_init(args: &[String], _verbose: bool) {
    let mut provider = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--provider" && i + 1 < args.len() {
            provider = Some(args[i + 1].as_str());
            i += 2;
        } else {
            i += 1;
        }
    }

    let mgr = StackManager::new(".");
    match mgr.init(provider) {
        Ok((cfg, compose)) => {
            println!("Initialized S3A self-hosted stack configuration:");
            println!("  Config:  {}", cfg.display());
            println!("  Compose: {}", compose.display());
            println!();
            println!("To start the stack:");
            println!("  s3a stack up");
        }
        Err(e) => {
            eprintln!("Failed to initialize stack: {}", e);
            std::process::exit(1);
        }
    }
}

fn handle_up(args: &[String], _verbose: bool) {
    let mut config_path = PathBuf::from("stack.toml");
    let mut use_docker = false;
    let mut foreground = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--config" if i + 1 < args.len() => {
                config_path = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            "--docker" => {
                use_docker = true;
                i += 1;
            }
            "--native" => {
                use_docker = false;
                i += 1;
            }
            "--foreground" => {
                foreground = true;
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    if !config_path.exists() {
        println!(
            "Config '{}' not found. Initializing default stack configuration...",
            config_path.display()
        );
        let mgr = StackManager::new(".");
        if let Err(e) = mgr.init(None) {
            eprintln!("Failed to initialize stack: {}", e);
            std::process::exit(1);
        }
    }

    let config = match StackConfig::load(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Invalid stack configuration: {}", e);
            std::process::exit(1);
        }
    };

    let mgr = StackManager::new(".");

    if foreground {
        println!(
            "Starting S3A stack '{}' in foreground...",
            config.stack.name
        );
        match mgr.start_native(&config) {
            Ok(_handle) => {
                println!("Stack running. Press Ctrl+C to stop.");
                loop {
                    thread::sleep(Duration::from_secs(1));
                }
            }
            Err(e) => {
                eprintln!("Error starting stack: {}", e);
                std::process::exit(1);
            }
        }
    }

    if use_docker {
        println!("Starting S3A stack via Docker Compose...");
        let compose_file = mgr.default_compose_path();
        if !compose_file.exists() {
            let compose_content = config.generate_docker_compose();
            let _ = fs::write(&compose_file, compose_content);
        }

        let status = Command::new("docker")
            .args(["compose", "-f", compose_file.to_str().unwrap(), "up", "-d"])
            .status();

        match status {
            Ok(s) if s.success() => {
                record_state(StackState {
                    pid: 0,
                    mode: "docker".to_string(),
                    config_path: config_path.to_string_lossy().to_string(),
                });
            }
            _ => {
                eprintln!("Failed to launch Docker containers. Ensure Docker is running.");
                std::process::exit(1);
            }
        }
    } else {
        // Native background process execution
        println!("Starting S3A stack native services in background...");
        let current_exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("s3a"));

        let child = Command::new(current_exe)
            .args([
                "stack",
                "_serve-native",
                "--config",
                config_path.to_str().unwrap(),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();

        match child {
            Ok(c) => {
                let pid = c.id();
                record_state(StackState {
                    pid,
                    mode: "native".to_string(),
                    config_path: config_path.to_string_lossy().to_string(),
                });
            }
            Err(e) => {
                eprintln!("Failed to spawn native stack process: {}", e);
                std::process::exit(1);
            }
        }
    }

    // Wait and verify real health checks
    println!("Waiting for services to pass health checks...");
    let start = Instant::now();
    let timeout = Duration::from_secs(5);
    let mut final_status = mgr.status(&config);

    while start.elapsed() < timeout {
        final_status = mgr.status(&config);
        if final_status.overall == HealthState::Up {
            break;
        }
        thread::sleep(Duration::from_millis(150));
    }

    println!();
    print!("{}", final_status.to_display());

    if final_status.overall != HealthState::Up {
        eprintln!("Stack startup verification failed: one or more services failed health check.");
        std::process::exit(1);
    }

    println!("S3A stack '{}' is UP and healthy.", config.stack.name);
}

fn handle_status(args: &[String], _verbose: bool) {
    let mut config_path = PathBuf::from("stack.toml");
    let mut json_mode = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--config" if i + 1 < args.len() => {
                config_path = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            "--json" => {
                json_mode = true;
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    let config = if config_path.exists() {
        match StackConfig::load(&config_path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Invalid stack configuration: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        StackConfig::default()
    };

    let mgr = StackManager::new(".");
    let status = mgr.status(&config);

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&status).unwrap());
    } else {
        print!("{}", status.to_display());
    }

    if status.overall == HealthState::Down || status.overall == HealthState::Unhealthy {
        std::process::exit(1);
    }
}

fn handle_down(args: &[String], _verbose: bool) {
    let mut config_path = PathBuf::from("stack.toml");
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--config" && i + 1 < args.len() {
            config_path = PathBuf::from(&args[i + 1]);
            i += 2;
        } else {
            i += 1;
        }
    }

    let state = load_state();

    if let Some(st) = &state {
        if st.mode == "docker" {
            println!("Stopping Docker Compose stack...");
            let _ = Command::new("docker").args(["compose", "down"]).status();
        } else if st.pid > 0 {
            println!("Terminating native stack process (PID {})...", st.pid);
            kill_process(st.pid);
        }
    } else {
        println!("No active stack state recorded. Verifying services are down...");
    }

    clear_state();

    // Verify all services are stopped
    let config = if config_path.exists() {
        StackConfig::load(&config_path).unwrap_or_default()
    } else {
        StackConfig::default()
    };

    let mgr = StackManager::new(".");
    let status = mgr.status(&config);

    let any_up = status.services.iter().any(|s| s.state == HealthState::Up);
    if any_up {
        eprintln!("Warning: Some services still appear to be responding:");
        print!("{}", status.to_display());
    } else {
        println!("S3A stack successfully shut down. All services are DOWN.");
    }
}

fn handle_serve_native(args: &[String], _verbose: bool) {
    let mut config_path = PathBuf::from("stack.toml");
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--config" && i + 1 < args.len() {
            config_path = PathBuf::from(&args[i + 1]);
            i += 2;
        } else {
            i += 1;
        }
    }

    let config = match StackConfig::load(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to load config: {}", e);
            std::process::exit(1);
        }
    };

    let mgr = StackManager::new(".");
    match mgr.start_native(&config) {
        Ok(_handle) => loop {
            thread::sleep(Duration::from_secs(1));
        },
        Err(e) => {
            eprintln!("Native stack error: {}", e);
            std::process::exit(1);
        }
    }
}

fn state_file_path() -> PathBuf {
    PathBuf::from(".stack").join("state.json")
}

fn record_state(state: StackState) {
    let p = state_file_path();
    if let Some(parent) = p.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string(&state) {
        let _ = fs::write(p, json);
    }
}

fn load_state() -> Option<StackState> {
    let p = state_file_path();
    if !p.exists() {
        return None;
    }
    let content = fs::read_to_string(p).ok()?;
    serde_json::from_str(&content).ok()
}

fn clear_state() {
    let p = state_file_path();
    let _ = fs::remove_file(p);
}

fn kill_process(pid: u32) {
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/F", "/PID", &pid.to_string()])
            .output();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill").args(["-9", &pid.to_string()]).output();
    }
}
