//! Crash-safe, append-only, hash-chained intake log.
//!
//! One JSON line per record. Each line stores the previous line's hash and its own
//! `hash = SHA-256(prev ‖ canonical(record))`, so any edit or reordering breaks the chain.
//! Every append is fsynced. On reopen, a torn final line (crash mid-write) is dropped;
//! damage anywhere else is reported as tampering.
//!
//! Records: dedup entries `(item_sha256, connector_id)`, scoped load labels, dead-letter
//! entries, and signed provenance events (one per intake step).
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DedupEntry {
    pub item_sha256: String,
    pub connector_id: String,
    pub raw_snapshot_hash: String,
    pub raw_archive: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LabelEntry {
    pub scope: String,
    pub label: String,
    pub raw_snapshot_hash: String,
    pub committed_at: u64,
}

/// A failed intake, held for a human. `mark` is `s3a_core::MARK_QUARANTINE`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeadLetter {
    pub mark: u32,
    pub step: String,
    pub source_path: String,
    /// Fingerprint of the original bytes, when they were fetched.
    pub item_sha256: Option<String>,
    /// Sealed raw archive holding the original, when sealing had happened.
    pub raw_archive: Option<String>,
    pub reason: String,
    pub attempts: u32,
    pub transient: bool,
    pub at: u64,
}

/// A signed provenance event for one intake step.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProvEvent {
    pub step: String,
    pub tool: String,
    pub tool_version: String,
    pub input_hashes: Vec<String>,
    pub output_hash: String,
    /// Hash of the log line before this event (chains events into the log).
    pub prev_hash: String,
    pub at: u64,
    pub signer_pubkey: String,
    pub signature: String,
}

