use std::fs::OpenOptions;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use bytemuck::{bytes_of, cast_slice, from_bytes};
use reed_solomon_erasure::galois_8::ReedSolomon;
use sha2::{Digest, Sha256};

use s3a_core::{
    ConsentRecord, FileHeader, HyperTileHeader, MarkRecord, S3AError, SealedDirectoryEntry, SealedEndCap,
    SealedEndCapV2, SealedIndexHeaderV2, SealedTileIndexEntry, TileType,
    FILE_HEADER_SIZE, FILE_HEADER_SLOT_A_OFFSET, FILE_HEADER_SLOT_B_OFFSET,
    FILE_HEADER_SLOT_SIZE, HYPER_TILE_HEADER_SIZE, HYPER_TILE_SIZE,
    MARK_CONSENT_WITHDRAWN,
    SEALED_ANCHORS_MAGIC, SEALED_ENDCAP_MAGIC_V1,
    SEALED_ENDCAP_MAGIC_V2, SEALED_ENDCAP_SIZE, SEALED_FLAG_PARITY_ENABLED,
    SEALED_INDEX_ENTRY_SIZE_V2, SEALED_INDEX_HEADER_SIZE_V2, SEALED_INDEX_MAGIC_V2,
};

use crate::erasure::{invert_matrix, ReedSolomonCodec, GF};
use crate::{compute_header_crc32, compute_tile_header_crc32, generate_file_uuid, resolve_active_header};

/// Canonical Markdown specification embedded in sealed archives on request (`--embed-spec`).
pub const SPEC_MARKDOWN: &str = include_str!("../../../docs/spec/S3A-TILE-SPEC.md");

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
    pub footer_version: u16,
    pub seal_hash: [u8; 32],
    pub prev_seal_hash: [u8; 32],
    pub seal_generation: u64,
    pub parity_enabled: bool,
    pub v2_entries: Vec<SealedTileIndexEntry>,
}

/// Configuration options for sealing an archive in v2 format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealOptionsV2 {
    pub enable_parity: bool,
    pub k: u16,
    pub m: u16,
    pub prev_seal_hash: [u8; 32],
    pub seal_generation: u64,
    pub write_manifest: bool,
    pub embed_spec: bool,
    pub title: Option<String>,
    pub creator: Option<String>,
}

impl Default for SealOptionsV2 {
    fn default() -> Self {
        Self {
            enable_parity: false,
            k: DEFAULT_SEAL_K,
            m: DEFAULT_SEAL_M,
            prev_seal_hash: [0u8; 32],
            seal_generation: 1,
            write_manifest: false,
            embed_spec: false,
            title: None,
            creator: None,
        }
    }
}

impl SealOptionsV2 {
    pub fn with_manifest(mut self) -> Self {
        self.write_manifest = true;
        self
    }

    pub fn with_embed_spec(mut self) -> Self {
        self.embed_spec = true;
        self
    }
}

/// Resolves standard human-readable canonical name for an S3A tile type.
pub fn resolve_type_name(type_id: u32) -> &'static str {
    match type_id {
        0 => "telemetry",
        1 => "embedding_128",
        2 => "hybrid_index",
        3 => "wearable_micro",
        4 => "robotics_kinematic",
        5 => "gis_survey_mesh",
        6 => "data_availability",
        7 => "quantized_embedding_int8",
        8 => "quantized_embedding_4bit",
        9 => "reserved_boundary",
        10 => "human_lrs",
        11 => "ai_trace_log",
        12 => "academic_papers",
        13 => "research_graph",
        14 => "srs_workspace",
        15 => "srs_manifest",
        16 => "ontology_concept",
        17 => "ontology_axiom",
        18 => "agent_registry",
        19 => "marks",
        20 => "note_claims",
        21 => "statement_body",
        22 => "statement_index",
        23 => "document_store",
        24 => "attachment_blob",
        25 => "elr_person",
        26 => "elr_organization",
        27 => "elr_learning_resource",
        28 => "elr_learning_event",
        29 => "elr_competency_assertion",
        30 => "elr_credential_assertion",
        31 => "elr_goal",
        32 => "citation_fingerprint",
        64 => "record",
        65 => "columnar",
        66 => "array",
        67 => "vector",
        68 => "blob",
        69 => "index",
        70 => "seal",
        71 => "consent",
        72 => "provenance",
        73 => "erasure",
        74 => "graph",
        75 => "time_series",
        76 => "spatial",
        77 => "schema",
        78 => "manifest",
        79 => "specification",
        _ => "custom",
    }
}

