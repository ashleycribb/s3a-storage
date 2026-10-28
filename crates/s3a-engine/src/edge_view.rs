//! Read-only accessors for the identity and seal records of an edge (micro-tile) archive.
//!
//! These read the records exactly as the importer and verifiers do, in one place, so a viewer does
//! not have to re-parse tile payloads. They read tiles with
//! [`MmapReader::try_get_edge_tile_unverified`]: each record carries its own CRC and (for seals) an
//! Ed25519 signature, and a viewer needs to see records in a damaged tile and report them as
//! damaged. **Nothing returned here is authenticated.** Callers must check `record.verify()`, the
//! tile CRC (via [`MmapReader::try_get_edge_tile`]) and the signature; see
//! [`s3a_edge::verify_signature_raw`] and [`MmapReader::edge_seal_chain`].

use s3a_edge::{
    compute_seal_hash, DeviceIdentityRecord, DeviceSealRecord, TILE_TYPE_RECORD, TILE_TYPE_SEAL,
};

use crate::MmapReader;

/// A device seal record plus where it was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EdgeSealEntry {
    /// Index of the seal tile that holds the record.
    pub container_tile: u32,
    /// The record as stored. Unauthenticated; call `record.verify()` and check the signature.
    pub record: DeviceSealRecord,
}

impl MmapReader {
    /// The device identity record: the first record-tile record (type 64, or legacy 33) that is
    /// at least 96 bytes, passes its own CRC and has a non-zero device id. Same rule as
    /// [`crate::import_edge_archive`]. `None` for non-edge archives or when there is none.
    pub fn edge_device_identity(&self) -> Option<DeviceIdentityRecord> {
        if !self.is_edge_archive() {
            return None;
        }
        let sz = std::mem::size_of::<DeviceIdentityRecord>();
        for i in 0..self.tile_count() as usize {
            let Ok((h, payload)) = self.try_get_edge_tile_unverified(i) else {
                continue;
            };
            if (h.type_id == TILE_TYPE_RECORD || h.type_id == 33) && payload.len() >= sz {
                let id: DeviceIdentityRecord = bytemuck::pod_read_unaligned(&payload[..sz]);
                if id.verify() && id.device_id != [0u8; 16] {
                    return Some(id);
                }
            }
        }
        None
    }

    /// Every device seal record in the file's seal tiles (type 70), in file order. Empty for
    /// non-edge archives. A seal tile whose payload is not a whole number of records is skipped.
    pub fn edge_seal_records(&self) -> Vec<EdgeSealEntry> {
        let mut out = Vec::new();
        if !self.is_edge_archive() {
            return out;
        }
        let sz = std::mem::size_of::<DeviceSealRecord>();
        for i in 0..self.tile_count() as usize {
            let Ok((h, payload)) = self.try_get_edge_tile_unverified(i) else {
                continue;
            };
            if h.type_id != TILE_TYPE_SEAL || payload.len() % sz != 0 {
                continue;
            }
            for c in payload.chunks_exact(sz) {
                out.push(EdgeSealEntry {
                    container_tile: i as u32,
                    record: bytemuck::pod_read_unaligned(c),
                });
            }
        }
        out
    }

    /// Recomputes the seal hash chain over the file's tiles for `device_id`
    /// (`compute_seal_hash(prev, generation, tile_index, payload_hash, device_id)`, with the same
    /// generation fallback as [`crate::import_edge_archive`]). Entry `i` is the expected seal hash
    /// for tile `i`. Stops at the first tile that fails its CRC check, since a damaged tile
    /// breaks the chain; the returned vector is then shorter than `tile_count`.
    pub fn edge_seal_chain(&self, device_id: &[u8; 16]) -> Vec<[u8; 32]> {
        let n = self.tile_count() as usize;
        let mut chain = Vec::new();
        if !self.is_edge_archive() {
            return chain;
        }
        let mut prev = [0u8; 32];
        for i in 0..n {
            let Ok((h, _)) = self.try_get_edge_tile(i) else {
                break;
            };
            let generation = if h.generation != 0 {
                h.generation as u64
            } else {
                self.file_header()
                    .generation
                    .saturating_sub(self.tile_count() as u64)
                    + i as u64
                    + 1
            };
            prev = compute_seal_hash(&prev, generation, i as u32, &h.payload_hash, device_id);
            chain.push(prev);
        }
        chain
    }
}
