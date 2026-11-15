//! One-command self-hosted S3A stack manager, configuration, and health monitoring.
//!
//! Provides `s3a stack init`, `s3a stack up`, `s3a stack status`, and `s3a stack down`.
//!
//! Architectural Guarantees:
//! 1. Real Health Checks: A service is reported as `Up` strictly after an actual network health
//!    check (TCP port + response verification). Failed checks override any "running" label.
//! 2. Zero Secrets in Config/Logs: Secrets stay strictly in the OS SecretStore or environment variables.
//!    Config parsing and docker-compose generation enforce zero plaintext secrets.
//! 3. Cross-Platform: Works identically on Linux and Windows.
//! 4. Core Isolation: Zero stack code in `s3a-core` or `s3a-engine`.

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

// =============================================================================
// Stack Configuration
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StackConfig {
    pub stack: StackSection,
    pub model: ModelSection,
    pub services: ServicesSection,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StackSection {
    pub name: String,
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,
    #[serde(default = "default_log_dir")]
    pub log_dir: PathBuf,
    #[serde(default)]
    pub dev: bool,
    #[serde(default)]
    pub mode: Option<String>,
}

impl StackSection {
    pub fn is_dev(&self) -> bool {
        self.dev
            || self
                .mode
                .as_deref()
                .map(|m| m.eq_ignore_ascii_case("dev"))
                .unwrap_or(false)
    }
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

fn default_data_dir() -> PathBuf {
    PathBuf::from("./data")
}

fn default_log_dir() -> PathBuf {
    PathBuf::from("./logs")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelSection {
    pub provider: String, // "local", "gemini", "anthropic", "gateway"
    pub model_name: String,
    #[serde(default = "default_model_base_url")]
    pub base_url: String,
    #[serde(default)]
    pub max_tokens: Option<usize>,
    #[serde(default)]
    pub temperature: Option<f64>,
}

fn default_model_base_url() -> String {
    "http://127.0.0.1:11434/v1".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServicesSection {
    pub vault: ServiceConfig,
    pub lrs: LrsServiceConfig,
    pub mcp: ServiceConfig,
    pub harness: HarnessServiceConfig,
    pub sandbox: SandboxServiceConfig,
    #[serde(default)]
    pub otel: OtelServiceConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServiceConfig {
    pub enabled: bool,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LrsServiceConfig {
    pub enabled: bool,
    pub port: u16,
    #[serde(default = "default_lrs_archive")]
    pub archive: String,
}

fn default_lrs_archive() -> String {
    "lrs.s3a".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HarnessServiceConfig {
    pub enabled: bool,
    pub port: u16,
    #[serde(default = "default_harness_config_path")]
    pub config_path: PathBuf,
}

fn default_harness_config_path() -> PathBuf {
    PathBuf::from("./agents/scholar_reviewer.toml")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SandboxServiceConfig {
    pub enabled: bool,
    #[serde(default = "default_sandbox_port")]
    pub port: u16,
    #[serde(default = "default_workers")]
    pub workers: usize,
    #[serde(default = "default_poll_interval")]
    pub poll_interval_ms: u64,
}

fn default_sandbox_port() -> u16 {
    8083
}

fn default_workers() -> usize {
    2
}

fn default_poll_interval() -> u64 {
    500
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OtelServiceConfig {
    pub enabled: bool,
    #[serde(default = "default_otel_port")]
    pub port: u16,
    #[serde(default = "default_otel_grpc_port")]
    pub grpc_port: u16,
}

fn default_otel_port() -> u16 {
    4318
}

fn default_otel_grpc_port() -> u16 {
    4317
}

impl Default for OtelServiceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: default_otel_port(),
            grpc_port: default_otel_grpc_port(),
        }
    }
}

impl Default for StackConfig {
    fn default() -> Self {
        Self {
            stack: StackSection {
                name: "s3a-node".to_string(),
                host: "127.0.0.1".to_string(),
                data_dir: PathBuf::from("./data"),
                log_dir: PathBuf::from("./logs"),
                dev: false,
                mode: None,
            },
            model: ModelSection {
                provider: "local".to_string(),
                model_name: "llama3".to_string(),
                base_url: "http://127.0.0.1:11434/v1".to_string(),
                max_tokens: Some(4096),
                temperature: Some(0.2),
            },
            services: ServicesSection {
                vault: ServiceConfig {
                    enabled: true,
                    port: 8042,
                },
                lrs: LrsServiceConfig {
                    enabled: true,
                    port: 8080,
                    archive: "lrs.s3a".to_string(),
                },
                mcp: ServiceConfig {
                    enabled: true,
                    port: 8081,
                },
                harness: HarnessServiceConfig {
                    enabled: true,
                    port: 8082,
                    config_path: PathBuf::from("./agents/scholar_reviewer.toml"),
                },
                sandbox: SandboxServiceConfig {
                    enabled: true,
                    port: default_sandbox_port(),
                    workers: 2,
                    poll_interval_ms: 500,
                },
                otel: OtelServiceConfig::default(),
            },
        }
    }
}

// =============================================================================
// TOML Parsing & Generation (Dependency-Free)
// =============================================================================

#[derive(Debug)]
pub enum StackError {
    Io(std::io::Error),
    Config(String),
    SecretInConfigFile(String),
    HealthCheckFailed { service: String, reason: String },
}

impl std::fmt::Display for StackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {}", e),
            Self::Config(s) => write!(f, "Config error: {}", s),
            Self::SecretInConfigFile(k) => write!(
                f,
                "Security policy violation: secret found in config file ({}). Secrets must stay in environment or SecretStore!",
                k
            ),
            Self::HealthCheckFailed { service, reason } => {
                write!(f, "Service '{}' failed health check: {}", service, reason)
            }
        }
    }
}

impl std::error::Error for StackError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for StackError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl StackConfig {
    /// Validates that no sensitive API keys or secret tokens are embedded in config.
    pub fn validate_no_secrets(&self, raw_toml: &str) -> Result<(), StackError> {
        let suspicious_keys = [
            "api_key",
            "apikey",
            "secret_key",
            "secret",
            "password",
            "auth_token",
            "access_token",
            "bearer_token",
            "private_key",
        ];

        let lower = raw_toml.to_lowercase();
        for line in lower.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('#') {
                continue;
            }
            for key in suspicious_keys {
                if trimmed.contains(key) && trimmed.contains('=') {
                    // Check if there is an actual non-empty value assigned
                    let parts: Vec<&str> = trimmed.splitn(2, '=').collect();
                    if parts.len() == 2 {
                        let val = parts[1].trim().trim_matches('"').trim_matches('\'');
                        if !val.is_empty() && !val.starts_with('$') {
                            return Err(StackError::SecretInConfigFile(key.to_string()));
                        }
                    }
                }
            }
            if trimmed.contains("sk-") || trimmed.contains("aizasy") {
                return Err(StackError::SecretInConfigFile(
                    "detected API key token pattern".to_string(),
                ));
            }
        }
        Ok(())
    }

    /// Parse `stack.toml` using simple structured section extraction.
    pub fn from_toml(content: &str) -> Result<Self, StackError> {
        let mut config = Self::default();
        config.validate_no_secrets(content)?;

        let mut current_section = String::new();
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                current_section = trimmed[1..trimmed.len() - 1].trim().to_string();
                continue;
            }

            if let Some((k, v)) = parse_kv(trimmed) {
                match current_section.as_str() {
                    "stack" => match k {
                        "name" => config.stack.name = unquote(v),
                        "host" => config.stack.host = unquote(v),
                        "data_dir" => config.stack.data_dir = PathBuf::from(unquote(v)),
                        "log_dir" => config.stack.log_dir = PathBuf::from(unquote(v)),
                        "dev" => config.stack.dev = parse_bool(v),
                        "mode" => config.stack.mode = Some(unquote(v)),
                        _ => {}
                    },
                    "model" => match k {
                        "provider" => config.model.provider = unquote(v),
                        "model_name" => config.model.model_name = unquote(v),
                        "base_url" => config.model.base_url = unquote(v),
                        "max_tokens" => config.model.max_tokens = v.parse().ok(),
                        "temperature" => config.model.temperature = v.parse().ok(),
                        _ => {}
                    },
                    "services.vault" => match k {
                        "enabled" => config.services.vault.enabled = parse_bool(v),
                        "port" => {
                            if let Ok(p) = v.parse() {
                                config.services.vault.port = p;
                            }
                        }
                        _ => {}
                    },
                    "services.lrs" => match k {
                        "enabled" => config.services.lrs.enabled = parse_bool(v),
                        "port" => {
                            if let Ok(p) = v.parse() {
                                config.services.lrs.port = p;
                            }
                        }
                        "archive" => config.services.lrs.archive = unquote(v),
                        _ => {}
                    },
                    "services.mcp" => match k {
                        "enabled" => config.services.mcp.enabled = parse_bool(v),
                        "port" => {
                            if let Ok(p) = v.parse() {
                                config.services.mcp.port = p;
                            }
                        }
                        _ => {}
                    },
                    "services.harness" => match k {
                        "enabled" => config.services.harness.enabled = parse_bool(v),
                        "port" => {
                            if let Ok(p) = v.parse() {
                                config.services.harness.port = p;
                            }
                        }
                        "config_path" => {
                            config.services.harness.config_path = PathBuf::from(unquote(v))
                        }
                        _ => {}
                    },
                    "services.sandbox" => match k {
                        "enabled" => config.services.sandbox.enabled = parse_bool(v),
                        "port" => {
                            if let Ok(p) = v.parse() {
                                config.services.sandbox.port = p;
                            }
                        }
                        "workers" => {
                            if let Ok(w) = v.parse() {
                                config.services.sandbox.workers = w;
                            }
                        }
                        "poll_interval_ms" => {
                            if let Ok(pi) = v.parse() {
                                config.services.sandbox.poll_interval_ms = pi;
                            }
                        }
                        _ => {}
                    },
                    "services.otel" => match k {
                        "enabled" => config.services.otel.enabled = parse_bool(v),
                        "port" => {
                            if let Ok(p) = v.parse() {
                                config.services.otel.port = p;
                            }
                        }
                        "grpc_port" => {
                            if let Ok(p) = v.parse() {
                                config.services.otel.grpc_port = p;
                            }
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
        }

        Ok(config)
    }

    /// Generate standard `stack.toml` string.
    pub fn to_toml(&self) -> String {
        format!(
            r#"# S3A One-Command Self-Hosted Stack Configuration
# Secrets (GEMINI_API_KEY, ANTHROPIC_API_KEY, etc.) must NEVER be placed in this file.
# They stay strictly in environment variables or SecretStore.

[stack]
name = "{name}"
host = "{host}"
data_dir = "{data_dir}"
log_dir = "{log_dir}"
dev = {dev}

[model]
provider = "{provider}"
model_name = "{model_name}"
base_url = "{base_url}"
max_tokens = {max_tokens}
temperature = {temperature:.2}

[services.vault]
enabled = {vault_enabled}
port = {vault_port}

[services.lrs]
enabled = {lrs_enabled}
port = {lrs_port}
archive = "{lrs_archive}"

[services.mcp]
enabled = {mcp_enabled}
port = {mcp_port}

[services.harness]
enabled = {harness_enabled}
port = {harness_port}
config_path = "{harness_config}"

[services.sandbox]
enabled = {sandbox_enabled}
port = {sandbox_port}
workers = {sandbox_workers}
poll_interval_ms = {sandbox_poll}

[services.otel]
enabled = {otel_enabled}
port = {otel_port}
grpc_port = {otel_grpc_port}
"#,
            name = self.stack.name,
            host = self.stack.host,
            data_dir = self.stack.data_dir.display(),
            log_dir = self.stack.log_dir.display(),
            dev = self.stack.is_dev(),
            provider = self.model.provider,
            model_name = self.model.model_name,
            base_url = self.model.base_url,
            max_tokens = self.model.max_tokens.unwrap_or(4096),
            temperature = self.model.temperature.unwrap_or(0.2),
            vault_enabled = self.services.vault.enabled,
            vault_port = self.services.vault.port,
            lrs_enabled = self.services.lrs.enabled,
            lrs_port = self.services.lrs.port,
            lrs_archive = self.services.lrs.archive,
            mcp_enabled = self.services.mcp.enabled,
            mcp_port = self.services.mcp.port,
            harness_enabled = self.services.harness.enabled,
            harness_port = self.services.harness.port,
            harness_config = self.services.harness.config_path.display(),
            sandbox_enabled = self.services.sandbox.enabled,
            sandbox_port = self.services.sandbox.port,
            sandbox_workers = self.services.sandbox.workers,
            sandbox_poll = self.services.sandbox.poll_interval_ms,
            otel_enabled = self.services.otel.enabled,
            otel_port = self.services.otel.port,
            otel_grpc_port = self.services.otel.grpc_port,
        )
    }

    /// Load config from file with safety checks.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, StackError> {
        let content = fs::read_to_string(path)?;
        Self::from_toml(&content)
    }

    /// Save config to file.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), StackError> {
        let toml = self.to_toml();
        if let Some(parent) = path.as_ref().parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, toml)?;
        Ok(())
    }

    /// Generate production-grade `docker-compose.yml` matching this configuration.
    pub fn generate_docker_compose(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            r#"# S3A One-Command Self-Hosted Lab Stack
# Generated for stack: {name}
# Secrets (GEMINI_API_KEY, ANTHROPIC_API_KEY) are passed via environment variables, NEVER hardcoded.

version: '3.8'

services:
"#,
            name = self.stack.name
        ));

        if self.services.vault.enabled {
            out.push_str(&format!(
                r#"  vault:
    image: ghcr.io/ashleycribb/s3a-storage:latest
    container_name: s3a-vault
    command: ["s3a", "stack", "up", "--role", "vault"]
    ports:
      - "{port}:{port}"
    volumes:
      - {data_dir}:/data
    restart: unless-stopped
    networks:
      - s3a-net

"#,
                port = self.services.vault.port,
                data_dir = self.stack.data_dir.display()
            ));
        }

        if self.services.lrs.enabled {
            out.push_str(&format!(
                r#"  lrs:
    image: ghcr.io/ashleycribb/s3a-lrs-server:latest
    container_name: s3a-lrs
    command: ["s3a-lrs-server", "--port", "{port}", "--data-dir", "/data", "-a", "{archive}"]
    ports:
      - "{port}:{port}"
    volumes:
      - {data_dir}:/data
    restart: unless-stopped
    networks:
      - s3a-net

"#,
                port = self.services.lrs.port,
                data_dir = self.stack.data_dir.display(),
                archive = self.services.lrs.archive
            ));
        }

        if self.services.mcp.enabled {
            out.push_str(&format!(
                r#"  mcp:
    image: ghcr.io/ashleycribb/s3a-harness:latest
    container_name: s3a-mcp
    command: ["s3a-harness", "mcp", "--port", "{port}"]
    ports:
      - "{port}:{port}"
    volumes:
      - {data_dir}:/data
    restart: unless-stopped
    networks:
      - s3a-net

"#,
                port = self.services.mcp.port,
                data_dir = self.stack.data_dir.display()
            ));
        }

        if self.services.harness.enabled {
            out.push_str(&format!(
                r#"  harness:
    image: ghcr.io/ashleycribb/s3a-harness:latest
    container_name: s3a-harness
    command: ["s3a-harness", "run", "--config", "{config_path}"]
    ports:
      - "{port}:{port}"
    environment:
      - GEMINI_API_KEY=${{GEMINI_API_KEY:-}}
      - ANTHROPIC_API_KEY=${{ANTHROPIC_API_KEY:-}}
      - MODEL_GATEWAY_KEY=${{MODEL_GATEWAY_KEY:-}}
    volumes:
      - {data_dir}:/data
      - {config_dir}:/configs:ro
    restart: unless-stopped
    networks:
      - s3a-net

"#,
                port = self.services.harness.port,
                config_path = self.services.harness.config_path.display(),
                data_dir = self.stack.data_dir.display(),
                config_dir = self
                    .services
                    .harness
                    .config_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .display()
            ));
        }

        if self.services.sandbox.enabled {
            out.push_str(&format!(
                r#"  sandbox:
    image: ghcr.io/ashleycribb/s3a-sandbox:latest
    container_name: s3a-sandbox-worker
    command: ["s3a-sandbox", "worker", "--vault", "/data/vault.s3a", "--interval", "{poll_interval}", "--port", "{port}"]
    ports:
      - "{port}:{port}"
    deploy:
      replicas: {workers}
    volumes:
      - {data_dir}:/data
    restart: unless-stopped
    networks:
      - s3a-net

"#,
                port = self.services.sandbox.port,
                poll_interval = self.services.sandbox.poll_interval_ms,
                workers = self.services.sandbox.workers,
                data_dir = self.stack.data_dir.display()
            ));
        }

        if self.services.otel.enabled {
            out.push_str(&format!(
                r#"  otel:
    image: otel/opentelemetry-collector-contrib:latest
    container_name: s3a-otel-collector
    ports:
      - "{http_port}:{http_port}"
      - "{grpc_port}:{grpc_port}"
    restart: unless-stopped
    networks:
      - s3a-net

"#,
                http_port = self.services.otel.port,
                grpc_port = self.services.otel.grpc_port
            ));
        }

        out.push_str(
            r#"networks:
  s3a-net:
    driver: bridge
"#,
        );

        out
    }
}

