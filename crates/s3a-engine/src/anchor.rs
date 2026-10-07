//! Snapshot anchoring and trust proofs for sealed S3A archives (v1.2-draft)
//!
//! Provides cryptographic anchoring of sealed S3A archives into public
//! OpenTimestamps (OTS) calendars and RFC 3161 Timestamp Authorities (TSA).
//!
//! Only the 32-byte SHA-256 fingerprint of the archive directory leaves the machine.
//! Anchors are preserved in a tamper-evident `"ANCHORS"` section in the sealed footer.

use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::path::Path;

use bytemuck::cast_slice;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use s3a_core::{
    S3AError, SealedDirectoryEntry, SealedEndCap, FILE_HEADER_SIZE,
    HYPER_TILE_SIZE, SEALED_ANCHORS_MAGIC, SEALED_ENDCAP_MAGIC, SEALED_ENDCAP_SIZE,
};

use crate::sealed::compute_endcap_crc32;

pub const DEFAULT_OTS_CALENDARS: &[&str] = &[
    "https://a.pool.opentimestamps.org",
    "https://b.pool.opentimestamps.org",
];

/// Computes the authoritative 32-byte SHA-256 archive fingerprint.
///
/// The fingerprint commits to:
/// - `file_uuid` (16 bytes, little-endian)
/// - `tile_count` (4 bytes, little-endian)
/// - The exact byte sequence of all `SealedDirectoryEntry` structs in the directory
pub fn compute_sealed_fingerprint(
    file_uuid: [u64; 2],
    tile_count: u32,
    directory: &[SealedDirectoryEntry],
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(&file_uuid[0].to_le_bytes());
    hasher.update(&file_uuid[1].to_le_bytes());
    hasher.update(&tile_count.to_le_bytes());
    let dir_bytes = cast_slice::<SealedDirectoryEntry, u8>(directory);
    hasher.update(dir_bytes);
    hasher.finalize().into()
}

/// Computes the archive fingerprint directly from a sealed S3A file on disk.
pub fn compute_sealed_file_fingerprint<P: AsRef<Path>>(path: P) -> io::Result<[u8; 32]> {
    let file_bytes = std::fs::read(path.as_ref())?;
    compute_sealed_bytes_fingerprint(&file_bytes)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
}

/// Computes the archive fingerprint directly from sealed S3A bytes in memory.
pub fn compute_sealed_bytes_fingerprint(file_bytes: &[u8]) -> Result<[u8; 32], S3AError> {
    let file_len = file_bytes.len();
    if file_len < FILE_HEADER_SIZE + SEALED_ENDCAP_SIZE {
        return Err(S3AError::BufferTooSmall {
            required: FILE_HEADER_SIZE + SEALED_ENDCAP_SIZE,
            provided: file_len,
        });
    }

    let endcap_bytes = &file_bytes[file_len - SEALED_ENDCAP_SIZE..];
    let endcap: SealedEndCap = bytemuck::pod_read_unaligned(endcap_bytes);
    if endcap.magic != SEALED_ENDCAP_MAGIC {
        return Err(S3AError::SealedEndCapCorrupt);
    }
    if endcap.endcap_crc32 != compute_endcap_crc32(&endcap) {
        return Err(S3AError::SealedEndCapCorrupt);
    }

    let footer_start = endcap.footer_offset as usize;
    let dir_len = (endcap.tile_count as usize) * std::mem::size_of::<SealedDirectoryEntry>();
    if footer_start + dir_len > file_len - SEALED_ENDCAP_SIZE {
        return Err(S3AError::SealedTruncated);
    }

    let mut dir_entries = Vec::with_capacity(endcap.tile_count as usize);
    let entry_sz = std::mem::size_of::<SealedDirectoryEntry>();
    for i in 0..endcap.tile_count as usize {
        let entry_offset = footer_start + i * entry_sz;
        let entry: SealedDirectoryEntry =
            bytemuck::pod_read_unaligned(&file_bytes[entry_offset..entry_offset + entry_sz]);
        dir_entries.push(entry);
    }
    Ok(compute_sealed_fingerprint(
        endcap.file_uuid,
        endcap.tile_count,
        &dir_entries,
    ))
}

/// A verifiable trust anchor proof stored in the sealed footer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnchorProof {
    /// Anchoring method ("opentimestamps" or "rfc3161")
    pub method: String,
    /// 64-character lowercase hex SHA-256 fingerprint
    pub fingerprint: String,
    /// Raw binary proof bytes (OTS stream or RFC 3161 DER token)
    #[serde(with = "hex_serde")]
    pub proof_bytes: Vec<u8>,
    /// Submission timestamp in Unix epoch seconds
    pub submitted_at: u64,
    /// Status: "pending" or "confirmed"
    pub status: String,
    /// Calendar or TSA endpoint URL
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    /// Bitcoin block height (for confirmed OTS proofs)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_height: Option<u64>,
    /// Bitcoin block hash hex (for confirmed OTS proofs)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_hash: Option<String>,
}

mod hex_serde {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        hex::decode(&s).map_err(serde::de::Error::custom)
    }
}

/// Extracts all anchor proofs from the sealed footer slice.
pub fn extract_anchors_from_footer(
    footer_slice: &[u8],
    dir_byte_len: usize,
    parity_byte_len: usize,
) -> Vec<AnchorProof> {
    let anchors_offset = dir_byte_len + parity_byte_len;
    if footer_slice.len() < anchors_offset + 12 {
        return Vec::new();
    }

    let anchors_slice = &footer_slice[anchors_offset..];
    if &anchors_slice[0..8] != &SEALED_ANCHORS_MAGIC {
        return Vec::new();
    }

    let payload_len = u32::from_le_bytes([
        anchors_slice[8],
        anchors_slice[9],
        anchors_slice[10],
        anchors_slice[11],
    ]) as usize;

    if anchors_slice.len() < 12 + payload_len {
        return Vec::new();
    }

    let json_bytes = &anchors_slice[12..12 + payload_len];
    serde_json::from_slice(json_bytes).unwrap_or_default()
}

/// Serializes an anchor list into the binary ANCHORS footer section.
pub fn serialize_anchors_section(anchors: &[AnchorProof]) -> io::Result<Vec<u8>> {
    let json_bytes = serde_json::to_vec(anchors)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let mut out = Vec::with_capacity(12 + json_bytes.len());
    out.extend_from_slice(&SEALED_ANCHORS_MAGIC);
    out.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(&json_bytes);
    let rem = out.len() % 8;
    if rem != 0 {
        out.resize(out.len() + (8 - rem), 0);
    }
    Ok(out)
}

/// Attaches or updates an anchor proof in an existing sealed S3A byte buffer.
pub fn attach_anchor_to_sealed_bytes(
    file_bytes: &mut Vec<u8>,
    new_anchor: AnchorProof,
) -> io::Result<()> {
    let file_len = file_bytes.len();
    if file_len < FILE_HEADER_SIZE + SEALED_ENDCAP_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "File too small for sealed end cap",
        ));
    }

    let endcap_offset = file_len - SEALED_ENDCAP_SIZE;
    let mut endcap: SealedEndCap = bytemuck::pod_read_unaligned(&file_bytes[endcap_offset..]);
    if endcap.magic != SEALED_ENDCAP_MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Missing sealed end cap magic",
        ));
    }

    let footer_start = endcap.footer_offset as usize;
    let footer_end = footer_start + (endcap.footer_length as usize);
    if footer_end > endcap_offset {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "Footer truncated before end cap",
        ));
    }

    let dir_byte_len = (endcap.tile_count as usize) * std::mem::size_of::<SealedDirectoryEntry>();
    let num_groups = if endcap.tile_count == 0 {
        0
    } else {
        (endcap.tile_count as usize + (endcap.k as usize) - 1) / (endcap.k as usize)
    };
    let parity_byte_len = num_groups * (endcap.m as usize) * HYPER_TILE_SIZE;

    let footer_slice = &file_bytes[footer_start..footer_end];
    let mut existing_anchors =
        extract_anchors_from_footer(footer_slice, dir_byte_len, parity_byte_len);

    // Replace if same method and uri, or append
    if let Some(pos) = existing_anchors
        .iter()
        .position(|a| a.method == new_anchor.method && a.uri == new_anchor.uri)
    {
        existing_anchors[pos] = new_anchor;
    } else {
        existing_anchors.push(new_anchor);
    }

    let anchors_section = serialize_anchors_section(&existing_anchors)?;

    // Directory + Parity stay untouched
    let base_footer_len = dir_byte_len + parity_byte_len;
    let mut new_footer = file_bytes[footer_start..footer_start + base_footer_len].to_vec();
    new_footer.extend_from_slice(&anchors_section);

    let new_footer_len = new_footer.len() as u64;
    let new_footer_crc = crc32c::crc32c(&new_footer);

    endcap.footer_length = new_footer_len;
    endcap.footer_crc32 = new_footer_crc;
    endcap.endcap_crc32 = compute_endcap_crc32(&endcap);

    // Truncate at footer_start, append new footer and endcap
    file_bytes.truncate(footer_start);
    file_bytes.extend_from_slice(&new_footer);
    file_bytes.extend_from_slice(bytemuck::bytes_of(&endcap));

    Ok(())
}

/// Attaches or updates an anchor proof in a sealed S3A file on disk.
pub fn attach_anchor_to_file<P: AsRef<Path>>(path: P, new_anchor: AnchorProof) -> io::Result<()> {
    let mut file_bytes = std::fs::read(path.as_ref())?;
    attach_anchor_to_sealed_bytes(&mut file_bytes, new_anchor)?;

    let mut file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path.as_ref())?;
    file.write_all(&file_bytes)?;
    file.flush()?;
    file.sync_all()?;
    Ok(())
}

// -----------------------------------------------------------------------------
// OpenTimestamps Binary Implementation & Verification
// -----------------------------------------------------------------------------

pub use opentimestamps;
pub use opentimestamps::{
    attestation::Attestation,
    op::Op,
    ser::{Deserializer, DigestType, Serializer},
    timestamp::{Step, StepData},
    DetachedTimestampFile, Timestamp,
};

/// Collects all attestations and their accumulated hashes from an OTS timestamp execution tree.
pub fn collect_attestations<'a>(step: &'a Step) -> Vec<(&'a Attestation, &'a [u8])> {
    let mut list = Vec::new();
    if let StepData::Attestation(ref att) = step.data {
        list.push((att, step.output.as_slice()));
    }
    for n in &step.next {
        list.extend(collect_attestations(n));
    }
    list
}

