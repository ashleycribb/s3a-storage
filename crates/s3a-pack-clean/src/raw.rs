//! Raw intake: untouched source bytes plus a fingerprint and source citation, sealed.
use crate::archive;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

/// Tile type for raw bytes (core generic BLOB tile; no new tile IDs).
pub const RAW_BLOB_TILE: u32 = s3a_core::TileType::BLOB;
/// Tile type for the raw manifest (core generic PROVENANCE tile).
pub const RAW_MANIFEST_TILE: u32 = s3a_core::TileType::PROVENANCE;

/// Who/where the data came from, when known.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SourceMeta {
    pub connector: String,
    pub org: Option<String>,
    pub user: Option<String>,
    pub device: Option<String>,
    pub location: Option<String>,
}

/// One raw source item and its citation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawItem {
    /// SHA-256 of the exact source bytes.
    pub item_sha256: String,
    pub size: u64,
    pub source_path: String,
    pub file_name: String,
    #[serde(flatten)]
    pub meta: SourceMeta,
    /// Capture time, unix seconds.
    pub captured_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawSnapshot {
    pub format: String,
    pub items: Vec<RawItem>,
    /// SHA-256 of the canonical item list. Cleaned versions cite this.
    pub snapshot_hash: String,
}

impl RawSnapshot {
    pub fn compute_hash(items: &[RawItem]) -> String {
        crate::sha256_hex(&crate::canonical(&("s3a-raw-v1", items)))
    }
    pub fn item(&self, sha: &str) -> Option<&RawItem> {
        self.items.iter().find(|i| i.item_sha256 == sha)
    }
}

/// Read each file untouched and write a new sealed raw archive at `out`.
pub fn intake(
    files: &[&Path],
    meta: &SourceMeta,
    captured_at: u64,
    out: &Path,
) -> io::Result<RawSnapshot> {
    let mut items = Vec::new();
    let mut tiles = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for f in files {
        let bytes = std::fs::read(f)?;
        let sha = crate::sha256_hex(&bytes);
        items.push(RawItem {
            item_sha256: sha.clone(),
            size: bytes.len() as u64,
            source_path: f.display().to_string(),
            file_name: f
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            meta: meta.clone(),
            captured_at,
        });
        if seen.insert(sha.clone()) {
            let key: [u8; 32] = hex::decode(&sha).unwrap().try_into().unwrap();
            tiles.extend(archive::chunk_tiles(RAW_BLOB_TILE, &key, &bytes));
        }
    }
    items.sort_by(|a, b| (&a.source_path, &a.item_sha256).cmp(&(&b.source_path, &b.item_sha256)));
    let snap = RawSnapshot {
        format: "s3a-raw-v1".into(),
        snapshot_hash: RawSnapshot::compute_hash(&items),
        items,
    };
    tiles.extend(archive::chunk_tiles(
        RAW_MANIFEST_TILE,
        &[0u8; 32],
        &crate::canonical(&snap),
    ));
    archive::write_sealed(out, &tiles, "S3A Clean Pack raw snapshot")?;
    Ok(snap)
}

/// Load a raw archive and check every item's bytes against its fingerprint.
pub fn load(path: &Path) -> io::Result<(RawSnapshot, BTreeMap<String, Vec<u8>>)> {
    let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_string());
    let man = archive::unchunk(&archive::read_tiles(path, RAW_MANIFEST_TILE)?)?;
    let mj = man
        .get(&[0u8; 32])
        .ok_or_else(|| bad("raw manifest missing"))?;
    let snap: RawSnapshot = serde_json::from_slice(mj).map_err(|e| bad(&e.to_string()))?;
    if RawSnapshot::compute_hash(&snap.items) != snap.snapshot_hash {
        return Err(bad("raw snapshot hash mismatch"));
    }
    let blobs = archive::unchunk(&archive::read_tiles(path, RAW_BLOB_TILE)?)?;
    let mut out = BTreeMap::new();
    for it in &snap.items {
        let key: [u8; 32] = hex::decode(&it.item_sha256)
            .ok()
            .and_then(|v| v.try_into().ok())
            .ok_or_else(|| bad("bad item hash"))?;
        let b = blobs
            .get(&key)
            .ok_or_else(|| bad("raw item bytes missing"))?;
        if crate::sha256_hex(b) != it.item_sha256 {
            return Err(bad("raw item bytes do not match fingerprint"));
        }
        out.insert(it.item_sha256.clone(), b.clone());
    }
    Ok((snap, out))
}