/// Generates the plain-text UTF-8 manifest text for a sealed S3A archive.
///
/// Guaranteed Privacy Invariant:
/// Contains aggregate counts only. Never includes consent-restricted payload data,
/// personal identifiers, or secret subject UUIDs.
pub fn generate_plain_text_manifest(
    title: Option<&str>,
    creator: Option<&str>,
    file_uuid: [u64; 2],
    created_ts: u64,
    sealed_ts: u64,
    tile_summaries: &[(u32, u32, &str, u32, usize)],
    consent_summary: (usize, usize, usize),
    seal_generation: u64,
    prev_seal_hash: &[u8; 32],
    enable_parity: bool,
) -> String {
    let mut out = String::new();
    out.push_str("================================================================================\n");
    out.push_str("                           THIS IS AN S3A FILE\n");
    out.push_str("      S3A Hyper-Tile Storage Archive (Immutable Cryptographic Container)\n");
    out.push_str(" Specification: https://github.com/ashleycribb/s3a-storage/blob/main/docs/spec/S3A-TILE-SPEC.md\n");
    out.push_str("================================================================================\n\n");

    out.push_str("ARCHIVE METADATA:\n");
    out.push_str(&format!("  Title / Purpose:  {}\n", title.unwrap_or("S3A Storage Archive")));
    out.push_str(&format!("  Creator:          {}\n", creator.unwrap_or("None (Unassigned)")));
    out.push_str(&format!("  File UUID:        {:016x}{:016x}\n", file_uuid[0], file_uuid[1]));
    let created_str = if created_ts > 0 {
        humantime::format_rfc3339_seconds(std::time::UNIX_EPOCH + std::time::Duration::from_secs(created_ts)).to_string()
    } else {
        "Genesis (Unset)".to_string()
    };
    out.push_str(&format!("  Created:          {}\n", created_str));
    let sealed_str = humantime::format_rfc3339_seconds(std::time::UNIX_EPOCH + std::time::Duration::from_secs(sealed_ts)).to_string();
    out.push_str(&format!("  Sealed:           {}\n\n", sealed_str));

    out.push_str("TILE DIRECTORY:\n");
    out.push_str("  Floor  Type ID  Type Name             Records    Tile Size\n");
    out.push_str("  -------------------------------------------------------------\n");
    for &(floor, type_id, name, rec_count, sz) in tile_summaries {
        out.push_str(&format!(
            "  {:<6} {:<8} {:<21} {:<10} {} B\n",
            floor, type_id, name, rec_count, sz
        ));
    }
    out.push_str(&format!("  Total Floors: {}\n\n", tile_summaries.len()));

    out.push_str("CONSENT GOVERNANCE:\n");
    out.push_str(&format!("  Total Consents:   {}\n", consent_summary.0));
    out.push_str(&format!("  Active Research:  {}\n", consent_summary.1));
    out.push_str(&format!("  Withdrawn:        {}\n", consent_summary.2));
    let status_str = if consent_summary.0 == 0 {
        "No participant consent records in archive"
    } else if consent_summary.2 > 0 {
        "Active with recorded withdrawals"
    } else {
        "All participant consents active"
    };
    out.push_str(&format!("  Status:           {}\n", status_str));
    out.push_str("  Policy Notice:    Protected under S3A consent governance (aggregate counts only; zero payload leaks).\n\n");

    out.push_str("SEAL INTEGRITY:\n");
    out.push_str("  Seal Status:      SEALED (v2 cryptographic hash chain active)\n");
    out.push_str(&format!("  Seal Generation:  {}\n", seal_generation));
    out.push_str(&format!("  Prev Seal Hash:   {}\n", hex::encode(prev_seal_hash)));
    let parity_str = if enable_parity {
        "Enabled (Reed-Solomon 16+2 erasure parity)"
    } else {
        "Disabled"
    };
    out.push_str(&format!("  Parity:           {}\n\n", parity_str));

    out.push_str("HOW TO READ:\n");
    out.push_str("  how to read: s3a-cli tour <file> or the S3A reader app\n");
    out.push_str("  Terminal: Run `s3a-cli tour <file>` to inspect and query this archive.\n");
    out.push_str("  GUI:      Open in the S3A reader app or compatible viewer.\n");
    out.push_str("================================================================================\n");

    out
}

/// Computes the CRC32C of the SealedEndCap (v1) excluding the endcap_crc32 field itself (first 52 bytes).
pub fn compute_endcap_crc32(endcap: &SealedEndCap) -> u32 {
    let bytes = bytes_of(endcap);
    crc32c::crc32c(&bytes[0..52])
}

/// Computes the CRC32C of the SealedEndCapV2 excluding the endcap_crc32 field itself (first 60 bytes).
pub fn compute_endcap_crc32_v2(endcap: &SealedEndCapV2) -> u32 {
    let bytes = bytes_of(endcap);
    crc32c::crc32c(&bytes[0..60])
}

/// Computes the authoritative 32-byte SHA-256 seal hash for a v2 sealed archive.
///
/// Commits to:
/// - prev_seal_hash (32 bytes)
/// - seal_generation (8 bytes, little-endian)
/// - tile_count (4 bytes, little-endian)
/// - file_uuid (16 bytes, little-endian)
/// - The byte sequence of all `SealedTileIndexEntry` records in the directory
pub fn compute_seal_hash_v2(
    prev_seal_hash: &[u8; 32],
    seal_generation: u64,
    tile_count: u32,
    file_uuid: [u64; 2],
    entries: &[SealedTileIndexEntry],
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"S3ASEAL2");
    hasher.update(prev_seal_hash);
    hasher.update(&seal_generation.to_le_bytes());
    hasher.update(&tile_count.to_le_bytes());
    hasher.update(&file_uuid[0].to_le_bytes());
    hasher.update(&file_uuid[1].to_le_bytes());
    let entries_bytes = cast_slice::<SealedTileIndexEntry, u8>(entries);
    hasher.update(entries_bytes);
    hasher.finalize().into()
}

/// Walks a sequence of v2 seal index headers and verifies the hash chain.
///
/// Fails if:
/// - Generation numbers are not strictly increasing by 1 (`SealedSwappedGeneration`).
/// - Any `prev_seal_hash` does not match the predecessor's `seal_hash` (`SealedChainBreak`).
/// - The root seal (generation 1) has a non-zero `prev_seal_hash` (`SealedChainBreak`).
pub fn verify_seal_chain(chain: &[SealedIndexHeaderV2]) -> Result<(), S3AError> {
    if chain.is_empty() {
        return Ok(());
    }
    if chain[0].seal_generation == 1 && chain[0].prev_seal_hash != [0u8; 32] {
        return Err(S3AError::SealedChainBreak);
    }
    for i in 1..chain.len() {
        if chain[i].seal_generation != chain[i - 1].seal_generation + 1 {
            return Err(S3AError::SealedSwappedGeneration);
        }
        if chain[i].prev_seal_hash != chain[i - 1].seal_hash {
            return Err(S3AError::SealedChainBreak);
        }
    }
    Ok(())
}