/// Splices an upgraded timestamp branch into an existing pending attestation step.
pub fn splice_attestation_with_upgraded(
    step: &mut Step,
    target_commitment: &[u8],
    upgraded_step: Step,
) -> bool {
    if let StepData::Attestation(Attestation::Pending { .. }) = &step.data {
        if step.output.as_slice() == target_commitment {
            *step = upgraded_step;
            return true;
        }
    }
    for next in &mut step.next {
        if splice_attestation_with_upgraded(next, target_commitment, upgraded_step.clone()) {
            return true;
        }
    }
    false
}

/// Source for independent Bitcoin block header lookup during verification.
#[derive(Debug, Clone)]
pub enum BlockHeaderSource<'a> {
    Esplora(String),
    OfflineMap(&'a std::collections::HashMap<u64, [u8; 80]>),
}

impl<'a> Default for BlockHeaderSource<'a> {
    fn default() -> Self {
        BlockHeaderSource::Esplora("https://blockstream.info/api".to_string())
    }
}

/// Extracts the 32-byte Merkle root from an 80-byte Bitcoin block header (bytes 36..68).
pub fn extract_merkle_root_from_header(header: &[u8; 80]) -> [u8; 32] {
    let mut root = [0u8; 32];
    root.copy_from_slice(&header[36..68]);
    root
}

/// Expands Bitcoin 32-bit compact target (nBits) to a 32-byte big-endian target threshold.
pub fn target_from_nbits(nbits: u32) -> [u8; 32] {
    let exponent = (nbits >> 24) as usize;
    let mantissa = nbits & 0x007f_ffff;
    let mut target = [0u8; 32];
    if exponent <= 3 {
        let val = mantissa >> (8 * (3 - exponent));
        target[28..32].copy_from_slice(&(val as u32).to_be_bytes());
    } else if exponent <= 32 {
        let offset = 32 - exponent;
        let m_bytes = (mantissa as u32).to_be_bytes(); // [0, m0, m1, m2]
        if offset < 32 {
            target[offset] = m_bytes[1];
        }
        if offset + 1 < 32 {
            target[offset + 1] = m_bytes[2];
        }
        if offset + 2 < 32 {
            target[offset + 2] = m_bytes[3];
        }
    }
    target
}

/// Fetches an 80-byte raw Bitcoin block header at a specified block height.
pub fn fetch_block_header(height: u64, source: &BlockHeaderSource) -> io::Result<[u8; 80]> {
    match source {
        BlockHeaderSource::OfflineMap(map) => {
            map.get(&height).copied().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("Bitcoin block header for height {} not in offline map", height),
                )
            })
        }
        BlockHeaderSource::Esplora(base_url) => {
            let height_url = format!("{}/block-height/{}", base_url.trim_end_matches('/'), height);
            let resp = ureq::get(&height_url)
                .timeout(std::time::Duration::from_secs(10))
                .call()
                .map_err(|e| io::Error::new(io::ErrorKind::ConnectionRefused, e))?;
            let block_hash = resp
                .into_string()
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            let header_url = format!("{}/block/{}/header", base_url.trim_end_matches('/'), block_hash.trim());
            let header_resp = ureq::get(&header_url)
                .timeout(std::time::Duration::from_secs(10))
                .call()
                .map_err(|e| io::Error::new(io::ErrorKind::ConnectionRefused, e))?;
            let header_hex = header_resp
                .into_string()
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            let header_bytes = hex::decode(header_hex.trim())
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            if header_bytes.len() != 80 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Invalid Bitcoin block header length: {}", header_bytes.len()),
                ));
            }
            let mut out = [0u8; 80];
            out.copy_from_slice(&header_bytes);

            // 1. Verify double-SHA256(header) reversed == hash returned by /block-height/{h}
            let h1 = Sha256::digest(&out);
            let mut h2 = Sha256::digest(&h1);
            h2.reverse();
            let computed_hash_hex = hex::encode(h2);
            if computed_hash_hex.to_lowercase() != block_hash.trim().to_lowercase() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "Block header hash mismatch: computed {}, expected {}",
                        computed_hash_hex,
                        block_hash.trim()
                    ),
                ));
            }

            // 2. Verify that the hash meets the header's nBits target
            let n_bits = u32::from_le_bytes(out[72..76].try_into().unwrap());
            let target = target_from_nbits(n_bits);
            if h2.as_slice() > target.as_slice() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "Block header hash {} exceeds nBits target {}",
                        computed_hash_hex,
                        hex::encode(target)
                    ),
                ));
            }

            Ok(out)
        }
    }
}

/// Verification report from evaluating an OpenTimestamps proof path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtsVerifyReport {
    pub status: String,
    pub uri: Option<String>,
    pub block_height: Option<u64>,
    pub block_hash: Option<String>,
    pub commitment_matched: bool,
}

/// Submits an archive fingerprint to at least 2 OpenTimestamps calendars.
/// Returns each valid timestamp returned by responding calendars.
/// Never fabricates a proof if calendar response fails or cannot be parsed.
#[cfg(not(target_arch = "wasm32"))]
pub fn submit_ots(fingerprint: [u8; 32], calendars: &[String]) -> io::Result<Vec<AnchorProof>> {
    if calendars.len() < 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "OpenTimestamps anchoring requires at least 2 calendars",
        ));
    }

    let now_sec = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut successful_proofs = Vec::new();

    for cal in calendars {
        let digest_url = format!("{}/digest", cal.trim_end_matches('/'));
        let res = ureq::post(&digest_url)
            .timeout(std::time::Duration::from_secs(10))
            .set("Content-Type", "application/octet-stream")
            .send_bytes(&fingerprint);

        match res {
            Ok(response) => {
                let mut body = Vec::new();
                if response.into_reader().read_to_end(&mut body).is_err() {
                    continue;
                }

                // Calendar /digest replies are bare timestamps without file header
                let mut deser = Deserializer::new(&body[..]);
                match Timestamp::deserialize(&mut deser, fingerprint.to_vec()) {
                    Ok(ts) => {
                        let detached = DetachedTimestampFile {
                            digest_type: DigestType::Sha256,
                            timestamp: ts,
                        };
                        let mut proof_bytes = Vec::new();
                        if detached.to_writer(&mut proof_bytes).is_ok() {
                            successful_proofs.push(AnchorProof {
                                method: "opentimestamps".to_string(),
                                fingerprint: hex::encode(fingerprint),
                                proof_bytes,
                                submitted_at: now_sec,
                                status: "pending".to_string(),
                                uri: Some(cal.clone()),
                                block_height: None,
                                block_hash: None,
                            });
                        }
                    }
                    Err(_) => {
                        // Never fabricate! If calendar reply cannot be parsed, that calendar failed.
                        continue;
                    }
                }
            }
            Err(_) => {
                continue;
            }
        }
    }

    if successful_proofs.is_empty() {
        Err(io::Error::new(
            io::ErrorKind::ConnectionRefused,
            format!(
                "All OpenTimestamps calendars failed to respond or returned invalid timestamps: {:?}",
                calendars
            ),
        ))
    } else {
        Ok(successful_proofs)
    }
}

/// Upgrades a pending OpenTimestamps anchor proof to confirmed Bitcoin attestation.
#[cfg(not(target_arch = "wasm32"))]
pub fn upgrade_ots(
    proof: &mut AnchorProof,
    calendar_override: Option<&str>,
) -> io::Result<bool> {
    if proof.method != "opentimestamps" {
        return Ok(false);
    }
    if proof.status == "confirmed" {
        return Ok(true);
    }

    let mut detached = DetachedTimestampFile::from_reader(&proof.proof_bytes[..])
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("Failed to parse OTS proof: {}", e)))?;

    let pending_items: Vec<(String, Vec<u8>)> = collect_attestations(&detached.timestamp.first_step)
        .into_iter()
        .filter_map(|(att, comm)| {
            if let Attestation::Pending { uri } = att {
                Some((uri.clone(), comm.to_vec()))
            } else {
                None
            }
        })
        .collect();

    let has_bitcoin = collect_attestations(&detached.timestamp.first_step)
        .into_iter()
        .any(|(att, _)| matches!(att, Attestation::Bitcoin { .. }));
    if has_bitcoin {
        proof.status = "confirmed".to_string();
        return Ok(true);
    }

    let mut any_upgraded = false;

    for (uri, commitment) in pending_items {
        let target_cal = calendar_override.unwrap_or(&uri);
        let upgrade_url = format!(
            "{}/timestamp/{}",
            target_cal.trim_end_matches('/'),
            hex::encode(&commitment)
        );

        let res = ureq::get(&upgrade_url)
            .timeout(std::time::Duration::from_secs(10))
            .call();

        if let Ok(response) = res {
            let mut body = Vec::new();
            if response.into_reader().read_to_end(&mut body).is_ok() {
                let mut deser = Deserializer::new(&body[..]);
                if let Ok(upgraded_ts) = Timestamp::deserialize(&mut deser, commitment.clone()) {
                    let up_atts = collect_attestations(&upgraded_ts.first_step);
                    let mut confirmed_height = None;
                    for (up_att, up_out) in up_atts {
                        if let Attestation::Bitcoin { height } = up_att {
                            confirmed_height = Some((*height, up_out.to_vec()));
                            break;
                        }
                    }

                    if let Some((height, btc_root)) = confirmed_height {
                        if splice_attestation_with_upgraded(
                            &mut detached.timestamp.first_step,
                            &commitment,
                            upgraded_ts.first_step,
                        ) {
                            proof.status = "confirmed".to_string();
                            proof.block_height = Some(height as u64);
                            proof.block_hash = Some(hex::encode(btc_root));
                            any_upgraded = true;
                        }
                    }
                }
            }
        }
    }

    if any_upgraded {
        let mut new_proof_bytes = Vec::new();
        detached.to_writer(&mut new_proof_bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("Failed to serialize upgraded OTS: {}", e)))?;
        proof.proof_bytes = new_proof_bytes;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Independently verifies an OpenTimestamps proof against the archive fingerprint.
