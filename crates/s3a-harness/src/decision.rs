use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::{DecisionThresholds, SafeDefaults};
use crate::model::{CompletionRequest, Model};
use crate::store::Store;

/// Permitted operational routing decision categories.
/// Models are strictly restricted to operational routing; claim judgments are prohibited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationalDecisionType {
    ToolSelection,
    TrimmingLargeResult,
    StuckRunDetection,
    ModelEscalation,
}

impl OperationalDecisionType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ToolSelection => "tool_selection",
            Self::TrimmingLargeResult => "trimming_large_result",
            Self::StuckRunDetection => "stuck_run_detection",
            Self::ModelEscalation => "model_escalation",
        }
    }
}

/// Typed decision variants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DecisionKind {
    Choice { options: Vec<String> },
    YesNo,
    Score,
}

/// Typed decision concrete values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DecisionValue {
    Choice(String),
    YesNo(bool),
    Score(f64),
}

impl DecisionValue {
    pub fn as_str(&self) -> String {
        match self {
            Self::Choice(s) => s.clone(),
            Self::YesNo(b) => b.to_string(),
            Self::Score(s) => format!("{:.4}", s),
        }
    }
}

/// Request for an operational decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionRequest {
    pub decision_type: OperationalDecisionType,
    pub question: String,
    pub context: String,
    pub kind: DecisionKind,
    pub safe_default: DecisionValue,
    #[serde(default)]
    pub is_claim_judgment: bool,
}

/// Evaluated result of a decision including calibrated confidence and provenance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionResult {
    pub decision_type: OperationalDecisionType,
    pub question: String,
    pub value: DecisionValue,
    pub confidence: f64,
    pub rationale: String,
    pub action_taken: String,
    pub used_safe_default: bool,
    pub routed_to_human: bool,
    pub model_used: String,
}

/// Checks whether an action or question involves subjective claim judgment.
/// Claim judgments (support, rigor, overstatement) MUST always route to ask_human.
/// AI models are strictly limited to objective existence checks (DOI exists, metadata match, retraction status, passage present).
pub fn is_claim_judgment_call(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("claim support")
        || lower.contains("support claim")
        || lower.contains("methodological rigor")
        || lower.contains("rigor")
        || lower.contains("overstatement")
        || lower.contains("extrapolation")
        || lower.contains("scientific validity")
        || lower.contains("novelty judgment")
}

/// Record of shadow model evaluation running alongside the live model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowEvaluation {
    pub decision_type: OperationalDecisionType,
    pub question: String,
    pub live_value: String,
    pub shadow_value: String,
    pub live_confidence: f64,
    pub shadow_confidence: f64,
    pub agreement: bool,
    pub live_action: String,
    pub shadow_action: String,
    pub live_model: String,
    pub shadow_model: String,
}

/// Operational Decision Engine powered by a small local model with strict fail-safes and xAPI provenance.
pub struct DecisionEngine {
    pub model: Arc<dyn Model>,
    pub escalation_model: Option<Arc<dyn Model>>,
    pub shadow_model: Option<Arc<dyn Model>>,
    pub thresholds: DecisionThresholds,
    pub safe_defaults: SafeDefaults,
    pub store: Arc<dyn Store>,
    pub session_id: String,
    pub shadow_evaluations: Arc<std::sync::Mutex<Vec<ShadowEvaluation>>>,
}

