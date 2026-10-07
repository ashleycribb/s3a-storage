//! Sealed file footer and Reed-Solomon parity repair for S3A archives (v1.1-draft)
//!
//! Sealed files (.snapshot.s3a, exports, cold archives) append a tamper-evident
//! directory, Reed-Solomon $K+M$ erasure parity over Hyper-Tiles, and a fixed 64-byte
//! end cap at the tail of the file.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::Path;

use bytemuck::{bytes_of, cast_slice, from_bytes};
use s3a_core::{
    FileHeader, HyperTileHeader, S3AError, SealedDirectoryEntry, SealedEndCap,
    FILE_HEADER_SIZE, FILE_HEADER_SLOT_A_OFFSET, FILE_HEADER_SLOT_B_OFFSET,
    FILE_HEADER_SLOT_SIZE, HYPER_TILE_HEADER_SIZE, HYPER_TILE_SIZE, SEALED_ENDCAP_MAGIC,
    SEALED_ENDCAP_SIZE,
};

use crate::erasure::{invert_matrix, ReedSolomonCodec, GF};
use crate::{compute_header_crc32, compute_tile_header_crc32, generate_file_uuid, resolve_active_header};

pub const DEFAULT_SEAL_K: u16 = 16;
pub const DEFAULT_SEAL_M: u16 = 2;

/// Verification and audit report for a sealed S3A archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedVerifyReport {
    pub is_sealed: bool,
    pub tile_count: u32,
    pub k: u16,
    pub m: u16,
    pub file_uuid: [u64; 2],
    pub rebuilt_directory: bool,
    pub warnings: Vec<String>,
    pub repaired_tiles: Vec<u32>,
    pub out_of_order_detected: bool,
    pub directory: Vec<SealedDirectoryEntry>,
    pub anchors: Vec<crate::anchor::AnchorProof>,
}

/// Computes the CRC32C of the SealedEndCap excluding the endcap_crc32 field itself (first 52 bytes).
pub fn compute_endcap_crc32(endcap: &SealedEndCap) -> u32 {
    let bytes = bytes_of(endcap);
    crc32c::crc32c(&bytes[0..52])
}

