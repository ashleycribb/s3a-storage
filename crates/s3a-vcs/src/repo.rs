//! High-level VCS operations: commit, log, branch, tag, diff, checkout, merge, export-git.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use bytemuck::from_bytes;
use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};

use s3a_core::consent::ConsentRecord;
use s3a_core::{
    FileHeader, HyperTileHeader, MarkRecord, SealedEndCapV2, SealedIndexHeaderV2,
    TileType, FILE_HEADER_SIZE, HYPER_TILE_SIZE,
    MARK_CONSENT_WITHDRAWN, SEALED_ENDCAP_MAGIC_V2, SEALED_ENDCAP_SIZE,
    SEALED_INDEX_HEADER_SIZE_V2,
};
use s3a_engine::sealed::{
    seal_buffer_v2, SealOptionsV2,
};
use s3a_engine::{MmapReader, TileWriter};
use s3a_pack_scholar::ScholarTileType;
use s3a_vault::address::GateAddress;

use crate::errors::VcsError;
use crate::records::{
    ConflictRecordDetail, VcsCommitRecord, VcsMergeResult, VcsRefRecord, VcsResolutionRecord,
    REF_KIND_BRANCH, REF_KIND_TAG, VCS_COMMIT_MESSAGE_MAX, VCS_REF_NAME_MAX,
};

/// Diff summary between two commits or states.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VcsDiffReport {
    pub from_seal: String,
    pub to_seal: String,
    pub tile_diffs: Vec<TileTypeDiff>,
    pub total_records_added: usize,
    pub total_records_withdrawn: usize,
    pub total_records_erased: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TileTypeDiff {
    pub type_id: u32,
    pub type_name: String,
    pub records_added: usize,
    pub records_withdrawn: usize,
    pub records_erased: usize,
    pub added_record_keys: Vec<String>,
}

/// S3A VCS Repository Handle wrapping an archive file.
pub struct VcsRepo {
    path: PathBuf,
}

impl VcsRepo {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, VcsError> {
        let p = path.as_ref().to_path_buf();
        if !p.exists() {
            return Err(VcsError::Generic(format!("Archive file not found: {:?}", p)));
        }
        Ok(Self { path: p })
    }