/// Walks the proof ops up to the Bitcoin attestation, fetches the block header independently,
/// and asserts that the computed hash equals the header's Merkle root.
pub fn verify_ots(
    proof: &AnchorProof,
    expected_fingerprint: &[u8; 32],
    header_source: Option<&BlockHeaderSource>,
) -> Result<OtsVerifyReport, String> {
    if proof.method != "opentimestamps" {
        return Err("Not an OpenTimestamps proof".into());
    }

    let detached = DetachedTimestampFile::from_reader(&proof.proof_bytes[..])
        .map_err(|e| format!("Corrupt OpenTimestamps proof: {}", e))?;

    if detached.digest_type != DigestType::Sha256 {
        return Err(format!("Unsupported OTS digest type: {:?}", detached.digest_type));
    }

    if detached.timestamp.start_digest.as_slice() != expected_fingerprint {
        return Err(format!(
            "OTS start digest mismatch: proof has {}, expected {}",
            hex::encode(&detached.timestamp.start_digest),
            hex::encode(expected_fingerprint)
        ));
    }

    let default_src = BlockHeaderSource::default();
    let src = header_source.unwrap_or(&default_src);

    let attestations = collect_attestations(&detached.timestamp.first_step);
    if attestations.is_empty() {
        return Err("OTS proof contains no attestations".into());
    }

    let mut best_confirmed: Option<(u64, [u8; 32])> = None;
    let mut pending_uri: Option<String> = None;

    for (att, calculated_hash) in attestations {
        match att {
            Attestation::Bitcoin { height } => {
                let h = *height as u64;
                let header = fetch_block_header(h, src)
                    .map_err(|e| format!("Failed to fetch Bitcoin block header at height {}: {}", h, e))?;
                let merkle_root = extract_merkle_root_from_header(&header);
                if calculated_hash != &merkle_root {
                    return Err(format!(
                        "Bitcoin block Merkle root mismatch at height {}: calculated {}, header has {}",
                        h,
                        hex::encode(calculated_hash),
                        hex::encode(merkle_root)
                    ));
                }
                best_confirmed = Some((h, merkle_root));
            }
            Attestation::Pending { uri } => {
                if pending_uri.is_none() {
                    pending_uri = Some(uri.clone());
                }
            }
            Attestation::Unknown { .. } => {}
        }
    }

    if let Some((height, root)) = best_confirmed {
        Ok(OtsVerifyReport {
            status: "confirmed".to_string(),
            uri: proof.uri.clone(),
            block_height: Some(height),
            block_hash: Some(hex::encode(root)),
            commitment_matched: true,
        })
    } else if let Some(uri) = pending_uri {
        Ok(OtsVerifyReport {
            status: "pending".to_string(),
            uri: Some(uri),
            block_height: None,
            block_hash: None,
            commitment_matched: true,
        })
    } else {
        Err("No valid Bitcoin or pending attestation found in proof".into())
    }
}

// -----------------------------------------------------------------------------
// RFC 3161 Standard Implementation & Verification
// -----------------------------------------------------------------------------

use cmpv2::status::PkiStatus;
use cms::signed_data::SignedData;
use der::{
    asn1::{Int, OctetString},
    oid::ObjectIdentifier,
    Decode, Encode,
};
use spki::AlgorithmIdentifier;
use x509_tsp::{MessageImprint, TimeStampReq, TimeStampResp, TstInfo, TspVersion};

pub const SHA256_OID_STR: &str = "2.16.840.1.101.3.4.2.1";

/// Parses a string containing one or more PEM-encoded certificates into `x509_cert::Certificate` instances.
pub fn parse_pem_certificates(pem_str: &str) -> Result<Vec<x509_cert::Certificate>, String> {
    use der::DecodePem;
    let mut certs = Vec::new();
    let begin_tag = "-----BEGIN CERTIFICATE-----";
    let end_tag = "-----END CERTIFICATE-----";
    let mut pos = 0;
    while let Some(start) = pem_str[pos..].find(begin_tag) {
        let abs_start = pos + start;
        let end = pem_str[abs_start..].find(end_tag)
            .ok_or_else(|| "Unclosed BEGIN CERTIFICATE block in PEM data".to_string())?;
        let abs_end = abs_start + end + end_tag.len();
        let block = &pem_str[abs_start..abs_end];
        let cert = x509_cert::Certificate::from_pem(block)
            .map_err(|e| format!("Failed to parse PEM certificate: {}", e))?;
        certs.push(cert);
        pos = abs_end;
    }
    if certs.is_empty() {
        return Err("No certificates found in PEM data".into());
    }
    Ok(certs)
}

/// Loads one or more PEM-encoded certificates from a file on disk.
pub fn load_pem_certificates<P: AsRef<Path>>(path: P) -> Result<Vec<x509_cert::Certificate>, String> {
    let s = std::fs::read_to_string(path.as_ref())
        .map_err(|e| format!("Failed to read certificate PEM file {:?}: {}", path.as_ref(), e))?;
    parse_pem_certificates(&s)
}

fn verify_signature_over_data(
    spki: &x509_cert::spki::SubjectPublicKeyInfoOwned,
    data: &[u8],
    signature: &[u8],
    sig_alg_oid: &ObjectIdentifier,
) -> Result<(), String> {
    use rsa::pkcs1::DecodeRsaPublicKey;
    use signature::Verifier;
    let pk_bytes = spki.subject_public_key.as_bytes()
        .ok_or_else(|| "Invalid subject public key bit string".to_string())?;

    let rsa_encryption = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");
    let sha256_rsa = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.11");
    let sha384_rsa = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.12");
    let sha512_rsa = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.13");
    let ec_public_key = ObjectIdentifier::new_unwrap("1.2.840.10045.2.1");

    if spki.algorithm.oid == rsa_encryption || spki.algorithm.oid == sha256_rsa {
        let rsa_key = rsa::RsaPublicKey::from_pkcs1_der(pk_bytes)
            .map_err(|e| format!("Invalid PKCS#1 RSA key: {}", e))?;
        let rsa_sig = rsa::pkcs1v15::Signature::try_from(signature)
            .map_err(|e| format!("Invalid RSA signature bytes: {}", e))?;

        if *sig_alg_oid == sha384_rsa {
            let vk = rsa::pkcs1v15::VerifyingKey::<sha2::Sha384>::new(rsa_key);
            vk.verify(data, &rsa_sig)
                .map_err(|e| format!("RSA SHA-384 signature verification failed: {}", e))?;
        } else if *sig_alg_oid == sha512_rsa {
            let vk = rsa::pkcs1v15::VerifyingKey::<sha2::Sha512>::new(rsa_key);
            vk.verify(data, &rsa_sig)
                .map_err(|e| format!("RSA SHA-512 signature verification failed: {}", e))?;
        } else {
            let vk = rsa::pkcs1v15::VerifyingKey::<sha2::Sha256>::new(rsa_key);
            vk.verify(data, &rsa_sig)
                .map_err(|e| format!("RSA SHA-256 signature verification failed: {}", e))?;
        }
        Ok(())
    } else if spki.algorithm.oid == ec_public_key {
        let vk = p256::ecdsa::VerifyingKey::from_sec1_bytes(pk_bytes)
            .map_err(|e| format!("Invalid P-256 public key: {}", e))?;
        let ec_sig = p256::ecdsa::Signature::from_der(signature)
            .map_err(|e| format!("Invalid ECDSA signature DER: {}", e))?;
        vk.verify(data, &ec_sig)
            .map_err(|e| format!("ECDSA signature verification failed: {}", e))?;
        Ok(())
    } else {
        Err(format!("Unsupported public key algorithm OID: {}", spki.algorithm.oid))
    }
}

fn is_trust_root(cert: &x509_cert::Certificate, trust_roots: &[x509_cert::Certificate]) -> Result<bool, String> {
    let cert_der = cert.to_der().map_err(|e| format!("Failed to encode certificate to DER: {}", e))?;
    let cert_spki = cert.tbs_certificate.subject_public_key_info.to_der()
        .map_err(|e| format!("Failed to encode SPKI to DER: {}", e))?;
    for root in trust_roots {
        let root_der = root.to_der().map_err(|e| format!("Failed to encode root to DER: {}", e))?;
        if cert_der == root_der {
            return Ok(true);
        }
        let root_spki = root.tbs_certificate.subject_public_key_info.to_der()
            .map_err(|e| format!("Failed to encode root SPKI to DER: {}", e))?;
        if cert_spki == root_spki && cert.tbs_certificate.subject == root.tbs_certificate.subject {
            return Ok(true);
        }
    }
    Ok(false)
}

fn check_cert_validity(
    cert: &x509_cert::Certificate,
    gen_time: std::time::SystemTime,
) -> Result<(), String> {
    let not_before = cert.tbs_certificate.validity.not_before.to_system_time();
    let not_after = cert.tbs_certificate.validity.not_after.to_system_time();
    if gen_time < not_before || gen_time > not_after {
        return Err(format!(
            "Certificate {:?} validity window ({:?} .. {:?}) does not cover TSTInfo genTime ({:?})",
            cert.tbs_certificate.subject, not_before, not_after, gen_time
        ));
    }
    Ok(())
}

pub fn cert_matches_ski(cert: &x509_cert::Certificate, ski: &[u8]) -> bool {
    let ski_oid = ObjectIdentifier::new_unwrap("2.5.29.14");
    cert.tbs_certificate.extensions.as_ref().map_or(false, |exts| {
        exts.iter().any(|ext| {
            if ext.extn_id == ski_oid {
                if let Ok(inner) = OctetString::from_der(ext.extn_value.as_bytes()) {
                    inner.as_bytes() == ski
                } else {
                    false
                }
            } else {
                false
            }
        })
    })
}

