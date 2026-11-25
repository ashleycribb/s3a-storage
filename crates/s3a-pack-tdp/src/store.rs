//! Storage helpers for writing TDP documents (PDF and STEP) to S3A vaults.

use std::path::Path;

use s3a_core::document::{DOC_KIND_BINARY, DOC_KIND_JSON};

use crate::pdf::{compute_sha256, extract_pdf_metadata, PdfMetadata};
use crate::step::{parse_step, StepAssemblyTree};

/// Appends a raw document record of the specified kind to an S3A vault.
/// Delegates to `s3a_engine::document` (prompt 010) so the CLI and TDP share
/// one layout decision: packed slots for empty/packed vaults, legacy stride
/// for vaults that already hold legacy tiles.
pub fn write_document_record(
    vault_path: &Path,
    doc_kind: u32,
    key: &str,
    content: &[u8],
) -> Result<(), String> {
    s3a_engine::document::write_document_record_kind(vault_path, doc_kind, key, content)
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
