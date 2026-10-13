//! Concurrent repository push coordinator and state synchronizer.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::Instant;

use s3a_core::TileType;
use s3a_engine::sealed::{seal_buffer_v2, SealOptionsV2};
use s3a_engine::TileWriter;
use s3a_vcs::records::{VcsCommitRecord, VcsRefRecord, REF_KIND_BRANCH, VCS_REF_NAME_MAX};
use s3a_vcs::errors::VcsError;

use crate::agent::AgentCommitBundle;
use crate::eval::evaluate_shadow_commit;
use crate::grant::{record_grant_rejection, validate_commit_grant, GrantRejectionReason};
use crate::human_queue::AskHumanQueue;
use crate::merge::{RecordMergeOutcome, RepoRecordEntry};

/// Result of a push operation to the shared repository.
#[derive(Debug, Clone)]
pub enum PushResult {
    Merged {
        commit_id: [u8; 32],
        seal_hash: [u8; 32],
        records_added: usize,
    },
    RejectedByGrant {
        reason: GrantRejectionReason,
    },
    SentToHuman {
        conflict_id: String,
        record_id: String,
    },
    QuarantinedInShadow {
        reason: String,
    },
}

/// Shared in-memory and disk-backed VCS Repository Coordinator.
pub struct SharedRepoCoordinator {
    pub active_records: HashMap<String, RepoRecordEntry>,
    pub commits: Vec<VcsCommitRecord>,
    pub raw_tiles: Vec<(u32, Vec<u8>)>,
    pub current_seal: [u8; 32],
    pub human_queue: AskHumanQueue,
    pub latencies_us: Vec<u64>,
    pub auto_merged: usize,
    pub rejected_by_grant: usize,
    pub sent_to_human: usize,
    pub shadow_mode: bool,
}

impl SharedRepoCoordinator {
    pub fn new(shadow_mode: bool) -> Self {
        Self {
            active_records: HashMap::new(),
            commits: Vec::new(),
            raw_tiles: Vec::new(),
            current_seal: [0u8; 32],
            human_queue: AskHumanQueue::new(),
            latencies_us: Vec::new(),
            auto_merged: 0,
            rejected_by_grant: 0,
            sent_to_human: 0,
            shadow_mode,
        }
    }

    /// Processes an incoming push from an agent.
    pub fn push(
        &mut self,
        bundle: AgentCommitBundle,
        allowed_prefix: &str,
        op_idx: u32,
    ) -> Result<PushResult, String> {
        let start = Instant::now();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // 1. Strict signature verification of incoming commit
        if !bundle.commit.verify_signature() {
            return Err("Incoming commit failed cryptographic signature verification".to_string());
        }

        // 2. Scoped delegation grant evaluation (fail-safe default)
        if let Err(reason) = validate_commit_grant(&bundle.grant, allowed_prefix, &bundle.paths, now, op_idx) {
            let author_addr = format!(
                "s3a-{:02x}",
                bundle.commit.author_checksum
            );
            record_grant_rejection(
                allowed_prefix,
                &author_addr,
                reason.clone(),
                bundle.paths.clone(),
            );
            self.rejected_by_grant += 1;
            let elapsed = start.elapsed().as_micros() as u64;
            self.latencies_us.push(elapsed);
            return Ok(PushResult::RejectedByGrant { reason });
        }

        // 3. Shadow branch evaluation if --shadow active
        if self.shadow_mode {
            let eval_outcome = evaluate_shadow_commit(&bundle.commit, &bundle.records);
            if !eval_outcome.passed {
                let elapsed = start.elapsed().as_micros() as u64;
                self.latencies_us.push(elapsed);
                return Ok(PushResult::QuarantinedInShadow {
                    reason: eval_outcome.rationale,
                });
            }
            // Eval passed: automatically promoted to main
        }

        // 4. Epistemic conflict and deterministic record merge
        let mut has_epistemic_conflict = false;
        let mut conflict_id_out = String::new();
        let mut conflict_rec_out = String::new();

        for incoming in &bundle.records {
            if let Some(existing) = self.active_records.get(&incoming.record_id) {
                let outcome = existing.merge_with(incoming);
                match outcome {
                    RecordMergeOutcome::DisjointAdded => {
                        self.active_records.insert(incoming.record_id.clone(), incoming.clone());
                        self.auto_merged += 1;
                    }
                    RecordMergeOutcome::SameRecordUpdated { .. } => {
                        self.active_records.insert(incoming.record_id.clone(), incoming.clone());
                        self.auto_merged += 1;
                    }
                    RecordMergeOutcome::SameRecordPreservedExisting { .. } => {
                        // Existing version is newer or won signature tie-break; preserved deterministically
                        self.auto_merged += 1;
                    }
                    RecordMergeOutcome::EpistemicConflictNeedsHuman { record_id, target_version, source_version } => {
                        has_epistemic_conflict = true;
                        conflict_rec_out = record_id.clone();
                        conflict_id_out = self.human_queue.enqueue(
                            &record_id,
                            incoming.tile_type,
                            &incoming.type_name,
                            &target_version,
                            &source_version,
                            &existing.author_gate_address,
                            &incoming.author_gate_address,
                        );
                        self.sent_to_human += 1;
                    }
                }
            } else {
                // Disjoint record added as part of set union
                self.active_records.insert(incoming.record_id.clone(), incoming.clone());
                self.auto_merged += 1;
            }
        }

        let elapsed = start.elapsed().as_micros() as u64;
        self.latencies_us.push(elapsed);

        if has_epistemic_conflict {
            Ok(PushResult::SentToHuman {
                conflict_id: conflict_id_out,
                record_id: conflict_rec_out,
            })
        } else {
            // Commit successfully merged into repository HEAD
            let commit_id = bundle.commit.commit_id;
            let seal_hash = bundle.commit.seal_hash;
            self.current_seal = seal_hash;
            self.commits.push(bundle.commit);
            self.raw_tiles.push((bundle.raw_tile_type, bundle.raw_tile_payload));

            Ok(PushResult::Merged {
                commit_id,
                seal_hash,
                records_added: bundle.records.len(),
            })
        }
    }