    /// Initializes a repository if not present or commits initial Genesis state.
    pub fn init<P: AsRef<Path>>(path: P) -> Result<Self, VcsError> {
        let p = path.as_ref().to_path_buf();
        if !p.exists() {
            let writer = TileWriter::create(&p)?;
            drop(writer);
        }
        Ok(Self { path: p })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns all commits stored across any VCS_COMMIT tiles in the archive.
    pub fn list_commits(&self) -> Result<Vec<VcsCommitRecord>, VcsError> {
        let reader = MmapReader::open(&self.path)?;
        let mut commits = Vec::new();
        let commit_size = std::mem::size_of::<VcsCommitRecord>();

        for i in 0..reader.tile_count() as usize {
            if let Some((hdr, payload)) = reader.get_tile(i) {
                if hdr.tile_type == TileType::VCS_COMMIT {
                    let count = (hdr.payload_bytes as usize) / commit_size;
                    for c in 0..count {
                        if (c + 1) * commit_size <= payload.len() {
                            let rec: &VcsCommitRecord =
                                from_bytes(&payload[c * commit_size..(c + 1) * commit_size]);
                            commits.push(*rec);
                        }
                    }
                }
            }
        }
        Ok(commits)
    }

    /// Returns all references (branches and tags) stored in VCS_REF tiles.
    pub fn list_refs(&self) -> Result<Vec<VcsRefRecord>, VcsError> {
        let reader = MmapReader::open(&self.path)?;
        let mut refs = Vec::new();
        let ref_size = std::mem::size_of::<VcsRefRecord>();

        for i in 0..reader.tile_count() as usize {
            if let Some((hdr, payload)) = reader.get_tile(i) {
                if hdr.tile_type == TileType::VCS_REF {
                    let count = (hdr.payload_bytes as usize) / ref_size;
                    for c in 0..count {
                        if (c + 1) * ref_size <= payload.len() {
                            let rec: &VcsRefRecord =
                                from_bytes(&payload[c * ref_size..(c + 1) * ref_size]);
                            refs.push(*rec);
                        }
                    }
                }
            }
        }
        Ok(refs)
    }

    /// Returns the active HEAD ref (defaults to "main").
    pub fn get_ref(&self, name: &str) -> Result<Option<VcsRefRecord>, VcsError> {
        let refs = self.list_refs()?;
        Ok(refs.into_iter().rev().find(|r| r.ref_name() == name))
    }

    /// Returns all signed human resolution records stored in VCS_RESOLUTION tiles.
    pub fn list_resolutions(&self) -> Result<Vec<VcsResolutionRecord>, VcsError> {
        let reader = MmapReader::open(&self.path)?;
        let mut resolutions = Vec::new();
        let res_size = std::mem::size_of::<VcsResolutionRecord>();

        for i in 0..reader.tile_count() as usize {
            if let Some((hdr, payload)) = reader.get_tile(i) {
                if hdr.tile_type == TileType::VCS_RESOLUTION {
                    let count = (hdr.payload_bytes as usize) / res_size;
                    for c in 0..count {
                        if (c + 1) * res_size <= payload.len() {
                            let rec: &VcsResolutionRecord =
                                from_bytes(&payload[c * res_size..(c + 1) * res_size]);
                            resolutions.push(*rec);
                        }
                    }
                }
            }
        }
        Ok(resolutions)
    }

    /// Appends a signed human resolution record to the archive.
    pub fn write_resolution(&self, resolution: &VcsResolutionRecord) -> Result<(), VcsError> {
        if !resolution.verify_signature() {
            return Err(VcsError::InvalidSignature("Invalid human resolution signature".to_string()));
        }
        self.append_records(TileType::VCS_RESOLUTION, &[*resolution])?;
        Ok(())
    }

    /// Creates or updates a branch reference.
    pub fn set_branch(
        &self,
        name: &str,
        target_seal_hash: [u8; 32],
        signing_key: &SigningKey,
    ) -> Result<VcsRefRecord, VcsError> {
        let existing = self.get_ref(name)?;
        if let Some(r) = existing {
            if r.ref_kind == REF_KIND_TAG {
                return Err(VcsError::TagImmutable(name.to_string()));
            }
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let env = VcsRefRecord::compute_envelope_bytes(
            REF_KIND_BRANCH,
            name,
            &target_seal_hash,
            now,
        );
        let sig: ed25519_dalek::Signature = signing_key.sign(&env);
        let pubkey = signing_key.verifying_key().to_bytes();

        let mut name_bytes = [0u8; VCS_REF_NAME_MAX];
        let bytes = name.as_bytes();
        let len = bytes.len().min(VCS_REF_NAME_MAX);
        name_bytes[..len].copy_from_slice(&bytes[..len]);

        let ref_record = VcsRefRecord {
            ref_kind: REF_KIND_BRANCH,
            _pad: [0u8; 6],
            target_seal_hash,
            author_pubkey: pubkey,
            updated_timestamp: now,
            signature: sig.to_bytes(),
            name: name_bytes,
        };

        self.append_records(TileType::VCS_REF, &[ref_record])?;
        Ok(ref_record)
    }

    /// Creates an immutable tag reference.
    pub fn set_tag(
        &self,
        name: &str,
        target_seal_hash: [u8; 32],
        signing_key: &SigningKey,
    ) -> Result<VcsRefRecord, VcsError> {
        if let Some(existing) = self.get_ref(name)? {
            if existing.target_seal_hash != target_seal_hash {
                return Err(VcsError::TagImmutable(name.to_string()));
            }
            return Ok(existing);
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let env = VcsRefRecord::compute_envelope_bytes(
            REF_KIND_TAG,
            name,
            &target_seal_hash,
            now,
        );
        let sig: ed25519_dalek::Signature = signing_key.sign(&env);
        let pubkey = signing_key.verifying_key().to_bytes();

        let mut name_bytes = [0u8; VCS_REF_NAME_MAX];
        let bytes = name.as_bytes();
        let len = bytes.len().min(VCS_REF_NAME_MAX);
        name_bytes[..len].copy_from_slice(&bytes[..len]);

        let tag_record = VcsRefRecord {
            ref_kind: REF_KIND_TAG,
            _pad: [0u8; 6],
            target_seal_hash,
            author_pubkey: pubkey,
            updated_timestamp: now,
            signature: sig.to_bytes(),
            name: name_bytes,
        };

        self.append_records(TileType::VCS_REF, &[tag_record])?;
        Ok(tag_record)
    }

    /// Seals archive with updated commit record and advances active branch ref.
    pub fn commit(
        &self,
        message: &str,
        branch: &str,
        signing_key: &SigningKey,
        secondary_parent: Option<[u8; 32]>,
    ) -> Result<VcsCommitRecord, VcsError> {
        let pubkey = signing_key.verifying_key().to_bytes();
        let gate_addr = GateAddress::from_public_key(&pubkey);

        // 1. Resolve current branch HEAD as primary parent
        let current_ref = self.get_ref(branch)?;
        let mut parent_count = 0u8;
        let mut parent_seal_hashes = [[0u8; 32]; 2];

        if let Some(r) = current_ref {
            parent_count = 1;
            parent_seal_hashes[0] = r.target_seal_hash;
        }

        if let Some(sec) = secondary_parent {
            parent_count = 2;
            parent_seal_hashes[1] = sec;
        }

        // 2. Read existing unsealed data bytes (excluding any old trailing footer)
        let in_bytes = fs::read(&self.path)?;
        let active_len = get_active_data_len(&in_bytes)?;
        let unsealed_bytes = &in_bytes[..active_len];

        // 3. Compute prospective seal hash of current state
        let prev_hash = parent_seal_hashes[0];
        let seal_generation = if current_ref.is_some() {
            (self.log(Some(branch))?.len() as u64) + 1
        } else {
            1
        };

        let mut opts = SealOptionsV2::default();
        opts.write_manifest = true;
        opts.prev_seal_hash = prev_hash;
        opts.seal_generation = seal_generation;
        opts.creator = Some(gate_addr.to_uri());
        opts.title = Some(message.to_string());

        let sealed_bytes = seal_buffer_v2(unsealed_bytes, &opts)?;
        let prospective_seal_hash = extract_seal_hash_from_buffer(&sealed_bytes)?;

        // 4. Build and sign commit record
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let env = VcsCommitRecord::compute_envelope_bytes(
            &prospective_seal_hash,
            parent_count,
            &parent_seal_hashes,
            now,
            message,
        );
        let commit_id = VcsCommitRecord::compute_commit_id(&env);
        let signature: ed25519_dalek::Signature = signing_key.sign(&env);

        let mut msg_bytes = [0u8; VCS_COMMIT_MESSAGE_MAX];
        let mb = message.as_bytes();
        let m_len = mb.len().min(VCS_COMMIT_MESSAGE_MAX);
        msg_bytes[..m_len].copy_from_slice(&mb[..m_len]);

        let commit_record = VcsCommitRecord {
            commit_id,
            seal_hash: prospective_seal_hash,
            author_pubkey: pubkey,
            author_address_prefix: gate_addr.prefix,
            author_checksum: gate_addr.checksum,
            parent_count,
            _pad: [0u8; 6],
            parent_seal_hashes,
            committed_timestamp: now,
            signature: signature.to_bytes(),
            message_bytes: msg_bytes,
        };

        // 5. Build and sign ref record
        let ref_env = VcsRefRecord::compute_envelope_bytes(
            REF_KIND_BRANCH,
            branch,
            &prospective_seal_hash,
            now,
        );
        let ref_sig: ed25519_dalek::Signature = signing_key.sign(&ref_env);
        let mut name_bytes = [0u8; VCS_REF_NAME_MAX];
        let bytes = branch.as_bytes();
        let len = bytes.len().min(VCS_REF_NAME_MAX);
        name_bytes[..len].copy_from_slice(&bytes[..len]);

        let ref_record = VcsRefRecord {
            ref_kind: REF_KIND_BRANCH,
            _pad: [0u8; 6],
            target_seal_hash: prospective_seal_hash,
            author_pubkey: pubkey,
            updated_timestamp: now,
            signature: ref_sig.to_bytes(),
            name: name_bytes,
        };

        // 6. Append commit and ref records into archive
        self.append_records(TileType::VCS_COMMIT, &[commit_record])?;
        self.append_records(TileType::VCS_REF, &[ref_record])?;

        // 7. Write sealed archive with manifest
        let updated_bytes = fs::read(&self.path)?;
        let updated_active_len = get_active_data_len(&updated_bytes)?;
        let final_sealed = seal_buffer_v2(&updated_bytes[..updated_active_len], &opts)?;
        fs::write(&self.path, final_sealed)?;

        Ok(commit_record)
    }

    /// Traverses commit history from a ref or commit hash backwards along parent chain.
    pub fn log(&self, from_ref_or_hash: Option<&str>) -> Result<Vec<VcsCommitRecord>, VcsError> {
        let all_commits = self.list_commits()?;
        let commit_map: HashMap<[u8; 32], VcsCommitRecord> = all_commits
            .iter()
            .map(|c| (c.seal_hash, *c))
            .collect();

        let start_seal = if let Some(target) = from_ref_or_hash {
            if let Some(r) = self.get_ref(target)? {
                r.target_seal_hash
            } else if let Ok(h) = hex::decode(target) {
                let mut arr = [0u8; 32];
                if h.len() == 32 {
                    arr.copy_from_slice(&h);
                    arr
                } else {
                    return Err(VcsError::CommitNotFound(target.to_string()));
                }
            } else {
                return Err(VcsError::CommitNotFound(target.to_string()));
            }
        } else if let Some(main_ref) = self.get_ref("main")? {
            main_ref.target_seal_hash
        } else if let Some(last) = all_commits.last() {
            last.seal_hash
        } else {
            return Ok(Vec::new());
        };

        let mut history = Vec::new();
        let mut curr = start_seal;
        let mut visited = HashSet::new();

        while curr != [0u8; 32] && !visited.contains(&curr) {
            visited.insert(curr);
            if let Some(commit) = commit_map.get(&curr) {
                history.push(*commit);
                curr = commit.parent_seal_hashes[0];
            } else {
                break;
            }
        }

        Ok(history)
    }

    /// Evaluates difference between two commit seal hashes.
    pub fn diff(&self, from_seal: &[u8; 32], to_seal: &[u8; 32]) -> Result<VcsDiffReport, VcsError> {
        let reader = MmapReader::open(&self.path)?;
        let count = reader.tile_count() as usize;
        let commit_size = std::mem::size_of::<VcsCommitRecord>();

        // Map each seal hash to the tile index where its commit record was written
        let mut commit_tile_indices: HashMap<[u8; 32], usize> = HashMap::new();
        for i in 0..count {
            if let Some((hdr, payload)) = reader.get_tile(i) {
                if hdr.tile_type == TileType::VCS_COMMIT {
                    let c_count = (hdr.payload_bytes as usize) / commit_size;
                    for c in 0..c_count {
                        if (c + 1) * commit_size <= payload.len() {
                            let rec: &VcsCommitRecord =
                                from_bytes(&payload[c * commit_size..(c + 1) * commit_size]);
                            commit_tile_indices.insert(rec.seal_hash, i);
                        }
                    }
                }
            }
        }

        let from_limit = if *from_seal == [0u8; 32] {
            0
        } else {
            commit_tile_indices.get(from_seal).copied().unwrap_or(count)
        };

        let to_limit = if *to_seal == [0u8; 32] {
            0
        } else {
            commit_tile_indices.get(to_seal).copied().unwrap_or(count)
        };

        let mut from_records: HashMap<u32, Vec<String>> = HashMap::new();
        let mut to_records: HashMap<u32, Vec<String>> = HashMap::new();

        let consent_size = std::mem::size_of::<ConsentRecord>();
        let mark_size = std::mem::size_of::<MarkRecord>();

        let mut withdrawn_from = 0usize;
        let mut withdrawn_to = 0usize;

        let max_scan = from_limit.max(to_limit);
        for i in 0..max_scan {
            if let Some((hdr, payload)) = reader.get_tile(i) {
                // Ignore metadata and vcs tiles for data diffs
                if hdr.tile_type == TileType::MANIFEST
                    || hdr.tile_type == TileType::SPECIFICATION
                    || hdr.tile_type == TileType::VCS_COMMIT
                    || hdr.tile_type == TileType::VCS_REF
                    || hdr.tile_type == TileType::VCS_RESOLUTION
                {
                    continue;
                }

                let mut keys = Vec::new();
                for r in 0..hdr.record_count {
                    keys.push(format!("tile_{}_rec_{}", hdr.tile_type, r));
                }

                if i < from_limit {
                    from_records.entry(hdr.tile_type).or_default().extend(keys.clone());
                    if hdr.tile_type == TileType::CONSENT {
                        let c_count = (hdr.payload_bytes as usize) / consent_size;
                        for c in 0..c_count {
                            if (c + 1) * consent_size <= payload.len() {
                                let rec: &ConsentRecord =
                                    from_bytes(&payload[c * consent_size..(c + 1) * consent_size]);
                                if rec.withdrawn_at > 0 {
                                    withdrawn_from += 1;
                                }
                            }
                        }
                    } else if hdr.tile_type == TileType::MARKS {
                        let m_count = (hdr.payload_bytes as usize) / mark_size;
                        for m in 0..m_count {
                            if (m + 1) * mark_size <= payload.len() {
                                let rec: &MarkRecord =
                                    from_bytes(&payload[m * mark_size..(m + 1) * mark_size]);
                                if rec.mark_kind == MARK_CONSENT_WITHDRAWN {
                                    withdrawn_from += 1;
                                }
                            }
                        }
                    }
                }

                if i < to_limit {
                    to_records.entry(hdr.tile_type).or_default().extend(keys);
                    if hdr.tile_type == TileType::CONSENT {
                        let c_count = (hdr.payload_bytes as usize) / consent_size;
                        for c in 0..c_count {
                            if (c + 1) * consent_size <= payload.len() {
                                let rec: &ConsentRecord =
                                    from_bytes(&payload[c * consent_size..(c + 1) * consent_size]);
                                if rec.withdrawn_at > 0 {
                                    withdrawn_to += 1;
                                }
                            }
                        }
                    } else if hdr.tile_type == TileType::MARKS {
                        let m_count = (hdr.payload_bytes as usize) / mark_size;
                        for m in 0..m_count {
                            if (m + 1) * mark_size <= payload.len() {
                                let rec: &MarkRecord =
                                    from_bytes(&payload[m * mark_size..(m + 1) * mark_size]);
                                if rec.mark_kind == MARK_CONSENT_WITHDRAWN {
                                    withdrawn_to += 1;
                                }
                            }
                        }
                    }
                }
            }
        }

        let mut tile_diffs = Vec::new();
        let mut total_added = 0usize;

        for (&type_id, to_keys) in &to_records {
            let from_keys = from_records.get(&type_id).cloned().unwrap_or_default();
            let added_count = to_keys.len().saturating_sub(from_keys.len());
            total_added += added_count;

            tile_diffs.push(TileTypeDiff {
                type_id,
                type_name: reader.resolve_tile_type_name(type_id),
                records_added: added_count,
                records_withdrawn: withdrawn_to.saturating_sub(withdrawn_from),
                records_erased: 0,
                added_record_keys: to_keys[from_keys.len()..].to_vec(),
            });
        }

        Ok(VcsDiffReport {
            from_seal: hex::encode(from_seal),
            to_seal: hex::encode(to_seal),
            tile_diffs,
            total_records_added: total_added,
            total_records_withdrawn: withdrawn_to.saturating_sub(withdrawn_from),
            total_records_erased: 0,
        })
    }

    /// Merges `source_branch` into active `target_branch`.
    ///
    /// Mandatory Safety Invariant:
    /// When a merge conflict involves claim, evidence, citation_fingerprint, or ai_trace records,
    /// this function MUST NOT auto-resolve or pick a side. It returns `ConflictNeedsHuman` listing
    /// the conflicting records. Merge completes only after a signed human resolution record
    /// is written to the archive.
    pub fn merge(
        &self,
        source_branch: &str,
        target_branch: &str,
        signing_key: &SigningKey,
        _is_human_judgment_approved: bool,
    ) -> Result<VcsMergeResult, VcsError> {
        let src_ref = self
            .get_ref(source_branch)?
            .ok_or_else(|| VcsError::RefNotFound(source_branch.to_string()))?;
        let tgt_ref = self
            .get_ref(target_branch)?
            .ok_or_else(|| VcsError::RefNotFound(target_branch.to_string()))?;

        // 0. Identical branches
        if src_ref.target_seal_hash == tgt_ref.target_seal_hash {
            let history = self.log(Some(target_branch))?;
            let last = history.first().copied().unwrap();
            return Ok(VcsMergeResult::Merged(last));
        }

        // 1. Fast-forward check: if target is ancestor of source, simply advance
        let src_log = self.log(Some(source_branch))?;
        if src_log.iter().any(|c| c.seal_hash == tgt_ref.target_seal_hash) {
            self.set_branch(target_branch, src_ref.target_seal_hash, signing_key)?;
            let last_commit = src_log.first().copied().unwrap();
            return Ok(VcsMergeResult::Merged(last_commit));
        }

        // 2. Multi-parent divergence check
        let tgt_log = self.log(Some(target_branch))?;
        let src_seals: HashSet<[u8; 32]> = src_log.iter().map(|c| c.seal_hash).collect();
        let merge_base = tgt_log
            .iter()
            .find(|c| src_seals.contains(&c.seal_hash))
            .map(|c| c.seal_hash)
            .unwrap_or([0u8; 32]);

        // Map commits to their respective tile indexes
        let reader = MmapReader::open(&self.path)?;
        let tile_count = reader.tile_count() as usize;
        let commit_size = std::mem::size_of::<VcsCommitRecord>();
        let mut commit_tile_indices: HashMap<[u8; 32], usize> = HashMap::new();
        let mut sorted_commit_indices: Vec<usize> = Vec::new();

        for i in 0..tile_count {
            if let Some((hdr, payload)) = reader.get_tile(i) {
                if hdr.tile_type == TileType::VCS_COMMIT {
                    let count = (hdr.payload_bytes as usize) / commit_size;
                    for c in 0..count {
                        if (c + 1) * commit_size <= payload.len() {
                            let rec: &VcsCommitRecord =
                                from_bytes(&payload[c * commit_size..(c + 1) * commit_size]);
                            commit_tile_indices.insert(rec.seal_hash, i);
                            sorted_commit_indices.push(i);
                        }
                    }
                }
            }
        }
        sorted_commit_indices.sort_unstable();
        sorted_commit_indices.dedup();

        let tiles_for_commit = |commit_seal: &[u8; 32]| -> Vec<usize> {
            if let Some(&c_idx) = commit_tile_indices.get(commit_seal) {
                let prev_idx = sorted_commit_indices
                    .iter()
                    .rev()
                    .find(|&&idx| idx < c_idx)
                    .copied()
                    .unwrap_or(0);
                let start = if prev_idx == 0 { 0 } else { prev_idx + 1 };
                (start..c_idx).collect()
            } else {
                Vec::new()
            }
        };

        // Identify tiles on each branch since merge_base
        let mut tgt_branch_tiles = HashSet::new();
        for c in tgt_log.iter().take_while(|c| c.seal_hash != merge_base) {
            for t in tiles_for_commit(&c.seal_hash) {
                tgt_branch_tiles.insert(t);
            }
        }

        let mut src_branch_tiles = HashSet::new();
        for c in src_log.iter().take_while(|c| c.seal_hash != merge_base) {
            for t in tiles_for_commit(&c.seal_hash) {
                src_branch_tiles.insert(t);
            }
        }

        // Extract records from each branch's diverged tiles
        let mut tgt_records: HashMap<String, (u32, String, [u8; 32], String)> = HashMap::new();
        for &t in &tgt_branch_tiles {
            if let Some((hdr, payload)) = reader.get_tile(t) {
                let type_name = reader.resolve_tile_type_name(hdr.tile_type);
                for (id, tt, tn, ch, sum) in extract_records_from_tile(hdr, payload, &type_name) {
                    tgt_records.insert(id, (tt, tn, ch, sum));
                }
            }
        }

        let mut src_records: HashMap<String, (u32, String, [u8; 32], String)> = HashMap::new();
        for &t in &src_branch_tiles {
            if let Some((hdr, payload)) = reader.get_tile(t) {
                let type_name = reader.resolve_tile_type_name(hdr.tile_type);
                for (id, tt, tn, ch, sum) in extract_records_from_tile(hdr, payload, &type_name) {
                    src_records.insert(id, (tt, tn, ch, sum));
                }
            }
        }

        drop(reader); // release mmap before possible writes

        // Load existing signed resolutions
        let existing_resolutions = self.list_resolutions()?;

        let mut needs_human_conflicts = Vec::new();
        let mut non_claim_conflicts = Vec::new();

        for (rec_id, (tgt_type, tgt_name, tgt_hash, tgt_summary)) in &tgt_records {
            if let Some((src_type, _src_name, src_hash, src_summary)) = src_records.get(rec_id) {
                if tgt_hash != src_hash {
                    // Check if a signed human resolution exists for this record ID
                    let resolved = existing_resolutions
                        .iter()
                        .rev()
                        .find(|r| r.conflict_record_id() == *rec_id && r.verify_signature());

                    if resolved.is_none() {
                        let is_claim_related = is_human_judgment_required(*tgt_type, tgt_name)
                            || is_human_judgment_required(*src_type, tgt_name);

                        if is_claim_related {
                            needs_human_conflicts.push(ConflictRecordDetail {
                                record_id: rec_id.clone(),
                                tile_type: *tgt_type,
                                type_name: tgt_name.clone(),
                                target_version: tgt_summary.clone(),
                                source_version: src_summary.clone(),
                                requires_human_judgment: true,
                            });
                        } else {
                            non_claim_conflicts.push(rec_id.clone());
                        }
                    }
                }
            }
        }

        // 3. Conflict evaluation
        if !needs_human_conflicts.is_empty() {
            // DO NOT auto-resolve. DO NOT create a merge commit.
            return Ok(VcsMergeResult::ConflictNeedsHuman {
                message: format!(
                    "Merge blocked: {} epistemic/empirical record(s) in conflict require signed human judgment",
                    needs_human_conflicts.len()
                ),
                conflicting_records: needs_human_conflicts,
            });
        }

        if !non_claim_conflicts.is_empty() {
            return Err(VcsError::MergeConflict {
                reason: "Merge conflict on non-claim records".to_string(),
                conflicting_record_ids: non_claim_conflicts,
                requires_human_judgment: false,
            });
        }

        // 4. All conflicts resolved or clean union -> seal merge commit
        let msg = format!("Merge branch '{}' into '{}'", source_branch, target_branch);
        let commit_record = self.commit(&msg, target_branch, signing_key, Some(src_ref.target_seal_hash))?;
        Ok(VcsMergeResult::Merged(commit_record))
    }

    /// Appends arbitrary records to archive via TileWriter.
    fn append_records<T: bytemuck::Pod>(&self, tile_type: u32, records: &[T]) -> Result<(), VcsError> {
        let mut writer = TileWriter::open_append(&self.path)?;
        writer.write_hyper_tile(tile_type, records, None, None)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Checks whether a tile type or type name represents epistemic claims, research evidence,
/// citation fingerprints, or AI trace records requiring mandatory human supervisor judgment.
pub fn is_human_judgment_required(tile_type: u32, type_name: &str) -> bool {
    tile_type == ScholarTileType::NOTE_CLAIMS // 20
        || tile_type == ScholarTileType::RESEARCH_GRAPH // 13
        || tile_type == ScholarTileType::AI_TRACE_LOG // 11
        || tile_type == ScholarTileType::CITATION_FINGERPRINT // 32
        || type_name.contains("claim")
        || type_name.contains("evidence")
        || type_name.contains("citation_fingerprint")
        || type_name.contains("ai_trace")
}

fn extract_records_from_tile(
    hdr: &HyperTileHeader,
    payload: &[u8],
    type_name: &str,
) -> Vec<(String, u32, String, [u8; 32], String)> {
    let mut out = Vec::new();
    if hdr.tile_type == TileType::MANIFEST
        || hdr.tile_type == TileType::SPECIFICATION
        || hdr.tile_type == TileType::VCS_COMMIT
        || hdr.tile_type == TileType::VCS_REF
        || hdr.tile_type == TileType::VCS_RESOLUTION
    {
        return out;
    }

    if hdr.tile_type == ScholarTileType::NOTE_CLAIMS {
        let claim_size = std::mem::size_of::<s3a_pack_scholar::records::ScholarNoteClaimRecord>();
        let count = (hdr.payload_bytes as usize) / claim_size;
        for r in 0..count {
            if (r + 1) * claim_size <= payload.len() {
                let rec: &s3a_pack_scholar::records::ScholarNoteClaimRecord =
                    from_bytes(&payload[r * claim_size..(r + 1) * claim_size]);
                let rec_id = if rec.session_uuid[0] == 0 && rec.session_uuid[1] > 0 {
                    format!("claim_{}", rec.session_uuid[1])
                } else if rec.session_uuid != [0, 0] {
                    format!("claim_{:016x}{:016x}", rec.session_uuid[0], rec.session_uuid[1])
                } else if rec.author_hash > 0 {
                    format!("claim_{}_{}", rec.author_hash, rec.kind)
                } else {
                    format!("claim_rec_{}", r)
                };
                let content_hash = rec.quote_sha256;
                let summary = format!(
                    "Claim(kind={}, conf={:.2}, text_hash={:08x})",
                    rec.kind,
                    rec.confidence,
                    rec.text_hash
                );
                out.push((rec_id, hdr.tile_type, type_name.to_string(), content_hash, summary));
            }
        }
    } else if hdr.tile_type == TileType::TELEMETRY {
        let tel_size = std::mem::size_of::<s3a_core::TelemetryRecord>();
        let count = (hdr.payload_bytes as usize) / tel_size;
        for r in 0..count {
            if (r + 1) * tel_size <= payload.len() {
                let rec: &s3a_core::TelemetryRecord =
                    from_bytes(&payload[r * tel_size..(r + 1) * tel_size]);
                let rec_id = format!("telemetry_{}_{}_{}", rec.sensor_id, rec.metric_id, rec.timestamp);
                let mut hasher = Sha256::new();
                hasher.update(&rec.value.to_le_bytes());
                let content_hash: [u8; 32] = hasher.finalize().into();
                let summary = format!("Telemetry(val={})", rec.value);
                out.push((rec_id, hdr.tile_type, type_name.to_string(), content_hash, summary));
            }
        }
    } else {
        let rec_count = hdr.record_count as usize;
        if rec_count > 0 && hdr.payload_bytes > 0 {
            let rec_size = (hdr.payload_bytes as usize) / rec_count;
            for r in 0..rec_count {
                if (r + 1) * rec_size <= payload.len() {
                    let slice = &payload[r * rec_size..(r + 1) * rec_size];
                    let rec_id = format!("{}_rec_{}", type_name, r);
                    let content_hash: [u8; 32] = Sha256::digest(slice).into();
                    let summary = format!("{}(hash={})", type_name, hex::encode(&content_hash[..8]));
                    out.push((rec_id, hdr.tile_type, type_name.to_string(), content_hash, summary));
                }
            }
        }
    }

    out
}

fn get_active_data_len(buf: &[u8]) -> Result<usize, VcsError> {
    if buf.len() < FILE_HEADER_SIZE {
        return Err(VcsError::Generic("Buffer too small for header".into()));
    }
    let slot_a: &FileHeader = from_bytes(&buf[0..64]);
    let slot_b: &FileHeader = from_bytes(&buf[64..128]);
    let (active, _) = s3a_engine::resolve_active_header(slot_a, slot_b)
        .map_err(|e| VcsError::Generic(e.to_string()))?;
    let len = FILE_HEADER_SIZE + (active.tile_count as usize * HYPER_TILE_SIZE);
    Ok(len.min(buf.len()))
}

fn extract_seal_hash_from_buffer(buf: &[u8]) -> Result<[u8; 32], VcsError> {
    if buf.len() < FILE_HEADER_SIZE + SEALED_ENDCAP_SIZE {
        return Err(VcsError::Generic("Buffer too small for sealed end cap".into()));
    }
    let endcap_slice = &buf[buf.len() - SEALED_ENDCAP_SIZE..];
    let endcap: &SealedEndCapV2 = from_bytes(endcap_slice);
    if endcap.magic != SEALED_ENDCAP_MAGIC_V2 {
        return Err(VcsError::Generic("Missing SEALED_ENDCAP_MAGIC_V2".into()));
    }

    let index_offset = endcap.index_offset as usize;
    let index_len = endcap.index_len as usize;
    if index_offset + index_len > buf.len() {
        return Err(VcsError::Generic("Truncated index".into()));
    }

    let idx_hdr: &SealedIndexHeaderV2 = from_bytes(&buf[index_offset..index_offset + SEALED_INDEX_HEADER_SIZE_V2]);
    Ok(idx_hdr.seal_hash)
}
