//! `s3a-cli prove` / `s3a-cli verify-proof`: RFC 6962-style per-record inclusion proofs.
//!
//! Records of a tile are its payload (`payload_bytes`) split into `record_count` equal
//! slices. The proof is a self-contained JSON document; verifying it needs no S3A file.
//! See docs/adr/0012-lessons-from-bitcoin-block-design.md.

use s3a_edge::merkle;
use s3a_engine::MmapReader;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecordProof {
    pub scheme: String,
    pub tile_index: u32,
    pub tile_id: u64,
    pub record_index: u64,
    pub tree_size: u64,
    pub record_hex: String,
    pub root_hex: String,
    pub path_hex: Vec<String>,
}

pub const SCHEME: &str = "s3a-merkle-rfc6962-sha256-v1";

/// Splits a tile payload into its fixed-size records.
fn tile_records(
    payload: &[u8],
    payload_bytes: usize,
    count: usize,
) -> Result<(&[u8], usize), String> {
    if count == 0 {
        return Err("tile has no records".into());
    }
    if payload_bytes > payload.len() || !payload_bytes.is_multiple_of(count) {
        return Err(format!(
            "payload_bytes {} not divisible into {} fixed-size records",
            payload_bytes, count
        ));
    }
    Ok((&payload[..payload_bytes], payload_bytes / count))
}

/// Merkle root over all records of tile `tile`.
pub fn tile_record_root(path: &str, tile: usize) -> Result<([u8; 32], u64), String> {
    let reader = MmapReader::open(path).map_err(|e| e.to_string())?;
    let (h, payload) = reader.try_get_tile(tile).map_err(|e| format!("{:?}", e))?;
    let (data, rs) = tile_records(payload, h.payload_bytes as usize, h.record_count as usize)?;
    let root = merkle::merkle_root_fixed(data, rs).ok_or("bad record size")?;
    Ok((root, h.record_count as u64))
}

pub fn prove(path: &str, tile: usize, record: usize) -> Result<RecordProof, String> {
    let reader = MmapReader::open(path).map_err(|e| e.to_string())?;
    let (h, payload) = reader.try_get_tile(tile).map_err(|e| format!("{:?}", e))?;
    let n = h.record_count as usize;
    let (data, rs) = tile_records(payload, h.payload_bytes as usize, n)?;
    if record >= n {
        return Err(format!("record {} out of range (tile has {})", record, n));
    }
    let root = merkle::merkle_root_fixed(data, rs).ok_or("bad record size")?;
    let mut buf = [[0u8; 32]; merkle::MAX_PROOF_LEN];
    let len = merkle::inclusion_proof_fixed(data, rs, record, &mut buf).ok_or("proof failed")?;
    Ok(RecordProof {
        scheme: SCHEME.into(),
        tile_index: tile as u32,
        tile_id: h.tile_id,
        record_index: record as u64,
        tree_size: n as u64,
        record_hex: hex::encode(&data[record * rs..(record + 1) * rs]),
        root_hex: hex::encode(root),
        path_hex: buf[..len].iter().map(hex::encode).collect(),
    })
}

/// Verifies a proof on its own. If `expected_root` is given it must also match.
pub fn verify(p: &RecordProof, expected_root: Option<&[u8; 32]>) -> Result<bool, String> {
    if p.scheme != SCHEME {
        return Err(format!("unknown scheme {}", p.scheme));
    }
    let rec = hex::decode(&p.record_hex).map_err(|e| e.to_string())?;
    let root: [u8; 32] = hex::decode(&p.root_hex)
        .map_err(|e| e.to_string())?
        .try_into()
        .map_err(|_| "root must be 32 bytes")?;
    if let Some(e) = expected_root {
        if e != &root {
            return Ok(false);
        }
    }
    let mut path = Vec::with_capacity(p.path_hex.len());
    for s in &p.path_hex {
        let b: [u8; 32] = hex::decode(s)
            .map_err(|e| e.to_string())?
            .try_into()
            .map_err(|_| "path entry must be 32 bytes")?;
        path.push(b);
    }
    Ok(merkle::verify_inclusion(
        &rec,
        p.record_index,
        p.tree_size,
        &path,
        &root,
    ))
}
