//! Shared document record store (Tile 23, `DOCUMENT_STORE` tiles).
//!
//! Prompt 010: the write path stores each document tile at its smallest
//! fitting slab size class (a "packed" archive) when the vault is empty or
//! already packed, and keeps the legacy 128 KiB stride when the vault
//! already holds legacy tiles. Reads and listings work on both layouts
//! through [`MmapReader`], which walks packed slots at open.
//!
//! Prompt 011: a document whose `64 + key + content` exceeds one tile
//! payload (`HYPER_TILE_PAYLOAD_SIZE`, 130,560 bytes) is stored as a
//! chunked document — one logical file across multiple tiles/slabs:
//! * one `DOC_KIND_CHUNK_DATA` record per chunk, under the derived key
//!   `"{key}\u{1f}chunk\u{1f}{index:06}"` (index 0..n-1, in order). Each
//!   chunk record fits in one slab class (prompt 010 rules); the last chunk
//!   uses the smallest class that fits it, not a forced 128 KiB pad. The
//!   128 KiB ceiling applies per chunk / slot, never to the logical file.
//! * one `DOC_KIND_CHUNK_MANIFEST` record under the logical key, written
//!   after all chunks, carrying the versioned manifest JSON:
//!   `{"v":1,"key":...,"kind":<logical doc kind>,"total_bytes":...,
//!   "sha256":"<hex over the concatenated logical bytes>","chunk_count":n}`.
//!   The whole-file SHA-256 is computed here from the logical bytes; a
//!   caller-supplied hash is never trusted as the stored value.
//! * reads reassemble chunks in order and fail closed on a missing chunk,
//!   a chunk CRC32 mismatch, a wrong chunk length, or a whole-file SHA-256
//!   mismatch. Chunks are written before the manifest, so a crash mid-write
//!   leaves orphan chunks but never a manifest pointing at incomplete data.
//! * record roots / Merkle proofs cover every chunk automatically: chunks
//!   are ordinary `DOCUMENT_STORE` tiles, so `seal` (with `--record-roots`),
//!   `verify`, `prove`, `flatten`, and `unflatten` handle them with no
//!   special casing.
//!
//! `s3a-cli` (`first_run`) and `s3a-pack-tdp` (`store`) both delegate to
//! this module so the layout decision lives in exactly one place.

use crate::{MmapReader, TileWriter};
use s3a_core::{
    compute_key_hash, DocumentRecordHeader, TileType, DOC_FLAG_ACTIVE, DOC_FLAG_IS_JSON,
    DOC_KIND_BINARY, DOC_KIND_CHUNK_DATA, DOC_KIND_CHUNK_MANIFEST, DOC_KIND_GENERIC_KV,
    DOC_KIND_JSON, HYPER_TILE_PAYLOAD_SIZE,
};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

fn infer_doc_kind(content: &[u8]) -> u32 {
    let is_json = serde_json::from_slice::<serde_json::Value>(content).is_ok();
    let is_binary = !is_json && std::str::from_utf8(content).is_err();
    if is_json {
        DOC_KIND_JSON
    } else if is_binary {
        DOC_KIND_BINARY
    } else {
        DOC_KIND_GENERIC_KV
    }
}

/// One chunk of a chunked document, as reported by listings.
pub struct ChunkMeta {
    pub index: usize,
    pub total_chunks: usize,
    pub chunk_bytes: usize,
    pub tile_index: u64,
    pub tile_id: u64,
    pub crc32: u32,
}

pub struct StoredDocumentMeta {
    pub key: String,
    /// Logical document kind (16/17/18): the record kind for single-slot
    /// documents, the manifest's stored kind for chunked ones.
    pub kind: u32,
    pub size_bytes: usize,
    pub timestamp_ms: u64,
    /// Hex SHA-256 of the logical bytes: computed over the content for
    /// single-slot documents, taken from the manifest for chunked ones.
    pub sha256: String,
    /// 1 for a single-slot document, n for a chunked one.
    pub chunk_count: usize,
    /// Empty for a single-slot document; one entry per chunk, in order,
    /// for a chunked one.
    pub chunks: Vec<ChunkMeta>,
}