fn parse_kv(line: &str) -> Option<(&str, &str)> {
    let mut parts = line.splitn(2, '=');
    let k = parts.next()?.trim();
    let v = parts.next()?.trim();
    Some((k, v))
}

fn unquote(s: &str) -> String {
    s.trim_matches('"').trim_matches('\'').to_string()
}

fn parse_bool(s: &str) -> bool {
    matches!(s.to_lowercase().as_str(), "true" | "1" | "yes")
}

// =============================================================================
// Real Health Checks & Status Evaluation
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealthState {
    Up,
    Down,
    Unhealthy,
    Disabled,
}

impl std::fmt::Display for HealthState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Up => write!(f, "UP"),
            Self::Down => write!(f, "DOWN"),
            Self::Unhealthy => write!(f, "UNHEALTHY"),
            Self::Disabled => write!(f, "DISABLED"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceKind {
    Vault,
    Lrs,
    Mcp,
    Harness,
    Sandbox,
    Otel,
}

impl ServiceKind {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Vault => "vault",
            Self::Lrs => "lrs",
            Self::Mcp => "mcp",
            Self::Harness => "harness",
            Self::Sandbox => "sandbox",
            Self::Otel => "otel",
        }
    }

    pub fn binary_name(&self) -> &'static str {
        match self {
            Self::Vault => "s3a",
            Self::Lrs => "s3a-lrs-server",
            Self::Mcp => "s3a-harness",
            Self::Harness => "s3a-harness",
            Self::Sandbox => "s3a-sandbox",
            Self::Otel => "otelcol",
        }
    }
}