fn verify_certificate_chain(
    leaf: &x509_cert::Certificate,
    pool: &[x509_cert::Certificate],
    trust_roots: &[x509_cert::Certificate],
    gen_time: std::time::SystemTime,
) -> Result<(), String> {
    let mut current = leaf.clone();
    for _ in 0..10 {
        check_cert_validity(&current, gen_time)?;

        if is_trust_root(&current, trust_roots)? {
            return Ok(());
        }

        let mut parent_opt = None;
        for candidate in trust_roots.iter().chain(pool.iter()) {
            if candidate.tbs_certificate.subject == current.tbs_certificate.issuer {
                parent_opt = Some(candidate);
                break;
            }
        }

        let parent = parent_opt.ok_or_else(|| {
            format!(
                "Certificate chain broken: issuer {:?} not found in certificate pool or trust store",
                current.tbs_certificate.issuer
            )
        })?;

        check_cert_validity(parent, gen_time)?;

        let p_spki = &parent.tbs_certificate.subject_public_key_info;
        let tbs_der = current.tbs_certificate.to_der()
            .map_err(|e| format!("Failed to encode TBSCertificate to DER: {}", e))?;
        let sig = current.signature.as_bytes()
            .ok_or_else(|| "Invalid signature bit string in certificate".to_string())?;

        verify_signature_over_data(p_spki, &tbs_der, sig, &current.signature_algorithm.oid)
            .map_err(|e| format!("Certificate signature verification by parent failed: {}", e))?;

        if is_trust_root(parent, trust_roots)? {
            return Ok(());
        }

        if parent.tbs_certificate.subject == current.tbs_certificate.subject {
            return Err("Self-signed certificate encountered that is not in trust store".into());
        }

        current = parent.clone();
    }
    Err("Certificate chain exceeds maximum depth of 10".into())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn submit_rfc3161(fingerprint: [u8; 32], tsa_url: &str) -> io::Result<AnchorProof> {
    let now_sec = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let sha256_oid = ObjectIdentifier::new_unwrap(SHA256_OID_STR);
    let hash_algorithm = AlgorithmIdentifier {
        oid: sha256_oid,
        parameters: Some(der::Any::new(der::Tag::Null, vec![])
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?),
    };
    let hashed_message = OctetString::new(&fingerprint)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    let message_imprint = MessageImprint {
        hash_algorithm,
        hashed_message,
    };

    let mut nonce_bytes = [0u8; 8];
    getrandom::getrandom(&mut nonce_bytes)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("getrandom error: {}", e)))?;
    nonce_bytes[0] &= 0x7f; // Ensure non-negative in ASN.1 DER INTEGER
    let nonce = Int::new(&nonce_bytes)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

    let req = TimeStampReq {
        version: TspVersion::V1,
        message_imprint,
        req_policy: None,
        nonce: Some(nonce),
        cert_req: true,
        extensions: None,
    };

    let req_der = req.to_der()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

    let res = ureq::post(tsa_url)
        .timeout(std::time::Duration::from_secs(10))
        .set("Content-Type", "application/timestamp-query")
        .set("Accept", "application/timestamp-reply")
        .send_bytes(&req_der);

    match res {
        Ok(response) => {
            let mut token = Vec::new();
            response
                .into_reader()
                .read_to_end(&mut token)
                .map_err(|e| io::Error::new(io::ErrorKind::UnexpectedEof, e))?;

            let resp = TimeStampResp::from_der(&token)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("Failed to parse RFC 3161 response: {}", e)))?;

            if resp.status.status != PkiStatus::Accepted && resp.status.status != PkiStatus::GrantedWithMods {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("RFC 3161 TSA rejected request with status: {:?}", resp.status.status),
                ));
            }

            let ts_token = resp.time_stamp_token.ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "TSA reply missing timeStampToken")
            })?;

            let sd = SignedData::from_der(&ts_token.content.to_der()
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("Invalid CMS SignedData: {}", e)))?;

            let encap = sd.encap_content_info.econtent.as_ref().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "Missing econtent in SignedData")
            })?;

            let tst = TstInfo::from_der(encap.value())
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("Invalid TSTInfo: {}", e)))?;

            let resp_nonce = tst.nonce.as_ref().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "TSA reply missing nonce")
            })?;
            if resp_nonce.as_bytes() != &nonce_bytes {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "Nonce mismatch in TSA reply: expected {:?}, got {:?}",
                        &nonce_bytes,
                        resp_nonce.as_bytes()
                    ),
                ));
            }

            if tst.message_imprint.hashed_message.as_bytes() != fingerprint {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "TSA messageImprint does not match submitted fingerprint",
                ));
            }

            Ok(AnchorProof {
                method: "rfc3161".to_string(),
                fingerprint: hex::encode(fingerprint),
                proof_bytes: token,
                submitted_at: now_sec,
                status: "unverified".to_string(),
                uri: Some(tsa_url.to_string()),
                block_height: None,
                block_hash: None,
            })
        }
        Err(e) => Err(io::Error::new(
            io::ErrorKind::ConnectionRefused,
            format!("RFC 3161 TSA request failed: {}", e),
        )),
    }
}

pub fn verify_rfc3161(
    proof: &AnchorProof,
    expected_fingerprint: &[u8; 32],
    trust_store: Option<&[x509_cert::Certificate]>,
) -> Result<bool, String> {
    if proof.method != "rfc3161" {
        return Err("Not an RFC 3161 proof".into());
    }

    if !hex::encode(expected_fingerprint).eq_ignore_ascii_case(&proof.fingerprint) {
        return Ok(false);
    }

    // No trust store supplied => status "unverified", never "confirmed"
    let roots = match trust_store {
        Some(r) if !r.is_empty() => r,
        _ => {
            return Ok(false);
        }
    };

    let (sd_bytes, status_ok) = if let Ok(resp) = TimeStampResp::from_der(&proof.proof_bytes) {
        let is_ok = resp.status.status == PkiStatus::Accepted || resp.status.status == PkiStatus::GrantedWithMods;
        if !is_ok {
            return Err(format!("RFC 3161 token status not accepted: {:?}", resp.status.status));
        }
        let token = resp.time_stamp_token.ok_or("Missing timeStampToken in TimeStampResp")?;
        (token.content.to_der().map_err(|e| e.to_string())?, true)
    } else if let Ok(token) = x509_tsp::TimeStampToken::from_der(&proof.proof_bytes) {
        (token.content.to_der().map_err(|e| e.to_string())?, true)
    } else {
        return Err("Invalid RFC 3161 DER token payload".into());
    };

    if !status_ok {
        return Ok(false);
    }

    let sd = SignedData::from_der(&sd_bytes)
        .map_err(|e| format!("Failed to parse SignedData: {}", e))?;
    let encap = sd.encap_content_info.econtent.as_ref().ok_or("Missing econtent in SignedData")?;
    let tst = TstInfo::from_der(encap.value())
        .map_err(|e| format!("Failed to parse TSTInfo: {}", e))?;

    let sha256_oid = ObjectIdentifier::new_unwrap(SHA256_OID_STR);
    if tst.message_imprint.hash_algorithm.oid != sha256_oid {
        return Err(format!(
            "RFC 3161 algorithm mismatch: expected sha256 ({}), got {}",
            SHA256_OID_STR, tst.message_imprint.hash_algorithm.oid
        ));
    }

    if tst.message_imprint.hashed_message.as_bytes() != expected_fingerprint {
        return Err("RFC 3161 messageImprint does not match archive fingerprint".into());
    }

    // Check signed attributes
    let signer_info = sd.signer_infos.0.get(0)
        .ok_or("SignedData has empty signer_infos")?;

    let signed_attrs = signer_info.signed_attrs.as_ref()
        .ok_or("SignerInfo missing signed attributes")?;

    let id_content_type = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.3");
    let id_ct_tstinfo = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.1.4");
    let id_message_digest = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.4");

    let mut found_content_type = false;
    let mut found_message_digest = false;

    for attr in signed_attrs.iter() {
        if attr.oid == id_content_type {
            let val = attr.values.get(0).ok_or("Empty contentType value")?;
            let val_der = val.to_der().map_err(|e| format!("Invalid contentType DER: {}", e))?;
            let val_oid = ObjectIdentifier::from_der(&val_der)
                .map_err(|e| format!("Invalid contentType OID: {}", e))?;
            if val_oid == id_ct_tstinfo {
                found_content_type = true;
            } else {
                return Err(format!("Signed attribute contentType mismatch: expected {}, got {}", id_ct_tstinfo, val_oid));
            }
        } else if attr.oid == id_message_digest {
            let val = attr.values.get(0).ok_or("Empty messageDigest value")?;
            let val_der = val.to_der().map_err(|e| format!("Invalid messageDigest DER: {}", e))?;
            let val_oct = OctetString::from_der(&val_der)
                .map_err(|e| format!("Invalid messageDigest OctetString: {}", e))?;
            let expected_digest = Sha256::digest(encap.value());
            if val_oct.as_bytes() == expected_digest.as_slice() {
                found_message_digest = true;
            } else {
                return Err("Signed attribute messageDigest does not match SHA-256(eContent)".into());
            }
        }
    }

    if !found_content_type {
        return Err("Missing required signed attribute contentType (id-ct-TSTInfo)".into());
    }
    if !found_message_digest {
        return Err("Missing required signed attribute messageDigest".into());
    }

    // RFC 5652 §5.4: Encode signed attributes as SET OF
    let mut set_der = signed_attrs.to_der()
        .map_err(|e| format!("Failed to encode signed attributes: {}", e))?;
    if set_der.is_empty() {
        return Err("Empty signed attributes DER".into());
    }
    if set_der[0] == 0xa0 {
        set_der[0] = 0x31;
    }

    // Locate signer certificate
    let cert_choices = sd.certificates.as_ref().map(|c| &c.0);
    let mut pool = Vec::new();
    let mut signer_cert: Option<x509_cert::Certificate> = None;

    if let Some(choices) = cert_choices {
        for choice in choices.iter() {
            if let cms::cert::CertificateChoices::Certificate(ref c) = choice {
                pool.push(c.clone());
                match &signer_info.sid {
                    cms::signed_data::SignerIdentifier::IssuerAndSerialNumber(ref iasn) => {
                        if c.tbs_certificate.issuer == iasn.issuer
                            && c.tbs_certificate.serial_number == iasn.serial_number
                        {
                            signer_cert = Some(c.clone());
                        }
                    }
                    cms::signed_data::SignerIdentifier::SubjectKeyIdentifier(ref ski) => {
                        if cert_matches_ski(c, ski.0.as_bytes()) {
                            signer_cert = Some(c.clone());
                        }
                    }
                }
            }
        }
    }

    let tsa_cert = signer_cert.ok_or("Signer certificate not found in CMS SignedData")?;

    // Require EKU id-kp-timeStamping on the TSA cert
    let eku_oid = ObjectIdentifier::new_unwrap("2.5.29.37");
    let id_kp_timestamping = ObjectIdentifier::new_unwrap("1.3.6.1.5.5.7.3.8");

    let has_ts_eku = tsa_cert.tbs_certificate.extensions.as_ref().map_or(false, |exts| {
        exts.iter().any(|ext| {
            if ext.extn_id == eku_oid {
                if let Ok(seq) = der::asn1::SequenceOf::<ObjectIdentifier, 16>::from_der(ext.extn_value.as_bytes()) {
                    seq.iter().any(|oid| *oid == id_kp_timestamping)
                } else {
                    false
                }
            } else {
                false
            }
        })
    });

    if !has_ts_eku {
        return Err("TSA certificate missing required EKU id-kp-timeStamping (1.3.6.1.5.5.7.3.8)".into());
    }

    // Enforce validity window on TSA cert at TSTInfo genTime
    let gen_time = tst.gen_time.to_system_time();
    check_cert_validity(&tsa_cert, gen_time)?;

    // Verify signature over DER(signedAttrs as SET) with TSA cert public key
    let spki = &tsa_cert.tbs_certificate.subject_public_key_info;
    let sig_bytes = signer_info.signature.as_bytes();
    verify_signature_over_data(spki, &set_der, sig_bytes, &signer_info.signature_algorithm.oid)?;

    // Verify certificate chain to trust store and enforce validity window at TSTInfo genTime
    verify_certificate_chain(&tsa_cert, &pool, roots, gen_time)?;

    Ok(true)
}

