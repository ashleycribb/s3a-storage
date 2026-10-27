//! Runtime enforcement of the agent-permissions policy (PR #153) at the harness boundary.
//!
//! Every agent-initiated tool call (the [`crate::tools::ToolRegistry`] and the MCP dispatcher)
//! goes through [`EnforcementGate::check`]:
//!
//! * **Deny by default.** Without an installed [`PermissionPolicy`] only read/existence-check
//!   tools run; anything that writes is denied.
//! * **Core is off limits.** Tools or targets that touch S3A core are always denied to agents.
//! * **Human-only actions fail closed.** Judgments, approvals, vault/custody and merge actions
//!   run only with a valid human sign-off token (ed25519, signed by a registered human key,
//!   bound to agent + action + target, unexpired, single use) or an attached
//!   [`crate::permissions::HumanApprover`] that approves the call.
//! * **Every decision is audited** to an append-only, hash-chained record. If the audit record
//!   cannot be written the call is denied.
//!
//! This module adds no S3A format or core semantics; it only gates what agents may call.
use crate::permissions::{is_vault_tool, PermissionPolicy, SideEffects, ToolRule};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, RwLock};

/// Argument key carrying a human sign-off token on a tool call.
pub const SIGNOFF_ARG: &str = "human_signoff";

/// How the gate treats a tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionClass {
    /// Reads and existence checks: the only thing AI may do without a policy.
    Read,
    /// Writes into sandbox/session records: needs an installed policy that allows it.
    Write,
    /// Judgments, approvals, custody, merges: needs a human sign-off.
    HumanOnly,
    /// Anything touching S3A core: never allowed for agents.
    Core,
}

fn touches_core(s: &str) -> bool {
    let t = s.to_ascii_lowercase();
    t.contains("s3a_core")
        || t.contains("s3a-core")
        || t.starts_with("core_")
        || t.contains("core_write")
}

/// Classify a call with no side-effect declaration (never `Read`).
pub fn classify(tool: &str, target: &str) -> ActionClass {
    classify_with(tool, target, None)
}

/// Classify a tool call. Core wins over everything, then human-only, then read, else write.
/// `Read` requires an explicit [`SideEffects::ReadOnly`] declaration; names never decide it.
pub fn classify_with(tool: &str, target: &str, side_effects: Option<SideEffects>) -> ActionClass {
    let read_only = SideEffects::is_read_only(side_effects);
    let t = tool.to_ascii_lowercase();
    if touches_core(&t) || touches_core(target) {
        return ActionClass::Core;
    }
    if s3a_vault::is_judgment_action(&t)
        || (is_vault_tool(&t) && !read_only)
        || [
            "approve",
            "approval",
            "human_decision",
            "merge",
            "judg",
            "sign_off",
            "signoff",
        ]
        .iter()
        .any(|w| t.contains(w))
    {
        return ActionClass::HumanOnly;
    }
    if read_only {
        return ActionClass::Read;
    }
    ActionClass::Write
}

/// Best-effort target of a call, for classification and the audit record.
pub fn call_target(args: &Value) -> String {
    for k in ["target", "archive_name", "path", "scope", "record_id"] {
        if let Some(s) = args.get(k).and_then(|v| v.as_str()) {
            return s.to_string();
        }
    }
    String::new()
}

/// A human's signed permission for one specific agent action.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignOff {
    pub human_id: String,
    pub agent_id: String,
    pub action: String,
    pub target: String,
    pub expires_at: u64,
    pub nonce: String,
    /// Hex ed25519 signature over [`SignOff::message`].
    pub signature: String,
}

impl SignOff {
    pub fn message(
        human_id: &str,
        agent_id: &str,
        action: &str,
        target: &str,
        expires_at: u64,
        nonce: &str,
    ) -> Vec<u8> {
        format!("s3a-harness-signoff-v1\n{human_id}\n{agent_id}\n{action}\n{target}\n{expires_at}\n{nonce}").into_bytes()
    }
    /// Create a sign-off. Only a human's own key should ever be passed here.
    pub fn sign(
        key: &SigningKey,
        human_id: &str,
        agent_id: &str,
        action: &str,
        target: &str,
        expires_at: u64,
        nonce: &str,
    ) -> Self {
        let sig = key.sign(&Self::message(
            human_id, agent_id, action, target, expires_at, nonce,
        ));
        Self {
            human_id: human_id.into(),
            agent_id: agent_id.into(),
            action: action.into(),
            target: target.into(),
            expires_at,
            nonce: nonce.into(),
            signature: hex::encode(sig.to_bytes()),
        }
    }
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateDecision {
    Allow,
    Deny,
}

/// One audit record. `hash` = sha256(prev_hash || canonical entry without hash).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditEntry {
    pub seq: u64,
    pub timestamp: u64,
    pub agent_id: String,
    pub action: String,
    pub target: String,
    pub class: ActionClass,
    pub decision: GateDecision,
    pub reason: String,
    pub signed_off_by: Option<String>,
    pub prev_hash: String,
    pub hash: String,
}