/// Reads and verifies the tile directory of a v2 sealed S3A archive by reading ONLY the tail bytes.
///
/// This performs an index lookup without reading the file body, supporting efficient HTTP range reads.
pub fn read_sealed_index_v2<R: Read + Seek>(
    reader: &mut R,
    file_len: u64,
) -> Result<(SealedEndCapV2, SealedIndexHeaderV2, Vec<SealedTileIndexEntry>), S3AError> {
    if file_len < (FILE_HEADER_SIZE + SEALED_ENDCAP_SIZE) as u64 {
        return Err(S3AError::BufferTooSmall {
            required: FILE_HEADER_SIZE + SEALED_ENDCAP_SIZE,
            provided: file_len as usize,
        });
    }

    // 1. Read the 64-byte end cap at the tail of the file
    reader
        .seek(SeekFrom::Start(file_len - (SEALED_ENDCAP_SIZE as u64)))
        .map_err(|_| S3AError::OutOfBounds)?;

    let mut endcap_buf = [0u8; SEALED_ENDCAP_SIZE];
    reader
        .read_exact(&mut endcap_buf)
        .map_err(|_| S3AError::SealedTruncated)?;

    let endcap: SealedEndCapV2 = bytemuck::pod_read_unaligned(&endcap_buf);
    if endcap.magic != SEALED_ENDCAP_MAGIC_V2 {
        return Err(S3AError::SealedEndCapCorrupt);
    }

    let expected_crc = compute_endcap_crc32_v2(&endcap);
    if endcap.endcap_crc32 != expected_crc {
        return Err(S3AError::SealedEndCapCorrupt);
    }

    if endcap.index_offset + endcap.index_len + (SEALED_ENDCAP_SIZE as u64) > file_len {
        return Err(S3AError::SealedTruncated);
    }

    if endcap.index_len < SEALED_INDEX_HEADER_SIZE_V2 as u64 {
        return Err(S3AError::CorruptedHeader);
    }

    // 2. Read ONLY the index directory range
    reader
        .seek(SeekFrom::Start(endcap.index_offset))
        .map_err(|_| S3AError::OutOfBounds)?;

    let mut index_bytes = vec![0u8; endcap.index_len as usize];
    reader
        .read_exact(&mut index_bytes)
        .map_err(|_| S3AError::SealedTruncated)?;

    // 3. Verify index_hash
    let actual_hash: [u8; 32] = Sha256::digest(&index_bytes).into();
    if actual_hash != endcap.index_hash {
        return Err(S3AError::SealedIndexHashMismatch);
    }

    // 4. Parse index header
    let header: SealedIndexHeaderV2 =
        bytemuck::pod_read_unaligned(&index_bytes[..SEALED_INDEX_HEADER_SIZE_V2]);
    if header.magic != SEALED_INDEX_MAGIC_V2 {
        return Err(S3AError::CorruptedHeader);
    }

    let expected_entries_len = (header.tile_count as usize) * SEALED_INDEX_ENTRY_SIZE_V2;
    if index_bytes.len() != SEALED_INDEX_HEADER_SIZE_V2 + expected_entries_len {
        return Err(S3AError::CorruptedHeader);
    }

    let entries_slice = &index_bytes[SEALED_INDEX_HEADER_SIZE_V2..];
    let entries_vec: &[SealedTileIndexEntry] = cast_slice(entries_slice);
    let entries = entries_vec.to_vec();

    // 5. Verify seal hash internally
    let expected_seal_hash = compute_seal_hash_v2(
        &header.prev_seal_hash,
        header.seal_generation,
        header.tile_count,
        header.file_uuid,
        &entries,
    );
    if expected_seal_hash != header.seal_hash {
        return Err(S3AError::ChecksumMismatch {
            expected: crc32c::crc32c(&expected_seal_hash),
            actual: crc32c::crc32c(&header.seal_hash),
        });
    }

    Ok((endcap, header, entries))
}