/// Locates a compiled binary across build output directories and system PATH.
pub fn find_binary(name: &str) -> Option<PathBuf> {
    let binary_name = if cfg!(windows) && !name.ends_with(".exe") {
        format!("{}.exe", name)
    } else {
        name.to_string()
    };

    let mut search_dirs = Vec::new();

    if let Ok(cur) = std::env::current_exe() {
        if let Some(p) = cur.parent() {
            search_dirs.push(p.to_path_buf());
            if let Some(pp) = p.parent() {
                search_dirs.push(pp.to_path_buf());
            }
        }
    }

    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        let p = PathBuf::from(manifest);
        search_dirs.push(p.join("../../target/debug"));
        search_dirs.push(p.join("../../target/release"));
        search_dirs.push(p.join("../target/debug"));
        search_dirs.push(p.join("../target/release"));
        search_dirs.push(p.join("target/debug"));
        search_dirs.push(p.join("target/release"));
    }

    search_dirs.push(PathBuf::from("target").join("debug"));
    search_dirs.push(PathBuf::from("target").join("release"));
    search_dirs.push(PathBuf::from("..").join("target").join("debug"));
    search_dirs.push(PathBuf::from("../..").join("target").join("debug"));

    for dir in &search_dirs {
        let candidate = dir.join(&binary_name);
        if candidate.is_file() {
            if let Ok(canonical) = candidate.canonicalize() {
                return Some(canonical);
            }
            return Some(candidate);
        }
    }

    if let Ok(paths) = std::env::var("PATH") {
        for path in std::env::split_paths(&paths) {
            let candidate = path.join(&binary_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    None
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServiceStatus {
    pub service: String,
    pub enabled: bool,
    pub port: Option<u16>,
    pub state: HealthState,
    pub latency_ms: Option<u64>,
    pub detail: String,
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StackStatus {
    pub stack_name: String,
    pub host: String,
    pub overall: HealthState,
    pub services: Vec<ServiceStatus>,
    #[serde(default)]
    pub dev_mode: bool,
}

impl StackStatus {
    pub fn to_display(&self) -> String {
        let mut out = String::new();
        let dev_label = if self.dev_mode { " (dev mode)" } else { "" };
        out.push_str(&format!(
            "S3A Stack: {} [{}]{}\n",
            self.stack_name, self.overall, dev_label
        ));
        out.push_str("----------------------------------------------------------------------\n");
        out.push_str(&format!(
            "{:<12} {:<10} {:<8} {:<10} {}\n",
            "SERVICE", "STATE", "PORT", "LATENCY", "DETAIL"
        ));
        out.push_str("----------------------------------------------------------------------\n");
        for s in &self.services {
            let port_str = s
                .port
                .map(|p| p.to_string())
                .unwrap_or_else(|| "-".to_string());
            let lat_str = s
                .latency_ms
                .map(|l| format!("{}ms", l))
                .unwrap_or_else(|| "-".to_string());
            out.push_str(&format!(
                "{:<12} {:<10} {:<8} {:<10} {}\n",
                s.service, s.state, port_str, lat_str, s.detail
            ));
        }
        out.push_str("----------------------------------------------------------------------\n");
        out
    }
}

/// Executes a genuine network health check against a service.
///
/// CRITICAL RULES:
/// 1. Reports `Up` only after an actual network health check against it (port + response).
///    Never reports `Up` from config or a started process alone.
///    A failed check strictly overrides every "running" label.
/// 2. S3A services must return HTTP 200 with valid JSON specifying service name, non-empty version,
///    status == "ok", and passing dependency checks.
/// 3. OTel counts as up only after a real OTLP or health response, never TCP alone.
pub fn check_service_health(host: &str, port: u16, kind: ServiceKind) -> ServiceStatus {
    let start = Instant::now();
    let addr_str = format!("{}:{}", host, port);
    let socket_addr = addr_str
        .parse::<SocketAddr>()
        .unwrap_or_else(|_| SocketAddr::from(([127, 0, 0, 1], port)));

    // 1. Fast TCP probe (instant fail if service is down / not listening)
    let tcp_timeout = Duration::from_millis(300);
    if let Err(tcp_err) = TcpStream::connect_timeout(&socket_addr, tcp_timeout) {
        let bin_exists = match kind {
            ServiceKind::Vault => find_binary("s3a")
                .or_else(|| find_binary("s3a-cli"))
                .is_some(),
            ServiceKind::Lrs => find_binary("s3a-lrs-server").is_some(),
            ServiceKind::Mcp | ServiceKind::Harness => find_binary("s3a-harness").is_some(),
            ServiceKind::Sandbox => find_binary("s3a-sandbox").is_some(),
            ServiceKind::Otel => true,
        };

        let detail = if !bin_exists {
            format!(
                "Binary '{}' not implemented / not found",
                kind.binary_name()
            )
        } else {
            format!("Connection refused ({})", tcp_err)
        };

        return ServiceStatus {
            service: kind.name().to_string(),
            enabled: true,
            port: Some(port),
            state: HealthState::Down,
            latency_ms: None,
            detail,
            pid: None,
        };
    }

    // 2. OTel Collector probe (OTLP or HTTP health endpoint)
    if kind == ServiceKind::Otel {
        let url = format!("http://{}:{}/", host, port);
        return match ureq::get(&url).timeout(Duration::from_millis(1000)).call() {
            Ok(resp) => {
                let status = resp.status();
                let elapsed = start.elapsed().as_millis() as u64;
                if status == 200 || status == 204 {
                    ServiceStatus {
                        service: "otel".to_string(),
                        enabled: true,
                        port: Some(port),
                        state: HealthState::Up,
                        latency_ms: Some(elapsed),
                        detail: format!("OTel HTTP response {} ({}ms)", status, elapsed),
                        pid: None,
                    }
                } else {
                    ServiceStatus {
                        service: "otel".to_string(),
                        enabled: true,
                        port: Some(port),
                        state: HealthState::Unhealthy,
                        latency_ms: Some(elapsed),
                        detail: format!("OTel HTTP status {}", status),
                        pid: None,
                    }
                }
            }
            Err(ureq::Error::Status(status, _)) => {
                let elapsed = start.elapsed().as_millis() as u64;
                ServiceStatus {
                    service: "otel".to_string(),
                    enabled: true,
                    port: Some(port),
                    state: HealthState::Unhealthy,
                    latency_ms: Some(elapsed),
                    detail: format!("OTel HTTP error status {}", status),
                    pid: None,
                }
            }
            Err(e) => ServiceStatus {
                service: "otel".to_string(),
                enabled: true,
                port: Some(port),
                state: HealthState::Unhealthy,
                latency_ms: None,
                detail: format!("OTel probe failed (not OTLP/HTTP): {}", e),
                pid: None,
            },
        };
    }

    // 3. HTTP GET /health probe for S3A native services
    let url = format!("http://{}:{}/health", host, port);
    let http_timeout = Duration::from_millis(1500);

    match ureq::get(&url).timeout(http_timeout).call() {
        Ok(resp) => {
            let status = resp.status();
            let elapsed = start.elapsed().as_millis() as u64;

            if status != 200 {
                return ServiceStatus {
                    service: kind.name().to_string(),
                    enabled: true,
                    port: Some(port),
                    state: HealthState::Unhealthy,
                    latency_ms: Some(elapsed),
                    detail: format!("HTTP status {}", status),
                    pid: None,
                };
            }

            let mut reader = resp.into_reader();
            let mut body = String::new();
            if let Err(e) = reader.read_to_string(&mut body) {
                return ServiceStatus {
                    service: kind.name().to_string(),
                    enabled: true,
                    port: Some(port),
                    state: HealthState::Unhealthy,
                    latency_ms: Some(elapsed),
                    detail: format!("Failed to read health response body: {}", e),
                    pid: None,
                };
            }

            let json: serde_json::Value = match serde_json::from_str(&body) {
                Ok(j) => j,
                Err(e) => {
                    return ServiceStatus {
                        service: kind.name().to_string(),
                        enabled: true,
                        port: Some(port),
                        state: HealthState::Unhealthy,
                        latency_ms: Some(elapsed),
                        detail: format!("Invalid JSON response: {}", e),
                        pid: None,
                    };
                }
            };

            // Strict check 1: service name
            let service_name = json.get("service").and_then(|v| v.as_str()).unwrap_or("");
            if service_name != kind.name() {
                return ServiceStatus {
                    service: kind.name().to_string(),
                    enabled: true,
                    port: Some(port),
                    state: HealthState::Unhealthy,
                    latency_ms: Some(elapsed),
                    detail: format!(
                        "Service name mismatch: expected '{}', got '{}'",
                        kind.name(),
                        service_name
                    ),
                    pid: None,
                };
            }

            // Strict check 2: non-empty version
            let version = json.get("version").and_then(|v| v.as_str()).unwrap_or("");
            if version.is_empty() {
                return ServiceStatus {
                    service: kind.name().to_string(),
                    enabled: true,
                    port: Some(port),
                    state: HealthState::Unhealthy,
                    latency_ms: Some(elapsed),
                    detail: "Version missing or empty in health JSON".to_string(),
                    pid: None,
                };
            }

            // Strict check 3: status == "ok" or "healthy"
            let health_status = json.get("status").and_then(|v| v.as_str()).unwrap_or("");
            if health_status != "ok" && health_status != "healthy" {
                return ServiceStatus {
                    service: kind.name().to_string(),
                    enabled: true,
                    port: Some(port),
                    state: HealthState::Unhealthy,
                    latency_ms: Some(elapsed),
                    detail: format!("Service reported status '{}'", health_status),
                    pid: None,
                };
            }

            // Strict check 4: actual service dependency verification
            let dep_ok = match kind {
                ServiceKind::Vault => {
                    json.get("vault_opened").and_then(|v| v.as_bool()) == Some(true)
                }
                ServiceKind::Lrs => {
                    json.get("store_writable").and_then(|v| v.as_bool()) == Some(true)
                }
                ServiceKind::Mcp => {
                    json.get("sandbox_ready").and_then(|v| v.as_bool()) == Some(true)
                }
                ServiceKind::Harness => {
                    json.get("agent_configured").and_then(|v| v.as_bool()) == Some(true)
                }
                ServiceKind::Sandbox => {
                    json.get("queue_accessible").and_then(|v| v.as_bool()) == Some(true)
                }
                ServiceKind::Otel => true,
            };

            if !dep_ok {
                return ServiceStatus {
                    service: kind.name().to_string(),
                    enabled: true,
                    port: Some(port),
                    state: HealthState::Unhealthy,
                    latency_ms: Some(elapsed),
                    detail: format!(
                        "Dependency verification failed for service '{}'",
                        kind.name()
                    ),
                    pid: None,
                };
            }

            ServiceStatus {
                service: kind.name().to_string(),
                enabled: true,
                port: Some(port),
                state: HealthState::Up,
                latency_ms: Some(elapsed),
                detail: format!("Healthy: {} v{} ({}ms)", kind.name(), version, elapsed),
                pid: None,
            }
        }
        Err(e) => ServiceStatus {
            service: kind.name().to_string(),
            enabled: true,
            port: Some(port),
            state: HealthState::Unhealthy,
            latency_ms: None,
            detail: format!("Port open but HTTP health probe failed: {}", e),
            pid: None,
        },
    }
}

// =============================================================================
// Native Stack Process Handle
// =============================================================================

/// Internal handle to real child processes running in background.
pub struct NativeStackHandle {
    pub children: Vec<std::process::Child>,
    pub services: Vec<String>,
    pub pids: Vec<u32>,
    pub ports: HashMap<String, u16>,
}

impl NativeStackHandle {
    pub fn stop(&mut self) {
        for child in self.children.iter_mut() {
            let pid = child.id();
            let _ = child.kill();

            #[cfg(windows)]
            {
                let _ = std::process::Command::new("taskkill")
                    .args(["/F", "/T", "/PID", &pid.to_string()])
                    .output();
            }
            #[cfg(not(windows))]
            {
                let _ = std::process::Command::new("kill")
                    .args(["-9", &pid.to_string()])
                    .output();
            }

            let start = Instant::now();
            let timeout = Duration::from_secs(10);
            while start.elapsed() < timeout {
                match child.try_wait() {
                    Ok(Some(_status)) => break,
                    Ok(None) => thread::sleep(Duration::from_millis(50)),
                    Err(_) => break,
                }
            }
        }
    }
}

impl Drop for NativeStackHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

fn spawn_stack_child(bin: &Path, args: &[&str]) -> std::io::Result<std::process::Child> {
    let mut cmd = std::process::Command::new(bin);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW (0x08000000) | CREATE_NEW_PROCESS_GROUP (0x00000200) | DETACHED_PROCESS (0x00000008)
        cmd.creation_flags(0x08000000 | 0x00000200 | 0x00000008);
    }

    cmd.spawn()
}

// =============================================================================
// Stack Manager
// =============================================================================

pub struct StackManager {
    root_dir: PathBuf,
}

impl StackManager {
    pub fn new<P: AsRef<Path>>(root_dir: P) -> Self {
        Self {
            root_dir: root_dir.as_ref().to_path_buf(),
        }
    }

    pub fn default_config_path(&self) -> PathBuf {
        self.root_dir.join("stack.toml")
    }

    pub fn default_compose_path(&self) -> PathBuf {
        self.root_dir.join("docker-compose.yml")
    }

    pub fn default_state_path(&self) -> PathBuf {
        self.root_dir.join(".stack").join("state.json")
    }

    /// Initialize stack files (`stack.toml` and `docker-compose.yml`).
    pub fn init(&self, provider: Option<&str>) -> Result<(PathBuf, PathBuf), StackError> {
        let mut config = StackConfig::default();
        if let Some(p) = provider {
            config.model.provider = p.to_string();
        }

        let cfg_path = self.default_config_path();
        let compose_path = self.default_compose_path();

        config.save(&cfg_path)?;
        let compose_content = config.generate_docker_compose();
        fs::write(&compose_path, compose_content)?;

        Ok((cfg_path, compose_path))
    }

    /// Query live status of all services.
    ///
    /// CRITICAL RULE 1:
    /// Reports `Up` only after an actual network health check against it (port + response).
    /// Failed checks strictly override any "running" label.
    pub fn status(&self, config: &StackConfig) -> StackStatus {
        let mut services = Vec::new();
        let host = &config.stack.host;

        // 1. Vault
        if config.services.vault.enabled {
            services.push(check_service_health(
                host,
                config.services.vault.port,
                ServiceKind::Vault,
            ));
        } else {
            services.push(disabled_status("vault", Some(config.services.vault.port)));
        }

        // 2. LRS Server
        if config.services.lrs.enabled {
            services.push(check_service_health(
                host,
                config.services.lrs.port,
                ServiceKind::Lrs,
            ));
        } else {
            services.push(disabled_status("lrs", Some(config.services.lrs.port)));
        }

        // 3. MCP Server
        if config.services.mcp.enabled {
            services.push(check_service_health(
                host,
                config.services.mcp.port,
                ServiceKind::Mcp,
            ));
        } else {
            services.push(disabled_status("mcp", Some(config.services.mcp.port)));
        }

        // 4. Harness Agent Server
        if config.services.harness.enabled {
            services.push(check_service_health(
                host,
                config.services.harness.port,
                ServiceKind::Harness,
            ));
        } else {
            services.push(disabled_status(
                "harness",
                Some(config.services.harness.port),
            ));
        }

        // 5. Sandbox Workers
        if config.services.sandbox.enabled {
            services.push(check_service_health(
                host,
                config.services.sandbox.port,
                ServiceKind::Sandbox,
            ));
        } else {
            services.push(disabled_status(
                "sandbox",
                Some(config.services.sandbox.port),
            ));
        }

        // 6. Optional OTel Collector
        if config.services.otel.enabled {
            services.push(check_service_health(
                host,
                config.services.otel.port,
                ServiceKind::Otel,
            ));
        } else {
            services.push(disabled_status("otel", Some(config.services.otel.port)));
        }

        // Compute overall state: failed checks override any positive label
        let overall = if services.iter().any(|s| {
            s.enabled && (s.state == HealthState::Down || s.state == HealthState::Unhealthy)
        }) {
            HealthState::Down
        } else if services
            .iter()
            .any(|s| s.enabled && s.state == HealthState::Up)
        {
            HealthState::Up
        } else {
            HealthState::Disabled
        };

        StackStatus {
            stack_name: config.stack.name.clone(),
            host: config.stack.host.clone(),
            overall,
            services,
            dev_mode: config.stack.is_dev(),
        }
    }

    /// Launches real native binary child processes for all enabled services and verifies live health.
    pub fn start_native(&self, config: &StackConfig) -> Result<NativeStackHandle, StackError> {
        let mut children = Vec::new();
        let mut services = Vec::new();
        let mut pids = Vec::new();
        let mut ports = HashMap::new();

        fs::create_dir_all(&config.stack.data_dir)?;
        fs::create_dir_all(&config.stack.log_dir)?;

        let vault_path = config.stack.data_dir.join("vault.s3a");
        if !vault_path.exists() {
            let _ = s3a_engine::TileWriter::create(&vault_path);
        }

        // 1. Vault
        if config.services.vault.enabled {
            let bin = find_binary("s3a")
                .or_else(|| find_binary("s3a-cli"))
                .ok_or_else(|| StackError::HealthCheckFailed {
                    service: "vault".to_string(),
                    reason: "Binary 's3a' not found (not implemented)".to_string(),
                })?;
            let port_str = config.services.vault.port.to_string();
            let vault_str = vault_path.to_str().unwrap();
            let child =
                spawn_stack_child(&bin, &["serve", "--port", &port_str, "--vault", vault_str])?;
            pids.push(child.id());
            children.push(child);
            services.push("vault".to_string());
            ports.insert("vault".to_string(), config.services.vault.port);
        }

        // 2. LRS
        if config.services.lrs.enabled {
            let bin =
                find_binary("s3a-lrs-server").ok_or_else(|| StackError::HealthCheckFailed {
                    service: "lrs".to_string(),
                    reason: "Binary 's3a-lrs-server' not found (not implemented)".to_string(),
                })?;
            let port_str = config.services.lrs.port.to_string();
            let data_dir_str = config.stack.data_dir.to_str().unwrap();
            let mut lrs_args = vec![
                "--port",
                &port_str,
                "--data-dir",
                data_dir_str,
                "-a",
                &config.services.lrs.archive,
            ];
            if config.stack.is_dev() {
                lrs_args.push("--insecure-dev");
            }
            let child = spawn_stack_child(&bin, &lrs_args)?;
            pids.push(child.id());
            children.push(child);
            services.push("lrs".to_string());
            ports.insert("lrs".to_string(), config.services.lrs.port);
        }

        // 3. MCP
        if config.services.mcp.enabled {
            let bin = find_binary("s3a-harness").ok_or_else(|| StackError::HealthCheckFailed {
                service: "mcp".to_string(),
                reason: "Binary 's3a-harness' not found (not implemented)".to_string(),
            })?;
            let port_str = config.services.mcp.port.to_string();
            let data_dir_str = config.stack.data_dir.to_str().unwrap();
            let child =
                spawn_stack_child(&bin, &["mcp", "--port", &port_str, "--root", data_dir_str])?;
            pids.push(child.id());
            children.push(child);
            services.push("mcp".to_string());
            ports.insert("mcp".to_string(), config.services.mcp.port);
        }

        // 4. Harness
        if config.services.harness.enabled {
            let bin = find_binary("s3a-harness").ok_or_else(|| StackError::HealthCheckFailed {
                service: "harness".to_string(),
                reason: "Binary 's3a-harness' not found (not implemented)".to_string(),
            })?;
            if let Some(parent) = config.services.harness.config_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if !config.services.harness.config_path.exists() {
                let _ = fs::write(
                    &config.services.harness.config_path,
                    "[agent]\nname = \"scholar_reviewer\"\n",
                );
            }
            let port_str = config.services.harness.port.to_string();
            let config_str = config.services.harness.config_path.to_str().unwrap();
            let child = spawn_stack_child(
                &bin,
                &["serve", "--port", &port_str, "--config", config_str],
            )?;
            pids.push(child.id());
            children.push(child);
            services.push("harness".to_string());
            ports.insert("harness".to_string(), config.services.harness.port);
        }

        // 5. Sandbox
        if config.services.sandbox.enabled {
            let bin = find_binary("s3a-sandbox").ok_or_else(|| StackError::HealthCheckFailed {
                service: "sandbox".to_string(),
                reason: "Binary 's3a-sandbox' not found (not implemented)".to_string(),
            })?;
            let interval_str = config.services.sandbox.poll_interval_ms.to_string();
            let port_str = config.services.sandbox.port.to_string();
            let vault_str = vault_path.to_str().unwrap();
            let child = spawn_stack_child(
                &bin,
                &[
                    "worker",
                    "--vault",
                    vault_str,
                    "--interval",
                    &interval_str,
                    "--port",
                    &port_str,
                ],
            )?;
            pids.push(child.id());
            children.push(child);
            services.push("sandbox".to_string());
            ports.insert("sandbox".to_string(), config.services.sandbox.port);
        }

        let mut handle = NativeStackHandle {
            children,
            services,
            pids,
            ports,
        };

        // Wait up to 30 seconds for all services to pass live health checks
        let start = Instant::now();
        let timeout = Duration::from_secs(30);
        loop {
            let status = self.status(config);
            if status.overall == HealthState::Up {
                break;
            }
            if start.elapsed() > timeout {
                let mut failing = Vec::new();
                for s in &status.services {
                    if s.enabled && s.state != HealthState::Up {
                        failing.push(format!("'{}' ({}: {})", s.service, s.state, s.detail));
                    }
                }
                handle.stop();
                return Err(StackError::HealthCheckFailed {
                    service: "stack".to_string(),
                    reason: format!(
                        "Timeout waiting for native stack binaries to pass live health checks (30s limit exceeded). Failing services: [{}]",
                        failing.join(", ")
                    ),
                });
            }
            thread::sleep(Duration::from_millis(100));
        }

        Ok(handle)
    }
}

fn disabled_status(service: &str, port: Option<u16>) -> ServiceStatus {
    ServiceStatus {
        service: service.to_string(),
        enabled: false,
        port,
        state: HealthState::Disabled,
        latency_ms: None,
        detail: "Service disabled in config".to_string(),
        pid: None,
    }
}