impl DecisionEngine {
    pub fn new(
        model: Arc<dyn Model>,
        escalation_model: Option<Arc<dyn Model>>,
        thresholds: DecisionThresholds,
        safe_defaults: SafeDefaults,
        store: Arc<dyn Store>,
        session_id: impl Into<String>,
    ) -> Self {
        Self {
            model,
            escalation_model,
            shadow_model: None,
            thresholds,
            safe_defaults,
            store,
            session_id: session_id.into(),
            shadow_evaluations: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    pub fn with_shadow_model(mut self, shadow: Arc<dyn Model>) -> Self {
        self.shadow_model = Some(shadow);
        self
    }

    /// Evaluates an operational decision with confidence gating, fail-safe fallback, and xAPI logging.
    pub fn evaluate(
        &self,
        req: &DecisionRequest,
        _stuck_state: bool,
        escalation_used: bool,
    ) -> DecisionResult {
        // 1. Mandatory Rule: Claim judgments (support, rigor, overstatement) ALWAYS route to ask_human regardless of confidence
        if req.is_claim_judgment
            || is_claim_judgment_call(&req.question)
            || is_claim_judgment_call(&req.context)
        {
            let res = DecisionResult {
                decision_type: req.decision_type,
                question: req.question.clone(),
                value: req.safe_default.clone(),
                confidence: 0.0,
                rationale: "Claim judgment detected (claim support, rigor, or overstatement). AI tools may only perform existence checks. Routed to human supervisor.".to_string(),
                action_taken: "ask_human".to_string(),
                used_safe_default: true,
                routed_to_human: true,
                model_used: self.model.model_id().to_string(),
            };
            self.record_decision_xapi(&res, req);
            return res;
        }

        // 2. Select model (stronger model if run has escalated and stronger model is available)
        let active_model = if escalation_used && self.escalation_model.is_some() {
            self.escalation_model.as_ref().unwrap()
        } else {
            &self.model
        };

        // 3. Format structured decision prompt
        let prompt = self.build_prompt(req);
        let completion_req = CompletionRequest::with_system(
            "You are a calibrated operational decision router. Output only valid JSON with fields: answer, confidence (0.0 to 1.0), and rationale.",
            prompt,
        );

        // 4. Fail-safe model execution: model errors/timeouts fall back to configured safe default; never crash
        let (raw_value, confidence, rationale, used_safe_default, model_id) =
            match active_model.complete(&completion_req) {
                Ok(resp) => match self.parse_model_output(&resp.content, req) {
                    Ok((val, conf, rat)) => (val, conf, rat, false, resp.model),
                    Err(e) => (
                        req.safe_default.clone(),
                        0.0,
                        format!(
                            "JSON parsing error from model: {}. Fallback to safe default.",
                            e
                        ),
                        true,
                        resp.model,
                    ),
                },
                Err(e) => (
                    req.safe_default.clone(),
                    0.0,
                    format!("Model execution failure: {}. Fallback to safe default.", e),
                    true,
                    active_model.model_id().to_string(),
                ),
            };

        // 5. Confidence Gating: check threshold per decision type
        let threshold = self.get_threshold(req.decision_type);
        let (final_value, final_confidence, final_action, routed_to_human, final_safe_default) =
            if used_safe_default {
                (
                    raw_value,
                    confidence,
                    req.safe_default.as_str(),
                    false,
                    true,
                )
            } else if confidence < threshold {
                // Below threshold: fall back to safe default or ask_human
                if req.decision_type == OperationalDecisionType::ToolSelection
                    && self.safe_defaults.tool_selection == "ask_human"
                {
                    (
                        req.safe_default.clone(),
                        confidence,
                        "ask_human".to_string(),
                        true,
                        true,
                    )
                } else {
                    (
                        req.safe_default.clone(),
                        confidence,
                        format!("safe_default_{}", req.safe_default.as_str()),
                        false,
                        true,
                    )
                }
            } else {
                let act = raw_value.as_str();
                (raw_value, confidence, act, false, false)
            };

        let result = DecisionResult {
            decision_type: req.decision_type,
            question: req.question.clone(),
            value: final_value,
            confidence: final_confidence,
            rationale,
            action_taken: final_action,
            used_safe_default: final_safe_default,
            routed_to_human,
            model_used: model_id,
        };

        // 6. Record every decision as xAPI provenance
        self.record_decision_xapi(&result, req);

        // 7. Shadow Evaluation: if shadow candidate model configured, evaluate concurrently but never act
        if let Some(ref shadow) = self.shadow_model {
            self.evaluate_shadow(shadow, req, &result);
        }

        result
    }

    fn evaluate_shadow(
        &self,
        shadow_model: &Arc<dyn Model>,
        req: &DecisionRequest,
        live_res: &DecisionResult,
    ) {
        let prompt = self.build_prompt(req);
        let completion_req = CompletionRequest::with_system(
            "You are a calibrated operational decision router. Output only valid JSON with fields: answer, confidence (0.0 to 1.0), and rationale.",
            prompt,
        );

        let (shadow_val, shadow_conf, shadow_act) = match shadow_model.complete(&completion_req) {
            Ok(resp) => match self.parse_model_output(&resp.content, req) {
                Ok((val, conf, _)) => {
                    let threshold = self.get_threshold(req.decision_type);
                    let act = if conf < threshold {
                        if req.decision_type == OperationalDecisionType::ToolSelection
                            && self.safe_defaults.tool_selection == "ask_human"
                        {
                            "ask_human".to_string()
                        } else {
                            format!("safe_default_{}", req.safe_default.as_str())
                        }
                    } else {
                        val.as_str()
                    };
                    (val.as_str(), conf, act)
                }
                Err(_) => (req.safe_default.as_str(), 0.0, req.safe_default.as_str()),
            },
            Err(_) => (req.safe_default.as_str(), 0.0, req.safe_default.as_str()),
        };

        let agreement = live_res.action_taken == shadow_act;
        let eval = ShadowEvaluation {
            decision_type: req.decision_type,
            question: req.question.clone(),
            live_value: live_res.value.as_str(),
            shadow_value: shadow_val,
            live_confidence: live_res.confidence,
            shadow_confidence: shadow_conf,
            agreement,
            live_action: live_res.action_taken.clone(),
            shadow_action: shadow_act,
            live_model: live_res.model_used.clone(),
            shadow_model: shadow_model.model_id().to_string(),
        };

        self.shadow_evaluations.lock().unwrap().push(eval.clone());

        let stmt = json!({
            "actor": {
                "mbox": "mailto:shadow-model@s3a.local",
                "name": format!("Shadow Model ({})", eval.shadow_model)
            },
            "verb": {
                "id": "https://ashecribb.dev/xapi/verbs/shadow-evaluated",
                "display": { "en-US": "shadow-evaluated" }
            },
            "object": {
                "id": format!("urn:s3a:decision:{}", eval.decision_type.as_str()),
                "definition": {
                    "name": { "en-US": eval.question.clone() }
                }
            },
            "result": {
                "response": eval.shadow_value.clone(),
                "score": { "scaled": eval.shadow_confidence },
                "extensions": {
                    "https://ashecribb.dev/xapi/ext/agreement": eval.agreement,
                    "https://ashecribb.dev/xapi/ext/live-action": eval.live_action.clone(),
                    "https://ashecribb.dev/xapi/ext/shadow-action": eval.shadow_action.clone()
                }
            },
            "context": {
                "registration": self.session_id.clone()
            }
        });
        let _ = self.store.record_xapi_statement(&stmt);
    }

    fn get_threshold(&self, dt: OperationalDecisionType) -> f64 {
        match dt {
            OperationalDecisionType::ToolSelection => self.thresholds.tool_selection,
            OperationalDecisionType::TrimmingLargeResult => self.thresholds.trimming,
            OperationalDecisionType::StuckRunDetection => self.thresholds.stuck_detection,
            OperationalDecisionType::ModelEscalation => self.thresholds.escalation,
        }
    }

    fn build_prompt(&self, req: &DecisionRequest) -> String {
        let kind_spec = match &req.kind {
            DecisionKind::Choice { options } => {
                format!(
                    "Select one of the following exact options: [{}]",
                    options.join(", ")
                )
            }
            DecisionKind::YesNo => "Answer must be boolean true or false.".to_string(),
            DecisionKind::Score => {
                "Answer must be a calibrated score between 0.0 and 1.0.".to_string()
            }
        };

        format!(
            "Operational Category: {}\nQuestion: {}\nContext: {}\nRequirement: {}\n\nOutput JSON schema:\n{{\n  \"answer\": <value>,\n  \"confidence\": <float 0.0-1.0>,\n  \"rationale\": \"<brief text>\"\n}}",
            req.decision_type.as_str(),
            req.question,
            req.context,
            kind_spec
        )
    }

    fn parse_model_output(
        &self,
        raw: &str,
        req: &DecisionRequest,
    ) -> Result<(DecisionValue, f64, String), String> {
        let trimmed = raw.trim();
        let json_start = trimmed
            .find('{')
            .ok_or("No JSON object '{' found in output")?;
        let json_end = trimmed
            .rfind('}')
            .ok_or("No JSON object '}' found in output")?;
        let json_str = &trimmed[json_start..=json_end];

        let v: Value = serde_json::from_str(json_str).map_err(|e| e.to_string())?;

        let conf = v
            .get("confidence")
            .and_then(|c| c.as_f64())
            .unwrap_or(0.5)
            .clamp(0.0, 1.0);

        let rat = v
            .get("rationale")
            .and_then(|r| r.as_str())
            .unwrap_or("Model operational routing")
            .to_string();

        let val = match &req.kind {
            DecisionKind::Choice { options } => {
                let ans = v
                    .get("answer")
                    .and_then(|a| a.as_str())
                    .unwrap_or("")
                    .to_string();
                if options.iter().any(|opt| opt == &ans) {
                    DecisionValue::Choice(ans)
                } else if let Some(matched) = options.iter().find(|opt| ans.contains(opt.as_str()))
                {
                    DecisionValue::Choice(matched.clone())
                } else {
                    return Err(format!("Answer '{}' not among options {:?}", ans, options));
                }
            }
            DecisionKind::YesNo => {
                let ans = if let Some(b) = v.get("answer").and_then(|a| a.as_bool()) {
                    b
                } else if let Some(s) = v.get("answer").and_then(|a| a.as_str()) {
                    s.eq_ignore_ascii_case("true") || s.eq_ignore_ascii_case("yes")
                } else {
                    return Err("Failed to parse yes_no boolean answer".to_string());
                };
                DecisionValue::YesNo(ans)
            }
            DecisionKind::Score => {
                let score = v
                    .get("answer")
                    .and_then(|a| a.as_f64())
                    .unwrap_or(0.5)
                    .clamp(0.0, 1.0);
                DecisionValue::Score(score)
            }
        };

        Ok((val, conf, rat))
    }

    fn record_decision_xapi(&self, res: &DecisionResult, req: &DecisionRequest) {
        let stmt = json!({
            "actor": {
                "mbox": "mailto:harness@s3a.local",
                "name": format!("S3A Harness ({})", res.model_used)
            },
            "verb": {
                "id": "https://ashecribb.dev/xapi/verbs/decided",
                "display": { "en-US": "decided" }
            },
            "object": {
                "id": format!("urn:s3a:decision:{}", res.decision_type.as_str()),
                "definition": {
                    "name": { "en-US": req.question.clone() },
                    "description": { "en-US": res.rationale.clone() }
                }
            },
            "result": {
                "response": res.value.as_str(),
                "score": {
                    "scaled": res.confidence
                },
                "extensions": {
                    "https://ashecribb.dev/xapi/ext/action-taken": res.action_taken,
                    "https://ashecribb.dev/xapi/ext/used-safe-default": res.used_safe_default,
                    "https://ashecribb.dev/xapi/ext/routed-to-human": res.routed_to_human,
                    "https://ashecribb.dev/xapi/ext/model-used": res.model_used
                }
            },
            "context": {
                "registration": self.session_id.clone()
            }
        });

        let _ = self.store.record_xapi_statement(&stmt);
    }
}