/// Seals an in-memory S3A archive byte buffer according to v1 format.
pub fn seal_buffer_v1(in_bytes: &[u8], k: u16, m: u16) -> io::Result<Vec<u8>> {
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
        magic: SEALED_ENDCAP_MAGIC_V1,
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

/// Seals an S3A archive file according to v1 format.
pub fn seal_archive_v1<P: AsRef<Path>, Q: AsRef<Path>>(
    in_path: P,
    out_path: Q,
    k: u16,
    m: u16,
) -> io::Result<()> {
    let in_bytes = std::fs::read(in_path.as_ref())?;
    let out_data = seal_buffer_v1(&in_bytes, k, m)?;

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

/// Seals an in-memory S3A archive byte buffer according to v2 format.
pub fn seal_buffer_v2(in_bytes: &[u8], options: &SealOptionsV2) -> io::Result<Vec<u8>> {
    let k = options.k;
    let m = options.m;
    if options.enable_parity && (k == 0 || m == 0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "RS parameters k and m must be >= 1 when parity is enabled",
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

    let mut tile_count = active_header.tile_count;
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

    if options.write_manifest {
        // 1. Gather consent counts and existing tile summaries
        let mut created_ts = u64::MAX;

        let consent_size = std::mem::size_of::<ConsentRecord>();
        let mark_size = std::mem::size_of::<MarkRecord>();

        let mut existing_data_tiles: Vec<Vec<u8>> = Vec::new();

        let mut subject_map: std::collections::HashMap<[u8; 16], bool> = std::collections::HashMap::new();
        let mut mark_withdrawn_count = 0usize;

        for i in 0..tile_count as usize {
            let offset = FILE_HEADER_SIZE + i * HYPER_TILE_SIZE;
            let tile_slice = &out_data[offset..offset + HYPER_TILE_SIZE];
            let hdr: &HyperTileHeader = from_bytes(&tile_slice[..HYPER_TILE_HEADER_SIZE]);

            if hdr.min_timestamp > 0 && hdr.min_timestamp < created_ts {
                created_ts = hdr.min_timestamp;
            }

            // Exclude prior manifest or specification tiles if re-sealing
            if hdr.tile_type == TileType::MANIFEST || hdr.tile_type == TileType::SPECIFICATION {
                continue;
            }

            if hdr.tile_type == TileType::CONSENT {
                let rec_count = hdr.record_count as usize;
                let payload = &tile_slice[HYPER_TILE_HEADER_SIZE..];
                for c in 0..rec_count {
                    if (c + 1) * consent_size <= payload.len() {
                        let rec: &ConsentRecord = from_bytes(&payload[c * consent_size..(c + 1) * consent_size]);
                        let is_active = rec.withdrawn_at == 0;
                        subject_map.insert(rec.subject_id, is_active);
                    }
                }
            } else if hdr.tile_type == TileType::MARKS {
                let rec_count = hdr.record_count as usize;
                let payload = &tile_slice[HYPER_TILE_HEADER_SIZE..];
                for m in 0..rec_count {
                    if (m + 1) * mark_size <= payload.len() {
                        let rec: &MarkRecord = from_bytes(&payload[m * mark_size..(m + 1) * mark_size]);
                        if rec.mark_kind == MARK_CONSENT_WITHDRAWN {
                            mark_withdrawn_count += 1;
                        }
                    }
                }
            }

            existing_data_tiles.push(tile_slice.to_vec());
        }

        let mut total_consents = subject_map.len();
        let mut active_research = 0usize;
        let mut withdrawn_consents = mark_withdrawn_count;
        for &is_act in subject_map.values() {
            if is_act {
                active_research += 1;
            } else {
                withdrawn_consents += 1;
            }
        }
        if total_consents == 0 && withdrawn_consents > 0 {
            total_consents = withdrawn_consents;
        }

        if created_ts == u64::MAX {
            created_ts = 0;
        }

        let sealed_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Build list of tile summaries for manifest
        let mut tile_summaries = Vec::new();
        // Floor 0: Manifest
        tile_summaries.push((0u32, TileType::MANIFEST, "manifest", 0u32, HYPER_TILE_SIZE));

        let mut next_floor = 1u32;
        if options.embed_spec {
            tile_summaries.push((next_floor, TileType::SPECIFICATION, "specification", 0u32, HYPER_TILE_SIZE));
            next_floor += 1;
        }

        for (idx, tile_bytes) in existing_data_tiles.iter().enumerate() {
            let hdr: &HyperTileHeader = from_bytes(&tile_bytes[..HYPER_TILE_HEADER_SIZE]);
            let floor_idx = next_floor + idx as u32;
            let type_name = resolve_type_name(hdr.tile_type);
            tile_summaries.push((floor_idx, hdr.tile_type, type_name, hdr.record_count, HYPER_TILE_SIZE));
        }

        let manifest_text = generate_plain_text_manifest(
            options.title.as_deref(),
            options.creator.as_deref(),
            file_uuid,
            created_ts,
            sealed_ts,
            &tile_summaries,
            (total_consents, active_research, withdrawn_consents),
            options.seal_generation,
            &options.prev_seal_hash,
            options.enable_parity,
        );

        let manifest_bytes = manifest_text.as_bytes();
        let mut manifest_tile = vec![0u8; HYPER_TILE_SIZE];
        let mut manifest_hdr = HyperTileHeader::new(0, TileType::MANIFEST);
        manifest_hdr.record_count = 0;
        manifest_hdr.record_layout_version = 1;
        manifest_hdr.payload_bytes = manifest_bytes.len() as u32;
        manifest_hdr.data_crc32 = crc32c::crc32c(manifest_bytes);
        manifest_hdr.generation = active_header.generation;
        manifest_hdr.tile_index = 0;
        manifest_hdr.file_uuid = file_uuid;
        manifest_hdr.header_crc32 = compute_tile_header_crc32(&manifest_hdr);
        manifest_tile[..HYPER_TILE_HEADER_SIZE].copy_from_slice(bytes_of(&manifest_hdr));
        manifest_tile[HYPER_TILE_HEADER_SIZE..HYPER_TILE_HEADER_SIZE + manifest_bytes.len()]
            .copy_from_slice(manifest_bytes);

        let mut spec_tile_opt = None;
        if options.embed_spec {
            let compressed_spec = miniz_oxide::deflate::compress_to_vec_zlib(SPEC_MARKDOWN.as_bytes(), 6);
            let mut spec_tile = vec![0u8; HYPER_TILE_SIZE];
            let mut spec_hdr = HyperTileHeader::new(1, TileType::SPECIFICATION);
            spec_hdr.record_count = 0;
            spec_hdr.record_layout_version = 1;
            spec_hdr.payload_bytes = compressed_spec.len() as u32;
            spec_hdr.data_crc32 = crc32c::crc32c(&compressed_spec);
            spec_hdr.generation = active_header.generation;
            spec_hdr.tile_index = 1;
            spec_hdr.file_uuid = file_uuid;
            spec_hdr.header_crc32 = compute_tile_header_crc32(&spec_hdr);
            spec_tile[..HYPER_TILE_HEADER_SIZE].copy_from_slice(bytes_of(&spec_hdr));
            spec_tile[HYPER_TILE_HEADER_SIZE..HYPER_TILE_HEADER_SIZE + compressed_spec.len()]
                .copy_from_slice(&compressed_spec);
            spec_tile_opt = Some(spec_tile);
        }

        // Reassemble out_data: header + manifest + [spec] + existing_data_tiles
        let mut new_tiles = Vec::new();
        new_tiles.push(manifest_tile);
        if let Some(spec) = spec_tile_opt {
            new_tiles.push(spec);
        }
        new_tiles.extend(existing_data_tiles);

        tile_count = new_tiles.len() as u32;
        out_data.truncate(FILE_HEADER_SIZE);
        for t in &new_tiles {
            out_data.extend_from_slice(t);
        }

        // Update tile_count in header slot A and slot B in out_data
        let slot_a_mut: &mut FileHeader = bytemuck::from_bytes_mut(
            &mut out_data[FILE_HEADER_SLOT_A_OFFSET..FILE_HEADER_SLOT_A_OFFSET + FILE_HEADER_SLOT_SIZE],
        );
        slot_a_mut.tile_count = tile_count;
        slot_a_mut.file_uuid = file_uuid;
        slot_a_mut.header_crc32 = compute_header_crc32(slot_a_mut);

        let slot_b_mut: &mut FileHeader = bytemuck::from_bytes_mut(
            &mut out_data[FILE_HEADER_SLOT_B_OFFSET..FILE_HEADER_SLOT_B_OFFSET + FILE_HEADER_SLOT_SIZE],
        );
        if slot_b_mut.magic == s3a_core::S3A_MAGIC {
            slot_b_mut.tile_count = tile_count;
            slot_b_mut.file_uuid = file_uuid;
            slot_b_mut.header_crc32 = compute_header_crc32(slot_b_mut);
        }
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

    // 1. Build v2 directory entries
    let mut entries: Vec<SealedTileIndexEntry> = Vec::with_capacity(tile_count as usize);
    for i in 0..tile_count as usize {
        let offset = (FILE_HEADER_SIZE + i * HYPER_TILE_SIZE) as u64;
        let tile_bytes = &out_data[offset as usize..offset as usize + HYPER_TILE_SIZE];
        let hdr: &HyperTileHeader = from_bytes(&tile_bytes[..HYPER_TILE_HEADER_SIZE]);
        let payload_bytes = hdr.payload_bytes as usize;
        let payload = if HYPER_TILE_HEADER_SIZE + payload_bytes <= HYPER_TILE_SIZE {
            &tile_bytes[HYPER_TILE_HEADER_SIZE..HYPER_TILE_HEADER_SIZE + payload_bytes]
        } else {
            &tile_bytes[HYPER_TILE_HEADER_SIZE..]
        };
        let payload_hash: [u8; 32] = Sha256::digest(payload).into();

        entries.push(SealedTileIndexEntry {
            offset,
            length: HYPER_TILE_SIZE as u64,
            type_id: hdr.tile_type,
            type_version: hdr.record_layout_version as u16,
            _pad: [0u8; 2],
            record_count: hdr.record_count,
            tile_index: hdr.tile_index,
            min_key: hdr.min_timestamp,
            max_key: hdr.max_timestamp,
            payload_hash,
        });
    }

    // 2. Parity shards if enabled via Reed-Solomon (galois_8)
    let mut parity_bytes: Vec<u8> = Vec::new();
    let k_usize = k as usize;
    let m_usize = m as usize;
    if options.enable_parity && tile_count > 0 {
        let num_groups = (tile_count as usize + k_usize - 1) / k_usize;
        for g in 0..num_groups {
            let start_tile = g * k_usize;
            let end_tile = (start_tile + k_usize).min(tile_count as usize);
            let k_g = end_tile - start_tile;

            let rs = ReedSolomon::new(k_g, m_usize)
                .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("ReedSolomon error: {}", e)))?;

            let mut shards: Vec<Vec<u8>> = Vec::with_capacity(k_g + m_usize);
            for i in start_tile..end_tile {
                let offset = FILE_HEADER_SIZE + i * HYPER_TILE_SIZE;
                shards.push(out_data[offset..offset + HYPER_TILE_SIZE].to_vec());
            }
            for _ in 0..m_usize {
                shards.push(vec![0u8; HYPER_TILE_SIZE]);
            }

            rs.encode(&mut shards)
                .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("RS encode error: {}", e)))?;

            for p in 0..m_usize {
                parity_bytes.extend_from_slice(&shards[k_g + p]);
            }
        }
    }

    // 3. Assemble index directory
    let seal_hash = compute_seal_hash_v2(
        &options.prev_seal_hash,
        options.seal_generation,
        tile_count,
        file_uuid,
        &entries,
    );

    let sealed_timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let flags = if options.enable_parity {
        SEALED_FLAG_PARITY_ENABLED
    } else {
        0
    };

    let index_header = SealedIndexHeaderV2 {
        magic: SEALED_INDEX_MAGIC_V2,
        tile_count,
        flags,
        k,
        m,
        _padding: [0u8; 6],
        file_uuid,
        prev_seal_hash: options.prev_seal_hash,
        seal_generation: options.seal_generation,
        sealed_timestamp,
        seal_hash,
        _reserved: [0u8; 8],
    };

    let index_offset = (out_data.len() + parity_bytes.len()) as u64;
    let mut index_bytes = Vec::with_capacity(SEALED_INDEX_HEADER_SIZE_V2 + entries.len() * SEALED_INDEX_ENTRY_SIZE_V2);
    index_bytes.extend_from_slice(bytes_of(&index_header));
    index_bytes.extend_from_slice(cast_slice(&entries));

    let index_len = index_bytes.len() as u64;
    let index_hash: [u8; 32] = Sha256::digest(&index_bytes).into();

    // 4. Construct v2 End Cap
    let mut endcap = SealedEndCapV2 {
        magic: SEALED_ENDCAP_MAGIC_V2,
        index_offset,
        index_len,
        index_hash,
        footer_version: 2,
        flags,
        endcap_crc32: 0,
    };
    endcap.endcap_crc32 = compute_endcap_crc32_v2(&endcap);

    out_data.extend_from_slice(&parity_bytes);
    out_data.extend_from_slice(&index_bytes);
    out_data.extend_from_slice(bytes_of(&endcap));

    Ok(out_data)
}

