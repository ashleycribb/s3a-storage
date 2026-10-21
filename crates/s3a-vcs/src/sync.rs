//! Sync protocol over gate:// with tile-hash inventory negotiation and consent governance.

use std::collections::HashSet;

use ed25519_dalek::SigningKey;
use sha2::{Digest, Sha256};

use s3a_core::consent::ConsentRecord;
use s3a_core::{HyperTileHeader, TileType};
use s3a_engine::{MmapReader, TileWriter};

use crate::errors::VcsError;
use crate::repo::VcsRepo;

/// Inventory of tile hashes and active DAG head presented during push/pull sync.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VcsSyncInventory {
    pub gate_address: String,
    pub branch: String,
    pub head_seal_hash: [u8; 32],
    pub known_tile_hashes: Vec<[u8; 32]>,
    pub consent_purpose_filter: Option<u32>,
}

/// Negotiates and transmits missing tiles from `source_repo` to `target_repo`.
pub fn sync_push(
    source_repo: &VcsRepo,
    target_repo: &VcsRepo,
    branch: &str,
    allow_non_fast_forward: bool,
    consent_purpose: Option<u32>,
    signing_key: &SigningKey,
) -> Result<usize, VcsError> {
    let src_ref = source_repo
        .get_ref(branch)?
        .ok_or_else(|| VcsError::RefNotFound(branch.to_string()))?;

    let tgt_ref = target_repo.get_ref(branch)?;

    // Fast-forward check: if target has branch and source doesn't contain target's seal, reject
    if let Some(t_ref) = &tgt_ref {
        if t_ref.target_seal_hash != src_ref.target_seal_hash {
            let src_history = source_repo.log(Some(branch))?;
            let is_ancestor = src_history.iter().any(|c| c.seal_hash == t_ref.target_seal_hash);
            if !is_ancestor && !allow_non_fast_forward {
                return Err(VcsError::NonFastForward {
                    branch: branch.to_string(),
                    target_seal: hex::encode(src_ref.target_seal_hash),
                    remote_seal: hex::encode(t_ref.target_seal_hash),
                });
            }
        }
    }

    // 1. Build target inventory (hash of each tile payload in target)
    let mut known_hashes = HashSet::new();
    {
        let tgt_reader = MmapReader::open(target_repo.path())?;
        for i in 0..tgt_reader.tile_count() as usize {
            if let Some((_, payload)) = tgt_reader.get_tile(i) {
                let h: [u8; 32] = Sha256::digest(payload).into();
                known_hashes.insert(h);
            }
        }
    } // tgt_reader dropped to release file lock on Windows

    // 2. Scan source tiles and gather missing ones
    let mut missing_tiles: Vec<(HyperTileHeader, Vec<u8>)> = Vec::new();
    let consent_size = std::mem::size_of::<ConsentRecord>();
    {
        let src_reader = MmapReader::open(source_repo.path())?;
        for i in 0..src_reader.tile_count() as usize {
            if let Some((hdr, payload)) = src_reader.get_tile(i) {
                let h: [u8; 32] = Sha256::digest(payload).into();
                if !known_hashes.contains(&h) {
                    // Check consent purpose gate if specified
                    if let Some(req_purpose) = consent_purpose {
                        if hdr.tile_type == TileType::CONSENT {
                            let c_count = (hdr.payload_bytes as usize) / consent_size;
                            for c in 0..c_count {
                                if (c + 1) * consent_size <= payload.len() {
                                    let rec: &ConsentRecord =
                                        &bytemuck::pod_read_unaligned::<ConsentRecord>(&payload[c * consent_size..(c + 1) * consent_size]);
                                    if rec.purpose != req_purpose && rec.withdrawn_at == 0 {
                                        return Err(VcsError::ConsentRestricted(format!(
                                            "Participant record purpose {} is incompatible with sync request scope {}",
                                            rec.purpose, req_purpose
                                        )));
                                    }
                                }
                            }
                        }
                    }

                    missing_tiles.push((*hdr, payload.to_vec()));
                }
            }
        }
    } // src_reader dropped

    // 3. Append missing tiles to target using TileWriter
    if !missing_tiles.is_empty() {
        let mut writer = TileWriter::open_append(target_repo.path())?;
        for (hdr, payload) in &missing_tiles {
            writer.write_raw_hyper_tile(hdr, payload)?;
        }
        drop(writer);
    }

    // 4. Update target branch pointer
    target_repo.set_branch(branch, src_ref.target_seal_hash, signing_key)?;

    Ok(missing_tiles.len())
}
