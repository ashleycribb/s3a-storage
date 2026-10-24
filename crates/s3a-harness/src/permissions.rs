//! Permission modes, per-tool rules, human approval and agent-pass checks for
//! [`crate::tools::ToolRegistry`] (Block deep dive top-5 #1).
//!
//! Decision order for each tool call, once a [`PermissionPolicy`] is installed:
//! 1. The rule for the tool, taking the mode into account (see [`PermissionPolicy::decide`]).
//!    `Never` stops here.
//! 2. The agent's owner-signed pass (s3a-vault `AgentPass`) must verify now, its owner must
//!    not be revoked, and it must cover the tool and the call's `scope` argument.
//!    Judgment tools can never be covered by a pass; they only run after human approval.
//! 3. `Ask` needs an attached [`HumanApprover`]. With none attached the call is denied.
use s3a_vault::rotation::RevocationRegistry;
use s3a_vault::AgentPass;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    /// No tool use at all.
    Chat,
    /// Every tool call asks a human (unless a rule says never). The default.
    #[default]
    Approve,
    /// Read-only tools run; everything else asks. Never auto-approves a sensitive tool.
    Smart,
    /// Tools run unless a rule says ask or never. Sensitive tools still ask.
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolRule {
    Always,
    Ask,
    Never,
}

/// Outcome of the rule step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Ask,
    Deny,
}

/// What a human sees when asked.
#[derive(Debug, Clone, Serialize)]
pub struct ApprovalRequest {
    pub agent_id: String,
    pub tool: String,
    pub args: serde_json::Value,
    pub reason: String,
    pub judgment: bool,
}

/// A human in the loop. Returning `false` denies the call.
pub trait HumanApprover: Send + Sync {
    fn approve(&self, req: &ApprovalRequest) -> bool;
}

/// Tools touching keys, grants or custody: pinned to ask or never.
pub fn is_vault_tool(tool: &str) -> bool {
    let t = tool.to_ascii_lowercase();
    t.starts_with("vault")
        || t.contains("_vault")
        || t.contains("keystore")
        || t.contains("delegation")
}

/// Tools considered read-only for `Smart` mode.
pub fn is_read_only_tool(tool: &str) -> bool {
    let t = tool.to_ascii_lowercase();
    ["query", "lookup", "check", "read", "list", "get", "search"]
        .iter()
        .any(|w| t.contains(w))
        && !["write", "delete", "send", "update", "create"]
            .iter()
            .any(|w| t.contains(w))
}

pub fn is_sensitive_tool(tool: &str) -> bool {
    s3a_vault::is_judgment_action(tool) || is_vault_tool(tool)
}

#[derive(Clone, Default)]
pub struct PermissionPolicy {
    pub mode: PermissionMode,
    pub rules: HashMap<String, ToolRule>,
    /// Require an agent pass on every call (default true via [`PermissionPolicy::new`]).
    pub require_pass: bool,
    /// agent_id -> (agent public key hex, pass)
    pub passes: HashMap<String, (String, AgentPass)>,
    pub revocations: Arc<RevocationRegistry>,
    pub approver: Option<Arc<dyn HumanApprover>>,
}

impl std::fmt::Debug for PermissionPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PermissionPolicy")
            .field("mode", &self.mode)
            .field("rules", &self.rules)
            .field("require_pass", &self.require_pass)
            .field("agents_with_pass", &self.passes.keys().collect::<Vec<_>>())
            .field("approver_attached", &self.approver.is_some())
            .finish()
    }
}

impl PermissionPolicy {
    pub fn new(mode: PermissionMode) -> Self {
        Self {
            mode,
            require_pass: true,
            ..Default::default()
        }
    }
    pub fn with_rule(mut self, tool: &str, rule: ToolRule) -> Self {
        self.rules.insert(tool.to_string(), rule);
        self
    }
    pub fn with_pass(mut self, agent_id: &str, agent_pubkey_hex: &str, pass: AgentPass) -> Self {
        self.passes
            .insert(agent_id.to_string(), (agent_pubkey_hex.to_string(), pass));
        self
    }
    pub fn with_revocations(mut self, r: RevocationRegistry) -> Self {
        self.revocations = Arc::new(r);
        self
    }
    pub fn with_approver(mut self, a: Arc<dyn HumanApprover>) -> Self {
        self.approver = Some(a);
        self
    }

    /// The rule step. Sensitive (vault and judgment) tools are pinned: `Always` is read as
    /// `Ask`, judgment tools default to `Never` and vault tools to `Ask`, and no mode can
    /// turn them into `Allow`.
    pub fn decide(&self, tool: &str) -> Decision {
        let rule = self.rules.get(tool).copied();
        if self.mode == PermissionMode::Chat || rule == Some(ToolRule::Never) {
            return Decision::Deny;
        }
        if is_sensitive_tool(tool) {
            return match rule {
                Some(ToolRule::Ask) | Some(ToolRule::Always) => Decision::Ask,
                _ if s3a_vault::is_judgment_action(tool) => Decision::Deny,
                _ => Decision::Ask,
            };
        }
        match (self.mode, rule) {
            (_, Some(ToolRule::Ask)) => Decision::Ask,
            (PermissionMode::Approve, _) => Decision::Ask,
            (_, Some(ToolRule::Always)) => Decision::Allow,
            (PermissionMode::Auto, None) => Decision::Allow,
            (PermissionMode::Smart, None) if is_read_only_tool(tool) => Decision::Allow,
            (PermissionMode::Smart, None) => Decision::Ask,
            (PermissionMode::Chat, _) | (_, Some(ToolRule::Never)) => Decision::Deny,
        }
    }

    /// Full check for one call. `Ok(())` means the call may run.
    pub fn authorize(
        &self,
        agent_id: &str,
        tool: &str,
        args: &serde_json::Value,
        now: u64,
    ) -> Result<(), String> {
        let decision = self.decide(tool);
        if decision == Decision::Deny {
            return Err(format!(
                "permission denied: tool '{tool}' is not allowed in {:?} mode or by rule",
                self.mode
            ));
        }
        let judgment = s3a_vault::is_judgment_action(tool);
        if self.require_pass {
            let (pk, pass) = self.passes.get(agent_id).ok_or_else(|| {
                format!("permission denied: agent '{agent_id}' has no owner-signed pass")
            })?;
            let scope = args.get("scope").and_then(|v| v.as_str());
            let res = if judgment {
                // A pass proves who the agent is, but can never grant a judgment action.
                s3a_vault::verify_agent_pass(pass, now, &self.revocations).and_then(|_| {
                    if pass.conditions.agent_pubkey.eq_ignore_ascii_case(pk) {
                        Ok(())
                    } else {
                        Err(s3a_vault::VaultError::InvalidSignature(
                            "agent pass is for a different agent key",
                        ))
                    }
                })
            } else {
                s3a_vault::check_agent_pass(pass, pk, tool, scope, now, &self.revocations)
            };
            res.map_err(|e| format!("permission denied: agent pass check failed: {e}"))?;
        }
        if decision == Decision::Ask {
            let Some(h) = &self.approver else {
                return Err(format!("permission denied: tool '{tool}' needs human approval and no human handler is attached"));
            };
            let req = ApprovalRequest {
                agent_id: agent_id.into(),
                tool: tool.into(),
                args: args.clone(),
                reason: if judgment {
                    "judgment action: a human must decide".into()
                } else {
                    format!("{:?} mode asks before this tool", self.mode)
                },
                judgment,
            };
            if !h.approve(&req) {
                return Err(format!("permission denied: human declined '{tool}'"));
            }
        }
        Ok(())
    }
}
