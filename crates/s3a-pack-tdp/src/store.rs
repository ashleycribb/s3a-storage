//! Storage helpers for writing TDP documents (PDF and STEP) to S3A vaults.

use std::path::Path;
use std::time::SystemTime;

use s3a_core::document::{
    compute_key_hash, DocumentRecordHeader, DOC_FLAG_ACTIVE, DOC_FLAG_IS_JSON, DOC_KIND_BINARY,
    DOC_KIND_JSON,
};
use s3a_core::{HyperTileHeader, TileType, HYPER_TILE_PAYLOAD_SIZE};
use s3a_engine::TileWriter;

use crate::pdf::{compute_sha256, extract_pdf_metadata, PdfMetadata};
use crate::step::{parse_step, StepAssemblyTree};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Appends a raw document record of the specified kind to an S3A vault.
pub fn write_document_record(
    vault_path: &Path,
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
            "Payload size ({} bytes) exceeds single tile payload limit ({} bytes)",
            content.len(),
            HYPER_TILE_PAYLOAD_SIZE - header_size - key_bytes.len()
        ));
    }

    let mut tile_payload = Vec::with_capacity(total_record_size);
    tile_payload.extend_from_slice(bytemuck::bytes_of(&doc_hdr));
    tile_payload.extend_from_slice(key_bytes);
    tile_payload.extend_from_slice(content);

    let mut tile_hdr = HyperTileHeader::new(0, TileType::DOCUMENT_STORE);
    tile_hdr.record_count = 1;
    tile_hdr.payload_bytes = tile_payload.len() as u32;
    tile_hdr.data_crc32 = crc32c::crc32c(&tile_payload);

    let mut writer = TileWriter::open_append(vault_path).map_err(|e| {
        format!(
            "Failed to open vault '{}' for writing: {}",
            vault_path.display(),
            e
        )
    })?;

    writer
        .write_raw_hyper_tile(&tile_hdr, &tile_payload)
        .map_err(|e| format!("Failed to commit tile record: {}", e))?;

    Ok(())
}

/// Stores a PDF document and its sidecar metadata in an S3A vault.
///
/// 1. Stores the raw PDF at `<key>` with `DOC_KIND_BINARY`.
/// 2. Extracts `PdfMetadata` (page count, title, sha256).
/// 3. Stores the JSON metadata sidecar at `<key>.meta` with `DOC_KIND_JSON`.
pub fn store_pdf(vault_path: &Path, key: &str, pdf_bytes: &[u8]) -> Result<PdfMetadata, String> {
    // 1. Extract metadata first to ensure PDF is valid
    let meta = extract_pdf_metadata(key, pdf_bytes)?;

    // 2. Store binary PDF payload
    write_document_record(vault_path, DOC_KIND_BINARY, key, pdf_bytes)?;

    // 3. Store sidecar metadata record
    let meta_json = serde_json::to_vec_pretty(&meta)
        .map_err(|e| format!("Failed to serialize PDF metadata: {}", e))?;
    let sidecar_key = format!("{}.meta", key);
    write_document_record(vault_path, DOC_KIND_JSON, &sidecar_key, &meta_json)?;

    Ok(meta)
}

/// Stores an AP242 STEP assembly model and its assembly tree in an S3A vault.
///
/// 1. Stores raw STEP bytes at `<key>` with `DOC_KIND_BINARY`.
/// 2. Parses the assembly tree into `StepAssemblyTree`.
/// 3. Stores the assembly tree at `<key>.assembly` with `DOC_KIND_JSON`.
/// 4. Stores summary metadata at `<key>.meta` with `DOC_KIND_JSON`.
pub fn store_step(
    vault_path: &Path,
    key: &str,
    step_bytes: &[u8],
) -> Result<StepAssemblyTree, String> {
    // 1. Parse STEP assembly structure
    let tree = parse_step(step_bytes)?;

    // 2. Store raw binary STEP payload
    write_document_record(vault_path, DOC_KIND_BINARY, key, step_bytes)?;

    // 3. Store assembly tree JSON record
    let assembly_json = serde_json::to_vec_pretty(&tree)
        .map_err(|e| format!("Failed to serialize assembly tree JSON: {}", e))?;
    let assembly_key = format!("{}.assembly", key);
    write_document_record(vault_path, DOC_KIND_JSON, &assembly_key, &assembly_json)?;

    // 4. Store sidecar summary metadata
    let sha256 = compute_sha256(step_bytes);
    let summary_meta = serde_json::json!({
        "key": key,
        "mime_type": "application/step",
        "size_bytes": step_bytes.len(),
        "sha256": sha256,
        "root_part": tree.root_part_number,
        "component_count": tree.parts.len(),
        "header": tree.header,
    });
    let meta_json = serde_json::to_vec_pretty(&summary_meta)
        .map_err(|e| format!("Failed to serialize STEP metadata: {}", e))?;
    let sidecar_key = format!("{}.meta", key);
    write_document_record(vault_path, DOC_KIND_JSON, &sidecar_key, &meta_json)?;

    Ok(tree)
}