/// Derived storage key for chunk `index` of logical key `key`.
/// `\u{1f}` is the ASCII unit separator; chunk records are distinguished
/// from user documents by their `DOC_KIND_CHUNK_DATA` kind, not by the key.
fn chunk_key(key: &str, index: usize) -> String {
    format!("{key}\u{1f}chunk\u{1f}{index:06}")
}

/// Largest chunk content that fits in one tile payload for this key:
/// 64-byte record header + chunk key + chunk bytes <= `HYPER_TILE_PAYLOAD_SIZE`.
fn max_chunk_content_bytes(key: &str) -> Result<usize, String> {
    let header_size = std::mem::size_of::<DocumentRecordHeader>();
    let chunk_key_len = key.len() + 13; // "\u{1f}chunk\u{1f}" (7 bytes) + 6-digit index
    HYPER_TILE_PAYLOAD_SIZE
        .checked_sub(header_size + chunk_key_len)
        .filter(|&n| n > 0)
        .ok_or_else(|| format!("key '{}' is too long to span across tiles", key))
}

/// Versioned chunk-manifest JSON (record-layout version 1).
fn manifest_json(
    key: &str,
    doc_kind: u32,
    total_bytes: usize,
    sha256: &str,
    chunk_count: usize,
) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "v": 1,
        "key": key,
        "kind": doc_kind,
        "total_bytes": total_bytes,
        "sha256": sha256,
        "chunk_count": chunk_count,
    }))
    .expect("manifest JSON serialization cannot fail")
}

struct ParsedManifest {
    kind: u32,
    total_bytes: usize,
    sha256: String,
    chunk_count: usize,
}

fn parse_manifest(content: &[u8], key: &str) -> Result<ParsedManifest, String> {
    let v: serde_json::Value = serde_json::from_slice(content)
        .map_err(|e| format!("chunk manifest for '{}' is not valid JSON: {}", key, e))?;
    if v.get("v").and_then(|x| x.as_u64()) != Some(1) {
        return Err(format!(
            "chunk manifest for '{}' has an unsupported layout version",
            key
        ));
    }
    if v.get("key").and_then(|x| x.as_str()) != Some(key) {
        return Err(format!("chunk manifest key mismatch for '{}'", key));
    }
    let kind = v
        .get("kind")
        .and_then(|x| x.as_u64())
        .ok_or_else(|| format!("chunk manifest for '{}' is missing its document kind", key))?
        as u32;
    let total_bytes = v
        .get("total_bytes")
        .and_then(|x| x.as_u64())
        .ok_or_else(|| {
            format!(
                "chunk manifest for '{}' is missing its total byte count",
                key
            )
        })? as usize;
    let sha256 = v
        .get("sha256")
        .and_then(|x| x.as_str())
        .ok_or_else(|| format!("chunk manifest for '{}' is missing its SHA-256", key))?
        .to_string();
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!(
            "chunk manifest for '{}' carries a malformed SHA-256",
            key
        ));
    }
    let chunk_count = v
        .get("chunk_count")
        .and_then(|x| x.as_u64())
        .ok_or_else(|| format!("chunk manifest for '{}' is missing its chunk count", key))?
        as usize;
    if chunk_count == 0 {
        return Err(format!("chunk manifest for '{}' claims zero chunks", key));
    }
    Ok(ParsedManifest {
        kind,
        total_bytes,
        sha256,
        chunk_count,
    })
}

/// Stores `content` under `key`, inferring the document kind from the
/// content (JSON, binary, or generic key-value) exactly as the CLI has
/// always done.
pub fn write_document_record(path: &Path, key: &str, content: &[u8]) -> Result<(), String> {
    write_document_record_kind(path, infer_doc_kind(content), key, content)
}

/// Writes one document record in a single tile, inferring the document kind
/// from the content exactly as `write_document_record` does. Fails if the
/// record does not fit in one tile payload — the prompt 010 ceiling, kept
/// as a primitive so spanning callers and tests can rely on it.
pub fn write_single_slot_document_record(
    path: &Path,
    key: &str,
    content: &[u8],
) -> Result<(), String> {
    write_single_slot_at(path, infer_doc_kind(content), key, content, now_ms())
}

