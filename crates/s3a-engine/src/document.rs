//! Shared document record store (Tile 23, `DOCUMENT_STORE` tiles).
//!
//! Prompt 010: the write path stores each document tile at its smallest
//! fitting slab size class (a "packed" archive) when the vault is empty or
//! already packed, and keeps the legacy 128 KiB stride when the vault
//! already holds legacy tiles. Reads and listings work on both layouts
//! through [`MmapReader`], which walks packed slots at open.
//!
//! `s3a-cli` (`first_run`) and `s3a-pack-tdp` (`store`) both delegate to
//! this module so the layout decision lives in exactly one place.

use crate::{MmapReader, TileWriter};
use s3a_core::{
    compute_key_hash, DocumentRecordHeader, TileType, DOC_FLAG_ACTIVE, DOC_FLAG_IS_JSON,
    DOC_KIND_BINARY, DOC_KIND_GENERIC_KV, DOC_KIND_JSON, HYPER_TILE_PAYLOAD_SIZE,
};
use std::collections::HashMap;
use std::path::Path;

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub struct StoredDocumentMeta {
    pub key: String,
    pub size_bytes: usize,
    pub timestamp_ms: u64,
}

/// Stores `content` under `key`, inferring the document kind from the
/// content (JSON, binary, or generic key-value) exactly as the CLI has
/// always done.
pub fn write_document_record(path: &Path, key: &str, content: &[u8]) -> Result<(), String> {
    let is_json = serde_json::from_slice::<serde_json::Value>(content).is_ok();
    let is_binary = !is_json && std::str::from_utf8(content).is_err();
    let doc_kind = if is_json {
        DOC_KIND_JSON
    } else if is_binary {
        DOC_KIND_BINARY
    } else {
        DOC_KIND_GENERIC_KV
    };
    write_document_record_kind(path, doc_kind, key, content)
}

/// Stores `content` under `key` with an explicit document kind (the
/// `s3a-pack-tdp` form: PDF/STEP payloads and their JSON sidecars).
pub fn write_document_record_kind(
    path: &Path,
    doc_kind: u32,
    key: &str,
    content: &[u8],
) -> Result<(), String> {
    let key_bytes = key.as_bytes();
    let content_crc = crc32c::crc32c(content);
    let mut flags = DOC_FLAG_ACTIVE;
    if doc_kind == DOC_KIND_JSON {
        flags |= DOC_FLAG_IS_JSON;
    }

    let doc_hdr = DocumentRecordHeader {
        doc_kind,
        flags,
        key_hash: compute_key_hash(doc_kind, key),
        timestamp_ms: now_ms(),
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

pub fn read_document_record(path: &Path, key: &str) -> Result<Option<Vec<u8>>, String> {
    let reader = MmapReader::open(path)
        .map_err(|e| format!("Could not open vault '{}': {}", path.display(), e))?;

    let tile_count = reader.tile_count() as usize;

    for idx in (0..tile_count).rev() {
        if let Some((hdr, payload)) = reader.get_tile(idx) {
            if hdr.tile_type == TileType::DOCUMENT_STORE {
                let mut offset = 0;
                let header_size = std::mem::size_of::<DocumentRecordHeader>();
                let mut matches = Vec::new();

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

                    if doc_hdr.key_hash == compute_key_hash(doc_hdr.doc_kind, key)
                        && k_bytes == key.as_bytes()
                    {
                        matches.push((doc_hdr, c_bytes.to_vec()));
                    }
                }

                if let Some((last_hdr, last_content)) = matches.pop() {
                    if last_hdr.is_deleted() {
                        return Ok(None);
                    } else {
                        return Ok(Some(last_content));
                    }
                }
            }
        }
    }

    Ok(None)
}

pub fn list_all_documents(path: &Path) -> Result<Vec<StoredDocumentMeta>, String> {
    let reader = MmapReader::open(path)
        .map_err(|e| format!("Could not open vault '{}': {}", path.display(), e))?;

    let mut map: HashMap<String, StoredDocumentMeta> = HashMap::new();
    let tile_count = reader.tile_count() as usize;

    for idx in 0..tile_count {
        if let Some((hdr, payload)) = reader.get_tile(idx) {
            if hdr.tile_type == TileType::DOCUMENT_STORE {
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
                    offset += c_len;

                    let key_str = String::from_utf8_lossy(k_bytes).to_string();
                    if doc_hdr.is_deleted() {
                        map.remove(&key_str);
                    } else {
                        map.insert(
                            key_str.clone(),
                            StoredDocumentMeta {
                                key: key_str,
                                size_bytes: c_len,
                                timestamp_ms: doc_hdr.timestamp_ms,
                            },
                        );
                    }
                }
            }
        }
    }

    let mut items: Vec<StoredDocumentMeta> = map.into_values().collect();
    items.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(items)
}
