//! Record-root tile (type 83, ADR-0012 follow-up).
//!
//! An optional tile written just before sealing. It lists, for every tile whose payload splits
//! into `record_count` equal records, the RFC 6962 Merkle root of those records. Because it is an
//! ordinary tile, the existing `compute_seal_hash_v2` covers it unchanged. Files without it seal,
//! verify and prove exactly as before.
//!
//! The tile is *derived* data: writers never copy it forward. Sealing always drops any prior
//! record-root tile and regenerates it (if requested); the compactor and fusion drop it.

use s3a_core::{HyperTileHeader, TileType, HYPER_TILE_HEADER_SIZE, HYPER_TILE_SIZE};
use s3a_edge::merkle;

/// One entry per covered tile. 64 bytes, little-endian.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RecordRootEntry {
    pub tile_index: u32,
    pub tile_type: u32,
    pub tile_id: u64,
    pub record_count: u64,
    /// Seal generation the roots were computed for (stale-root guard).
    pub seal_generation: u64,
    pub root: [u8; 32],
}

pub const RECORD_ROOT_ENTRY_SIZE: usize = core::mem::size_of::<RecordRootEntry>();
pub const ENTRIES_PER_TILE: usize =
    (HYPER_TILE_SIZE - HYPER_TILE_HEADER_SIZE) / RECORD_ROOT_ENTRY_SIZE;

/// RFC 6962 root over a tile's fixed-size records, or `None` if it has none / isn't divisible.
pub fn tile_root(h: &HyperTileHeader, payload: &[u8]) -> Option<[u8; 32]> {
    let n = h.record_count as usize;
    let pb = h.payload_bytes as usize;
    if n == 0 || pb == 0 || pb > payload.len() || !pb.is_multiple_of(n) {
        return None;
    }
    merkle::merkle_root_fixed(&payload[..pb], pb / n)
}

/// Entries for every tile in an iterator of `(index, header, payload)`, skipping record-root tiles.
pub fn compute_entries<'a, I>(tiles: I, seal_generation: u64) -> Vec<RecordRootEntry>
where
    I: IntoIterator<Item = (usize, &'a HyperTileHeader, &'a [u8])>,
{
    tiles
        .into_iter()
        .filter(|(_, h, _)| h.tile_type != TileType::RECORD_ROOT)
        .filter_map(|(i, h, p)| {
            tile_root(h, p).map(|root| RecordRootEntry {
                tile_index: i as u32,
                tile_type: h.tile_type,
                tile_id: h.tile_id,
                record_count: h.record_count as u64,
                seal_generation,
                root,
            })
        })
        .collect()
}

/// Outcome of checking a file's record-root tile(s) against recomputed roots.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecordRootReport {
    pub present: bool,
    pub checked: usize,
    pub mismatches: Vec<String>,
}

impl RecordRootReport {
    pub fn ok(&self) -> bool {
        self.mismatches.is_empty()
    }
}

/// Reads all record-root entries in a file (empty if the file has none).
pub fn read_entries(reader: &crate::MmapReader) -> Vec<RecordRootEntry> {
    let mut out = Vec::new();
    for i in 0..reader.tile_count() as usize {
        if let Some((h, p)) = reader.get_tile(i) {
            if h.tile_type == TileType::RECORD_ROOT {
                let n = (h.record_count as usize).min(p.len() / RECORD_ROOT_ENTRY_SIZE);
                for c in 0..n {
                    out.push(bytemuck::pod_read_unaligned::<RecordRootEntry>(
                        &p[c * RECORD_ROOT_ENTRY_SIZE..(c + 1) * RECORD_ROOT_ENTRY_SIZE],
                    ));
                }
            }
        }
    }
    out
}

/// Root recorded for `tile_index`, if the file carries a record-root tile.
pub fn sealed_root_for(reader: &crate::MmapReader, tile_index: u32) -> Option<RecordRootEntry> {
    read_entries(reader)
        .into_iter()
        .find(|e| e.tile_index == tile_index)
}

/// Recomputes every root and compares with the record-root tile. Files without the tile report
/// `present: false` and no mismatches.
pub fn check(reader: &crate::MmapReader) -> RecordRootReport {
    let entries = read_entries(reader);
    let mut rep = RecordRootReport {
        present: !entries.is_empty(),
        ..Default::default()
    };
    if !rep.present {
        return rep;
    }
    let gen0 = entries[0].seal_generation;
    let tiles: Vec<(usize, &HyperTileHeader, &[u8])> = (0..reader.tile_count() as usize)
        .filter_map(|i| reader.get_tile(i).map(|(h, p)| (i, h, p)))
        .collect();
    let fresh = compute_entries(tiles.iter().copied(), gen0);
    for e in &entries {
        rep.checked += 1;
        if e.seal_generation != gen0 {
            rep.mismatches.push(format!(
                "tile {}: mixed seal generations (stale record-root tile)",
                e.tile_index
            ));
            continue;
        }
        match fresh.iter().find(|f| f.tile_index == e.tile_index) {
            Some(f) if f == e => {}
            Some(_) => rep
                .mismatches
                .push(format!("tile {}: record root mismatch", e.tile_index)),
            None => rep.mismatches.push(format!(
                "tile {}: listed but missing or has no records",
                e.tile_index
            )),
        }
    }
    for f in &fresh {
        if !entries.iter().any(|e| e.tile_index == f.tile_index) {
            rep.mismatches.push(format!(
                "tile {}: has records but no sealed root",
                f.tile_index
            ));
        }
    }
    rep
}

/// Builds the record-root tile buffers (one or more full 128 KB tiles) for `entries`.
pub fn build_tiles(
    entries: &[RecordRootEntry],
    first_index: u32,
    generation: u64,
    file_uuid: [u64; 2],
) -> Vec<Vec<u8>> {
    entries
        .chunks(ENTRIES_PER_TILE)
        .enumerate()
        .map(|(k, chunk)| {
            let payload: &[u8] = bytemuck::cast_slice(chunk);
            let mut t = vec![0u8; HYPER_TILE_SIZE];
            let mut h = HyperTileHeader::new(0, TileType::RECORD_ROOT);
            h.record_count = chunk.len() as _;
            h.record_layout_version = 1;
            h.payload_bytes = payload.len() as u32;
            h.data_crc32 = crc32c::crc32c(payload);
            h.generation = generation;
            h.tile_index = first_index + k as u32;
            h.file_uuid = file_uuid;
            h.header_crc32 = crate::compute_tile_header_crc32(&h);
            t[..HYPER_TILE_HEADER_SIZE].copy_from_slice(bytemuck::bytes_of(&h));
            t[HYPER_TILE_HEADER_SIZE..HYPER_TILE_HEADER_SIZE + payload.len()]
                .copy_from_slice(payload);
            t
        })
        .collect()
}