    /// Verifies every commit signature stored in the repository.
    pub fn verify_all_commit_signatures(&self) -> (usize, usize) {
        let total = self.commits.len();
        let mut verified = 0;
        for c in &self.commits {
            if c.verify_signature() {
                verified += 1;
            }
        }
        (verified, total)
    }

    /// Flushes repository into a real .s3a archive and seals it with seal_buffer_v2.
    pub fn finalize_archive<P: AsRef<Path>>(&self, path: P) -> Result<String, VcsError> {
        let p = path.as_ref();
        if let Some(parent) = p.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if p.exists() {
            let _ = fs::remove_file(p);
        }

        // Initialize archive
        let mut writer = TileWriter::create(p)?;
        writer.enable_addon(s3a_core::ADDON_EDU)?;

        // Write packed active records into domain hyper-tiles
        let mut telemetry_records: Vec<s3a_core::TelemetryRecord> = Vec::new();
        let mut claim_records: Vec<s3a_pack_scholar::records::ScholarNoteClaimRecord> = Vec::new();

        for (_id, rec) in &self.active_records {
            if rec.tile_type == TileType::TELEMETRY {
                telemetry_records.push(s3a_core::TelemetryRecord::new(
                    rec.committed_timestamp,
                    1,
                    1,
                    100.0,
                ));
            } else if rec.tile_type == s3a_pack_scholar::ScholarTileType::NOTE_CLAIMS {
                claim_records.push(s3a_pack_scholar::records::ScholarNoteClaimRecord::new(
                    1,
                    1,
                    [0, 100],
                    1,
                    0.9,
                    s3a_pack_scholar::records::CLAIM_KIND_EMPIRICAL,
                    &rec.summary,
                ));
            }
        }

        // Chunk telemetry records into 128KB tiles (max ~1000 per tile)
        for chunk in telemetry_records.chunks(1000) {
            let _ = writer.write_hyper_tile(TileType::TELEMETRY, chunk, None, None);
        }

        // Chunk claim records into 128KB tiles (max ~100 per tile)
        for chunk in claim_records.chunks(100) {
            let _ = writer.write_hyper_tile(s3a_pack_scholar::ScholarTileType::NOTE_CLAIMS, chunk, None, None);
        }

        // Write VCS commit records chunked into 128KB tiles (max 250 commits per tile)
        for chunk in self.commits.chunks(250) {
            writer.write_hyper_tile(TileType::VCS_COMMIT, chunk, None, None)?;
        }

        // Write VCS ref record pointing to current head
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let mut name_bytes = [0u8; VCS_REF_NAME_MAX];
        name_bytes[..4].copy_from_slice(b"main");

        let ref_rec = VcsRefRecord {
            ref_kind: REF_KIND_BRANCH,
            _pad: [0u8; 6],
            target_seal_hash: self.current_seal,
            author_pubkey: [0u8; 32],
            updated_timestamp: now,
            signature: [0u8; 64],
            name: name_bytes,
        };
        writer.write_hyper_tile(TileType::VCS_REF, &[ref_rec], None, None)?;
        drop(writer);

        // Read unsealed bytes and seal with SealOptionsV2
        let unsealed_bytes = fs::read(p)?;
        let mut opts = SealOptionsV2::default();
        opts.write_manifest = true;
        opts.title = Some("S3A Swarm Benchmark Final Repository".to_string());
        opts.prev_seal_hash = self.current_seal;

        let sealed = seal_buffer_v2(&unsealed_bytes, &opts)
            .map_err(|e| VcsError::Generic(e.to_string()))?;
        fs::write(p, &sealed)?;

        // Extract seal hash from final sealed buffer
        let final_hash = s3a_vcs::repo::extract_seal_hash_from_buffer(&sealed)
            .map_err(|e| VcsError::Generic(e.to_string()))?;

        Ok(hex::encode(final_hash))
    }
}
