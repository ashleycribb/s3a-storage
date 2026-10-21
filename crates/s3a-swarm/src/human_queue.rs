//! Ask-Human Supervisory Queue for Epistemic/Empirical Merge Disagreements.
//!
//! Mandatory Invariant:
//! Epistemic claim/evidence disagreements are NEVER auto-resolved. They are routed
//! to this queue and require a signed VcsResolutionRecord from an authorized human.

use ed25519_dalek::SigningKey;
use s3a_vcs::records::{VcsResolutionRecord, RESOLUTION_CHOICE_SOURCE, RESOLUTION_CHOICE_TARGET};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Pending claim/evidence conflict waiting for human supervisor judgment.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AskHumanConflict {
    pub conflict_id: String,
    pub record_id: String,
    #[serde(default)]
    pub tile_type: u32,
    #[serde(default)]
    pub type_name: String,
    #[serde(default, alias = "version_a")]
    pub target_version: String,
    #[serde(default, alias = "version_b")]
    pub source_version: String,
    #[serde(default, alias = "author_a")]
    pub target_author: String,
    #[serde(default, alias = "author_b")]
    pub source_author: String,
    #[serde(default, alias = "timestamp")]
    pub created_timestamp: u64,
    pub resolved: bool,
    pub choice: Option<u8>,
    pub rationale: Option<String>,
}

/// Thread-safe in-memory and persistable supervisory queue.
#[derive(Debug, Default, Clone)]
pub struct AskHumanQueue {
    conflicts: Arc<Mutex<HashMap<String, AskHumanConflict>>>,
}

impl AskHumanQueue {
    pub fn new() -> Self {
        Self {
            conflicts: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Enqueues a new claim conflict for human decision.
    pub fn enqueue(
        &self,
        record_id: &str,
        tile_type: u32,
        type_name: &str,
        target_version: &str,
        source_version: &str,
        target_author: &str,
        source_author: &str,
    ) -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let conflict_id = format!("conflict_{:016x}", {
            let mut h = sha2::Sha256::default();
            use sha2::Digest;
            h.update(record_id.as_bytes());
            h.update(target_version.as_bytes());
            h.update(source_version.as_bytes());
            h.update(now.to_le_bytes());
            let out = h.finalize();
            u64::from_le_bytes(out[0..8].try_into().unwrap())
        });

        let conflict = AskHumanConflict {
            conflict_id: conflict_id.clone(),
            record_id: record_id.to_string(),
            tile_type,
            type_name: type_name.to_string(),
            target_version: target_version.to_string(),
            source_version: source_version.to_string(),
            target_author: target_author.to_string(),
            source_author: source_author.to_string(),
            created_timestamp: now,
            resolved: false,
            choice: None,
            rationale: None,
        };

        self.conflicts
            .lock()
            .unwrap()
            .insert(conflict_id.clone(), conflict);
        conflict_id
    }

    /// Returns list of all pending unresolved conflicts.
    pub fn list_pending(&self) -> Vec<AskHumanConflict> {
        let lock = self.conflicts.lock().unwrap();
        lock.values().filter(|c| !c.resolved).cloned().collect()
    }

    /// Returns all conflicts (pending and resolved).
    pub fn list_all(&self) -> Vec<AskHumanConflict> {
        let lock = self.conflicts.lock().unwrap();
        lock.values().cloned().collect()
    }

    /// Resolves a conflict with human supervisor choice and generates a signed VcsResolutionRecord.
    pub fn resolve(
        &self,
        conflict_id: &str,
        chosen_side: u8,
        rationale: &str,
        human_signing_key: &SigningKey,
    ) -> Result<VcsResolutionRecord, String> {
        if chosen_side != RESOLUTION_CHOICE_TARGET && chosen_side != RESOLUTION_CHOICE_SOURCE {
            return Err("Choice must be target (1) or source (2)".to_string());
        }

        let mut lock = self.conflicts.lock().unwrap();
        let conflict = lock
            .get_mut(conflict_id)
            .ok_or_else(|| format!("Conflict '{}' not found in queue", conflict_id))?;

        if conflict.resolved {
            return Err(format!("Conflict '{}' is already resolved", conflict_id));
        }

        let resolution = VcsResolutionRecord::new_signed(
            &conflict.record_id,
            chosen_side,
            rationale,
            human_signing_key,
        );

        conflict.resolved = true;
        conflict.choice = Some(chosen_side);
        conflict.rationale = Some(rationale.to_string());

        Ok(resolution)
    }

    /// Count of currently pending conflicts.
    pub fn pending_count(&self) -> usize {
        let lock = self.conflicts.lock().unwrap();
        lock.values().filter(|c| !c.resolved).count()
    }

    /// Total count of all conflicts enqueued.
    pub fn total_enqueued(&self) -> usize {
        self.conflicts.lock().unwrap().len()
    }
}
