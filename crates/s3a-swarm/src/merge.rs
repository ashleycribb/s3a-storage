//! Deterministic merge rules, disjoint tile union, and epistemic conflict detection.
//!
//! # Merge Rule Specification
//!
//! 1. **Disjoint Tiles (Set Union)**:
//!    - Hyper-Tiles containing disjoint record keys merge unconditionally as a set union.
//!    - Zero locks and zero rebase storms: independent agent writes do not contend or invalidate
//!      each other's branches.
//!
//! 2. **Same-Record Data Edits (Deterministic Merge)**:
//!    - When concurrent commits touch the exact same data record key:
//!      - **Primary Ordering**: `committed_timestamp` (Last-Write-Wins / Lamport timestamp).
//!      - **Deterministic Tie-Break**: If timestamps are equal, lexicographical comparison
//!        of the commit author's Ed25519 signature (`sig_A > sig_B`).
//!    - Because Ed25519 signatures are cryptographic pseudo-random digests, this provides
//!      strict total ordering with zero ambiguity and without any distributed coordination.
//!
//! 3. **Epistemic Disagreements (Claim/Evidence Invariant)**:
//!    - When concurrent commits modify the same claim or empirical evidence record with
//!      conflicting assertions, the engine strictly forbids auto-resolution.
//!    - Routed to `ask_human` supervisory queue for signed human judgment.

use s3a_vcs::repo::is_human_judgment_required;

/// Outcome of attempting to merge a record edit into the shared state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordMergeOutcome {
    /// Record was disjoint and added as part of set union.
    DisjointAdded,
    /// Same-record edit merged deterministically (existing replaced by newer/higher tie-break).
    SameRecordUpdated {
        winner_timestamp: u64,
        rule_applied: &'static str,
    },
    /// Same-record edit rejected because incoming version is older than current version.
    SameRecordPreservedExisting {
        rule_applied: &'static str,
    },
    /// Conflict on claim/evidence requires signed human judgment.
    EpistemicConflictNeedsHuman {
        record_id: String,
        target_version: String,
        source_version: String,
    },
}

/// Represents an active record state entry in the repository working view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoRecordEntry {
    pub record_id: String,
    pub tile_type: u32,
    pub type_name: String,
    pub payload_hash: [u8; 32],
    pub summary: String,
    pub committed_timestamp: u64,
    pub commit_signature: [u8; 64],
    pub author_gate_address: String,
}

impl RepoRecordEntry {
    /// Applies deterministic merge rules when comparing existing record vs incoming record edit.
    pub fn merge_with(
        &self,
        incoming: &RepoRecordEntry,
    ) -> RecordMergeOutcome {
        // 1. Identical payloads: idempotent no-op
        if self.payload_hash == incoming.payload_hash {
            return RecordMergeOutcome::SameRecordPreservedExisting {
                rule_applied: "idempotent_identical_content",
            };
        }

        // 2. Epistemic Claim/Evidence Invariant: AI tools must never auto-resolve claim disagreements
        let is_claim = is_human_judgment_required(self.tile_type, &self.type_name)
            || is_human_judgment_required(incoming.tile_type, &incoming.type_name)
            || self.record_id.starts_with("claim_")
            || self.record_id.starts_with("evidence_");

        if is_claim {
            return RecordMergeOutcome::EpistemicConflictNeedsHuman {
                record_id: self.record_id.clone(),
                target_version: self.summary.clone(),
                source_version: incoming.summary.clone(),
            };
        }

        // 3. Same-Record Deterministic Merge:
        // Rule: Latest committed_timestamp wins. If tied, higher Ed25519 signature wins.
        if incoming.committed_timestamp > self.committed_timestamp {
            RecordMergeOutcome::SameRecordUpdated {
                winner_timestamp: incoming.committed_timestamp,
                rule_applied: "timestamp_lww",
            }
        } else if incoming.committed_timestamp < self.committed_timestamp {
            RecordMergeOutcome::SameRecordPreservedExisting {
                rule_applied: "timestamp_lww_existing_newer",
            }
        } else {
            // Timestamp tie-break: compare 64-byte Ed25519 signature bytes
            if incoming.commit_signature > self.commit_signature {
                RecordMergeOutcome::SameRecordUpdated {
                    winner_timestamp: incoming.committed_timestamp,
                    rule_applied: "signature_tie_break",
                }
            } else {
                RecordMergeOutcome::SameRecordPreservedExisting {
                    rule_applied: "signature_tie_break_existing_won",
                }
            }
        }
    }
}