// -----------------------------------------------------------------------------
// Offline Mock Calendar Server For Tests
// -----------------------------------------------------------------------------

pub mod mock_calendar {
    use super::*;
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    pub struct MockOtsCalendar {
        pub port: u16,
        confirmations: Arc<Mutex<HashMap<String, (u64, [u8; 80])>>>,
        return_garbage: Arc<std::sync::atomic::AtomicBool>,
        corrupt_esplora_header: Arc<std::sync::atomic::AtomicBool>,
        running: Arc<std::sync::atomic::AtomicBool>,
    }

    impl MockOtsCalendar {
        pub fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
            let port = listener.local_addr().unwrap().port();
            let confirmations: Arc<Mutex<HashMap<String, (u64, [u8; 80])>>> =
                Arc::new(Mutex::new(HashMap::new()));
            let return_garbage = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let corrupt_esplora_header = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let running = Arc::new(std::sync::atomic::AtomicBool::new(true));

            let conf_clone = Arc::clone(&confirmations);
            let garbage_clone = Arc::clone(&return_garbage);
            let corrupt_esplora_clone = Arc::clone(&corrupt_esplora_header);
            let run_clone = Arc::clone(&running);

            thread::spawn(move || {
                listener.set_nonblocking(true).unwrap();
                while run_clone.load(std::sync::atomic::Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            let mut reader = BufReader::new(stream.try_clone().unwrap());
                            let mut req_line = String::new();
                            if reader.read_line(&mut req_line).is_err() || req_line.is_empty() {
                                continue;
                            }

                            let mut content_length = 0usize;
                            loop {
                                let mut line = String::new();
                                if reader.read_line(&mut line).is_err() || line.trim().is_empty() {
                                    break;
                                }
                                if line.to_lowercase().starts_with("content-length:") {
                                    if let Some(val) = line.split(':').nth(1) {
                                        content_length = val.trim().parse().unwrap_or(0);
                                    }
                                }
                            }

                            if garbage_clone.load(std::sync::atomic::Ordering::Relaxed) {
                                let garbage = b"HTTP/1.1 200 OK\r\nContent-Length: 14\r\nContent-Type: text/plain\r\n\r\nGARBAGE_REPLY!";
                                let _ = stream.write_all(garbage);
                                continue;
                            }

                            if req_line.starts_with("POST /digest") {
                                let mut body = vec![0u8; content_length];
                                let _ = reader.read_exact(&mut body);
                                if body.len() >= 32 {
                                    let mut fp = [0u8; 32];
                                    fp.copy_from_slice(&body[0..32]);

                                    let cal_url = format!("http://127.0.0.1:{}", port);
                                    let op1 = Op::Append(cal_url.as_bytes().to_vec());
                                    let out1 = op1.execute(&fp);
                                    let op2 = Op::Sha256;
                                    let commitment = op2.execute(&out1);

                                    let attest_step = Step {
                                        data: StepData::Attestation(Attestation::Pending {
                                            uri: cal_url,
                                        }),
                                        output: commitment.clone(),
                                        next: Vec::new(),
                                    };
                                    let step2 = Step {
                                        data: StepData::Op(op2),
                                        output: commitment,
                                        next: vec![attest_step],
                                    };
                                    let first_step = Step {
                                        data: StepData::Op(op1),
                                        output: out1,
                                        next: vec![step2],
                                    };
                                    let ts = Timestamp {
                                        start_digest: fp.to_vec(),
                                        first_step,
                                    };

                                    let mut ser = Serializer::new(Vec::new());
                                    ts.serialize(&mut ser).unwrap();
                                    let reply = ser.into_inner();

                                    let resp = format!(
                                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\n\r\n",
                                        reply.len()
                                    );
                                    let _ = stream.write_all(resp.as_bytes());
                                    let _ = stream.write_all(&reply);
                                }
                            } else if req_line.starts_with("GET /timestamp/") {
                                let parts: Vec<&str> = req_line.split_whitespace().collect();
                                if parts.len() > 1 {
                                    let path = parts[1];
                                    let commit_hex = path.trim_start_matches("/timestamp/");
                                    let lock = conf_clone.lock().unwrap();
                                    if let Some(&(height, header)) = lock.get(commit_hex) {
                                        let merkle_root = extract_merkle_root_from_header(&header);
                                        let commit_bytes = hex::decode(commit_hex).unwrap_or_default();

                                        let upgraded_step = Step {
                                            data: StepData::Attestation(Attestation::Bitcoin {
                                                height: height as usize,
                                            }),
                                            output: merkle_root.to_vec(),
                                            next: Vec::new(),
                                        };

                                        let upgraded_ts = Timestamp {
                                            start_digest: commit_bytes,
                                            first_step: upgraded_step,
                                        };

                                        let mut ser = Serializer::new(Vec::new());
                                        upgraded_ts.serialize(&mut ser).unwrap();
                                        let reply = ser.into_inner();

                                        let resp = format!(
                                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\n\r\n",
                                            reply.len()
                                        );
                                        let _ = stream.write_all(resp.as_bytes());
                                        let _ = stream.write_all(&reply);
                                    } else {
                                        let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
                                        let _ = stream.write_all(resp.as_bytes());
                                    }
                                }
                            } else if req_line.starts_with("GET /block-height/") {
                                let parts: Vec<&str> = req_line.split_whitespace().collect();
                                if parts.len() > 1 {
                                    let h_str = parts[1].trim_start_matches("/block-height/");
                                    let h_val: u64 = h_str.parse().unwrap_or(0);
                                    let lock = conf_clone.lock().unwrap();
                                    let mut found_header = None;
                                    for (_commit, &(height, header)) in lock.iter() {
                                        if height == h_val {
                                            found_header = Some(header);
                                            break;
                                        }
                                    }
                                    if let Some(header) = found_header {
                                        let h1 = Sha256::digest(&header);
                                        let h2 = Sha256::digest(&h1);
                                        let mut rev = h2.to_vec();
                                        rev.reverse();
                                        let hash_hex = hex::encode(rev);
                                        let resp = format!(
                                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\n\r\n{}",
                                            hash_hex.len(), hash_hex
                                        );
                                        let _ = stream.write_all(resp.as_bytes());
                                    } else {
                                        let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
                                        let _ = stream.write_all(resp.as_bytes());
                                    }
                                }
                            } else if req_line.starts_with("GET /block/") && req_line.contains("/header") {
                                let parts: Vec<&str> = req_line.split_whitespace().collect();
                                if parts.len() > 1 {
                                    let path = parts[1];
                                    let hash = path.trim_start_matches("/block/").trim_end_matches("/header");
                                    let lock = conf_clone.lock().unwrap();
                                    let mut found_header = None;
                                    for (_commit, &(_h, header)) in lock.iter() {
                                        let h1 = Sha256::digest(&header);
                                        let h2 = Sha256::digest(&h1);
                                        let mut rev = h2.to_vec();
                                        rev.reverse();
                                        if hex::encode(rev).eq_ignore_ascii_case(hash) {
                                            found_header = Some(header);
                                            break;
                                        }
                                    }
                                    if let Some(mut header) = found_header {
                                        if corrupt_esplora_clone.load(std::sync::atomic::Ordering::Relaxed) {
                                            header[0] ^= 0xff;
                                        }
                                        let header_hex = hex::encode(header);
                                        let resp = format!(
                                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\n\r\n{}",
                                            header_hex.len(), header_hex
                                        );
                                        let _ = stream.write_all(resp.as_bytes());
                                    } else {
                                        let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
                                        let _ = stream.write_all(resp.as_bytes());
                                    }
                                }
                            }
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(std::time::Duration::from_millis(10));
                        }
                        Err(_) => break,
                    }
                }
            });

            Self {
                port,
                confirmations,
                return_garbage,
                corrupt_esplora_header,
                running,
            }
        }

        pub fn url(&self) -> String {
            format!("http://127.0.0.1:{}", self.port)
        }

        pub fn set_return_garbage(&self, val: bool) {
            self.return_garbage
                .store(val, std::sync::atomic::Ordering::Relaxed);
        }

        pub fn set_corrupt_esplora_header(&self, val: bool) {
            self.corrupt_esplora_header
                .store(val, std::sync::atomic::Ordering::Relaxed);
        }

        pub fn make_mock_block_header(merkle_root: [u8; 32]) -> [u8; 80] {
            let mut header = [0u8; 80];
            header[0..4].copy_from_slice(&20u32.to_le_bytes());
            header[36..68].copy_from_slice(&merkle_root);
            header[68..72].copy_from_slice(&12345678u32.to_le_bytes());
            header[72..76].copy_from_slice(&0x207fffffu32.to_le_bytes());
            let target = target_from_nbits(0x207fffff);
            for nonce in 0u32..10000 {
                header[76..80].copy_from_slice(&nonce.to_le_bytes());
                let h1 = Sha256::digest(&header);
                let h2 = Sha256::digest(&h1);
                let mut rev = h2.to_vec();
                rev.reverse();
                if rev.as_slice() <= &target[..] {
                    break;
                }
            }
            header
        }

        pub fn confirm(&self, commitment_hex: &str, height: u64, header: [u8; 80]) {
            let mut lock = self.confirmations.lock().unwrap();
            lock.insert(commitment_hex.to_string(), (height, header));
        }
    }

    impl Drop for MockOtsCalendar {
        fn drop(&mut self) {
            self.running
                .store(false, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anchor::mock_calendar::MockOtsCalendar;
    use std::collections::HashMap;

    #[test]
    fn test_offline_confirmed_ots_fixture_against_bitcoin_header() {
        let ots_bytes = include_bytes!("../tests/fixtures/merkle2_confirmed.ots");
        let txt_bytes = include_bytes!("../tests/fixtures/merkle2.txt");
        let header_bytes = include_bytes!("../tests/fixtures/block_429808.header");

        let mut hasher = Sha256::new();
        hasher.update(txt_bytes);
        let expected_fp: [u8; 32] = hasher.finalize().into();

        let mut header_map = HashMap::new();
        let mut h = [0u8; 80];
        h.copy_from_slice(header_bytes);
        header_map.insert(429808u64, h);
        let src = BlockHeaderSource::OfflineMap(&header_map);

        let proof = AnchorProof {
            method: "opentimestamps".to_string(),
            fingerprint: hex::encode(expected_fp),
            proof_bytes: ots_bytes.to_vec(),
            submitted_at: 1475853000,
            status: "confirmed".to_string(),
            uri: Some("https://alice.btc.calendar.opentimestamps.org".to_string()),
            block_height: Some(429808),
            block_hash: None,
        };

        let report = verify_ots(&proof, &expected_fp, Some(&src))
            .expect("verify confirmed fixture");
        assert_eq!(report.status, "confirmed");
        assert_eq!(report.block_height, Some(429808));
        assert!(report.commitment_matched);
        assert_eq!(
            report.block_hash,
            Some("2145a857e9128f67ea00cd166464dc9655e6062ba0ea094a64d913a96c24c365".to_string())
        );
    }

    #[test]
    fn test_offline_pending_ots_fixture() {
        let ots_bytes = include_bytes!("../tests/fixtures/merkle2.txt.ots");
        let txt_bytes = include_bytes!("../tests/fixtures/merkle2.txt");

        let mut hasher = Sha256::new();
        hasher.update(txt_bytes);
        let expected_fp: [u8; 32] = hasher.finalize().into();

        let proof = AnchorProof {
            method: "opentimestamps".to_string(),
            fingerprint: hex::encode(expected_fp),
            proof_bytes: ots_bytes.to_vec(),
            submitted_at: 1475850000,
            status: "pending".to_string(),
            uri: Some("https://alice.btc.calendar.opentimestamps.org".to_string()),
            block_height: None,
            block_hash: None,
        };

        let empty_map = HashMap::new();
        let src = BlockHeaderSource::OfflineMap(&empty_map);

        let report = verify_ots(&proof, &expected_fp, Some(&src))
            .expect("verify pending fixture");
        assert_eq!(report.status, "pending");
        assert!(report.commitment_matched);
    }

    #[test]
    #[ignore]
    fn test_fetch_live_freetsa() {
        let token = std::fs::read("tests/fixtures/digicert_response.tsr").unwrap();
        let resp = TimeStampResp::from_der(&token).unwrap();
        let ts_token = resp.time_stamp_token.unwrap();
        let sd = SignedData::from_der(&ts_token.content.to_der().unwrap()).unwrap();
        println!("econtent len: {:?}", sd.encap_content_info.econtent.as_ref().map(|v| v.value().len()));
        if let Some(ec) = &sd.encap_content_info.econtent {
            println!("econtent hex: {}", hex::encode(ec.value()));
            let tst_res = TstInfo::from_der(ec.value());
            println!("tst_res: {:?}", tst_res.as_ref().map(|t| t.version));
            if let Err(e) = tst_res {
                println!("tst error: {:?}", e);
            }
        }
    }

    #[test]
    fn test_ots_tampered_proof_fails_verification() {
        let ots_bytes = include_bytes!("../tests/fixtures/merkle2_confirmed.ots");
        let txt_bytes = include_bytes!("../tests/fixtures/merkle2.txt");

        let mut hasher = Sha256::new();
        hasher.update(txt_bytes);
        let expected_fp: [u8; 32] = hasher.finalize().into();

        let mut tampered = ots_bytes.to_vec();
        // Mutate byte in the proof path
        tampered[100] ^= 0xFF;

        let proof = AnchorProof {
            method: "opentimestamps".to_string(),
            fingerprint: hex::encode(expected_fp),
            proof_bytes: tampered,
            submitted_at: 1475853000,
            status: "confirmed".to_string(),
            uri: None,
            block_height: Some(429808),
            block_hash: None,
        };

        let empty_map = HashMap::new();
        let src = BlockHeaderSource::OfflineMap(&empty_map);

        let res = verify_ots(&proof, &expected_fp, Some(&src));
        assert!(res.is_err(), "Tampered proof MUST fail verification");
    }

    #[test]
    fn test_ots_forged_bitcoin_attestation_fails_verification() {
        let ots_bytes = include_bytes!("../tests/fixtures/merkle2_confirmed.ots");
        let txt_bytes = include_bytes!("../tests/fixtures/merkle2.txt");

        let mut hasher = Sha256::new();
        hasher.update(txt_bytes);
        let expected_fp: [u8; 32] = hasher.finalize().into();

        // Pass a block header whose Merkle root does NOT match the proof's computed Merkle root
        let mut fake_header = [0u8; 80];
        fake_header[36..68].copy_from_slice(&[0xEEu8; 32]); // forged root
        let mut header_map = HashMap::new();
        header_map.insert(429808u64, fake_header);
        let src = BlockHeaderSource::OfflineMap(&header_map);

        let proof = AnchorProof {
            method: "opentimestamps".to_string(),
            fingerprint: hex::encode(expected_fp),
            proof_bytes: ots_bytes.to_vec(),
            submitted_at: 1475853000,
            status: "confirmed".to_string(),
            uri: None,
            block_height: Some(429808),
            block_hash: None,
        };

        let res = verify_ots(&proof, &expected_fp, Some(&src));
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert!(err.contains("Merkle root mismatch"));
    }

    #[test]
    fn test_ots_calendar_returns_garbage_fails_submission() {
        let cal1 = MockOtsCalendar::start();
        let cal2 = MockOtsCalendar::start();
        cal1.set_return_garbage(true);
        cal2.set_return_garbage(true);

        let calendars = vec![cal1.url(), cal2.url()];
        let fp = [0x77u8; 32];

        let res = submit_ots(fp, &calendars);
        assert!(res.is_err(), "Garbage calendar reply MUST fail submission without fabrication");
    }

    #[test]
    fn test_ots_mock_calendar_pending_and_upgrade() {
        let cal1 = MockOtsCalendar::start();
        let cal2 = MockOtsCalendar::start();

        let calendars = vec![cal1.url(), cal2.url()];
        let fingerprint = [0x55u8; 32];

        // 1. Submit to calendars
        let proofs = submit_ots(fingerprint, &calendars).expect("submit ots");
        assert_eq!(proofs.len(), 2, "Stored each returned timestamp");
        let mut proof = proofs[0].clone();
        assert_eq!(proof.status, "pending");
        assert_eq!(proof.fingerprint, hex::encode(fingerprint));

        // 2. Verify pending proof evaluates correctly
        let empty_map = HashMap::new();
        let src = BlockHeaderSource::OfflineMap(&empty_map);
        let rep = verify_ots(&proof, &fingerprint, Some(&src)).expect("verify pending");
        assert_eq!(rep.status, "pending");

        // 3. Upgrade before confirmation -> still pending (returns false)
        let upgraded = upgrade_ots(&mut proof, Some(&cal1.url())).expect("upgrade check");
        assert!(!upgraded);
        assert_eq!(proof.status, "pending");

        // 4. Extract commitment and confirm in calendar
        let detached = DetachedTimestampFile::from_reader(&proof.proof_bytes[..]).unwrap();
        let atts = collect_attestations(&detached.timestamp.first_step);
        let commitment = atts[0].1;

        let block_height = 890123u64;
        let mut merkle_root = [0u8; 32];
        if commitment.len() >= 32 {
            merkle_root.copy_from_slice(&commitment[0..32]);
        } else {
            merkle_root[0..commitment.len()].copy_from_slice(commitment);
        }

        let header = MockOtsCalendar::make_mock_block_header(merkle_root);
        cal1.confirm(&hex::encode(commitment), block_height, header);

        // 5. Upgrade again -> confirmed!
        let upgraded = upgrade_ots(&mut proof, Some(&cal1.url())).expect("upgrade check");
        assert!(upgraded);
        assert_eq!(proof.status, "confirmed");
        assert_eq!(proof.block_height, Some(block_height));

        // 6. Verify confirmed proof with header
        let mut map = HashMap::new();
        map.insert(block_height, header);
        let src_confirmed = BlockHeaderSource::OfflineMap(&map);
        let rep = verify_ots(&proof, &fingerprint, Some(&src_confirmed)).expect("verify confirmed");
        assert_eq!(rep.status, "confirmed");
        assert_eq!(rep.block_height, Some(block_height));
    }

    #[test]
    #[ignore]
    fn test_official_ots_cli_interop() {
        use std::process::Command;
        let dir = std::env::temp_dir();
        let doc_path = dir.join("test_interop_doc.txt");
        let ots_path = dir.join("test_interop_doc.txt.ots");

        let doc_bytes = b"OpenTimestamps official interop test payload";
        std::fs::write(&doc_path, doc_bytes).expect("write doc");

        let mut hasher = Sha256::new();
        hasher.update(doc_bytes);
        let fp: [u8; 32] = hasher.finalize().into();

        // Submit to public calendars
        let cals = vec![
            "https://a.pool.opentimestamps.org".to_string(),
            "https://b.pool.opentimestamps.org".to_string(),
        ];
        let proofs = submit_ots(fp, &cals).expect("submit to public calendars");
        assert!(!proofs.is_empty());

        std::fs::write(&ots_path, &proofs[0].proof_bytes).expect("write ots");

        // Execute official opentimestamps-client ots verify
        let status = Command::new("ots")
            .arg("verify")
            .arg(&ots_path)
            .status();

        match status {
            Ok(s) => assert!(s.success(), "ots verify failed on stamped file"),
            Err(e) => eprintln!("ots command not available locally: {}", e),
        }

        let _ = std::fs::remove_file(&doc_path);
        let _ = std::fs::remove_file(&ots_path);
    }

    #[test]
    fn test_anchor_sealed_archive_roundtrip_and_tamper_fails() {
        use crate::{TileType, TileWriter};
        use s3a_core::TelemetryRecord;

        let dir = std::env::temp_dir();
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let raw_path = dir.join(format!("test_raw_{}.s3a", unique));
        let sealed_path = dir.join(format!("test_sealed_{}.s3a", unique));

        {
            let mut writer = TileWriter::create(&raw_path).expect("create raw");
            let rec = vec![TelemetryRecord::new(100, 1, 1, 42.0)];
            writer.write_hyper_tile(TileType::TELEMETRY, &rec, None, None).expect("write");
            drop(writer);
        }

        crate::seal_archive(&raw_path, &sealed_path, 16, 2).expect("seal");

        let initial_fp = compute_sealed_file_fingerprint(&sealed_path).expect("fp");

        // Verify initial sealed file
        let report = crate::verify_sealed_archive(&sealed_path, None).expect("verify");
        assert!(report.is_sealed);
        assert!(report.anchors.is_empty());

        // Start mock calendar and submit anchor
        let cal1 = MockOtsCalendar::start();
        let cal2 = MockOtsCalendar::start();
        let calendars = vec![cal1.url(), cal2.url()];

        let proofs = submit_ots(initial_fp, &calendars).expect("submit");
        for proof in proofs {
            attach_anchor_to_file(&sealed_path, proof).expect("attach");
        }

        // Verify sealed file with anchor
        let report2 = crate::verify_sealed_archive(&sealed_path, None).expect("verify");
        assert!(report2.is_sealed);
        assert_eq!(report2.anchors.len(), 2);
        assert_eq!(report2.anchors[0].fingerprint, hex::encode(initial_fp));
        assert_eq!(report2.anchors[0].status, "pending");

        // Fingerprint unchanged after attaching anchor
        let fp_after = compute_sealed_file_fingerprint(&sealed_path).expect("fp after");
        assert_eq!(initial_fp, fp_after);

        // Tamper 1 byte in the directory inside the sealed file
        let mut tampered_bytes = std::fs::read(&sealed_path).expect("read");
        let endcap_offset = tampered_bytes.len() - s3a_core::SEALED_ENDCAP_SIZE;
        let endcap: s3a_core::SealedEndCap = bytemuck::pod_read_unaligned(&tampered_bytes[endcap_offset..]);
        let dir_offset = endcap.footer_offset as usize;
        tampered_bytes[dir_offset + 5] ^= 0xFF;
        let tampered_path = dir.join(format!("test_tampered_{}.s3a", unique));
        std::fs::write(&tampered_path, &tampered_bytes).expect("write tampered");

        let tampered_fp = compute_sealed_bytes_fingerprint(&tampered_bytes);
        if let Ok(new_fp) = tampered_fp {
            assert_ne!(new_fp, initial_fp, "Tampered fingerprint MUST differ from anchor proof");
        }

        let _ = std::fs::remove_file(&raw_path);
        let _ = std::fs::remove_file(&sealed_path);
        let _ = std::fs::remove_file(&tampered_path);
    }

    #[test]
    fn test_rfc3161_real_fixture_verifies_ok() {
        let tsr_bytes = include_bytes!("../tests/fixtures/digicert_response.tsr");
        let ca_bytes = include_bytes!("../tests/fixtures/digicert_root_ca.pem");
        let trust_store = parse_pem_certificates(std::str::from_utf8(ca_bytes).unwrap()).unwrap();

        let fp = [0x42u8; 32];
        let proof = AnchorProof {
            method: "rfc3161".to_string(),
            fingerprint: hex::encode(fp),
            proof_bytes: tsr_bytes.to_vec(),
            submitted_at: 1775510000,
            status: "unverified".to_string(),
            uri: Some("http://timestamp.digicert.com".to_string()),
            block_height: None,
            block_hash: None,
        };

        // Real fixture verifies OK against real DigiCert root CA
        let ok = verify_rfc3161(&proof, &fp, Some(&trust_store)).expect("verify rfc3161");
        assert!(ok, "DigiCert fixture must verify OK with DigiCert root CA");

        // Without trust store supplied => returns Ok(false) (status unverified, never confirmed)
        let unverified = verify_rfc3161(&proof, &fp, None).expect("verify without trust store");
        assert!(!unverified, "No trust store must return Ok(false)");
    }

    #[test]
    fn test_rfc3161_tampered_signature_fails() {
        let tsr_bytes = include_bytes!("../tests/fixtures/digicert_response.tsr");
        let ca_bytes = include_bytes!("../tests/fixtures/digicert_root_ca.pem");
        let trust_store = parse_pem_certificates(std::str::from_utf8(ca_bytes).unwrap()).unwrap();

        let fp = [0x42u8; 32];
        let resp = TimeStampResp::from_der(tsr_bytes).unwrap();
        let token = resp.time_stamp_token.unwrap();
        let sd = SignedData::from_der(&token.content.to_der().unwrap()).unwrap();
        let sig_bytes = sd.signer_infos.0.as_slice()[0].signature.as_bytes();
        let sig_pos = tsr_bytes.windows(sig_bytes.len()).position(|w| w == sig_bytes).unwrap();

        let mut tampered_bytes = tsr_bytes.to_vec();
        tampered_bytes[sig_pos + 10] ^= 0xff;

        let proof = AnchorProof {
            method: "rfc3161".to_string(),
            fingerprint: hex::encode(fp),
            proof_bytes: tampered_bytes,
            submitted_at: 1775510000,
            status: "unverified".to_string(),
            uri: Some("http://timestamp.digicert.com".to_string()),
            block_height: None,
            block_hash: None,
        };

        let res = verify_rfc3161(&proof, &fp, Some(&trust_store));
        assert!(res.is_err(), "Tampered signature MUST fail verification");
    }

    #[test]
    fn test_rfc3161_resigned_with_self_made_key_fails_trust_store() {
        use rsa::pkcs1v15::SigningKey;
        use signature::{SignatureEncoding, Signer};

        let tsr_bytes = include_bytes!("../tests/fixtures/digicert_response.tsr");
        let ca_bytes = include_bytes!("../tests/fixtures/digicert_root_ca.pem");
        let trust_store = parse_pem_certificates(std::str::from_utf8(ca_bytes).unwrap()).unwrap();

        let fp = [0x42u8; 32];

        // Parse token and re-sign with a self-made RSA key
        let resp = TimeStampResp::from_der(tsr_bytes).unwrap();
        let token = resp.time_stamp_token.clone().unwrap();
        let mut sd = SignedData::from_der(&token.content.to_der().unwrap()).unwrap();

        let mut rng = rand::thread_rng();
        let priv_key = rsa::RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let signing_key = SigningKey::<sha2::Sha256>::new(priv_key);

        let mut signer_info = sd.signer_infos.0.as_slice()[0].clone();
        let signed_attrs = signer_info.signed_attrs.as_ref().unwrap();
        let mut set_der = signed_attrs.to_der().unwrap();
        if set_der[0] == 0xa0 {
            set_der[0] = 0x31;
        }
        let forged_sig = signing_key.sign(&set_der);

        // Replace the signature with the forged one
        signer_info.signature = der::asn1::OctetString::new(forged_sig.to_vec()).unwrap();
        sd.signer_infos = cms::signed_data::SignerInfos::try_from(vec![signer_info]).unwrap();

        let mut forged_token = token.clone();
        forged_token.content = der::Any::from_der(&sd.to_der().unwrap()).unwrap();
        let mut forged_resp = resp.clone();
        forged_resp.time_stamp_token = Some(forged_token);
        let forged_bytes = forged_resp.to_der().unwrap();

        let proof = AnchorProof {
            method: "rfc3161".to_string(),
            fingerprint: hex::encode(fp),
            proof_bytes: forged_bytes,
            submitted_at: 1775510000,
            status: "unverified".to_string(),
            uri: Some("http://timestamp.digicert.com".to_string()),
            block_height: None,
            block_hash: None,
        };

        // When verified against the real DigiCert trust store, the forged token MUST fail
        let res = verify_rfc3161(&proof, &fp, Some(&trust_store));
        assert!(res.is_err(), "Token re-signed with self-made key MUST fail against real trust store: {:?}", res);
    }

    #[test]
    fn test_rfc3161_nonce_mismatch_rejected() {
        use std::net::TcpListener;
        use std::io::{BufReader, BufRead, Write};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server_url = format!("http://127.0.0.1:{}", port);

        // Background thread serving one request with mismatched nonce
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            let mut content_length = 0usize;
            while reader.read_line(&mut line).is_ok() {
                if line.trim().is_empty() {
                    break;
                }
                if line.to_lowercase().starts_with("content-length:") {
                    if let Some(val) = line.split(':').nth(1) {
                        content_length = val.trim().parse().unwrap_or(0);
                    }
                }
                line.clear();
            }
            let mut body = vec![0u8; content_length];
            let _ = reader.read_exact(&mut body);

            // Respond with a TSR that has an explicit mismatched nonce
            let tsr_bytes = include_bytes!("../tests/fixtures/digicert_response.tsr");
            let mut resp = TimeStampResp::from_der(tsr_bytes).unwrap();
            let mut token = resp.time_stamp_token.unwrap();
            let mut sd = SignedData::from_der(&token.content.to_der().unwrap()).unwrap();
            let mut tst = TstInfo::from_der(sd.encap_content_info.econtent.as_ref().unwrap().value()).unwrap();
            let mismatch_nonce = der::asn1::Int::new(&[0x12, 0x34, 0x56, 0x78]).unwrap();
            tst.nonce = Some(mismatch_nonce);
            sd.encap_content_info.econtent = Some(der::Any::new(der::Tag::OctetString, tst.to_der().unwrap()).unwrap());
            token.content = der::Any::from_der(&sd.to_der().unwrap()).unwrap();
            resp.time_stamp_token = Some(token);
            let reply_bytes = resp.to_der().unwrap();

            let resp_hdr = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/timestamp-reply\r\nContent-Length: {}\r\n\r\n",
                reply_bytes.len()
            );
            let _ = stream.write_all(resp_hdr.as_bytes());
            let _ = stream.write_all(&reply_bytes);
        });

        let fp = [0x42u8; 32];
        let res = submit_rfc3161(fp, &server_url);
        assert!(res.is_err(), "Nonce mismatch MUST be rejected by submit_rfc3161");
        let err_str = res.unwrap_err().to_string();
        assert!(err_str.contains("Nonce mismatch"), "Expected nonce mismatch error, got: {}", err_str);
    }

    #[test]
    fn test_esplora_mismatched_block_header_hash_fails() {
        let cal = MockOtsCalendar::start();
        let merkle_root = [0x88u8; 32];
        let header = MockOtsCalendar::make_mock_block_header(merkle_root);
        let block_height = 890123u64;
        cal.confirm("fake_commit", block_height, header);

        // Corrupt header returned by Esplora route
        cal.set_corrupt_esplora_header(true);

        let src = BlockHeaderSource::Esplora(cal.url());
        let res = fetch_block_header(block_height, &src);
        assert!(res.is_err(), "Mismatched block header hash MUST fail");
        let err = res.unwrap_err().to_string();
        assert!(
            err.contains("Block header hash mismatch"),
            "Expected hash mismatch error, got: {}",
            err
        );
    }

    #[test]
    fn test_rfc3161_forged_root_subject_serial_different_key_fails() {
        use rsa::pkcs1::EncodeRsaPublicKey;
        use rsa::pkcs1v15::SigningKey;
        use signature::{SignatureEncoding, Signer};

        let ca_bytes = include_bytes!("../tests/fixtures/digicert_root_ca.pem");
        let trust_store = parse_pem_certificates(std::str::from_utf8(ca_bytes).unwrap()).unwrap();
        let real_root = &trust_store[0];

        // Attacker creates a certificate copying real_root's subject and serial_number, but with their own RSA key
        let mut rng = rand::thread_rng();
        let attacker_priv = rsa::RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let attacker_pub = attacker_priv.to_public_key();
        let attacker_spki_doc = attacker_pub.to_pkcs1_der().unwrap();

        let mut forged_tbs = real_root.tbs_certificate.clone();
        forged_tbs.subject_public_key_info.subject_public_key =
            der::asn1::BitString::from_bytes(attacker_spki_doc.as_bytes()).unwrap();

        let forged_tbs_der = forged_tbs.to_der().unwrap();
        let signing_key = SigningKey::<sha2::Sha256>::new(attacker_priv);
        let forged_sig = signing_key.sign(&forged_tbs_der);

        let forged_cert = x509_cert::Certificate {
            tbs_certificate: forged_tbs,
            signature_algorithm: real_root.signature_algorithm.clone(),
            signature: der::asn1::BitString::from_bytes(forged_sig.to_vec().as_slice()).unwrap(),
        };

        let gen_time = real_root.tbs_certificate.validity.not_before.to_system_time()
            + std::time::Duration::from_secs(3600);

        // 1. Used as leaf -> MUST fail (full DER and SPKI differ from trust root)
        let res_leaf = verify_certificate_chain(&forged_cert, &[], &trust_store, gen_time);
        assert!(res_leaf.is_err(), "Forged cert copying root subject+serial MUST fail as leaf: {:?}", res_leaf);

        // 2. Used as intermediate / CA in certificate pool -> MUST fail
        let mut leaf_tbs = real_root.tbs_certificate.clone();
        leaf_tbs.serial_number = x509_cert::serial_number::SerialNumber::new(&[0x11, 0x22]).unwrap();
        // Give leaf its own key and distinct subject so it is not mistaken for the trust root
        let leaf_priv = rsa::RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let leaf_pub = leaf_priv.to_public_key();
        let leaf_spki_doc = leaf_pub.to_pkcs1_der().unwrap();
        leaf_tbs.subject_public_key_info.subject_public_key =
            der::asn1::BitString::from_bytes(leaf_spki_doc.as_bytes()).unwrap();
        leaf_tbs.issuer = forged_cert.tbs_certificate.subject.clone();

        let leaf_tbs_der = leaf_tbs.to_der().unwrap();
        let leaf_sig = signing_key.sign(&leaf_tbs_der);
        let attacker_leaf = x509_cert::Certificate {
            tbs_certificate: leaf_tbs,
            signature_algorithm: real_root.signature_algorithm.clone(),
            signature: der::asn1::BitString::from_bytes(leaf_sig.to_vec().as_slice()).unwrap(),
        };

        let res_inter = verify_certificate_chain(&attacker_leaf, &[forged_cert.clone()], &trust_store, gen_time);
        assert!(res_inter.is_err(), "Forged cert copying root subject+serial MUST fail as intermediate: {:?}", res_inter);

        // 3. Confirm is_trust_root directly rejects the forged cert
        assert!(!is_trust_root(&forged_cert, &trust_store).unwrap(), "forged_cert must not be recognized as trust root");
    }

    #[test]
    fn test_rfc3161_gentime_outside_validity_window_fails() {
        use rsa::pkcs1v15::SigningKey;
        use signature::{SignatureEncoding, Signer};

        let tsr_bytes = include_bytes!("../tests/fixtures/digicert_response.tsr");
        let ca_bytes = include_bytes!("../tests/fixtures/digicert_root_ca.pem");
        let trust_store = parse_pem_certificates(std::str::from_utf8(ca_bytes).unwrap()).unwrap();

        // 1. Direct unit test on check_cert_validity with expired / not yet valid time
        let cert = &trust_store[0];
        let pre_epoch = std::time::UNIX_EPOCH;
        assert!(check_cert_validity(cert, pre_epoch).is_err(), "Pre-epoch genTime must fail validity check");

        // 2. Full end-to-end test on re-signed fixture with shifted genTime
        let fp = [0x42u8; 32];
        let resp = TimeStampResp::from_der(tsr_bytes).unwrap();
        let token = resp.time_stamp_token.clone().unwrap();
        let mut sd = SignedData::from_der(&token.content.to_der().unwrap()).unwrap();
        let mut tst = TstInfo::from_der(sd.encap_content_info.econtent.as_ref().unwrap().value()).unwrap();

        // Shift gen_time to year 1999 (far outside validity window)
        let shifted_date = der::DateTime::new(1999, 1, 1, 0, 0, 0).unwrap();
        tst.gen_time = der::asn1::GeneralizedTime::from_date_time(shifted_date);

        let tst_der = tst.to_der().unwrap();
        sd.encap_content_info.econtent = Some(der::Any::new(der::Tag::OctetString, tst_der.clone()).unwrap());

        let mut signer_info = sd.signer_infos.0.as_slice()[0].clone();
        let id_message_digest = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.4");
        let new_digest = Sha256::digest(&tst_der);

        let mut new_attrs = Vec::new();
        for attr in signer_info.signed_attrs.as_ref().unwrap().iter() {
            if attr.oid == id_message_digest {
                let oct = OctetString::new(new_digest.to_vec()).unwrap();
                let mut vals = der::asn1::SetOfVec::new();
                vals.insert(der::Any::from_der(&oct.to_der().unwrap()).unwrap()).unwrap();
                new_attrs.push(x509_cert::attr::Attribute {
                    oid: id_message_digest,
                    values: vals,
                });
            } else {
                new_attrs.push(attr.clone());
            }
        }
        signer_info.signed_attrs = Some(der::asn1::SetOfVec::try_from(new_attrs).unwrap());

        let mut set_der = signer_info.signed_attrs.as_ref().unwrap().to_der().unwrap();
        if set_der[0] == 0xa0 {
            set_der[0] = 0x31;
        }

        let mut rng = rand::thread_rng();
        let priv_key = rsa::RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let signing_key = SigningKey::<sha2::Sha256>::new(priv_key);
        let forged_sig = signing_key.sign(&set_der);
        signer_info.signature = der::asn1::OctetString::new(forged_sig.to_vec()).unwrap();
        sd.signer_infos = cms::signed_data::SignerInfos::try_from(vec![signer_info]).unwrap();

        let mut forged_token = token.clone();
        forged_token.content = der::Any::from_der(&sd.to_der().unwrap()).unwrap();
        let mut forged_resp = resp.clone();
        forged_resp.time_stamp_token = Some(forged_token);
        let forged_bytes = forged_resp.to_der().unwrap();

        let proof = AnchorProof {
            method: "rfc3161".to_string(),
            fingerprint: hex::encode(fp),
            proof_bytes: forged_bytes,
            submitted_at: 1775510000,
            status: "unverified".to_string(),
            uri: Some("http://timestamp.digicert.com".to_string()),
            block_height: None,
            block_hash: None,
        };

        let res = verify_rfc3161(&proof, &fp, Some(&trust_store));
        assert!(res.is_err(), "Shifted genTime outside validity window MUST fail: {:?}", res);
        let err_msg = res.unwrap_err();
        assert!(err_msg.contains("validity window"), "Expected validity window error, got: {}", err_msg);
    }

    #[test]
    fn test_subject_key_identifier_matcher() {
        let ca_bytes = include_bytes!("../tests/fixtures/digicert_root_ca.pem");
        let trust_store = parse_pem_certificates(std::str::from_utf8(ca_bytes).unwrap()).unwrap();
        let cert = &trust_store[0];

        // Find SKI extension (2.5.29.14) in the cert
        let ski_oid = ObjectIdentifier::new_unwrap("2.5.29.14");
        let ext = cert.tbs_certificate.extensions.as_ref().unwrap()
            .iter().find(|e| e.extn_id == ski_oid).expect("Root CA has SKI extension");

        // The outer extn_value is an OCTET STRING containing the DER-encoded inner OCTET STRING
        let inner_ski = OctetString::from_der(ext.extn_value.as_bytes()).unwrap();

        // 1. Matches when comparing against inner decoded key identifier octets
        assert!(cert_matches_ski(cert, inner_ski.as_bytes()), "Decoded inner SKI must match");

        // 2. Fails when passing raw extn_value bytes (which include outer ASN.1 tag 0x04 and length prefix)
        assert!(!cert_matches_ski(cert, ext.extn_value.as_bytes()), "Raw extn_value with tag/len must NOT match");

        // 3. Fails when passing wrong SKI bytes
        assert!(!cert_matches_ski(cert, &[0x99; 20]), "Mismatched SKI must NOT match");
    }
}

