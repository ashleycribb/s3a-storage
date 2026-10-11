use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

use crate::runtime::{AgentRuntime, RunState, RunStatus};

/// Single action-asserted evaluation test case.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalCase {
    pub id: String,
    pub goal: String,
    #[serde(default)]
    pub expected_tool: Option<String>,
    #[serde(default)]
    pub should_hit_gate: Option<bool>,
    #[serde(default)]
    pub should_ask_human: Option<bool>,
    #[serde(default)]
    pub max_turns: Option<usize>,
}

/// Execution outcome for an individual evaluation case.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalCaseResult {
    pub id: String,
    pub passed: bool,
    pub goal: String,
    pub tools_called: Vec<String>,
    pub gate_hit: bool,
    pub ask_human_raised: bool,
    pub failure_reason: Option<String>,
}

/// Aggregated metrics across an entire evaluation test set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalResultSet {
    pub agent_id: String,
    pub eval_set_path: String,
    pub total_cases: usize,
    pub passed_cases: usize,
    pub failed_cases: usize,
    pub success_rate: f64,
    pub gate_hits: usize,
    pub ask_human_count: usize,
    pub cases: Vec<EvalCaseResult>,
}

/// Executes an evaluation suite asserting on concrete actions taken (tool chosen, gate hit, ask_human raised).
pub fn run_eval_set(
    runtime: &AgentRuntime,
    eval_file_path: &Path,
) -> Result<EvalResultSet, String> {
    if !eval_file_path.exists() {
        return Err(format!(
            "Evaluation set file '{}' not found",
            eval_file_path.display()
        ));
    }

    let content = fs::read_to_string(eval_file_path).map_err(|e| e.to_string())?;
    let mut cases = Vec::new();

    for (line_idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let case: EvalCase = serde_json::from_str(trimmed)
            .map_err(|e| format!("Malformed JSON at line {}: {}", line_idx + 1, e))?;
        cases.push(case);
    }

    let mut case_results = Vec::new();
    let mut total_passed = 0;
    let mut total_gate_hits = 0;
    let mut total_ask_humans = 0;

    for (idx, case) in cases.iter().enumerate() {
        let session_uuid = [0x550e_8400, 0x1000 + idx as u64];
        let project_uuid = [0xdead_beef, 0xcafe_babe];

        let state = RunState::new(&runtime.config.id, &case.goal, session_uuid, project_uuid);
        let run_res = runtime.run(state);

        match run_res {
            Ok(finished_state) => {
                let gate_hit =
                    matches!(finished_state.status, RunStatus::PausedWaitingHuman { .. });
                let ask_human_raised = finished_state
                    .executed_tools
                    .iter()
                    .any(|t| t == "ask_human")
                    || !finished_state.human_decisions.is_empty()
                    || gate_hit;

                if gate_hit {
                    total_gate_hits += 1;
                }
                if ask_human_raised {
                    total_ask_humans += 1;
                }

                let mut passed = true;
                let mut failure_reason = None;

                // 1. Assert on expected tool if specified
                if let Some(ref exp_tool) = case.expected_tool {
                    if !finished_state.executed_tools.contains(exp_tool) {
                        passed = false;
                        failure_reason = Some(format!(
                            "Expected tool '{}' was not invoked. Executed tools: {:?}",
                            exp_tool, finished_state.executed_tools
                        ));
                    }
                }

                // 2. Assert on gate hit if specified
                if let Some(expected_gate) = case.should_hit_gate {
                    if expected_gate != gate_hit {
                        passed = false;
                        failure_reason = Some(format!(
                            "Expected should_hit_gate = {}, but actual gate_hit = {}",
                            expected_gate, gate_hit
                        ));
                    }
                }

                // 3. Assert on ask_human if specified
                if let Some(expected_ask) = case.should_ask_human {
                    if expected_ask != ask_human_raised {
                        passed = false;
                        failure_reason = Some(format!(
                            "Expected should_ask_human = {}, but actual ask_human_raised = {}",
                            expected_ask, ask_human_raised
                        ));
                    }
                }

                if passed {
                    total_passed += 1;
                }

                case_results.push(EvalCaseResult {
                    id: case.id.clone(),
                    passed,
                    goal: case.goal.clone(),
                    tools_called: finished_state.executed_tools,
                    gate_hit,
                    ask_human_raised,
                    failure_reason,
                });
            }
            Err(e) => {
                case_results.push(EvalCaseResult {
                    id: case.id.clone(),
                    passed: false,
                    goal: case.goal.clone(),
                    tools_called: Vec::new(),
                    gate_hit: false,
                    ask_human_raised: false,
                    failure_reason: Some(format!("Runtime execution error: {}", e)),
                });
            }
        }
    }

    let total_cases = cases.len();
    let failed_cases = total_cases - total_passed;
    let success_rate = if total_cases > 0 {
        (total_passed as f64 / total_cases as f64) * 100.0
    } else {
        0.0
    };

    Ok(EvalResultSet {
        agent_id: runtime.config.id.clone(),
        eval_set_path: eval_file_path.display().to_string(),
        total_cases,
        passed_cases: total_passed,
        failed_cases,
        success_rate,
        gate_hits: total_gate_hits,
        ask_human_count: total_ask_humans,
        cases: case_results,
    })
}
