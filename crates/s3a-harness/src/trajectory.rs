use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

use crate::runtime::RunState;
use crate::sandbox::Sandbox;

/// Export target format for fine-tuning trajectories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrajectoryFormat {
    ChatMl,
    ShareGpt,
}

impl TrajectoryFormat {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "chatml" | "chat_ml" | "openai" => Some(Self::ChatMl),
            "sharegpt" | "share_gpt" => Some(Self::ShareGpt),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ChatMl => "chatml",
            Self::ShareGpt => "sharegpt",
        }
    }
}

/// Checks whether a run is permitted for the requested usage purpose under consent policy.
pub fn is_consent_permitted(run: &RunState, required_purpose: &str) -> bool {
    run.consent_purposes
        .iter()
        .any(|p| p.eq_ignore_ascii_case(required_purpose))
}

/// Formats a single RunState into OpenAI ChatML tool-call format.
pub fn run_to_chatml(run: &RunState) -> Value {
    let mut messages = Vec::new();

    messages.push(json!({
        "role": "system",
        "content": "You are an autonomous scientific research reviewer agent adhering to strict epistemic rigor and citation preflight checks."
    }));

    messages.push(json!({
        "role": "user",
        "content": run.task_goal
    }));

    for prov in &run.tool_provenance {
        let call_id = format!(
            "call_{}",
            &prov.inputs_hash[..8.min(prov.inputs_hash.len())]
        );

        // Assistant tool call turn
        messages.push(json!({
            "role": "assistant",
            "content": null,
            "tool_calls": [
                {
                    "id": call_id,
                    "type": "function",
                    "function": {
                        "name": prov.tool_name,
                        "arguments": serde_json::to_string(&prov.inputs).unwrap_or_else(|_| "{}".to_string())
                    }
                }
            ]
        }));

        // Tool output turn
        messages.push(json!({
            "role": "tool",
            "tool_call_id": call_id,
            "name": prov.tool_name,
            "content": serde_json::to_string(&prov.output).unwrap_or_else(|_| "{}".to_string())
        }));
    }

    // Terminal completion assistant turn
    let terminal_msg = match &run.status {
        crate::runtime::RunStatus::Completed { turns, tool_calls } => {
            format!(
                "Research task completed successfully in {} turns ({} tool invocations).",
                turns, tool_calls
            )
        }
        crate::runtime::RunStatus::PausedWaitingHuman { gate_id, reason } => {
            format!(
                "Supervisory oversight requested at gate {}: {}",
                gate_id, reason
            )
        }
        crate::runtime::RunStatus::LimitExceeded { reason } => {
            format!("Execution halted: {}", reason)
        }
        _ => "Execution halted.".to_string(),
    };

    messages.push(json!({
        "role": "assistant",
        "content": terminal_msg
    }));

    json!({
        "id": run.run_id,
        "base_snapshot_id": run.base_snapshot_id,
        "forked_snapshot_id": run.forked_snapshot_id,
        "messages": messages
    })
}

/// Formats a single RunState into ShareGPT multi-turn conversational format.
pub fn run_to_sharegpt(run: &RunState) -> Value {
    let mut conversations = Vec::new();

    conversations.push(json!({
        "from": "system",
        "value": "You are an autonomous scientific research reviewer agent adhering to strict epistemic rigor and citation preflight checks."
    }));

    conversations.push(json!({
        "from": "human",
        "value": run.task_goal
    }));

    for prov in &run.tool_provenance {
        let call_text = format!(
            "<tool_call>\n{{\"name\": \"{}\", \"arguments\": {}}}\n</tool_call>",
            prov.tool_name, prov.inputs
        );
        conversations.push(json!({
            "from": "gpt",
            "value": call_text
        }));

        let resp_text = format!("<tool_response>\n{}\n</tool_response>", prov.output);
        conversations.push(json!({
            "from": "tool",
            "value": resp_text
        }));
    }

    let final_val = match &run.status {
        crate::runtime::RunStatus::Completed { turns, .. } => {
            format!("Goal verified and accomplished in {} turns.", turns)
        }
        _ => "Goal processing ended.".to_string(),
    };

    conversations.push(json!({
        "from": "gpt",
        "value": final_val
    }));

    json!({
        "id": run.run_id,
        "conversations": conversations
    })
}

/// Summary report of the trajectory export operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryExportSummary {
    pub total_runs_found: usize,
    pub consent_accepted: usize,
    pub consent_filtered: usize,
    pub exported_records: usize,
    pub output_file: PathBuf,
    pub format: TrajectoryFormat,
}

/// Scans runs in sandbox, applies consent filtering for required purpose, and exports trajectories.
pub fn export_trajectories(
    sandbox: &Sandbox,
    format: TrajectoryFormat,
    output_path: &Path,
    required_purpose: &str,
) -> Result<TrajectoryExportSummary, String> {
    let runs_dir = sandbox.root().join("runs");
    if !runs_dir.exists() {
        return Err(format!(
            "Runs directory '{}' does not exist",
            runs_dir.display()
        ));
    }

    let mut total_runs = 0;
    let mut accepted = 0;
    let mut filtered = 0;
    let mut output_lines = Vec::new();

    for entry in fs::read_dir(&runs_dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(run) = serde_json::from_str::<RunState>(&content) {
                    total_runs += 1;
                    if is_consent_permitted(&run, required_purpose) {
                        accepted += 1;
                        let json_val = match format {
                            TrajectoryFormat::ChatMl => run_to_chatml(&run),
                            TrajectoryFormat::ShareGpt => run_to_sharegpt(&run),
                        };
                        output_lines
                            .push(serde_json::to_string(&json_val).map_err(|e| e.to_string())?);
                    } else {
                        filtered += 1;
                    }
                }
            }
        }
    }

    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }

    let joined = output_lines.join("\n");
    fs::write(output_path, joined).map_err(|e| e.to_string())?;

    Ok(TrajectoryExportSummary {
        total_runs_found: total_runs,
        consent_accepted: accepted,
        consent_filtered: filtered,
        exported_records: output_lines.len(),
        output_file: output_path.to_path_buf(),
        format,
    })
}