/// Stores `content` under `key` with an explicit document kind (the
/// `s3a-pack-tdp` form: PDF/STEP payloads and their JSON sidecars).
/// Documents that fit in one tile take the single-slot path; larger ones
/// span across chunk tiles plus a manifest (prompt 011).
pub fn write_document_record_kind(
    path: &Path,
    doc_kind: u32,
    key: &str,
    content: &[u8],
) -> Result<(), String> {
    let header_size = std::mem::size_of::<DocumentRecordHeader>();
    if header_size + key.len() + content.len() <= HYPER_TILE_PAYLOAD_SIZE {
        return write_single_slot_at(path, doc_kind, key, content, now_ms());
    }

    // Prompt 011: span the document across chunk tiles plus a manifest.
    let max_chunk = max_chunk_content_bytes(key)?;
    let chunk_count = content.len().div_ceil(max_chunk);
    let whole_sha256 = sha256_hex(content);
    let timestamp_ms = now_ms();
    // Chunks first, manifest last: a manifest is only committed once every
    // chunk is on disk, so a crash mid-write leaves orphan chunks but never
    // a manifest pointing at incomplete data.
    for (index, chunk) in content.chunks(max_chunk).enumerate() {
        write_single_slot_at(
            path,
            DOC_KIND_CHUNK_DATA,
            &chunk_key(key, index),
            chunk,
            timestamp_ms,
        )?;
    }
    let manifest = manifest_json(key, doc_kind, content.len(), &whole_sha256, chunk_count);
    write_single_slot_at(path, DOC_KIND_CHUNK_MANIFEST, key, &manifest, timestamp_ms)?;
    Ok(())
}

/// The single-tile write primitive: one `DOCUMENT_STORE` record in one tile
/// (packed slot or legacy 128 KiB stride, per the vault's layout). Errors
/// when the record does not fit in one tile payload.
fn write_single_slot_at(
    path: &Path,
    doc_kind: u32,
    key: &str,
    content: &[u8],
    timestamp_ms: u64,
) -> Result<(), String> {
    let key_bytes = key.as_bytes();
    let content_crc = crc32c::crc32c(content);
    let mut flags = DOC_FLAG_ACTIVE;
    if doc_kind == DOC_KIND_JSON || doc_kind == DOC_KIND_CHUNK_MANIFEST {
        flags |= DOC_FLAG_IS_JSON;
    }

    let doc_hdr = DocumentRecordHeader {
        doc_kind,
        flags,
        key_hash: compute_key_hash(doc_kind, key),
        timestamp_ms,
        key_len: key_bytes.len() as u32,
        content_len: content.len() as u32,
        content_crc32: content_crc,
        ..Default::default()
    };

    let header_size = std::mem::size_of::<DocumentRecordHeader>();
    let total_record_size = header_size + key_bytes.len() + content.len();

    if total_record_size > HYPER_TILE_PAYLOAD_SIZE {
        return Err(format!(
            "Value size ({} bytes) exceeds single tile payload limit ({} bytes)",
            content.len(),
            HYPER_TILE_PAYLOAD_SIZE - header_size - key_bytes.len()
        ));
    }

    let mut tile_payload = Vec::with_capacity(total_record_size);
    tile_payload.extend_from_slice(bytemuck::bytes_of(&doc_hdr));
    tile_payload.extend_from_slice(key_bytes);
    tile_payload.extend_from_slice(content);

    let mut tile_hdr = s3a_core::HyperTileHeader::new(0, TileType::DOCUMENT_STORE);
    tile_hdr.record_count = 1;
    tile_hdr.payload_bytes = tile_payload.len() as u32;
    tile_hdr.data_crc32 = crc32c::crc32c(&tile_payload);

    let mut writer = TileWriter::open_append(path).map_err(|e| {
        format!(
            "Failed to open vault '{}' for writing: {}",
            path.display(),
            e
        )
    })?;

    // Prompt 010: an empty vault (or one already packed) takes the packed
    // path — one slot per document at its smallest fitting size class. A
    // vault that already holds legacy 128 KiB-stride tiles keeps the legacy
    // write path, so existing archives never change layout underneath a
    // reader that expects the stride.
    let packed = writer.tile_count() == 0 || writer.is_packed();
    let write_result = if packed {
        writer.write_packed_hyper_tile(&tile_hdr, &tile_payload)
    } else {
        writer.write_raw_hyper_tile(&tile_hdr, &tile_payload)
    };
    write_result.map_err(|e| format!("Failed to commit record: {}", e))?;

    Ok(())
}

