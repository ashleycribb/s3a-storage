use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentConfig {
    pub id: String,
    pub name: String,
    pub system_prompt: String,
    pub model: String,
    pub escalation_model: Option<String>,
    pub limits: RunLimits,
    pub thresholds: DecisionThresholds,
    pub safe_defaults: SafeDefaults,
    pub allowed_tools: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunLimits {
    pub max_turns: usize,
    pub max_tool_calls: usize,
    pub timeout_secs: u64,
    pub token_budget: usize,
}

impl Default for RunLimits {
    fn default() -> Self {
        Self {
            max_turns: 15,
            max_tool_calls: 30,
            timeout_secs: 300,
            token_budget: 32_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecisionThresholds {
    pub tool_selection: f64,
    pub trimming: f64,
    pub stuck_detection: f64,
    pub escalation: f64,
}

impl Default for DecisionThresholds {
    fn default() -> Self {
        Self {
            tool_selection: 0.70,
            trimming: 0.65,
            stuck_detection: 0.80,
            escalation: 0.75,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SafeDefaults {
    pub tool_selection: String,
    pub trimming: String,
    pub stuck_detection: bool,
    pub escalation: bool,
}

impl Default for SafeDefaults {
    fn default() -> Self {
        Self {
            tool_selection: "ask_human".to_string(),
            trimming: "head_tail_trim".to_string(),
            stuck_detection: false,
            escalation: false,
        }
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    ParseError(String),
    MissingField(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "Configuration I/O error: {}", e),
            Self::ParseError(msg) => write!(f, "Configuration parse error: {}", msg),
            Self::MissingField(field) => {
                write!(f, "Missing required configuration field: '{}'", field)
            }
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<std::io::Error> for ConfigError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl AgentConfig {
    /// Load agent configuration from a TOML file.
    pub fn load_file<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let content = fs::read_to_string(path)?;
        Self::parse_toml(&content)
    }

    /// Parse AgentConfig from a TOML string.
    pub fn parse_toml(input: &str) -> Result<Self, ConfigError> {
        let sections = parse_toml_sections(input)?;

        let agent_sec = sections
            .get("agent")
            .ok_or_else(|| ConfigError::MissingField("[agent] section".to_string()))?;

        let id = agent_sec
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ConfigError::MissingField("agent.id".to_string()))?
            .to_string();

        let name = agent_sec
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(&id)
            .to_string();

        let system_prompt = agent_sec
            .get("system_prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("You are a rigorous, evidence-grounded research assistant.")
            .to_string();

        let model = agent_sec
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("llama3.2:3b")
            .to_string();

        let escalation_model = agent_sec
            .get("escalation_model")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let mut limits = RunLimits::default();
        if let Some(lim) = sections.get("limits") {
            if let Some(v) = lim.get("max_turns").and_then(|v| v.as_int()) {
                limits.max_turns = v as usize;
            }
            if let Some(v) = lim.get("max_tool_calls").and_then(|v| v.as_int()) {
                limits.max_tool_calls = v as usize;
            }
            if let Some(v) = lim.get("timeout_secs").and_then(|v| v.as_int()) {
                limits.timeout_secs = v as u64;
            }
            if let Some(v) = lim.get("token_budget").and_then(|v| v.as_int()) {
                limits.token_budget = v as usize;
            }
        }

        let mut thresholds = DecisionThresholds::default();
        if let Some(dt) = sections.get("decision_thresholds") {
            if let Some(v) = dt.get("tool_selection").and_then(|v| v.as_float()) {
                thresholds.tool_selection = v;
            }
            if let Some(v) = dt.get("trimming").and_then(|v| v.as_float()) {
                thresholds.trimming = v;
            }
            if let Some(v) = dt.get("stuck_detection").and_then(|v| v.as_float()) {
                thresholds.stuck_detection = v;
            }
            if let Some(v) = dt.get("escalation").and_then(|v| v.as_float()) {
                thresholds.escalation = v;
            }
        }

        let mut safe_defaults = SafeDefaults::default();
        if let Some(sd) = sections.get("safe_defaults") {
            if let Some(v) = sd.get("tool_selection").and_then(|v| v.as_str()) {
                safe_defaults.tool_selection = v.to_string();
            }
            if let Some(v) = sd.get("trimming").and_then(|v| v.as_str()) {
                safe_defaults.trimming = v.to_string();
            }
            if let Some(v) = sd.get("stuck_detection").and_then(|v| v.as_bool()) {
                safe_defaults.stuck_detection = v;
            }
            if let Some(v) = sd.get("escalation").and_then(|v| v.as_bool()) {
                safe_defaults.escalation = v;
            }
        }

        let mut allowed_tools = vec![
            "s3a_search_library".to_string(),
            "get_synthesis_matrix".to_string(),
            "preflight_check".to_string(),
            "ask_human".to_string(),
        ];
        if let Some(tools_sec) = sections.get("tools") {
            if let Some(arr) = tools_sec.get("allowed").and_then(|v| v.as_string_array()) {
                allowed_tools = arr;
            }
        }

        Ok(AgentConfig {
            id,
            name,
            system_prompt,
            model,
            escalation_model,
            limits,
            thresholds,
            safe_defaults,
            allowed_tools,
        })
    }
}

#[derive(Debug, Clone)]
enum TomlValue {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Array(Vec<String>),
}

impl TomlValue {
    fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }
    fn as_int(&self) -> Option<i64> {
        match self {
            Self::Int(i) => Some(*i),
            _ => None,
        }
    }
    fn as_float(&self) -> Option<f64> {
        match self {
            Self::Float(f) => Some(*f),
            Self::Int(i) => Some(*i as f64),
            _ => None,
        }
    }
    fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }
    fn as_string_array(&self) -> Option<Vec<String>> {
        match self {
            Self::Array(a) => Some(a.clone()),
            _ => None,
        }
    }
}

fn parse_toml_sections(
    input: &str,
) -> Result<HashMap<String, HashMap<String, TomlValue>>, ConfigError> {
    let mut sections: HashMap<String, HashMap<String, TomlValue>> = HashMap::new();
    let mut current_section = "agent".to_string();

    for (line_no, line) in input.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            current_section = trimmed[1..trimmed.len() - 1].trim().to_string();
            sections.entry(current_section.clone()).or_default();
            continue;
        }

        let parts: Vec<&str> = trimmed.splitn(2, '=').collect();
        if parts.len() != 2 {
            continue;
        }

        let key = parts[0].trim().to_string();
        let raw_val = parts[1].trim();

        let val = parse_val(raw_val).map_err(|e| {
            ConfigError::ParseError(format!(
                "Line {}: {} (value: '{}')",
                line_no + 1,
                e,
                raw_val
            ))
        })?;

        sections
            .entry(current_section.clone())
            .or_default()
            .insert(key, val);
    }

    Ok(sections)
}

fn parse_val(raw: &str) -> Result<TomlValue, String> {
    let val_str = if let Some(idx) = raw.find('#') {
        // Only strip # if outside quotes
        if !raw.starts_with('"') {
            raw[..idx].trim()
        } else {
            raw
        }
    } else {
        raw
    };

    if val_str.starts_with('"') {
        if val_str.ends_with('"') && val_str.len() >= 2 {
            let inner = &val_str[1..val_str.len() - 1];
            let unescaped = inner.replace("\\\"", "\"").replace("\\n", "\n");
            return Ok(TomlValue::Str(unescaped));
        } else {
            return Err("Unclosed string quote".to_string());
        }
    }

    if val_str.starts_with('[') && val_str.ends_with(']') {
        let inner = &val_str[1..val_str.len() - 1];
        let items: Vec<String> = inner
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.trim_matches('"').to_string())
            .collect();
        return Ok(TomlValue::Array(items));
    }

    if val_str == "true" {
        return Ok(TomlValue::Bool(true));
    }
    if val_str == "false" {
        return Ok(TomlValue::Bool(false));
    }

    if let Ok(i) = val_str.parse::<i64>() {
        return Ok(TomlValue::Int(i));
    }

    if let Ok(f) = val_str.parse::<f64>() {
        return Ok(TomlValue::Float(f));
    }

    Ok(TomlValue::Str(val_str.to_string()))
}
