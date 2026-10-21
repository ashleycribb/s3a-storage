//! S3A Task Tracing & Fine-Tuning Replay Engine
//! Records every Workflow step as an append-only S3A trace with human vs AI provenance
//! and xAPI verbs. Provides replay timeline and JSONL export for fine-tuning.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActorProvenance {
    Human {
        supervisor_id: String,
        reviewer_pubkey: String,
        rationale: Option<String>,
    },
    Ai {
        agent_id: String,
        model: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum XapiVerb {
    Planned,
    Authored,
    Verified,
    Tested,
    Signed,
    Approved,
    Rejected,
    Custom(String),
}

impl XapiVerb {
    pub fn uri(&self) -> String {
        match self {
            XapiVerb::Planned => "http://adlnet.gov/expapi/verbs/planned".to_string(),
            XapiVerb::Authored => "http://activitystrea.ms/schema/1.0/author".to_string(),
            XapiVerb::Verified => "http://adlnet.gov/expapi/verbs/verified".to_string(),
            XapiVerb::Tested => "http://adlnet.gov/expapi/verbs/tested".to_string(),
            XapiVerb::Signed => "http://adlnet.gov/expapi/verbs/signed".to_string(),
            XapiVerb::Approved => "http://adlnet.gov/expapi/verbs/approved".to_string(),
            XapiVerb::Rejected => "http://adlnet.gov/expapi/verbs/rejected".to_string(),
            XapiVerb::Custom(s) => s.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceStep {
    pub step_id: String,
    pub task_id: String,
    pub step_index: usize,
    pub step_name: String,
    pub actor: ActorProvenance,
    pub xapi_verb: String,
    pub prompt: Option<String>,
    pub model: Option<String>,
    pub patch: Option<String>,
    pub build_test_log: Option<String>,
    pub qa_evidence: Option<serde_json::Value>,
    pub human_decision: Option<serde_json::Value>,
    pub ts: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineSnapshot {
    pub step_index: usize,
    pub step_name: String,
    pub actor_summary: String,
    pub description: String,
    pub status: String,
    pub ts: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FineTuningSample {
    pub messages: Vec<FineTuningMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FineTuningMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskTrace {
    pub task_id: String,
    pub title: String,
    pub steps: Vec<TraceStep>,
    pub created_at: u64,
    pub updated_at: u64,
}

impl TaskTrace {
    pub fn new(task_id: &str, title: &str, now_ts: u64) -> Self {
        Self {
            task_id: task_id.to_string(),
            title: title.to_string(),
            steps: Vec::new(),
            created_at: now_ts,
            updated_at: now_ts,
        }
    }

    pub fn append_step(
        &mut self,
        step_name: &str,
        actor: ActorProvenance,
        verb: XapiVerb,
        prompt: Option<String>,
        model: Option<String>,
        patch: Option<String>,
        build_test_log: Option<String>,
        qa_evidence: Option<serde_json::Value>,
        human_decision: Option<serde_json::Value>,
        now_ts: u64,
    ) -> &TraceStep {
        let index = self.steps.len() + 1;
        let step_id = format!("{}_step_{}", self.task_id, index);
        let step = TraceStep {
            step_id,
            task_id: self.task_id.clone(),
            step_index: index,
            step_name: step_name.to_string(),
            actor,
            xapi_verb: verb.uri(),
            prompt,
            model,
            patch,
            build_test_log,
            qa_evidence,
            human_decision,
            ts: now_ts,
        };
        self.steps.push(step);
        self.updated_at = now_ts;
        self.steps.last().unwrap()
    }

    /// Replay timeline view of the task lifecycle
    pub fn replay_timeline(&self) -> Vec<TimelineSnapshot> {
        self.steps
            .iter()
            .map(|s| {
                let (actor_summary, desc) = match &s.actor {
                    ActorProvenance::Human {
                        supervisor_id,
                        rationale,
                        ..
                    } => (
                        format!("Human Supervisor ({})", supervisor_id),
                        rationale
                            .clone()
                            .unwrap_or_else(|| "Supervisory Judgment".to_string()),
                    ),
                    ActorProvenance::Ai { agent_id, model } => (
                        format!("AI Agent {} ({})", agent_id, model),
                        s.step_name.clone(),
                    ),
                };

                TimelineSnapshot {
                    step_index: s.step_index,
                    step_name: s.step_name.clone(),
                    actor_summary,
                    description: desc,
                    status: "completed".to_string(),
                    ts: s.ts,
                }
            })
            .collect()
    }

    /// Export AI planning and code synthesis steps as fine-tuning JSONL samples
    pub fn export_to_harness_jsonl(&self) -> Vec<String> {
        let mut samples = Vec::new();

        for s in &self.steps {
            if let ActorProvenance::Ai { model, .. } = &s.actor {
                if let (Some(prompt), Some(patch)) = (&s.prompt, &s.patch) {
                    let sample = FineTuningSample {
                        messages: vec![
                            FineTuningMessage {
                                role: "system".to_string(),
                                content: format!(
                                    "You are an expert autonomous software engineer working on Brakets. Task: {}. Model: {}",
                                    self.title, model
                                ),
                            },
                            FineTuningMessage {
                                role: "user".to_string(),
                                content: prompt.clone(),
                            },
                            FineTuningMessage {
                                role: "assistant".to_string(),
                                content: patch.clone(),
                            },
                        ],
                    };
                    if let Ok(line) = serde_json::to_string(&sample) {
                        samples.push(line);
                    }
                }
            }
        }

        samples
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trace_lifecycle_and_replay() {
        let mut trace = TaskTrace::new("GH-3", "Auth role-aware login", 1792100000);

        // 1. Planning step (AI)
        trace.append_step(
            "plan-task",
            ActorProvenance::Ai {
                agent_id: "agent-builder-01".to_string(),
                model: "@cf/meta/llama-3.3-70b-instruct-fp8-fast".to_string(),
            },
            XapiVerb::Planned,
            Some("Plan role-aware login".to_string()),
            Some("@cf/meta/llama-3.3-70b-instruct-fp8-fast".to_string()),
            None,
            None,
            None,
            None,
            1792100005,
        );

        // 2. Write patch step (AI)
        trace.append_step(
            "write-patch",
            ActorProvenance::Ai {
                agent_id: "agent-builder-01".to_string(),
                model: "@cf/meta/llama-3.3-70b-instruct-fp8-fast".to_string(),
            },
            XapiVerb::Authored,
            Some("Write roles.ts".to_string()),
            Some("@cf/meta/llama-3.3-70b-instruct-fp8-fast".to_string()),
            Some("export type PlatformRole = 'player' | 'coach';".to_string()),
            None,
            None,
            None,
            1792100010,
        );

        // 3. Human Approval (Human)
        trace.append_step(
            "human-approval",
            ActorProvenance::Human {
                supervisor_id: "supervisor_ashley".to_string(),
                reviewer_pubkey: "3199704d08ce3e956096a6ac0d2d9a1a".to_string(),
                rationale: Some("Role security verified".to_string()),
            },
            XapiVerb::Approved,
            None,
            None,
            None,
            None,
            None,
            Some(serde_json::json!({ "signature": "verified" })),
            1792100020,
        );

        assert_eq!(trace.steps.len(), 3);
        let timeline = trace.replay_timeline();
        assert_eq!(timeline.len(), 3);
        assert_eq!(timeline[0].step_name, "plan-task");
        assert!(timeline[2].actor_summary.contains("Human Supervisor"));

        let jsonl = trace.export_to_harness_jsonl();
        assert_eq!(jsonl.len(), 1);
        assert!(jsonl[0].contains("export type PlatformRole"));
    }
}