/// Seals an S3A archive file according to v2 format.
pub fn seal_archive_v2<P: AsRef<Path>, Q: AsRef<Path>>(
    in_path: P,
    out_path: Q,
    options: &SealOptionsV2,
) -> io::Result<()> {
    let in_bytes = std::fs::read(in_path.as_ref())?;
    let out_data = seal_buffer_v2(&in_bytes, options)?;

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

/// Default seal buffer function (creates a v2 archive with parity enabled by default).
pub fn seal_buffer(in_bytes: &[u8], k: u16, m: u16) -> io::Result<Vec<u8>> {
    let options = SealOptionsV2 {
        enable_parity: true,
        k,
        m,
        prev_seal_hash: [0u8; 32],
        seal_generation: 1,
        write_manifest: false,
        embed_spec: false,
        title: None,
        creator: None,
    };
    seal_buffer_v2(in_bytes, &options)
}

/// Default seal archive function (creates a v2 archive with parity enabled by default).
pub fn seal_archive<P: AsRef<Path>, Q: AsRef<Path>>(
    in_path: P,
    out_path: Q,
    k: u16,
    m: u16,
) -> io::Result<()> {
    let options = SealOptionsV2 {
        enable_parity: true,
        k,
        m,
        prev_seal_hash: [0u8; 32],
        seal_generation: 1,
        write_manifest: false,
        embed_spec: false,
        title: None,
        creator: None,
    };
    seal_archive_v2(in_path, out_path, &options)
}

/// Seals an archive buffer writing a UTF-8 manifest tile placed at Tile 0.
pub fn seal_buffer_with_manifest(in_bytes: &[u8], k: u16, m: u16) -> io::Result<Vec<u8>> {
    let options = SealOptionsV2 {
        enable_parity: true,
        k,
        m,
        prev_seal_hash: [0u8; 32],
        seal_generation: 1,
        write_manifest: true,
        embed_spec: false,
        title: None,
        creator: None,
    };
    seal_buffer_v2(in_bytes, &options)
}

/// Seals an archive file writing a UTF-8 manifest tile placed at Tile 0.
pub fn seal_archive_with_manifest<P: AsRef<Path>, Q: AsRef<Path>>(
    in_path: P,
    out_path: Q,
    k: u16,
    m: u16,
    embed_spec: bool,
) -> io::Result<()> {
    let options = SealOptionsV2 {
        enable_parity: true,
        k,
        m,
        prev_seal_hash: [0u8; 32],
        seal_generation: 1,
        write_manifest: true,
        embed_spec,
        title: None,
        creator: None,
    };
    seal_archive_v2(in_path, out_path, &options)
}

/// Verifies a sealed S3A archive according to S3A specification reader rules.
///
/// Supports both v2 and v1 sealed formats with full backward compatibility:
/// - Detects v2 End Cap (`S3AEND\0\x02`), parses directory index, verifies index_hash & seal chain,
///   and reconstructs damaged tiles via Reed-Solomon erasure coding if parity is enabled.
/// - Falls back to v1 End Cap (`S3AEND\0\x01`) and directory structure for older archives.
/// - Rebuilds directory by scanning tiles if footer is missing or corrupted.
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

    // Detect version from candidate endcap at the tail
    if file_len >= FILE_HEADER_SIZE + SEALED_ENDCAP_SIZE {
        let candidate_bytes = &file_bytes[file_len - SEALED_ENDCAP_SIZE..];
        let magic = &candidate_bytes[0..8];

        if magic == &SEALED_ENDCAP_MAGIC_V2 {
            let endcap: SealedEndCapV2 = bytemuck::pod_read_unaligned(candidate_bytes);
            let expected_crc = compute_endcap_crc32_v2(&endcap);
            if endcap.endcap_crc32 != expected_crc {
                warnings.push("endcap_corrupt, directory rebuilt by scanning".into());
                rebuilt_directory = true;
            } else if endcap.index_offset + endcap.index_len + (SEALED_ENDCAP_SIZE as u64) > file_len as u64 {
                warnings.push("footer_truncated, directory rebuilt by scanning".into());
                rebuilt_directory = true;
            } else {
                // Read and verify index directory
                let index_slice = &file_bytes[endcap.index_offset as usize..(endcap.index_offset + endcap.index_len) as usize];
                let actual_index_hash: [u8; 32] = Sha256::digest(index_slice).into();
                if actual_index_hash != endcap.index_hash {
                    return Err(S3AError::SealedIndexHashMismatch);
                }

                if index_slice.len() < SEALED_INDEX_HEADER_SIZE_V2 {
                    return Err(S3AError::CorruptedHeader);
                }

                let index_hdr: SealedIndexHeaderV2 = bytemuck::pod_read_unaligned(&index_slice[..SEALED_INDEX_HEADER_SIZE_V2]);
                if index_hdr.magic != SEALED_INDEX_MAGIC_V2 {
                    return Err(S3AError::CorruptedHeader);
                }

                let expected_entries_len = (index_hdr.tile_count as usize) * SEALED_INDEX_ENTRY_SIZE_V2;
                if index_slice.len() != SEALED_INDEX_HEADER_SIZE_V2 + expected_entries_len {
                    return Err(S3AError::CorruptedHeader);
                }

                let entries_slice = &index_slice[SEALED_INDEX_HEADER_SIZE_V2..];
                let v2_entries_slice: &[SealedTileIndexEntry] = cast_slice(entries_slice);
                let v2_entries = v2_entries_slice.to_vec();

                // Verify seal hash and seal chain
                let expected_seal_hash = compute_seal_hash_v2(
                    &index_hdr.prev_seal_hash,
                    index_hdr.seal_generation,
                    index_hdr.tile_count,
                    index_hdr.file_uuid,
                    &v2_entries,
                );
                if expected_seal_hash != index_hdr.seal_hash {
                    return Err(S3AError::ChecksumMismatch {
                        expected: crc32c::crc32c(&expected_seal_hash),
                        actual: crc32c::crc32c(&index_hdr.seal_hash),
                    });
                }
                verify_seal_chain(&[index_hdr])?;

                let tile_count = index_hdr.tile_count;
                let mut tiles_raw = Vec::with_capacity(tile_count as usize);
                let mut directory = Vec::with_capacity(tile_count as usize);
                let mut out_of_order_detected = false;

                for (i, entry) in v2_entries.iter().enumerate() {
                    let offset = entry.offset as usize;
                    if offset + HYPER_TILE_SIZE > file_len {
                        return Err(S3AError::SealedTruncated);
                    }
                    let tile = file_bytes[offset..offset + HYPER_TILE_SIZE].to_vec();
                    let hdr: &HyperTileHeader = from_bytes(&tile[..HYPER_TILE_HEADER_SIZE]);
                    if hdr.tile_index != i as u32 {
                        out_of_order_detected = true;
                    }
                    directory.push(SealedDirectoryEntry {
                        tile_index: entry.tile_index,
                        tile_type: entry.type_id,
                        generation: index_hdr.seal_generation,
                        offset: entry.offset,
                        length: entry.length,
                        xxh3_64: 0,
                    });
                    tiles_raw.push(tile);
                }

                // Verify file_uuid
                for tile in &tiles_raw {
                    let hdr: &HyperTileHeader = from_bytes(&tile[..HYPER_TILE_HEADER_SIZE]);
                    if index_hdr.file_uuid != [0, 0] || hdr.file_uuid != [0, 0] {
                        let is_legacy = hdr.record_layout_version <= 1 && hdr.file_uuid == [0, 0];
                        if !is_legacy && hdr.file_uuid != index_hdr.file_uuid {
                            return Err(S3AError::FileUuidMismatch {
                                expected: index_hdr.file_uuid,
                                actual: hdr.file_uuid,
                            });
                        }
                    }
                }

                // Corruption and parity repair inspection
                let mut repaired_tiles = Vec::new();
                let parity_enabled = (endcap.flags & SEALED_FLAG_PARITY_ENABLED) != 0;
                let k_usize = index_hdr.k as usize;
                let m_usize = index_hdr.m as usize;
                let num_groups = if tile_count == 0 || k_usize == 0 {
                    0
                } else {
                    (tile_count as usize + k_usize - 1) / k_usize
                };

                let data_end_offset = FILE_HEADER_SIZE + (tile_count as usize * HYPER_TILE_SIZE);
                let parity_end_offset = if parity_enabled {
                    data_end_offset + (num_groups * m_usize * HYPER_TILE_SIZE)
                } else {
                    data_end_offset
                };

                for g in 0..num_groups {
                    let start_tile = g * k_usize;
                    let end_tile = (start_tile + k_usize).min(tile_count as usize);
                    let k_g = end_tile - start_tile;

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

                        let payload = if HYPER_TILE_HEADER_SIZE + payload_len <= tile.len() {
                            &tile[HYPER_TILE_HEADER_SIZE..HYPER_TILE_HEADER_SIZE + payload_len]
                        } else {
                            &tile[HYPER_TILE_HEADER_SIZE..]
                        };
                        let actual_payload_hash: [u8; 32] = Sha256::digest(payload).into();

                        let is_corrupted = (hdr.header_crc32 != 0 && hdr.header_crc32 != expected_header_crc)
                            || (hdr.data_crc32 != actual_data_crc)
                            || (actual_payload_hash != v2_entries[t_idx].payload_hash);

                        if is_corrupted {
                            corrupted_in_group.push(j);
                        }
                    }

                    if corrupted_in_group.is_empty() {
                        continue;
                    }

                    if !parity_enabled {
                        let bad_idx = start_tile + corrupted_in_group[0];
                        let bad_hdr: &HyperTileHeader = from_bytes(&tiles_raw[bad_idx][..HYPER_TILE_HEADER_SIZE]);
                        return Err(S3AError::ChecksumMismatch {
                            expected: bad_hdr.data_crc32,
                            actual: 0,
                        });
                    }

                    if corrupted_in_group.len() > m_usize {
                        return Err(S3AError::UnrepairableCorruption {
                            group_idx: g,
                            corrupted_count: corrupted_in_group.len(),
                            max_repairable: m_usize,
                        });
                    }

                    // RS reconstruction
                    let rs = ReedSolomon::new(k_g, m_usize)
                        .map_err(|_| S3AError::OutOfBounds)?;

                    let group_parity_offset = data_end_offset + (g * m_usize * HYPER_TILE_SIZE);
                    let mut shards: Vec<Option<Vec<u8>>> = Vec::with_capacity(k_g + m_usize);

                    for j in 0..k_g {
                        let t_idx = start_tile + j;
                        if corrupted_in_group.contains(&j) {
                            shards.push(None);
                        } else {
                            shards.push(Some(tiles_raw[t_idx].clone()));
                        }
                    }

                    for p in 0..m_usize {
                        let p_start = group_parity_offset + p * HYPER_TILE_SIZE;
                        let p_end = p_start + HYPER_TILE_SIZE;
                        if p_end <= file_len {
                            shards.push(Some(file_bytes[p_start..p_end].to_vec()));
                        } else {
                            shards.push(None);
                        }
                    }

                    rs.reconstruct(&mut shards)
                        .map_err(|_| S3AError::UnrepairableCorruption {
                            group_idx: g,
                            corrupted_count: corrupted_in_group.len(),
                            max_repairable: m_usize,
                        })?;

                    for &missing_col in &corrupted_in_group {
                        let repaired = shards[missing_col].as_ref().ok_or(S3AError::CorruptedHeader)?;
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
                        tiles_raw[global_idx as usize] = repaired.clone();
                        repaired_tiles.push(global_idx);
                    }
                }

                // Extract anchors if present
                let anchors = if endcap.index_offset as usize > parity_end_offset {
                    let anchors_slice = &file_bytes[parity_end_offset..endcap.index_offset as usize];
                    if anchors_slice.len() >= 12 && &anchors_slice[0..8] == &SEALED_ANCHORS_MAGIC {
                        crate::anchor::extract_anchors_from_footer(anchors_slice, 0, 0)
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                };

                // Output repair if requested
                if let Some(out_p) = repair_out {
                    let mut out_file = OpenOptions::new()
                        .write(true)
                        .create(true)
                        .truncate(true)
                        .open(out_p)
                        .map_err(|_| S3AError::CorruptedHeader)?;

                    out_file.write_all(&file_bytes[..FILE_HEADER_SIZE]).map_err(|_| S3AError::CorruptedHeader)?;
                    for tile in &tiles_raw {
                        out_file.write_all(tile).map_err(|_| S3AError::CorruptedHeader)?;
                    }
                    if endcap.index_offset as usize > data_end_offset {
                        out_file.write_all(&file_bytes[data_end_offset..file_len]).map_err(|_| S3AError::CorruptedHeader)?;
                    } else {
                        out_file.write_all(&file_bytes[endcap.index_offset as usize..file_len]).map_err(|_| S3AError::CorruptedHeader)?;
                    }
                    out_file.flush().map_err(|_| S3AError::CorruptedHeader)?;
                    out_file.sync_all().map_err(|_| S3AError::CorruptedHeader)?;
                }

                return Ok(SealedVerifyReport {
                    is_sealed: true,
                    tile_count,
                    k: index_hdr.k,
                    m: index_hdr.m,
                    file_uuid: index_hdr.file_uuid,
                    rebuilt_directory,
                    warnings,
                    repaired_tiles,
                    out_of_order_detected,
                    directory,
                    anchors,
                    footer_version: 2,
                    seal_hash: index_hdr.seal_hash,
                    prev_seal_hash: index_hdr.prev_seal_hash,
                    seal_generation: index_hdr.seal_generation,
                    parity_enabled,
                    v2_entries,
                });
            }
        }
    }

    // Version 1 verification path
    let mut endcap_opt: Option<SealedEndCap> = None;
    if file_len >= FILE_HEADER_SIZE + SEALED_ENDCAP_SIZE {
        let candidate_bytes = &file_bytes[file_len - SEALED_ENDCAP_SIZE..];
        let candidate: SealedEndCap = bytemuck::pod_read_unaligned(candidate_bytes);
        if candidate.magic == SEALED_ENDCAP_MAGIC_V1 {
            let expected_crc = compute_endcap_crc32(&candidate);
            if candidate.endcap_crc32 == expected_crc {
                let footer_end = candidate.footer_offset + candidate.footer_length;
                if footer_end + (SEALED_ENDCAP_SIZE as u64) <= file_len as u64 {
                    let footer_slice = &file_bytes[candidate.footer_offset as usize..footer_end as usize];
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
        } else if candidate_bytes[0..6] == *b"S3AEND" {
            warnings.push("endcap_corrupt, directory rebuilt by scanning".into());
            rebuilt_directory = true;
        } else {
            warnings.push("footer_missing, directory rebuilt by scanning".into());
            rebuilt_directory = true;
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

    let mut tiles_raw: Vec<Vec<u8>> = Vec::new();
    let mut directory: Vec<SealedDirectoryEntry> = Vec::new();
    let mut out_of_order_detected = false;
    let k = endcap_opt.as_ref().map(|ec| ec.k).unwrap_or(DEFAULT_SEAL_K);
    let m = endcap_opt.as_ref().map(|ec| ec.m).unwrap_or(DEFAULT_SEAL_M);

    if let Some(ec) = &endcap_opt {
        let footer_slice = &file_bytes[ec.footer_offset as usize..(ec.footer_offset + ec.footer_length) as usize];
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
        // Fallback: Scan self-addressed tiles
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
    for tile in &tiles_raw {
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

    // Parity repair inspection for v1
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
        let footer_slice = &file_bytes[ec.footer_offset as usize..(ec.footer_offset + ec.footer_length) as usize];
        let dir_byte_len = (ec.tile_count as usize) * std::mem::size_of::<SealedDirectoryEntry>();
        let parity_slice = &footer_slice[dir_byte_len..];

        for g in 0..num_groups {
            let start_tile = g * k_usize;
            let end_tile = (start_tile + k_usize).min(tile_count as usize);
            let k_g = end_tile - start_tile;

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

            let codec = ReedSolomonCodec::new(k_g, m_usize)
                .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{}", e)))
                .map_err(|_| S3AError::OutOfBounds)?;

            let group_parity_offset = g * m_usize * HYPER_TILE_SIZE;
            let mut parity_tiles: Vec<&[u8]> = Vec::with_capacity(m_usize);
            for p in 0..m_usize {
                let p_start = group_parity_offset + p * HYPER_TILE_SIZE;
                let p_end = p_start + HYPER_TILE_SIZE;
                if p_end <= parity_slice.len() {
                    parity_tiles.push(&parity_slice[p_start..p_end]);
                }
            }

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

            let mut submatrix = vec![vec![0u8; k_g]; k_g];
            for (r, &shard_idx) in available_shard_indices.iter().enumerate() {
                submatrix[r].copy_from_slice(&codec.matrix[shard_idx]);
            }

            let inv = invert_matrix(&submatrix, k_g).map_err(|_| S3AError::CorruptedHeader)?;

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
        for tile in &tiles_raw {
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

    if let Some(out_p) = repair_out {
        let mut out_file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(out_p)
            .map_err(|_| S3AError::CorruptedHeader)?;

        out_file.write_all(&file_bytes[..FILE_HEADER_SIZE]).map_err(|_| S3AError::CorruptedHeader)?;
        for tile in &tiles_raw {
            out_file.write_all(tile).map_err(|_| S3AError::CorruptedHeader)?;
        }
        out_file.flush().map_err(|_| S3AError::CorruptedHeader)?;
        out_file.sync_all().map_err(|_| S3AError::CorruptedHeader)?;
    }

    let anchors = if let Some(ec) = &endcap_opt {
        let footer_slice = &file_bytes[ec.footer_offset as usize..(ec.footer_offset + ec.footer_length) as usize];
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
        footer_version: if is_sealed { 1 } else { 0 },
        seal_hash: [0u8; 32],
        prev_seal_hash: [0u8; 32],
        seal_generation: 1,
        parity_enabled: is_sealed,
        v2_entries: Vec::new(),
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