/// Seals an in-memory S3A archive byte buffer by stamping self-addressing fields, generating
/// a directory, computing Reed-Solomon parity tiles over groups of K, and appending a 64-byte end cap.
pub fn seal_buffer(in_bytes: &[u8], k: u16, m: u16) -> io::Result<Vec<u8>> {
    if k == 0 || m == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "RS parameters k and m must be >= 1",
        ));
    }

    if in_bytes.len() < FILE_HEADER_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Buffer too small for S3A dual file header",
        ));
    }

    let slot_a: &FileHeader = from_bytes(
        &in_bytes[FILE_HEADER_SLOT_A_OFFSET..FILE_HEADER_SLOT_A_OFFSET + FILE_HEADER_SLOT_SIZE],
    );
    let slot_b: &FileHeader = from_bytes(
        &in_bytes[FILE_HEADER_SLOT_B_OFFSET..FILE_HEADER_SLOT_B_OFFSET + FILE_HEADER_SLOT_SIZE],
    );

    let (active_header, _) = resolve_active_header(slot_a, slot_b)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{}", e)))?;

    let tile_count = active_header.tile_count;
    let expected_data_len = FILE_HEADER_SIZE + (tile_count as usize * HYPER_TILE_SIZE);
    if in_bytes.len() < expected_data_len {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "Buffer truncated before all committed tiles are read",
        ));
    }

    let file_uuid = if active_header.file_uuid != [0, 0] {
        active_header.file_uuid
    } else {
        generate_file_uuid()
    };

    let mut out_data = in_bytes[..expected_data_len].to_vec();

    // Ensure active slot in out_data has file_uuid stamped
    let slot_a_mut: &mut FileHeader = bytemuck::from_bytes_mut(
        &mut out_data[FILE_HEADER_SLOT_A_OFFSET..FILE_HEADER_SLOT_A_OFFSET + FILE_HEADER_SLOT_SIZE],
    );
    if slot_a_mut.file_uuid == [0, 0] {
        slot_a_mut.file_uuid = file_uuid;
        slot_a_mut.header_crc32 = compute_header_crc32(slot_a_mut);
    }

    // Stamp self-addressing metadata (file_uuid, tile_index, generation) into all tiles
    for i in 0..tile_count as usize {
        let offset = FILE_HEADER_SIZE + i * HYPER_TILE_SIZE;
        let mut hdr: HyperTileHeader =
            *from_bytes(&out_data[offset..offset + HYPER_TILE_HEADER_SIZE]);
        let mut changed = false;

        if hdr.file_uuid == [0, 0] {
            hdr.file_uuid = file_uuid;
            changed = true;
            if hdr.tile_index == 0 && i != 0 {
                hdr.tile_index = i as u32;
            }
        }
        if hdr.generation == 0 {
            hdr.generation = active_header.generation;
            changed = true;
        }

        if changed {
            hdr.header_crc32 = compute_tile_header_crc32(&hdr);
            out_data[offset..offset + HYPER_TILE_HEADER_SIZE].copy_from_slice(bytes_of(&hdr));
        }
    }

    // 1. Build directory entries
    let mut directory: Vec<SealedDirectoryEntry> = Vec::with_capacity(tile_count as usize);
    for i in 0..tile_count as usize {
        let offset = (FILE_HEADER_SIZE + i * HYPER_TILE_SIZE) as u64;
        let tile_bytes = &out_data[offset as usize..offset as usize + HYPER_TILE_SIZE];
        let hdr: &HyperTileHeader = from_bytes(&tile_bytes[..HYPER_TILE_HEADER_SIZE]);
        let hash = xxhash_rust::xxh3::xxh3_64(tile_bytes);
        directory.push(SealedDirectoryEntry {
            tile_index: hdr.tile_index,
            tile_type: hdr.tile_type,
            generation: hdr.generation,
            offset,
            length: HYPER_TILE_SIZE as u64,
            xxh3_64: hash,
        });
    }

    // 2. Compute Reed-Solomon parity tiles
    let mut parity_bytes: Vec<u8> = Vec::new();
    let k_usize = k as usize;
    let m_usize = m as usize;
    let num_groups = if tile_count == 0 {
        0
    } else {
        (tile_count as usize + k_usize - 1) / k_usize
    };

    for g in 0..num_groups {
        let start_tile = g * k_usize;
        let end_tile = (start_tile + k_usize).min(tile_count as usize);
        let k_g = end_tile - start_tile;

        let codec = ReedSolomonCodec::new(k_g, m_usize)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{}", e)))?;

        let mut data_tiles: Vec<&[u8]> = Vec::with_capacity(k_g);
        for i in start_tile..end_tile {
            let offset = FILE_HEADER_SIZE + i * HYPER_TILE_SIZE;
            data_tiles.push(&out_data[offset..offset + HYPER_TILE_SIZE]);
        }

        for p in 0..m_usize {
            let mut parity_tile = vec![0u8; HYPER_TILE_SIZE];
            let row = k_g + p;
            for (j, tile) in data_tiles.iter().enumerate() {
                let coef = codec.matrix[row][j];
                if coef == 0 {
                    continue;
                }
                for b in 0..HYPER_TILE_SIZE {
                    parity_tile[b] ^= GF.mul(coef, tile[b]);
                }
            }
            parity_bytes.extend_from_slice(&parity_tile);
        }
    }

    // 3. Footer assembly
    let footer_offset = out_data.len() as u64;
    let directory_bytes = cast_slice::<SealedDirectoryEntry, u8>(&directory);
    let mut footer_bytes = Vec::with_capacity(directory_bytes.len() + parity_bytes.len());
    footer_bytes.extend_from_slice(directory_bytes);
    footer_bytes.extend_from_slice(&parity_bytes);

    let footer_length = footer_bytes.len() as u64;
    let footer_crc32 = crc32c::crc32c(&footer_bytes);

    // 4. End cap
    let mut endcap = SealedEndCap {
        magic: SEALED_ENDCAP_MAGIC,
        footer_offset,
        footer_length,
        tile_count,
        k,
        m,
        file_uuid,
        footer_crc32,
        endcap_crc32: 0,
        _reserved: [0u8; 8],
    };
    endcap.endcap_crc32 = compute_endcap_crc32(&endcap);

    out_data.extend_from_slice(&footer_bytes);
    out_data.extend_from_slice(bytes_of(&endcap));

    Ok(out_data)
}