enum Sink {
    Memory(Vec<AuditEntry>),
    File {
        path: PathBuf,
        buffered: Vec<AuditEntry>,
    },
}

/// Append-only audit record. Entries are never rewritten; a file sink opens in append mode.
pub struct AuditLog {
    sink: Mutex<Sink>,
    last: Mutex<(u64, String)>,
}

impl Default for AuditLog {
    fn default() -> Self {
        Self::in_memory()
    }
}

impl AuditLog {
    pub fn in_memory() -> Self {
        Self {
            sink: Mutex::new(Sink::Memory(Vec::new())),
            last: Mutex::new((0, "0".repeat(64))),
        }
    }
    /// Append-only JSON Lines file. Continues the hash chain if the file already has entries.
    pub fn file(path: impl Into<PathBuf>) -> std::io::Result<Self> {
        let path = path.into();
        let mut last = (0, "0".repeat(64));
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Some(e) = text.lines().rev().find(|l| !l.trim().is_empty()) {
                let e: AuditEntry = serde_json::from_str(e)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                last = (e.seq, e.hash);
            }
        }
        Ok(Self {
            sink: Mutex::new(Sink::File {
                path,
                buffered: Vec::new(),
            }),
            last: Mutex::new(last),
        })
    }

    fn append(&self, mut e: AuditEntry) -> std::io::Result<AuditEntry> {
        let mut last = self.last.lock().unwrap();
        e.seq = last.0 + 1;
        e.prev_hash = last.1.clone();
        e.hash = String::new();
        let mut h = Sha256::new();
        h.update(e.prev_hash.as_bytes());
        h.update(serde_json::to_vec(&e).unwrap_or_default());
        e.hash = hex::encode(h.finalize());
        match &mut *self.sink.lock().unwrap() {
            Sink::Memory(v) => v.push(e.clone()),
            Sink::File { path, buffered } => {
                let mut f = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)?;
                let mut line = serde_json::to_vec(&e).unwrap_or_default();
                line.push(b'\n');
                f.write_all(&line)?;
                f.sync_data()?;
                buffered.push(e.clone());
            }
        }
        *last = (e.seq, e.hash.clone());
        Ok(e)
    }

    /// Entries written through this handle (memory sink) or read back from the file.
    pub fn entries(&self) -> Vec<AuditEntry> {
        match &*self.sink.lock().unwrap() {
            Sink::Memory(v) => v.clone(),
            Sink::File { path, .. } => std::fs::read_to_string(path)
                .unwrap_or_default()
                .lines()
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect(),
        }
    }

    /// Recompute the hash chain. `Err(seq)` names the first entry that does not verify.
    pub fn verify_chain(entries: &[AuditEntry]) -> Result<(), u64> {
        let mut prev = "0".repeat(64);
        for e in entries {
            let mut c = e.clone();
            c.hash = String::new();
            if c.prev_hash != prev {
                return Err(e.seq);
            }
            let mut h = Sha256::new();
            h.update(c.prev_hash.as_bytes());
            h.update(serde_json::to_vec(&c).unwrap_or_default());
            if hex::encode(h.finalize()) != e.hash {
                return Err(e.seq);
            }
            prev = e.hash.clone();
        }
        Ok(())
    }
}

/// The enforcement point shared by the tool registry and the MCP dispatcher.
#[derive(Default)]
pub struct EnforcementGate {
    policy: RwLock<Option<PermissionPolicy>>,
    humans: RwLock<HashMap<String, VerifyingKey>>,
    used_nonces: Mutex<HashSet<String>>,
    audit: RwLock<std::sync::Arc<AuditLog>>,
}

