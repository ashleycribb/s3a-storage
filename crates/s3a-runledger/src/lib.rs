//! Minimal run ledger: append-only, hash-chained, Ed25519-signed JSON lines.
//! Records preregistered plans, amendments (never overwrite) and runs
//! (including null/failed outcomes). `verify` detects tampering, deletion,
//! reordering and dangling plan references. Judgements stay with humans:
//! this crate only proves what was recorded, when, and under which plan.
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Record {
    Plan { plan_id: String, hypothesis: String, plan_hash: String },
    Amendment { plan_id: String, prior_seq: u64, reason: String, new_plan_hash: String },
    Run {
        plan_id: String,
        input_hashes: Vec<String>,
        code_hash: String,
        params: serde_json::Value,
        /// "positive" | "null" | "failed" | free text — all must be recorded.
        outcome: String,
        timestamp: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub seq: u64,
    pub prev_hash: String,
    pub record: Record,
    pub signer: String,
    pub signature: String,
    pub hash: String,
}

#[derive(Debug, PartialEq)]
pub enum LedgerError {
    BadSeq(u64),
    BadPrev(u64),
    BadHash(u64),
    BadSig(u64),
    UnknownPlan(u64, String),
    BadAmendmentRef(u64),
    DuplicatePlan(u64, String),
    Parse(String),
}

const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn body(seq: u64, prev: &str, rec: &Record, signer: &str) -> Vec<u8> {
    serde_json::to_vec(&(seq, prev, rec, signer)).expect("serialize")
}

#[derive(Default, Debug, Clone)]
pub struct Ledger {
    pub entries: Vec<Entry>,
}

/// Which plan version (0 = original, n = after n-th amendment) a run used.
#[derive(Debug, PartialEq)]
pub struct RunReport {
    pub run_seq: u64,
    pub plan_id: String,
    pub plan_version: u32,
    pub plan_hash: String,
    pub switched: bool,
    pub outcome: String,
}

impl Ledger {
    pub fn append(&mut self, key: &SigningKey, record: Record) -> Result<&Entry, LedgerError> {
        let seq = self.entries.len() as u64;
        let prev = self.entries.last().map(|e| e.hash.clone()).unwrap_or_else(|| GENESIS.into());
        // validate references before writing
        let mut probe = self.clone();
        let signer = hex::encode(key.verifying_key().to_bytes());
        let b = body(seq, &prev, &record, &signer);
        let sig = key.sign(&b);
        let hash = hex::encode(Sha256::digest([b.as_slice(), &sig.to_bytes()].concat()));
        probe.entries.push(Entry { seq, prev_hash: prev, record, signer, signature: hex::encode(sig.to_bytes()), hash });
        probe.verify()?;
        *self = probe;
        Ok(self.entries.last().unwrap())
    }

    pub fn verify(&self) -> Result<(), LedgerError> {
        let mut prev = GENESIS.to_string();
        let mut plans: HashMap<String, u64> = HashMap::new(); // plan_id -> latest seq
        for (i, e) in self.entries.iter().enumerate() {
            let i = i as u64;
            if e.seq != i { return Err(LedgerError::BadSeq(i)); }
            if e.prev_hash != prev { return Err(LedgerError::BadPrev(i)); }
            let b = body(e.seq, &e.prev_hash, &e.record, &e.signer);
            let sig_bytes: [u8; 64] = hex::decode(&e.signature).ok().and_then(|v| v.try_into().ok()).ok_or(LedgerError::BadSig(i))?;
            let pk: [u8; 32] = hex::decode(&e.signer).ok().and_then(|v| v.try_into().ok()).ok_or(LedgerError::BadSig(i))?;
            let vk = VerifyingKey::from_bytes(&pk).map_err(|_| LedgerError::BadSig(i))?;
            let sig = Signature::from_bytes(&sig_bytes);
            vk.verify(&b, &sig).map_err(|_| LedgerError::BadSig(i))?;
            let h = hex::encode(Sha256::digest([b.as_slice(), &sig_bytes].concat()));
            if h != e.hash { return Err(LedgerError::BadHash(i)); }
            match &e.record {
                Record::Plan { plan_id, .. } => {
                    if plans.insert(plan_id.clone(), i).is_some() { return Err(LedgerError::DuplicatePlan(i, plan_id.clone())); }
                }
                Record::Amendment { plan_id, prior_seq, .. } => {
                    match plans.get(plan_id) {
                        None => return Err(LedgerError::UnknownPlan(i, plan_id.clone())),
                        Some(latest) if latest != prior_seq => return Err(LedgerError::BadAmendmentRef(i)),
                        _ => { plans.insert(plan_id.clone(), i); }
                    }
                }
                Record::Run { plan_id, .. } => {
                    if !plans.contains_key(plan_id) { return Err(LedgerError::UnknownPlan(i, plan_id.clone())); }
                }
            }
            prev = e.hash.clone();
        }
        Ok(())
    }

    /// For each run: plan version in force at run time, and whether the plan
    /// had been amended since the plan's first run (switched-plan detection).
    pub fn run_report(&self) -> Vec<RunReport> {
        let mut ver: HashMap<String, (u32, String)> = HashMap::new();
        let mut first_ver: HashMap<String, u32> = HashMap::new();
        let mut out = vec![];
        for e in &self.entries {
            match &e.record {
                Record::Plan { plan_id, plan_hash, .. } => { ver.insert(plan_id.clone(), (0, plan_hash.clone())); }
                Record::Amendment { plan_id, new_plan_hash, .. } => {
                    if let Some(v) = ver.get_mut(plan_id) { v.0 += 1; v.1 = new_plan_hash.clone(); }
                }
                Record::Run { plan_id, outcome, .. } => {
                    let (v, h) = ver.get(plan_id).cloned().unwrap_or((0, String::new()));
                    let f = *first_ver.entry(plan_id.clone()).or_insert(v);
                    out.push(RunReport { run_seq: e.seq, plan_id: plan_id.clone(), plan_version: v, plan_hash: h, switched: v != f, outcome: outcome.clone() });
                }
            }
        }
        out
    }

    pub fn to_jsonl(&self) -> String {
        self.entries.iter().map(|e| serde_json::to_string(e).unwrap() + "\n").collect()
    }

    /// Parses and verifies.
    pub fn from_jsonl(s: &str) -> Result<Self, LedgerError> {
        let entries = s.lines().filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).map_err(|e| LedgerError::Parse(e.to_string())))
            .collect::<Result<Vec<Entry>, _>>()?;
        let l = Ledger { entries };
        l.verify()?;
        Ok(l)
    }
}