impl ProvEvent {
    fn body(&self) -> Vec<u8> {
        let mut e = self.clone();
        e.signature.clear();
        crate::canonical(&("s3a-intake-event-v1", e))
    }
    pub fn verify_sig(&self) -> Result<(), String> {
        let pk: [u8; 32] = hex::decode(&self.signer_pubkey)
            .ok()
            .and_then(|v| v.try_into().ok())
            .ok_or("bad key")?;
        let sig: [u8; 64] = hex::decode(&self.signature)
            .ok()
            .and_then(|v| v.try_into().ok())
            .ok_or("bad sig")?;
        VerifyingKey::from_bytes(&pk)
            .map_err(|_| "bad key")?
            .verify(&self.body(), &Signature::from_bytes(&sig))
            .map_err(|_| format!("bad signature on event '{}'", self.step))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Record {
    Dedup(DedupEntry),
    Label(LabelEntry),
    DeadLetter(DeadLetter),
    Event(ProvEvent),
}

#[derive(Serialize, Deserialize)]
struct Line {
    prev: String,
    rec: Record,
    hash: String,
}

fn line_hash(prev: &str, rec: &Record) -> String {
    let mut b = prev.as_bytes().to_vec();
    b.extend(crate::canonical(rec));
    crate::sha256_hex(&b)
}

pub struct IntakeLog {
    path: PathBuf,
    pub records: Vec<Record>,
    last_hash: String,
    dedup: BTreeMap<(String, String), DedupEntry>,
    labels: BTreeMap<(String, String), LabelEntry>,
    /// True when a torn trailing line was dropped on open.
    pub recovered_torn_tail: bool,
}

impl IntakeLog {
    /// Open (or create) the log at `path`.
    pub fn open(path: &Path) -> io::Result<Self> {
        let bad = |m: String| io::Error::new(io::ErrorKind::InvalidData, m);
        let mut log = IntakeLog {
            path: path.to_path_buf(),
            records: vec![],
            last_hash: GENESIS.into(),
            dedup: BTreeMap::new(),
            labels: BTreeMap::new(),
            recovered_torn_tail: false,
        };
        if !path.exists() {
            if let Some(p) = path.parent() {
                std::fs::create_dir_all(p)?;
            }
            std::fs::File::create(path)?.sync_all()?;
            return Ok(log);
        }
        let data = std::fs::read(path)?;
        let ends_nl = data.last() == Some(&b'\n');
        let lines: Vec<&[u8]> = data
            .split(|b| *b == b'\n')
            .filter(|l| !l.is_empty())
            .collect();
        let n = lines.len();
        let mut good_len = 0usize;
        for (i, l) in lines.iter().enumerate() {
            let is_tail = i + 1 == n;
            let parsed: Option<Line> = serde_json::from_slice(l).ok();
            let ok = parsed.as_ref().is_some_and(|ln| {
                ln.prev == log.last_hash && ln.hash == line_hash(&ln.prev, &ln.rec)
            });
            if !ok {
                if is_tail && !ends_nl {
                    // Torn write from a crash: drop it and truncate.
                    log.recovered_torn_tail = true;
                    let f = std::fs::OpenOptions::new().write(true).open(path)?;
                    f.set_len(good_len as u64)?;
                    f.sync_all()?;
                    break;
                }
                return Err(bad(format!(
                    "intake log line {} fails hash chain (tampered or corrupt)",
                    i + 1
                )));
            }
            let ln = parsed.unwrap();
            if let Record::Event(e) = &ln.rec {
                e.verify_sig().map_err(bad)?;
                if e.prev_hash != ln.prev {
                    return Err(bad(format!("event at line {} has wrong prev_hash", i + 1)));
                }
            }
            log.last_hash = ln.hash.clone();
            log.index(&ln.rec);
            log.records.push(ln.rec);
            good_len += l.len() + 1;
        }
        Ok(log)
    }

    fn index(&mut self, r: &Record) {
        match r {
            Record::Dedup(d) => {
                self.dedup
                    .insert((d.item_sha256.clone(), d.connector_id.clone()), d.clone());
            }
            Record::Label(l) => {
                self.labels
                    .insert((l.scope.clone(), l.label.clone()), l.clone());
            }
            _ => {}
        }
    }

    pub fn last_hash(&self) -> &str {
        &self.last_hash
    }

    /// Append one record durably (write + fsync).
    pub fn append(&mut self, rec: Record) -> io::Result<()> {
        let hash = line_hash(&self.last_hash, &rec);
        let line = Line {
            prev: self.last_hash.clone(),
            rec,
            hash: hash.clone(),
        };
        let mut bytes = serde_json::to_vec(&line).map_err(io::Error::other)?;
        bytes.push(b'\n');
        let mut f = std::fs::OpenOptions::new().append(true).open(&self.path)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
        self.last_hash = hash;
        self.index(&line.rec);
        self.records.push(line.rec);
        Ok(())
    }

    /// Build, sign and append a provenance event chained to the current log head.
    pub fn event(
        &mut self,
        key: &SigningKey,
        step: &str,
        inputs: Vec<String>,
        output: String,
        at: u64,
    ) -> io::Result<()> {
        let mut e = ProvEvent {
            step: step.into(),
            tool: "s3a-pack-clean".into(),
            tool_version: env!("CARGO_PKG_VERSION").into(),
            input_hashes: inputs,
            output_hash: output,
            prev_hash: self.last_hash.clone(),
            at,
            signer_pubkey: hex::encode(key.verifying_key().to_bytes()),
            signature: String::new(),
        };
        e.signature = hex::encode(key.sign(&e.body()).to_bytes());
        self.append(Record::Event(e))
    }

    pub fn dedup(&self, item_sha256: &str, connector_id: &str) -> Option<&DedupEntry> {
        self.dedup
            .get(&(item_sha256.to_string(), connector_id.to_string()))
    }
    pub fn label(&self, scope: &str, label: &str) -> Option<&LabelEntry> {
        self.labels.get(&(scope.to_string(), label.to_string()))
    }
    pub fn dead_letters(&self) -> impl Iterator<Item = &DeadLetter> {
        self.records.iter().filter_map(|r| {
            if let Record::DeadLetter(d) = r {
                Some(d)
            } else {
                None
            }
        })
    }
    pub fn events(&self) -> impl Iterator<Item = &ProvEvent> {
        self.records.iter().filter_map(|r| {
            if let Record::Event(e) = r {
                Some(e)
            } else {
                None
            }
        })
    }
}
