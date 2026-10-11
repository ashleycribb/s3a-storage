use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::config::AgentConfig;
use crate::decision::{
    DecisionEngine, DecisionKind, DecisionRequest, DecisionValue, OperationalDecisionType,
};
use crate::model::Model;
use crate::sandbox::Sandbox;
use crate::store::Store;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RunStatus {
    Running,
    PausedWaitingHuman { gate_id: String, reason: String },
    Completed { turns: usize, tool_calls: usize },
    LimitExceeded { reason: String },
    Halted { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunState {
    pub run_id: String,
    pub agent_id: String,
    pub session_uuid: [u64; 2],
    pub project_uuid: [u64; 2],
    pub turn_count: usize,
    pub tool_call_count: usize,
    pub tokens_used: usize,
    pub elapsed_secs: u64,
    pub escalated: bool,
    pub status: RunStatus,
    pub task_goal: String,
    pub executed_tools: Vec<String>,
    pub human_decisions: Vec<String>,
    #[serde(default)]
    pub shadow_evaluations: Vec<crate::decision::ShadowEvaluation>,
}

impl RunState {
    pub fn new(
        agent_id: impl Into<String>,
        task_goal: impl Into<String>,
        session_uuid: [u64; 2],
        project_uuid: [u64; 2],
    ) -> Self {
        Self {
            run_id: crate::generate_uuid_v7_string(),
            agent_id: agent_id.into(),
            session_uuid,
            project_uuid,
            turn_count: 0,
            tool_call_count: 0,
            tokens_used: 0,
            elapsed_secs: 0,
            escalated: false,
            status: RunStatus::Running,
            task_goal: task_goal.into(),
            executed_tools: Vec::new(),
            human_decisions: Vec::new(),
            shadow_evaluations: Vec::new(),
        }
    }
}

pub struct AgentRuntime {
    pub config: AgentConfig,
    pub model: Arc<dyn Model>,
    pub escalation_model: Option<Arc<dyn Model>>,
    pub shadow_model: Option<Arc<dyn Model>>,
    pub store: Arc<dyn Store>,
    pub sandbox: Sandbox,
}

impl AgentRuntime {
    pub fn new(
        config: AgentConfig,
        model: Arc<dyn Model>,
        escalation_model: Option<Arc<dyn Model>>,
        store: Arc<dyn Store>,
        sandbox: Sandbox,
    ) -> Self {
        Self {
            config,
            model,
            escalation_model,
            shadow_model: None,
            store,
            sandbox,
        }
    }

    pub fn with_shadow_model(mut self, shadow: Arc<dyn Model>) -> Self {
        self.shadow_model = Some(shadow);
        self
    }

    /// Run the autonomous agent loop with limits, pre-tool gating, fail-safes, and ask_human pause.
    pub fn run(&self, mut state: RunState) -> Result<RunState, String> {
        let start_time = Instant::now();
        let timeout = Duration::from_secs(self.config.limits.timeout_secs);
        let session_str = crate::uuid_pair_to_string(state.session_uuid);

        let mut decision_engine = DecisionEngine::new(
            self.model.clone(),
            self.escalation_model.clone(),
            self.config.thresholds.clone(),
            self.config.safe_defaults.clone(),
            self.store.clone(),
            session_str.clone(),
        );
        if let Some(ref shadow) = self.shadow_model {
            decision_engine = decision_engine.with_shadow_model(shadow.clone());
        }

        while matches!(state.status, RunStatus::Running) {
            // 1. Limit Checks
            if state.turn_count >= self.config.limits.max_turns {
                state.status = RunStatus::LimitExceeded {
                    reason: format!("Max turn limit reached ({})", self.config.limits.max_turns),
                };
                break;
            }
            if state.tool_call_count >= self.config.limits.max_tool_calls {
                state.status = RunStatus::LimitExceeded {
                    reason: format!(
                        "Max tool call limit reached ({})",
                        self.config.limits.max_tool_calls
                    ),
                };
                break;
            }
            if state.tokens_used >= self.config.limits.token_budget {
                state.status = RunStatus::LimitExceeded {
                    reason: format!(
                        "Token budget exceeded ({}/{})",
                        state.tokens_used, self.config.limits.token_budget
                    ),
                };
                break;
            }
            if start_time.elapsed() >= timeout {
                state.status = RunStatus::LimitExceeded {
                    reason: format!(
                        "Execution timeout reached ({}s)",
                        self.config.limits.timeout_secs
                    ),
                };
                break;
            }

            state.turn_count += 1;
            state.tokens_used += 120; // token usage increment per turn

            // 2. Stuck-Run Detection (check repetition across executed tools)
            let is_stuck = state.executed_tools.len() >= 3
                && state.executed_tools[state.executed_tools.len() - 1]
                    == state.executed_tools[state.executed_tools.len() - 2]
                && state.executed_tools[state.executed_tools.len() - 2]
                    == state.executed_tools[state.executed_tools.len() - 3];

            if is_stuck {
                let stuck_decision = decision_engine.evaluate(
                    &DecisionRequest {
                        decision_type: OperationalDecisionType::StuckRunDetection,
                        question: "Is agent stuck in an unproductive loop?".to_string(),
                        context: format!(
                            "Recent tools: {:?}",
                            &state.executed_tools[state.executed_tools.len() - 3..]
                        ),
                        kind: DecisionKind::YesNo,
                        safe_default: DecisionValue::YesNo(true),
                        is_claim_judgment: false,
                    },
                    true,
                    state.escalated,
                );

                if stuck_decision.value == DecisionValue::YesNo(true) {
                    // Escalation policy: at most one switch to a stronger model per run
                    if !state.escalated && self.escalation_model.is_some() {
                        state.escalated = true;
                        self.record_escalation_xapi(&state, &session_str);
                    } else {
                        // Already escalated or no escalation model: pause run and ask human
                        let gate_id = format!("gate_{}", crate::generate_uuid_v7());
                        state.status = RunStatus::PausedWaitingHuman {
                            gate_id: gate_id.clone(),
                            reason:
                                "Agent stuck in repetitive loop; supervisory assistance requested."
                                    .to_string(),
                        };
                        state.shadow_evaluations =
                            decision_engine.shadow_evaluations.lock().unwrap().clone();
                        self.persist_run(&state)?;
                        return Ok(state);
                    }
                }
            }

            // 3. Operational Tool Selection via small model
            let select_decision = decision_engine.evaluate(
                &DecisionRequest {
                    decision_type: OperationalDecisionType::ToolSelection,
                    question: format!("Select next tool to advance goal: '{}'", state.task_goal),
                    context: format!(
                        "Current turn: {}, Executed: {:?}",
                        state.turn_count, state.executed_tools
                    ),
                    kind: DecisionKind::Choice {
                        options: self.config.allowed_tools.clone(),
                    },
                    safe_default: DecisionValue::Choice(
                        self.config.safe_defaults.tool_selection.clone(),
                    ),
                    is_claim_judgment: false,
                },
                is_stuck,
                state.escalated,
            );

            // 4. Pre-Tool Gate: Check if action routes to ask_human or subjective claim judgment
            if select_decision.routed_to_human || select_decision.action_taken == "ask_human" {
                let gate_id = format!("gate_{}", crate::generate_uuid_v7());
                state.status = RunStatus::PausedWaitingHuman {
                    gate_id: gate_id.clone(),
                    reason: select_decision.rationale.clone(),
                };
                self.record_ask_human_xapi(
                    &state,
                    &session_str,
                    &gate_id,
                    &select_decision.rationale,
                );
                state.shadow_evaluations =
                    decision_engine.shadow_evaluations.lock().unwrap().clone();
                self.persist_run(&state)?;
                return Ok(state);
            }

            // 5. Execute Tool
            let chosen_tool = select_decision.value.as_str();
            state.tool_call_count += 1;
            state.executed_tools.push(chosen_tool.clone());

            // Check if goal completed
            if chosen_tool == "preflight_check" && state.turn_count >= 2 {
                state.status = RunStatus::Completed {
                    turns: state.turn_count,
                    tool_calls: state.tool_call_count,
                };
                break;
            }
        }

        state.elapsed_secs = start_time.elapsed().as_secs();
        state.shadow_evaluations = decision_engine.shadow_evaluations.lock().unwrap().clone();
        self.persist_run(&state)?;
        Ok(state)
    }

    /// Resume a previously paused run with human supervisor decision.
    pub fn resume(&self, run_id: &str, human_decision: Option<String>) -> Result<RunState, String> {
        let mut state = self.load_run(run_id)?;

        let session_str = crate::uuid_pair_to_string(state.session_uuid);

        match &state.status {
            RunStatus::PausedWaitingHuman { gate_id, reason } => {
                let decision_text =
                    human_decision.unwrap_or_else(|| "Approved to proceed".to_string());
                state
                    .human_decisions
                    .push(format!("{}: {}", gate_id, decision_text));

                // Record supervisor approval decision in xAPI
                let stmt = json!({
                    "actor": {
                        "mbox": "mailto:human-supervisor@institute.edu",
                        "name": "Human Supervisor"
                    },
                    "verb": {
                        "id": "https://ashecribb.dev/xapi/verbs/approved",
                        "display": { "en-US": "approved" }
                    },
                    "object": {
                        "id": format!("urn:s3a:gate:{}", gate_id),
                        "definition": {
                            "name": { "en-US": "Human Supervisory Gate" },
                            "description": { "en-US": reason.clone() }
                        }
                    },
                    "result": {
                        "response": decision_text
                    },
                    "context": {
                        "registration": session_str
                    }
                });
                let _ = self.store.record_xapi_statement(&stmt);

                // Unpause and resume loop
                state.status = RunStatus::Running;
                self.run(state)
            }
            other => Err(format!(
                "Run {} cannot be resumed: current status is {:?}",
                run_id, other
            )),
        }
    }

    /// Persist RunState to <sandbox.root()>/runs/<run_id>.json.
    pub fn persist_run(&self, state: &RunState) -> Result<PathBuf, String> {
        let runs_dir = self.sandbox.root().join("runs");
        if !runs_dir.exists() {
            fs::create_dir_all(&runs_dir).map_err(|e| e.to_string())?;
        }
        let file_path = runs_dir.join(format!("{}.json", state.run_id));
        let data = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;
        fs::write(&file_path, data).map_err(|e| e.to_string())?;
        Ok(file_path)
    }

    /// Load RunState from <sandbox.root()>/runs/<run_id>.json.
    pub fn load_run(&self, run_id: &str) -> Result<RunState, String> {
        let file_path = self
            .sandbox
            .root()
            .join("runs")
            .join(format!("{}.json", run_id));
        if !file_path.exists() {
            return Err(format!("Run state file not found: {}", file_path.display()));
        }
        let content = fs::read_to_string(&file_path).map_err(|e| e.to_string())?;
        serde_json::from_str(&content).map_err(|e| e.to_string())
    }

    fn record_escalation_xapi(&self, state: &RunState, session_str: &str) {
        let model_id = self
            .escalation_model
            .as_ref()
            .map(|m| m.model_id())
            .unwrap_or("strong-model");
        let stmt = json!({
            "actor": {
                "mbox": "mailto:harness@s3a.local",
                "name": "S3A Harness"
            },
            "verb": {
                "id": "https://ashecribb.dev/xapi/verbs/escalated",
                "display": { "en-US": "escalated" }
            },
            "object": {
                "id": format!("urn:s3a:run:{}", state.run_id),
                "definition": {
                    "name": { "en-US": "Model Escalation" },
                    "description": { "en-US": format!("Escalated from {} to stronger model {}", self.model.model_id(), model_id) }
                }
            },
            "context": {
                "registration": session_str
            }
        });
        let _ = self.store.record_xapi_statement(&stmt);
    }

    fn record_ask_human_xapi(
        &self,
        state: &RunState,
        session_str: &str,
        gate_id: &str,
        reason: &str,
    ) {
        let stmt = json!({
            "actor": {
                "mbox": "mailto:harness@s3a.local",
                "name": format!("S3A Harness ({})", self.model.model_id())
            },
            "verb": {
                "id": "https://ashecribb.dev/xapi/verbs/requested-human-gate",
                "display": { "en-US": "requested-human-gate" }
            },
            "object": {
                "id": format!("urn:s3a:gate:{}", gate_id),
                "definition": {
                    "name": { "en-US": "ask_human" },
                    "description": { "en-US": reason }
                }
            },
            "context": {
                "registration": session_str,
                "extensions": {
                    "https://ashecribb.dev/xapi/ext/run-id": state.run_id
                }
            }
        });
        let _ = self.store.record_xapi_statement(&stmt);
    }

    /// Evaluates shadow model performance against the live model across past runs.
    pub fn shadow_report(
        sandbox: &Sandbox,
        run_id: Option<&str>,
    ) -> Result<ShadowReportSummary, String> {
        let runs_dir = sandbox.root().join("runs");
        if !runs_dir.exists() {
            return Err(format!("Runs directory '{}' not found", runs_dir.display()));
        }

        let mut all_evals = Vec::new();
        if let Some(id) = run_id {
            let file_path = runs_dir.join(format!("{}.json", id));
            let content = fs::read_to_string(&file_path)
                .map_err(|e| format!("Failed to read run {}: {}", id, e))?;
            let state: RunState = serde_json::from_str(&content)
                .map_err(|e| format!("Failed to parse run {}: {}", id, e))?;
            all_evals.extend(state.shadow_evaluations);
        } else {
            for entry in fs::read_dir(&runs_dir).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("json") {
                    if let Ok(content) = fs::read_to_string(&path) {
                        if let Ok(state) = serde_json::from_str::<RunState>(&content) {
                            all_evals.extend(state.shadow_evaluations);
                        }
                    }
                }
            }
        }

        let total = all_evals.len();
        if total == 0 {
            return Ok(ShadowReportSummary {
                total_decisions: 0,
                agreements: 0,
                disagreements: 0,
                agreement_rate: 0.0,
                avg_live_confidence: 0.0,
                avg_shadow_confidence: 0.0,
                mean_confidence_delta: 0.0,
                promote_recommended: false,
                details: Vec::new(),
            });
        }

        let agreements = all_evals.iter().filter(|e| e.agreement).count();
        let disagreements = total - agreements;
        let agreement_rate = (agreements as f64 / total as f64) * 100.0;

        let sum_live_conf: f64 = all_evals.iter().map(|e| e.live_confidence).sum();
        let sum_shadow_conf: f64 = all_evals.iter().map(|e| e.shadow_confidence).sum();
        let avg_live_confidence = sum_live_conf / total as f64;
        let avg_shadow_confidence = sum_shadow_conf / total as f64;
        let mean_confidence_delta = avg_shadow_confidence - avg_live_confidence;

        // Model promotion requires shadow agreement threshold of at least 80.0%
        let promote_recommended = agreement_rate >= 80.0;

        Ok(ShadowReportSummary {
            total_decisions: total,
            agreements,
            disagreements,
            agreement_rate,
            avg_live_confidence,
            avg_shadow_confidence,
            mean_confidence_delta,
            promote_recommended,
            details: all_evals,
        })
    }
}

/// Aggregated report of shadow model execution versus the live model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowReportSummary {
    pub total_decisions: usize,
    pub agreements: usize,
    pub disagreements: usize,
    pub agreement_rate: f64,
    pub avg_live_confidence: f64,
    pub avg_shadow_confidence: f64,
    pub mean_confidence_delta: f64,
    pub promote_recommended: bool,
    pub details: Vec<crate::decision::ShadowEvaluation>,
}