impl std::fmt::Debug for EnforcementGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnforcementGate")
            .field("policy", &*self.policy.read().unwrap())
            .field(
                "humans",
                &self.humans.read().unwrap().keys().collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl EnforcementGate {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set_policy(&self, p: PermissionPolicy) {
        *self.policy.write().unwrap() = Some(p);
    }
    /// Register a human who may sign off on human-only actions.
    pub fn register_human(&self, human_id: &str, key: VerifyingKey) {
        self.humans
            .write()
            .unwrap()
            .insert(human_id.to_string(), key);
    }
    pub fn set_audit_log(&self, log: std::sync::Arc<AuditLog>) {
        *self.audit.write().unwrap() = log;
    }
    pub fn audit_log(&self) -> std::sync::Arc<AuditLog> {
        self.audit.read().unwrap().clone()
    }

    fn verify_signoff(
        &self,
        agent_id: &str,
        tool: &str,
        target: &str,
        args: &Value,
        now: u64,
    ) -> Result<String, String> {
        let raw = args
            .get(SIGNOFF_ARG)
            .ok_or("human-only action and no human sign-off token")?;
        let s: SignOff =
            serde_json::from_value(raw.clone()).map_err(|e| format!("malformed sign-off: {e}"))?;
        if s.agent_id != agent_id || s.action != tool || s.target != target {
            return Err("sign-off does not match this agent, action and target".into());
        }
        if now > s.expires_at {
            return Err("sign-off expired".into());
        }
        let key = *self
            .humans
            .read()
            .unwrap()
            .get(&s.human_id)
            .ok_or("sign-off from an unregistered human")?;
        let sig_bytes: [u8; 64] = hex::decode(&s.signature)
            .ok()
            .and_then(|b| b.try_into().ok())
            .ok_or("malformed sign-off signature")?;
        key.verify(
            &SignOff::message(
                &s.human_id,
                &s.agent_id,
                &s.action,
                &s.target,
                s.expires_at,
                &s.nonce,
            ),
            &Signature::from_bytes(&sig_bytes),
        )
        .map_err(|_| "sign-off signature does not verify")?;
        if !self
            .used_nonces
            .lock()
            .unwrap()
            .insert(format!("{}:{}", s.human_id, s.nonce))
        {
            return Err("sign-off already used".into());
        }
        Ok(s.human_id)
    }

    #[allow(clippy::too_many_arguments)]
    fn decide(
        &self,
        agent_id: &str,
        tool: &str,
        side_effects: Option<SideEffects>,
        target: &str,
        class: ActionClass,
        args: &Value,
        now: u64,
    ) -> (GateDecision, String, Option<String>) {
        let policy = self.policy.read().unwrap();
        match class {
            ActionClass::Core => (
                GateDecision::Deny,
                "agents may not touch S3A core".into(),
                None,
            ),
            ActionClass::HumanOnly => {
                if policy.as_ref().and_then(|p| p.rules.get(tool)) == Some(&ToolRule::Never) {
                    return (GateDecision::Deny, "rule says never".into(), None);
                }
                match self.verify_signoff(agent_id, tool, target, args, now) {
                    Ok(h) => (
                        GateDecision::Allow,
                        "human sign-off verified".into(),
                        Some(h),
                    ),
                    Err(sign_err) => match policy.as_ref() {
                        // (messages keep the words "judgment action" for existing callers)
                        // A policy with an attached human approver can still approve interactively.
                        Some(p) if p.approver.is_some() => {
                            match p.authorize_with(agent_id, tool, side_effects, args, now) {
                                Ok(()) => (
                                    GateDecision::Allow,
                                    "approved by attached human approver".into(),
                                    Some("human_approver".into()),
                                ),
                                Err(e) => (
                                    GateDecision::Deny,
                                    format!("human-only judgment action: {sign_err}; {e}"),
                                    None,
                                ),
                            }
                        }
                        Some(p) => {
                            match p.authorize_with(agent_id, tool, side_effects, args, now) {
                                // Without an approver authorize cannot succeed for a human-only tool;
                                // treat any Ok defensively as deny (fail closed).
                                Ok(()) => (
                                    GateDecision::Deny,
                                    format!("human-only judgment action: {sign_err}"),
                                    None,
                                ),
                                Err(e) => (
                                    GateDecision::Deny,
                                    format!("human-only judgment action: {sign_err}; {e}"),
                                    None,
                                ),
                            }
                        }
                        None => (
                            GateDecision::Deny,
                            format!("human-only judgment action: {sign_err}"),
                            None,
                        ),
                    },
                }
            }
            ActionClass::Read | ActionClass::Write => match policy.as_ref() {
                None if class == ActionClass::Read => (
                    GateDecision::Allow,
                    "read/existence check (no policy installed)".into(),
                    None,
                ),
                None => (
                    GateDecision::Deny,
                    "deny by default: no permission policy installed".into(),
                    None,
                ),
                Some(p) => match p.authorize_with(agent_id, tool, side_effects, args, now) {
                    Ok(()) => (
                        GateDecision::Allow,
                        format!("allowed by policy ({:?} mode)", p.mode),
                        None,
                    ),
                    Err(e) => (GateDecision::Deny, e, None),
                },
            },
        }
    }

    /// Check a call with no side-effect declaration (treated as mutating).
    pub fn check(
        &self,
        agent_id: &str,
        tool: &str,
        args: &Value,
        now: u64,
    ) -> Result<AuditEntry, String> {
        self.check_with(agent_id, tool, None, args, now)
    }

    /// Check one agent call and audit the decision. `Ok(entry)` means the call may run.
    /// `side_effects` is the tool's explicit declaration; `None` fails closed.
    pub fn check_with(
        &self,
        agent_id: &str,
        tool: &str,
        side_effects: Option<SideEffects>,
        args: &Value,
        now: u64,
    ) -> Result<AuditEntry, String> {
        let target = call_target(args);
        let class = classify_with(tool, &target, side_effects);
        let (decision, reason, by) =
            self.decide(agent_id, tool, side_effects, &target, class, args, now);
        let entry = AuditEntry {
            seq: 0,
            timestamp: now,
            agent_id: agent_id.into(),
            action: tool.into(),
            target,
            class,
            decision,
            reason: reason.clone(),
            signed_off_by: by,
            prev_hash: String::new(),
            hash: String::new(),
        };
        let written = self
            .audit_log()
            .append(entry)
            .map_err(|e| format!("permission denied: audit record could not be written ({e})"))?;
        match decision {
            GateDecision::Allow => Ok(written),
            GateDecision::Deny => Err(format!("permission denied: {reason}")),
        }
    }
}