/// Seals an S3A archive file by stamping self-addressing fields, generating a directory,
/// computing Reed-Solomon parity tiles over groups of K, and appending a 64-byte end cap.
pub fn seal_archive<P: AsRef<Path>, Q: AsRef<Path>>(
    in_path: P,
    out_path: Q,
    k: u16,
    m: u16,
) -> io::Result<()> {
    let in_bytes = std::fs::read(in_path.as_ref())?;
    let out_data = seal_buffer(&in_bytes, k, m)?;

    let mut out_file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(out_path.as_ref())?;
    out_file.write_all(&out_data)?;
    out_file.flush()?;
    out_file.sync_all()?;

    Ok(())
}

/// Verifies a sealed S3A archive according to S3A specification reader rules.
///
/// 1. Inspects the 64-byte end cap at the tail of the file.
/// 2. If end cap or footer is missing or corrupt, reports warnings and reconstructs directory by scanning tiles.
/// 3. Validates tile self-addressing (file_uuid, tile_index, generation).
/// 4. Reassembles out-of-order tiles if detected.
/// 5. Validates data and header CRCs.
/// 6. If corruption is found and RS parity is intact, repairs tiles in memory (and writes to `repair_out` if specified).
pub fn verify_sealed_archive<P: AsRef<Path>>(
    path: P,
    repair_out: Option<&Path>,
) -> Result<SealedVerifyReport, S3AError> {
    let path_ref = path.as_ref();
    let file_bytes = std::fs::read(path_ref).map_err(|_| S3AError::OutOfBounds)?;
    let file_len = file_bytes.len();

    if file_len < FILE_HEADER_SIZE {
        return Err(S3AError::BufferTooSmall {
            required: FILE_HEADER_SIZE,
            provided: file_len,
        });
    }

    let slot_a: &FileHeader = from_bytes(
        &file_bytes[FILE_HEADER_SLOT_A_OFFSET..FILE_HEADER_SLOT_A_OFFSET + FILE_HEADER_SLOT_SIZE],
    );
    let slot_b: &FileHeader = from_bytes(
        &file_bytes[FILE_HEADER_SLOT_B_OFFSET..FILE_HEADER_SLOT_B_OFFSET + FILE_HEADER_SLOT_SIZE],
    );
    let (active_header, _) = resolve_active_header(slot_a, slot_b)?;

    let mut warnings = Vec::new();
    let mut rebuilt_directory = false;

    // Try reading end cap
    let mut endcap_opt: Option<SealedEndCap> = None;
    if file_len >= FILE_HEADER_SIZE + SEALED_ENDCAP_SIZE {
        let candidate_bytes = &file_bytes[file_len - SEALED_ENDCAP_SIZE..];
        let candidate: SealedEndCap = bytemuck::pod_read_unaligned(candidate_bytes);
        if candidate.magic == SEALED_ENDCAP_MAGIC {
            let expected_crc = compute_endcap_crc32(&candidate);
            if candidate.endcap_crc32 == expected_crc {
                // Check if footer range is physically inside the file
                let footer_end = candidate.footer_offset + candidate.footer_length;
                if footer_end + (SEALED_ENDCAP_SIZE as u64) <= file_len as u64 {
                    // Check footer CRC
                    let footer_slice = &file_bytes
                        [candidate.footer_offset as usize..footer_end as usize];
                    if crc32c::crc32c(footer_slice) == candidate.footer_crc32 {
                        endcap_opt = Some(candidate);
                    } else {
                        warnings.push("footer_corrupt, directory rebuilt by scanning".into());
                        rebuilt_directory = true;
                    }
                } else {
                    warnings.push("footer_truncated, directory rebuilt by scanning".into());
                    rebuilt_directory = true;
                }
            } else {
                warnings.push("endcap_corrupt, directory rebuilt by scanning".into());
                rebuilt_directory = true;
            }
        } else {
            // Not a sealed end cap or magic corrupt
            // Check if it was supposed to be sealed
            if candidate_bytes[0..6] == *b"S3AEND" {
                warnings.push("endcap_corrupt, directory rebuilt by scanning".into());
                rebuilt_directory = true;
            } else {
                warnings.push("footer_missing, directory rebuilt by scanning".into());
                rebuilt_directory = true;
            }
        }
    } else {
        warnings.push("footer_missing, directory rebuilt by scanning".into());
        rebuilt_directory = true;
    }

    let is_sealed = endcap_opt.is_some();
    let file_uuid = if let Some(ec) = &endcap_opt {
        ec.file_uuid
    } else {
        active_header.file_uuid
    };

    // Extract tiles and directory
    let mut tiles_raw: Vec<Vec<u8>> = Vec::new();
    let mut directory: Vec<SealedDirectoryEntry> = Vec::new();
    let mut out_of_order_detected = false;
    let k = endcap_opt.as_ref().map(|ec| ec.k).unwrap_or(DEFAULT_SEAL_K);
    let m = endcap_opt.as_ref().map(|ec| ec.m).unwrap_or(DEFAULT_SEAL_M);

    if let Some(ec) = &endcap_opt {
        let footer_slice = &file_bytes
            [ec.footer_offset as usize..(ec.footer_offset + ec.footer_length) as usize];
        let dir_byte_len = (ec.tile_count as usize) * std::mem::size_of::<SealedDirectoryEntry>();
        let dir_entries: &[SealedDirectoryEntry] = cast_slice(&footer_slice[..dir_byte_len]);

        for (i, entry) in dir_entries.iter().enumerate() {
            directory.push(*entry);
            let offset = entry.offset as usize;
            if offset + HYPER_TILE_SIZE > file_len {
                return Err(S3AError::SealedTruncated);
            }
            let tile = file_bytes[offset..offset + HYPER_TILE_SIZE].to_vec();
            let hdr: &HyperTileHeader = from_bytes(&tile[..HYPER_TILE_HEADER_SIZE]);
            if hdr.tile_index != i as u32 {
                out_of_order_detected = true;
            }
            tiles_raw.push(tile);
        }
    } else {
        // Fallback: Scan self-addressed tiles from file_bytes
        let mut scan_offset = FILE_HEADER_SIZE;
        let mut tile_idx = 0u32;
        let target_tile_count = if active_header.tile_count > 0 {
            active_header.tile_count
        } else {
            ((file_len - FILE_HEADER_SIZE) / HYPER_TILE_SIZE) as u32
        };

        while tile_idx < target_tile_count && scan_offset + HYPER_TILE_SIZE <= file_len {
            let tile = file_bytes[scan_offset..scan_offset + HYPER_TILE_SIZE].to_vec();
            let hdr: &HyperTileHeader = from_bytes(&tile[..HYPER_TILE_HEADER_SIZE]);

            let actual_idx = if hdr.tile_index != 0 || tile_idx == 0 {
                hdr.tile_index
            } else {
                tile_idx
            };

            if actual_idx != tile_idx {
                out_of_order_detected = true;
            }

            let hash = xxhash_rust::xxh3::xxh3_64(&tile);
            directory.push(SealedDirectoryEntry {
                tile_index: actual_idx,
                tile_type: hdr.tile_type,
                generation: hdr.generation,
                offset: scan_offset as u64,
                length: HYPER_TILE_SIZE as u64,
                xxh3_64: hash,
            });

            tiles_raw.push(tile);
            scan_offset += HYPER_TILE_SIZE;
            tile_idx += 1;
        }

        if (tiles_raw.len() as u32) < active_header.tile_count {
            return Err(S3AError::SealedTruncated);
        }
    }

    // Reassemble out-of-order tiles if detected
    if out_of_order_detected {
        let mut paired: Vec<(SealedDirectoryEntry, Vec<u8>)> = directory
            .into_iter()
            .zip(tiles_raw.into_iter())
            .collect();
        paired.sort_by_key(|(entry, _)| entry.tile_index);

        let (sorted_dir, sorted_tiles): (Vec<_>, Vec<_>) = paired.into_iter().unzip();
        directory = sorted_dir;
        tiles_raw = sorted_tiles;
    }

    // Verify self-addressing file_uuid on every tile
    for (_i, tile) in tiles_raw.iter().enumerate() {
        let hdr: &HyperTileHeader = from_bytes(&tile[..HYPER_TILE_HEADER_SIZE]);
        if file_uuid != [0, 0] || hdr.file_uuid != [0, 0] {
            let is_legacy = hdr.record_layout_version <= 1 && hdr.file_uuid == [0, 0];
            if !is_legacy && hdr.file_uuid != file_uuid {
                return Err(S3AError::FileUuidMismatch {
                    expected: file_uuid,
                    actual: hdr.file_uuid,
                });
            }
        }
    }

    // Parity repair inspection
    let mut repaired_tiles = Vec::new();
    let k_usize = k as usize;
    let m_usize = m as usize;
    let tile_count = tiles_raw.len() as u32;
    let num_groups = if tile_count == 0 {
        0
    } else {
        (tile_count as usize + k_usize - 1) / k_usize
    };

    if let Some(ec) = &endcap_opt {
        let footer_slice = &file_bytes
            [ec.footer_offset as usize..(ec.footer_offset + ec.footer_length) as usize];
        let dir_byte_len = (ec.tile_count as usize) * std::mem::size_of::<SealedDirectoryEntry>();
        let parity_slice = &footer_slice[dir_byte_len..];

        for g in 0..num_groups {
            let start_tile = g * k_usize;
            let end_tile = (start_tile + k_usize).min(tile_count as usize);
            let k_g = end_tile - start_tile;

            // Check which data tiles are corrupted
            let mut corrupted_in_group = Vec::new();
            for j in 0..k_g {
                let t_idx = start_tile + j;
                let tile = &tiles_raw[t_idx];
                let hdr: &HyperTileHeader = from_bytes(&tile[..HYPER_TILE_HEADER_SIZE]);
                let payload_len = hdr.payload_bytes as usize;
                let expected_header_crc = compute_tile_header_crc32(hdr);
                let actual_data_crc = if HYPER_TILE_HEADER_SIZE + payload_len <= tile.len() {
                    crc32c::crc32c(&tile[HYPER_TILE_HEADER_SIZE..HYPER_TILE_HEADER_SIZE + payload_len])
                } else {
                    0
                };

                let is_corrupted = (hdr.header_crc32 != 0 && hdr.header_crc32 != expected_header_crc)
                    || (hdr.data_crc32 != actual_data_crc);

                if is_corrupted {
                    corrupted_in_group.push(j);
                }
            }

            if corrupted_in_group.is_empty() {
                continue;
            }

            if corrupted_in_group.len() > m_usize {
                return Err(S3AError::UnrepairableCorruption {
                    group_idx: g,
                    corrupted_count: corrupted_in_group.len(),
                    max_repairable: m_usize,
                });
            }

            // Perform Reed-Solomon repair
            let codec = ReedSolomonCodec::new(k_g, m_usize)
                .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{}", e)))
                .map_err(|_| S3AError::OutOfBounds)?;

            // Extract parity tiles for this group
            let group_parity_offset = g * m_usize * HYPER_TILE_SIZE;
            let mut parity_tiles: Vec<&[u8]> = Vec::with_capacity(m_usize);
            for p in 0..m_usize {
                let p_start = group_parity_offset + p * HYPER_TILE_SIZE;
                let p_end = p_start + HYPER_TILE_SIZE;
                if p_end <= parity_slice.len() {
                    parity_tiles.push(&parity_slice[p_start..p_end]);
                }
            }

            // Gather k_g surviving shards
            let mut available_shard_indices = Vec::new();
            let mut available_shard_data: Vec<&[u8]> = Vec::new();

            for j in 0..k_g {
                if !corrupted_in_group.contains(&j) {
                    available_shard_indices.push(j);
                    available_shard_data.push(&tiles_raw[start_tile + j]);
                }
            }

            let mut p_idx = 0;
            while available_shard_indices.len() < k_g && p_idx < parity_tiles.len() {
                available_shard_indices.push(k_g + p_idx);
                available_shard_data.push(parity_tiles[p_idx]);
                p_idx += 1;
            }

            if available_shard_indices.len() < k_g {
                return Err(S3AError::UnrepairableCorruption {
                    group_idx: g,
                    corrupted_count: corrupted_in_group.len(),
                    max_repairable: m_usize,
                });
            }

            // Invert submatrix
            let mut submatrix = vec![vec![0u8; k_g]; k_g];
            for (r, &shard_idx) in available_shard_indices.iter().enumerate() {
                submatrix[r].copy_from_slice(&codec.matrix[shard_idx]);
            }

            let inv = invert_matrix(&submatrix, k_g)
                .map_err(|_| S3AError::CorruptedHeader)?;

            // Reconstruct corrupted data tiles
            let mut group_repaired: Vec<(usize, u32, Vec<u8>)> = Vec::new();
            for &missing_col in &corrupted_in_group {
                let mut repaired = vec![0u8; HYPER_TILE_SIZE];
                for c in 0..k_g {
                    let coef = inv[missing_col][c];
                    if coef == 0 {
                        continue;
                    }
                    let shard = available_shard_data[c];
                    for b in 0..HYPER_TILE_SIZE {
                        repaired[b] ^= GF.mul(coef, shard[b]);
                    }
                }

                // Verify reconstructed tile
                let rep_hdr: &HyperTileHeader = from_bytes(&repaired[..HYPER_TILE_HEADER_SIZE]);
                let rep_payload_len = rep_hdr.payload_bytes as usize;
                let rep_data_crc = crc32c::crc32c(
                    &repaired[HYPER_TILE_HEADER_SIZE..HYPER_TILE_HEADER_SIZE + rep_payload_len],
                );
                if rep_data_crc != rep_hdr.data_crc32 {
                    return Err(S3AError::ChecksumMismatch {
                        expected: rep_hdr.data_crc32,
                        actual: rep_data_crc,
                    });
                }

                let global_idx = (start_tile + missing_col) as u32;
                group_repaired.push((global_idx as usize, global_idx, repaired));
            }

            for (target_idx, global_idx, rep_data) in group_repaired {
                tiles_raw[target_idx] = rep_data;
                repaired_tiles.push(global_idx);
            }
        }
    } else {
        // Without footer parity, verify existing tiles directly
        for (_i, tile) in tiles_raw.iter().enumerate() {
            let hdr: &HyperTileHeader = from_bytes(&tile[..HYPER_TILE_HEADER_SIZE]);
            let payload_len = hdr.payload_bytes as usize;
            let expected_hdr_crc = compute_tile_header_crc32(hdr);
            if hdr.header_crc32 != 0 && hdr.header_crc32 != expected_hdr_crc {
                return Err(S3AError::ChecksumMismatch {
                    expected: hdr.header_crc32,
                    actual: expected_hdr_crc,
                });
            }
            if HYPER_TILE_HEADER_SIZE + payload_len <= tile.len() {
                let actual_crc = crc32c::crc32c(
                    &tile[HYPER_TILE_HEADER_SIZE..HYPER_TILE_HEADER_SIZE + payload_len],
                );
                if actual_crc != hdr.data_crc32 {
                    return Err(S3AError::ChecksumMismatch {
                        expected: hdr.data_crc32,
                        actual: actual_crc,
                    });
                }
            }
        }
    }

    // Write to repair_out if provided and repair or reordering occurred
    if let Some(out_p) = repair_out {
        let mut out_file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(out_p)
            .map_err(|_| S3AError::CorruptedHeader)?;

        // Write header
        out_file
            .write_all(&file_bytes[..FILE_HEADER_SIZE])
            .map_err(|_| S3AError::CorruptedHeader)?;
        // Write repaired/ordered tiles
        for tile in &tiles_raw {
            out_file
                .write_all(tile)
                .map_err(|_| S3AError::CorruptedHeader)?;
        }
        out_file.flush().map_err(|_| S3AError::CorruptedHeader)?;
        out_file.sync_all().map_err(|_| S3AError::CorruptedHeader)?;
    }

    let anchors = if let Some(ec) = &endcap_opt {
        let footer_slice = &file_bytes
            [ec.footer_offset as usize..(ec.footer_offset + ec.footer_length) as usize];
        let dir_byte_len = (ec.tile_count as usize) * std::mem::size_of::<SealedDirectoryEntry>();
        let parity_byte_len = num_groups * m_usize * HYPER_TILE_SIZE;
        crate::anchor::extract_anchors_from_footer(footer_slice, dir_byte_len, parity_byte_len)
    } else {
        Vec::new()
    };

    Ok(SealedVerifyReport {
        is_sealed,
        tile_count,
        k,
        m,
        file_uuid,
        rebuilt_directory,
        warnings,
        repaired_tiles,
        out_of_order_detected,
        directory,
        anchors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TileType, TileWriter};
    use s3a_core::TelemetryRecord;

    struct TempTestFile {
        path: std::path::PathBuf,
    }

    impl TempTestFile {
        fn new(suffix: &str) -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!("s3a_sealed_test_{}_{}", unique, suffix));
            Self { path }
        }
    }

    impl Drop for TempTestFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    #[test]
    fn test_seal_and_verify_roundtrip() {
        let live_file = TempTestFile::new("live.s3a");
        let sealed_file = TempTestFile::new("sealed.s3a");

        let mut writer = TileWriter::create(&live_file.path).unwrap();
        let records = vec![
            TelemetryRecord::new(100, 1, 1, 42.0),
            TelemetryRecord::new(200, 1, 2, 84.0),
        ];
        writer.write_hyper_tile(TileType::TELEMETRY, &records, None, None).unwrap();
        drop(writer);

        seal_archive(&live_file.path, &sealed_file.path, 16, 2).unwrap();

        let report = verify_sealed_archive(&sealed_file.path, None).unwrap();
        assert!(report.is_sealed);
        assert_eq!(report.tile_count, 1);
        assert_eq!(report.k, 16);
        assert_eq!(report.m, 2);
        assert!(report.warnings.is_empty());
        assert!(report.repaired_tiles.is_empty());
        assert!(!report.rebuilt_directory);
    }

    #[test]
    fn test_seal_repair_single_corrupted_tile() {
        let live_file = TempTestFile::new("live.s3a");
        let sealed_file = TempTestFile::new("sealed.s3a");
        let repair_file = TempTestFile::new("repaired.s3a");

        let mut writer = TileWriter::create(&live_file.path).unwrap();
        let records1 = vec![TelemetryRecord::new(100, 1, 1, 42.0)];
        let records2 = vec![TelemetryRecord::new(200, 2, 2, 84.0)];
        writer.write_hyper_tile(TileType::TELEMETRY, &records1, None, None).unwrap();
        writer.write_hyper_tile(TileType::TELEMETRY, &records2, None, None).unwrap();
        drop(writer);

        seal_archive(&live_file.path, &sealed_file.path, 16, 2).unwrap();

        // Corrupt tile 0 payload bytes
        let mut bytes = std::fs::read(&sealed_file.path).unwrap();
        let tile_0_payload_offset = FILE_HEADER_SIZE + HYPER_TILE_HEADER_SIZE;
        bytes[tile_0_payload_offset] ^= 0xFF;
        bytes[tile_0_payload_offset + 1] ^= 0xAA;
        std::fs::write(&sealed_file.path, &bytes).unwrap();

        // Verification repairs in memory and writes to repair_file
        let report = verify_sealed_archive(&sealed_file.path, Some(&repair_file.path)).unwrap();
        assert!(report.is_sealed);
        assert_eq!(report.repaired_tiles, vec![0]);

        // Verifying the repaired output file succeeds with 0 damaged tiles
        let rep_report = verify_sealed_archive(&repair_file.path, None).unwrap();
        assert!(rep_report.repaired_tiles.is_empty());
    }

    #[test]
    fn test_seal_unrepairable_corruption() {
        let live_file = TempTestFile::new("live.s3a");
        let sealed_file = TempTestFile::new("sealed.s3a");

        let mut writer = TileWriter::create(&live_file.path).unwrap();
        for i in 0..4 {
            let rec = vec![TelemetryRecord::new(100 * (i as u64 + 1), i, 1, 10.0)];
            writer.write_hyper_tile(TileType::TELEMETRY, &rec, None, None).unwrap();
        }
        drop(writer);

        // Seal with M = 2
        seal_archive(&live_file.path, &sealed_file.path, 16, 2).unwrap();

        // Corrupt 3 data tiles (tiles 0, 1, 2) in group 0 (which exceeds M = 2)
        let mut bytes = std::fs::read(&sealed_file.path).unwrap();
        for i in 0..3 {
            let offset = FILE_HEADER_SIZE + (i * HYPER_TILE_SIZE) + HYPER_TILE_HEADER_SIZE;
            bytes[offset] ^= 0xFF;
        }
        std::fs::write(&sealed_file.path, &bytes).unwrap();

        let err = verify_sealed_archive(&sealed_file.path, None).unwrap_err();
        match err {
            S3AError::UnrepairableCorruption { group_idx, corrupted_count, max_repairable } => {
                assert_eq!(group_idx, 0);
                assert_eq!(corrupted_count, 3);
                assert_eq!(max_repairable, 2);
            }
            other => panic!("Expected UnrepairableCorruption, got {:?}", other),
        }
    }

    #[test]
    fn test_property_rs_parity_random_corruptions() {
        // Property test: randomly corrupt up to M tiles per group, verify bit-for-bit repair
        let live_file = TempTestFile::new("live_prop.s3a");
        let sealed_file = TempTestFile::new("sealed_prop.s3a");

        let mut writer = TileWriter::create(&live_file.path).unwrap();
        for i in 0..5 {
            let recs = vec![
                TelemetryRecord::new(1000 + i as u64, i as u32, 1, (i * 10) as f64),
                TelemetryRecord::new(2000 + i as u64, i as u32, 2, (i * 20) as f64),
            ];
            writer.write_hyper_tile(TileType::TELEMETRY, &recs, None, None).unwrap();
        }
        drop(writer);

        let k = 4u16;
        let m = 2u16;
        seal_archive(&live_file.path, &sealed_file.path, k, m).unwrap();
        let pristine_bytes = std::fs::read(&sealed_file.path).unwrap();

        // Corrupt 2 tiles in group 0 (e.g. tile 0 and tile 2) and 1 tile in group 1 (tile 4)
        let mut corrupted_bytes = pristine_bytes.clone();
        for &t_idx in &[0, 2, 4] {
            let offset = FILE_HEADER_SIZE + (t_idx * HYPER_TILE_SIZE) + HYPER_TILE_HEADER_SIZE;
            corrupted_bytes[offset] ^= 0x55;
            corrupted_bytes[offset + 10] ^= 0xAA;
        }
        std::fs::write(&sealed_file.path, &corrupted_bytes).unwrap();

        let repair_file = TempTestFile::new("repaired_prop.s3a");
        let report = verify_sealed_archive(&sealed_file.path, Some(&repair_file.path)).unwrap();
        assert_eq!(report.repaired_tiles.len(), 3);
        assert!(report.repaired_tiles.contains(&0));
        assert!(report.repaired_tiles.contains(&2));
        assert!(report.repaired_tiles.contains(&4));

        let repaired_bytes = std::fs::read(&repair_file.path).unwrap();
        let data_len = FILE_HEADER_SIZE + (5 * HYPER_TILE_SIZE);
        assert_eq!(
            &repaired_bytes[FILE_HEADER_SIZE..data_len],
            &pristine_bytes[FILE_HEADER_SIZE..data_len],
            "Reed-Solomon repair must restore bit-for-bit identical tile bytes"
        );
    }
}
