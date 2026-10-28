//! A cheap, non-failing look at a file's seal footer (read-only).
//!
//! [`crate::verify_sealed_archive`] answers "is this seal good, and can the tiles be rebuilt?",
//! and returns `Err` for several kinds of damage. A viewer also needs the plainer question
//! "what does the tail of this file say?" without an error path, without rebuilding anything and
//! without touching the tile bodies. This answers that from the end cap and index alone.

use sha2::{Digest, Sha256};

use s3a_core::{
    SealedEndCap, SealedEndCapV2, SealedIndexHeaderV2, FILE_HEADER_SIZE, SEALED_ENDCAP_MAGIC_V1,
    SEALED_ENDCAP_MAGIC_V2, SEALED_ENDCAP_SIZE, SEALED_INDEX_ENTRY_SIZE_V2,
    SEALED_INDEX_HEADER_SIZE_V2, SEALED_INDEX_MAGIC_V2,
};

use crate::{compute_endcap_crc32, compute_endcap_crc32_v2};

/// What the tail of a file says about its seal. Nothing here is a judgement: it reports which
/// of the cheap structural checks held.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SealFooterProbe {
    /// A recognised end-cap magic sits at the end of the file (so the file claims to be sealed).
    pub has_endcap_magic: bool,
    /// Footer format from the magic: 1 or 2; 0 when there is no end-cap magic.
    pub footer_version: u16,
    /// The end cap's own CRC32C holds.
    pub endcap_crc_ok: bool,
    /// The footer (v1) or index (v2) lies inside the file, after the file header.
    pub footer_in_bounds: bool,
    /// v1: the footer CRC32C holds. v2: the index SHA-256 matches the end cap. `false` when the
    /// footer is out of bounds.
    pub footer_hash_ok: bool,
    /// v2 only: the index header magic and entry-count/length agree. Always `false` for v1.
    pub index_well_formed: bool,
    /// v2 only: the index's own seal hash recomputes. Always `false` for v1 (no seal hash).
    pub seal_hash_ok: bool,
    /// Tile count, `k`/`m` parity shape and file uuid as the footer states them (unverified when
    /// the matching `*_ok` flags are false).
    pub tile_count: u32,
    pub k: u16,
    pub m: u16,
    pub file_uuid: [u64; 2],
    /// v2 only, from the index header (all zero when it could not be parsed).
    pub seal_generation: u64,
    pub seal_hash: [u8; 32],
    pub prev_seal_hash: [u8; 32],
}

impl SealFooterProbe {
    /// Every cheap check held (a "looks intact" summary; it is not a substitute for
    /// [`crate::verify_sealed_archive`], which also checks the tile bodies against the directory).
    pub fn footer_intact(&self) -> bool {
        self.has_endcap_magic
            && self.endcap_crc_ok
            && self.footer_in_bounds
            && self.footer_hash_ok
            && (self.footer_version == 1 || (self.index_well_formed && self.seal_hash_ok))
    }
}

/// Inspects the seal footer of a whole file held in memory. Never fails and never panics on
/// malformed input; arithmetic on attacker-controlled offsets is checked.
pub fn probe_seal_footer(file: &[u8]) -> SealFooterProbe {
    let mut p = SealFooterProbe::default();
    let len = file.len();
    if len < FILE_HEADER_SIZE + SEALED_ENDCAP_SIZE {
        return p;
    }
    let cap = &file[len - SEALED_ENDCAP_SIZE..];
    let magic = &cap[..8];
    if magic == SEALED_ENDCAP_MAGIC_V1 {
        p.has_endcap_magic = true;
        p.footer_version = 1;
        let ec: SealedEndCap = bytemuck::pod_read_unaligned(cap);
        p.tile_count = ec.tile_count;
        p.k = ec.k;
        p.m = ec.m;
        p.file_uuid = ec.file_uuid;
        p.endcap_crc_ok = compute_endcap_crc32(&ec) == ec.endcap_crc32;
        if let Some(end) = ec.footer_offset.checked_add(ec.footer_length) {
            if ec.footer_offset >= FILE_HEADER_SIZE as u64
                && end <= (len - SEALED_ENDCAP_SIZE) as u64
            {
                p.footer_in_bounds = true;
                p.footer_hash_ok = crc32c::crc32c(&file[ec.footer_offset as usize..end as usize])
                    == ec.footer_crc32;
            }
        }
    } else if magic == SEALED_ENDCAP_MAGIC_V2 {
        p.has_endcap_magic = true;
        p.footer_version = 2;
        let ec: SealedEndCapV2 = bytemuck::pod_read_unaligned(cap);
        p.endcap_crc_ok = compute_endcap_crc32_v2(&ec) == ec.endcap_crc32;
        if let Some(end) = ec.index_offset.checked_add(ec.index_len) {
            if ec.index_offset >= FILE_HEADER_SIZE as u64
                && end <= (len - SEALED_ENDCAP_SIZE) as u64
                && ec.index_len >= SEALED_INDEX_HEADER_SIZE_V2 as u64
            {
                p.footer_in_bounds = true;
                let idx = &file[ec.index_offset as usize..end as usize];
                p.footer_hash_ok = <[u8; 32]>::from(Sha256::digest(idx)) == ec.index_hash;
                let hdr: SealedIndexHeaderV2 =
                    bytemuck::pod_read_unaligned(&idx[..SEALED_INDEX_HEADER_SIZE_V2]);
                p.tile_count = hdr.tile_count;
                p.k = hdr.k;
                p.m = hdr.m;
                p.file_uuid = hdr.file_uuid;
                p.seal_generation = hdr.seal_generation;
                p.seal_hash = hdr.seal_hash;
                p.prev_seal_hash = hdr.prev_seal_hash;
                let want = (hdr.tile_count as usize)
                    .checked_mul(SEALED_INDEX_ENTRY_SIZE_V2)
                    .and_then(|n| n.checked_add(SEALED_INDEX_HEADER_SIZE_V2));
                p.index_well_formed = hdr.magic == SEALED_INDEX_MAGIC_V2 && want == Some(idx.len());
                if p.index_well_formed {
                    // Entries are 8-aligned inside the index only if the index is; copy to be safe.
                    let entries: Vec<s3a_core::SealedTileIndexEntry> = idx
                        [SEALED_INDEX_HEADER_SIZE_V2..]
                        .chunks_exact(SEALED_INDEX_ENTRY_SIZE_V2)
                        .map(bytemuck::pod_read_unaligned)
                        .collect();
                    p.seal_hash_ok = crate::compute_seal_hash_v2(
                        &hdr.prev_seal_hash,
                        hdr.seal_generation,
                        hdr.tile_count,
                        hdr.file_uuid,
                        &entries,
                    ) == hdr.seal_hash;
                }
            }
        }
    }
    p
}