struct LatestRecord {
    header: DocumentRecordHeader,
    content: Vec<u8>,
    tile_index: usize,
    tile_id: u64,
}

/// Parses one tile's `DOCUMENT_STORE` payload into `(header, key, content)`
/// triples, in storage order.
fn parse_tile_records(payload: &[u8]) -> Vec<(DocumentRecordHeader, &[u8], &[u8])> {
    let mut out = Vec::new();
    let mut offset = 0;
    let header_size = std::mem::size_of::<DocumentRecordHeader>();
    while offset + header_size <= payload.len() {
        let doc_hdr: DocumentRecordHeader =
            bytemuck::pod_read_unaligned(&payload[offset..offset + header_size]);
        offset += header_size;

        let k_len = doc_hdr.key_len as usize;
        let c_len = doc_hdr.content_len as usize;
        if offset + k_len + c_len > payload.len() {
            break;
        }
        let k_bytes = &payload[offset..offset + k_len];
        offset += k_len;
        let c_bytes = &payload[offset..offset + c_len];
        offset += c_len;

        out.push((doc_hdr, k_bytes, c_bytes));
    }
    out
}

/// Latest `DOCUMENT_STORE` record for one key (newest tile wins; within a
/// tile the last record wins), or `None` if the key was never stored or was
/// deleted.
fn latest_record_for(path: &Path, key: &str) -> Result<Option<LatestRecord>, String> {
    let reader = MmapReader::open(path)
        .map_err(|e| format!("Could not open vault '{}': {}", path.display(), e))?;

    let tile_count = reader.tile_count() as usize;
    for idx in (0..tile_count).rev() {
        if let Some((hdr, payload)) = reader.get_tile(idx) {
            if hdr.tile_type == TileType::DOCUMENT_STORE {
                // The last match in storage order within this tile is the
                // latest record for the key in this tile; the first tile
                // (from the end) holding the key holds the latest overall.
                let mut found: Option<LatestRecord> = None;
                for (doc_hdr, k_bytes, c_bytes) in parse_tile_records(payload) {
                    if doc_hdr.key_hash == compute_key_hash(doc_hdr.doc_kind, key)
                        && k_bytes == key.as_bytes()
                    {
                        found = Some(LatestRecord {
                            header: doc_hdr,
                            content: c_bytes.to_vec(),
                            tile_index: idx,
                            tile_id: hdr.tile_id,
                        });
                    }
                }
                match found {
                    Some(rec) if rec.header.is_deleted() => return Ok(None),
                    Some(rec) => return Ok(Some(rec)),
                    None => continue,
                }
            }
        }
    }

    Ok(None)
}

pub fn read_document_record(path: &Path, key: &str) -> Result<Option<Vec<u8>>, String> {
    let Some(latest) = latest_record_for(path, key)? else {
        return Ok(None);
    };
    if latest.header.doc_kind != DOC_KIND_CHUNK_MANIFEST {
        return Ok(Some(latest.content));
    }

    // Prompt 011: reassemble the chunked document in order, failing closed
    // on a missing chunk, a bad chunk length or CRC, or a whole-file hash
    // mismatch. A missing or out-of-order chunk is an error, never a
    // silent truncate.
    let manifest = parse_manifest(&latest.content, key)?;
    let max_chunk = max_chunk_content_bytes(key)?;
    if manifest.chunk_count != manifest.total_bytes.div_ceil(max_chunk) {
        return Err(format!(
            "chunk manifest for '{}' claims {} chunks for {} bytes, expected {}",
            key,
            manifest.chunk_count,
            manifest.total_bytes,
            manifest.total_bytes.div_ceil(max_chunk)
        ));
    }
    let mut out = Vec::with_capacity(manifest.total_bytes);
    for index in 0..manifest.chunk_count {
        let chunk = latest_record_for(path, &chunk_key(key, index))?.ok_or_else(|| {
            format!(
                "chunk {} of {} for '{}' is missing",
                index, manifest.chunk_count, key
            )
        })?;
        if chunk.header.doc_kind != DOC_KIND_CHUNK_DATA {
            return Err(format!(
                "chunk {} of {} for '{}' is not a chunk record",
                index, manifest.chunk_count, key
            ));
        }
        let expected_len = if index + 1 == manifest.chunk_count {
            manifest.total_bytes - index * max_chunk
        } else {
            max_chunk
        };
        if chunk.content.len() != expected_len {
            return Err(format!(
                "chunk {} of {} for '{}' has {} bytes, expected {}",
                index,
                manifest.chunk_count,
                key,
                chunk.content.len(),
                expected_len
            ));
        }
        if crc32c::crc32c(&chunk.content) != chunk.header.content_crc32 {
            return Err(format!(
                "chunk {} of {} for '{}' fails its CRC32 check",
                index, manifest.chunk_count, key
            ));
        }
        out.extend_from_slice(&chunk.content);
    }
    if out.len() != manifest.total_bytes {
        return Err(format!(
            "reassembled '{}' has {} bytes, manifest says {}",
            key,
            out.len(),
            manifest.total_bytes
        ));
    }
    if sha256_hex(&out) != manifest.sha256 {
        return Err(format!(
            "reassembled '{}' fails its whole-file SHA-256 check",
            key
        ));
    }
    Ok(Some(out))
}

pub fn list_all_documents(path: &Path) -> Result<Vec<StoredDocumentMeta>, String> {
    let reader = MmapReader::open(path)
        .map_err(|e| format!("Could not open vault '{}': {}", path.display(), e))?;

    let mut map: HashMap<String, LatestRecord> = HashMap::new();
    let tile_count = reader.tile_count() as usize;

    for idx in 0..tile_count {
        if let Some((hdr, payload)) = reader.get_tile(idx) {
            if hdr.tile_type == TileType::DOCUMENT_STORE {
                for (doc_hdr, k_bytes, c_bytes) in parse_tile_records(payload) {
                    let key_str = String::from_utf8_lossy(k_bytes).to_string();
                    if doc_hdr.is_deleted() {
                        map.remove(&key_str);
                    } else if doc_hdr.doc_kind == DOC_KIND_CHUNK_DATA {
                        // Chunk payloads are internal to their manifest;
                        // keep the header (length, CRC, tile) but do not
                        // copy the payload.
                        map.insert(
                            key_str,
                            LatestRecord {
                                header: doc_hdr,
                                content: Vec::new(),
                                tile_index: idx,
                                tile_id: hdr.tile_id,
                            },
                        );
                    } else {
                        map.insert(
                            key_str,
                            LatestRecord {
                                header: doc_hdr,
                                content: c_bytes.to_vec(),
                                tile_index: idx,
                                tile_id: hdr.tile_id,
                            },
                        );
                    }
                }
            }
        }
    }

    let mut items: Vec<StoredDocumentMeta> = Vec::new();
    for (key, rec) in &map {
        if rec.header.doc_kind == DOC_KIND_CHUNK_DATA {
            continue; // internal chunk record, not a logical document
        }
        if rec.header.doc_kind == DOC_KIND_CHUNK_MANIFEST {
            let manifest = parse_manifest(&rec.content, key)?;
            let mut chunks = Vec::new();
            for index in 0..manifest.chunk_count {
                if let Some(c) = map.get(&chunk_key(key, index)) {
                    if c.header.doc_kind == DOC_KIND_CHUNK_DATA {
                        chunks.push(ChunkMeta {
                            index,
                            total_chunks: manifest.chunk_count,
                            chunk_bytes: c.header.content_len as usize,
                            tile_index: c.tile_index as u64,
                            tile_id: c.tile_id,
                            crc32: c.header.content_crc32,
                        });
                    }
                }
            }
            items.push(StoredDocumentMeta {
                key: key.clone(),
                kind: manifest.kind,
                size_bytes: manifest.total_bytes,
                timestamp_ms: rec.header.timestamp_ms,
                sha256: manifest.sha256.clone(),
                chunk_count: manifest.chunk_count,
                chunks,
            });
        } else {
            items.push(StoredDocumentMeta {
                key: key.clone(),
                kind: rec.header.doc_kind,
                size_bytes: rec.header.content_len as usize,
                timestamp_ms: rec.header.timestamp_ms,
                sha256: sha256_hex(&rec.content),
                chunk_count: 1,
                chunks: Vec::new(),
            });
        }
    }

    items.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(items)
}
