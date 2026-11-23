//! Encode and decode Technical Order records. Fail closed on bad input.

use crate::pack::ToTileType;
use crate::records::{
    ToFaultEdgeRecord, ToFaultNodeRecord, ToFigureAnchorRecord, ToLocatorRecord,
    ToProcedureStepRecord, ToReferenceRecord, ToScanAgreementRecord, ToSectionRecord,
    ToSpecCellRecord, ToWcnRecord, NODE_KIND_ACTION, NODE_KIND_CONCLUSION, NODE_KIND_TEST,
    PAGE_THOUSANDTHS_MAX, SCAN_FLAG_CER_PRESENT, SCAN_STATUS_HELD, SCAN_STATUS_HUMAN_VERIFIED,
    SCAN_STATUS_SCAN_AGREEMENT, SCAN_STATUS_VERIFIED, WCN_KIND_CAUTION, WCN_KIND_NOTE,
    WCN_KIND_WARNING,
};
use bytemuck::{pod_read_unaligned, Pod};
use sha2::{Digest, Sha256};

/// Errors returned by Technical Order encode and decode paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToError {
    MissingSourceId,
    MissingOriginPtr,
    MissingDocId,
    UnknownWcnKind { kind: u8 },
    UnknownNodeKind { kind: u16 },
    EmptyText,
    ZeroNodeId,
    SelfLoopEdge { from_node_id: u64, to_node_id: u64 },
    ZeroTableId,
    ZeroTableIndex,
    ZeroFigureId,
    NonPositiveBBox,
    BBoxOutOfPage { max: i32 },
    TruncatedRecord { expected: usize, actual: usize },
    UnsupportedLayoutVersion { tile_type: u32, version: u32 },
    UnknownTileType(u32),
    NonZeroPad,
    ZeroStepNo,
    ZeroPage,
    MissingTargetSourceId,
    TargetTileNotTo { tile_type: u32 },
    UnknownScanStatus { status: u8 },
    MissingEngineName,
    MissingEngineVersion,
    EngineFieldTooLong,
    MissingOcrHash,
    MissingCropHash,
    InvalidThreshold,
    MissingCer,
    CerExceedsThreshold,
    MissingReviewer,
    MissingReviewedAt,
    UnexpectedReviewEvidence,
}

/// Successfully decoded payload of one Technical Order tile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToDecoded {
    Section(Vec<ToSectionRecord>),
    Wcn(Vec<ToWcnRecord>),
    FaultNode(Vec<ToFaultNodeRecord>),
    FaultEdge(Vec<ToFaultEdgeRecord>),
    SpecCell(Vec<ToSpecCellRecord>),
    FigureAnchor(Vec<ToFigureAnchorRecord>),
    ProcedureStep(Vec<ToProcedureStepRecord>),
    Reference(Vec<ToReferenceRecord>),
    Locator(Vec<ToLocatorRecord>),
    ScanAgreement(Vec<ToScanAgreementRecord>),
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn require_source_id(source_id: [u8; 16]) -> Result<(), ToError> {
    if source_id == [0u8; 16] {
        Err(ToError::MissingSourceId)
    } else {
        Ok(())
    }
}

fn require_origin_ptr(origin_ptr: [u8; 16]) -> Result<(), ToError> {
    if origin_ptr == [0u8; 16] {
        Err(ToError::MissingOriginPtr)
    } else {
        Ok(())
    }
}

fn require_doc_id(doc_id: [u8; 16]) -> Result<(), ToError> {
    if doc_id == [0u8; 16] {
        Err(ToError::MissingDocId)
    } else {
        Ok(())
    }
}

fn require_non_empty(text: &[u8]) -> Result<(), ToError> {
    if text.is_empty() {
        Err(ToError::EmptyText)
    } else {
        Ok(())
    }
}

fn first16(hash: [u8; 32]) -> [u8; 16] {
    let mut out = [0u8; 16];
    out.copy_from_slice(&hash[..16]);
    out
}

fn require_page(page: u32) -> Result<(), ToError> {
    if page == 0 {
        Err(ToError::ZeroPage)
    } else {
        Ok(())
    }
}

fn is_to_target_tile(tile_type: u32) -> bool {
    (41..=46).contains(&tile_type) || (49..=51).contains(&tile_type)
}

fn locator_content_hash(para_path: &[u8], printed_page: &[u8]) -> [u8; 32] {
    let mut src = para_path.to_vec();
    src.push(0x1f);
    src.extend_from_slice(printed_page);
    sha256(&src)
}

fn require_wcn_kind(kind: u8) -> Result<(), ToError> {
    match kind {
        WCN_KIND_WARNING | WCN_KIND_CAUTION | WCN_KIND_NOTE => Ok(()),
        other => Err(ToError::UnknownWcnKind { kind: other }),
    }
}

fn require_node_kind(kind: u16) -> Result<(), ToError> {
    match kind {
        NODE_KIND_TEST | NODE_KIND_ACTION | NODE_KIND_CONCLUSION => Ok(()),
        other => Err(ToError::UnknownNodeKind { kind: other }),
    }
}

fn require_zero(pad: &[u8]) -> Result<(), ToError> {
    if pad.iter().all(|b| *b == 0) {
        Ok(())
    } else {
        Err(ToError::NonZeroPad)
    }
}

fn require_bbox(x: i32, y: i32, w: i32, h: i32) -> Result<(), ToError> {
    if w <= 0 || h <= 0 {
        return Err(ToError::NonPositiveBBox);
    }
    let max = PAGE_THOUSANDTHS_MAX;
    let x_end = x.checked_add(w);
    let y_end = y.checked_add(h);
    let in_page = x >= 0
        && y >= 0
        && x_end.map(|v| v <= max).unwrap_or(false)
        && y_end.map(|v| v <= max).unwrap_or(false);
    if !in_page {
        Err(ToError::BBoxOutOfPage { max })
    } else {
        Ok(())
    }
}

fn effective_layout_version(version: u32) -> u32 {
    if version == 0 {
        1
    } else {
        version
    }
}

fn require_layout(tile_type: u32, version: u32) -> Result<(), ToError> {
    let effective = effective_layout_version(version);
    if effective != 1 {
        Err(ToError::UnsupportedLayoutVersion { tile_type, version })
    } else {
        Ok(())
    }
}

fn split_records(payload: &[u8], size: usize) -> Result<(), ToError> {
    if !payload.len().is_multiple_of(size) {
        Err(ToError::TruncatedRecord {
            expected: size,
            actual: payload.len(),
        })
    } else {
        Ok(())
    }
}

/// Encodes a Technical Order section record.
#[allow(clippy::too_many_arguments)]
pub fn encode_to_section(
    source_bytes: &[u8],
    source_id: [u8; 16],
    origin_ptr: [u8; 16],
    doc_id: [u8; 16],
    chapter: u32,
    section: u32,
    page: u32,
    title: &[u8],
) -> Result<ToSectionRecord, ToError> {
    require_source_id(source_id)?;
    require_origin_ptr(origin_ptr)?;
    require_doc_id(doc_id)?;
    require_non_empty(source_bytes)?;
    require_non_empty(title)?;
    Ok(ToSectionRecord {
        source_id,
        content_hash: sha256(source_bytes),
        origin_ptr,
        doc_id,
        chapter,
        section,
        page,
        title_hash: sha256(title),
        _pad: [0u8; 4],
    })
}

fn validate_section(record: &ToSectionRecord) -> Result<(), ToError> {
    require_source_id(record.source_id)?;
    require_origin_ptr(record.origin_ptr)?;
    require_doc_id(record.doc_id)?;
    require_zero(&record._pad)
}

/// Encodes a Warning / Caution / Note record.
#[allow(clippy::too_many_arguments)]
pub fn encode_to_wcn(
    text: &[u8],
    source_id: [u8; 16],
    origin_ptr: [u8; 16],
    doc_id: [u8; 16],
    chapter: u32,
    section: u32,
    page: u32,
    kind: u8,
) -> Result<ToWcnRecord, ToError> {
    require_source_id(source_id)?;
    require_origin_ptr(origin_ptr)?;
    require_doc_id(doc_id)?;
    require_non_empty(text)?;
    require_wcn_kind(kind)?;
    let hash = sha256(text);
    Ok(ToWcnRecord {
        source_id,
        content_hash: hash,
        origin_ptr,
        doc_id,
        chapter,
        section,
        page,
        kind,
        _pad: [0u8; 3],
        text_hash: hash,
    })
}

fn validate_wcn(record: &ToWcnRecord) -> Result<(), ToError> {
    require_source_id(record.source_id)?;
    require_origin_ptr(record.origin_ptr)?;
    require_doc_id(record.doc_id)?;
    require_wcn_kind(record.kind)?;
    require_zero(&record._pad)
}

/// Encodes a fault isolation tree node.
pub fn encode_to_fault_node(
    label: &[u8],
    source_id: [u8; 16],
    origin_ptr: [u8; 16],
    doc_id: [u8; 16],
    node_id: u64,
    kind: u16,
) -> Result<ToFaultNodeRecord, ToError> {
    require_source_id(source_id)?;
    require_origin_ptr(origin_ptr)?;
    require_doc_id(doc_id)?;
    require_non_empty(label)?;
    require_node_kind(kind)?;
    if node_id == 0 {
        return Err(ToError::ZeroNodeId);
    }
    let hash = sha256(label);
    Ok(ToFaultNodeRecord {
        source_id,
        content_hash: hash,
        origin_ptr,
        doc_id,
        node_id,
        kind,
        _pad: [0u8; 2],
        label_hash: hash,
        _pad2: [0u8; 4],
    })
}

fn validate_fault_node(record: &ToFaultNodeRecord) -> Result<(), ToError> {
    require_source_id(record.source_id)?;
    require_origin_ptr(record.origin_ptr)?;
    require_doc_id(record.doc_id)?;
    require_node_kind(record.kind)?;
    if record.node_id == 0 {
        return Err(ToError::ZeroNodeId);
    }
    require_zero(&record._pad)?;
    require_zero(&record._pad2)
}

/// Encodes a fault isolation tree edge.
pub fn encode_to_fault_edge(
    outcome: &[u8],
    source_id: [u8; 16],
    origin_ptr: [u8; 16],
    doc_id: [u8; 16],
    from_node_id: u64,
    to_node_id: u64,
) -> Result<ToFaultEdgeRecord, ToError> {
    require_source_id(source_id)?;
    require_origin_ptr(origin_ptr)?;
    require_doc_id(doc_id)?;
    require_non_empty(outcome)?;
    if from_node_id == 0 || to_node_id == 0 {
        return Err(ToError::ZeroNodeId);
    }
    if from_node_id == to_node_id {
        return Err(ToError::SelfLoopEdge {
            from_node_id,
            to_node_id,
        });
    }
    let hash = sha256(outcome);
    Ok(ToFaultEdgeRecord {
        source_id,
        content_hash: hash,
        origin_ptr,
        doc_id,
        from_node_id,
        to_node_id,
        outcome_hash: hash,
    })
}

fn validate_fault_edge(record: &ToFaultEdgeRecord) -> Result<(), ToError> {
    require_source_id(record.source_id)?;
    require_origin_ptr(record.origin_ptr)?;
    require_doc_id(record.doc_id)?;
    if record.from_node_id == 0 || record.to_node_id == 0 {
        return Err(ToError::ZeroNodeId);
    }
    if record.from_node_id == record.to_node_id {
        return Err(ToError::SelfLoopEdge {
            from_node_id: record.from_node_id,
            to_node_id: record.to_node_id,
        });
    }
    Ok(())
}

/// Encodes a specification / tolerance table cell.
pub fn encode_to_spec_cell(
    value: &[u8],
    source_id: [u8; 16],
    origin_ptr: [u8; 16],
    table_id: u64,
    row: u32,
    col: u32,
    unit: &[u8],
) -> Result<ToSpecCellRecord, ToError> {
    require_source_id(source_id)?;
    require_origin_ptr(origin_ptr)?;
    require_non_empty(value)?;
    if table_id == 0 {
        return Err(ToError::ZeroTableId);
    }
    if row == 0 || col == 0 {
        return Err(ToError::ZeroTableIndex);
    }
    Ok(ToSpecCellRecord {
        source_id,
        content_hash: sha256(value),
        origin_ptr,
        table_id,
        row,
        col,
        value_hash: sha256(value),
        unit_hash: sha256(unit),
        _pad: [0u8; 16],
    })
}

fn validate_spec_cell(record: &ToSpecCellRecord) -> Result<(), ToError> {
    require_source_id(record.source_id)?;
    require_origin_ptr(record.origin_ptr)?;
    if record.table_id == 0 {
        return Err(ToError::ZeroTableId);
    }
    if record.row == 0 || record.col == 0 {
        return Err(ToError::ZeroTableIndex);
    }
    require_zero(&record._pad)
}

/// Encodes a figure / schematic anchor.
#[allow(clippy::too_many_arguments)]
pub fn encode_to_figure_anchor(
    quote: &[u8],
    source_id: [u8; 16],
    origin_ptr: [u8; 16],
    doc_id: [u8; 16],
    chapter: u32,
    section: u32,
    page: u32,
    figure_id: u32,
    bbox_x: i32,
    bbox_y: i32,
    bbox_w: i32,
    bbox_h: i32,
) -> Result<ToFigureAnchorRecord, ToError> {
    require_source_id(source_id)?;
    require_origin_ptr(origin_ptr)?;
    require_doc_id(doc_id)?;
    require_non_empty(quote)?;
    if figure_id == 0 {
        return Err(ToError::ZeroFigureId);
    }
    require_bbox(bbox_x, bbox_y, bbox_w, bbox_h)?;
    let hash = sha256(quote);
    Ok(ToFigureAnchorRecord {
        source_id,
        content_hash: hash,
        origin_ptr,
        doc_id,
        chapter,
        section,
        page,
        figure_id,
        bbox_x,
        bbox_y,
        bbox_w,
        bbox_h,
        quote_hash: hash,
        _pad: [0u8; 16],
    })
}

fn validate_figure(record: &ToFigureAnchorRecord) -> Result<(), ToError> {
    require_source_id(record.source_id)?;
    require_origin_ptr(record.origin_ptr)?;
    require_doc_id(record.doc_id)?;
    if record.figure_id == 0 {
        return Err(ToError::ZeroFigureId);
    }
    require_bbox(record.bbox_x, record.bbox_y, record.bbox_w, record.bbox_h)?;
    require_zero(&record._pad)
}

fn decode_pod_slice<T, F, V>(
    payload: &[u8],
    size: usize,
    validate: F,
    wrap: V,
) -> Result<ToDecoded, ToError>
where
    T: Copy + Pod,
    F: Fn(&T) -> Result<(), ToError>,
    V: Fn(Vec<T>) -> ToDecoded,
{
    split_records(payload, size)?;
    let mut out = Vec::with_capacity(payload.len() / size);
    for chunk in payload.chunks_exact(size) {
        let record: T = pod_read_unaligned(chunk);
        validate(&record)?;
        out.push(record);
    }
    Ok(wrap(out))
}

/// Decodes a Technical Order tile payload. Fails closed on an unknown tile
/// type, an unsupported layout version, a truncated record, or an invalid
/// record.

/// Encodes a procedure step record (tile 49).
#[allow(clippy::too_many_arguments)]
pub fn encode_to_procedure_step(
    text: &[u8],
    source_id: [u8; 16],
    origin_ptr: [u8; 16],
    doc_id: [u8; 16],
    para_path: &[u8],
    step_no: u32,
    page: u32,
) -> Result<ToProcedureStepRecord, ToError> {
    require_source_id(source_id)?;
    require_origin_ptr(origin_ptr)?;
    require_doc_id(doc_id)?;
    require_non_empty(text)?;
    require_non_empty(para_path)?;
    if step_no == 0 {
        return Err(ToError::ZeroStepNo);
    }
    require_page(page)?;
    Ok(ToProcedureStepRecord {
        source_id,
        content_hash: sha256(text),
        origin_ptr,
        doc_id,
        para_path_hash: sha256(para_path),
        step_no,
        page,
        text_hash: sha256(text),
        _pad: [0u8; 8],
    })
}

fn validate_procedure_step(record: &ToProcedureStepRecord) -> Result<(), ToError> {
    require_source_id(record.source_id)?;
    require_origin_ptr(record.origin_ptr)?;
    require_doc_id(record.doc_id)?;
    if record.step_no == 0 {
        return Err(ToError::ZeroStepNo);
    }
    require_page(record.page)?;
    require_zero(&record._pad)
}

/// Encodes a related-publication record (tile 50).
#[allow(clippy::too_many_arguments)]
pub fn encode_to_reference(
    title: &[u8],
    source_id: [u8; 16],
    origin_ptr: [u8; 16],
    doc_id: [u8; 16],
    pub_id: &[u8],
    page: u32,
) -> Result<ToReferenceRecord, ToError> {
    require_source_id(source_id)?;
    require_origin_ptr(origin_ptr)?;
    require_doc_id(doc_id)?;
    require_non_empty(title)?;
    require_non_empty(pub_id)?;
    require_page(page)?;
    Ok(ToReferenceRecord {
        source_id,
        content_hash: sha256(title),
        origin_ptr,
        doc_id,
        pub_id_hash: sha256(pub_id),
        page,
        _pad: [0u8; 12],
    })
}

fn validate_reference(record: &ToReferenceRecord) -> Result<(), ToError> {
    require_source_id(record.source_id)?;
    require_origin_ptr(record.origin_ptr)?;
    require_doc_id(record.doc_id)?;
    require_page(record.page)?;
    require_zero(&record._pad)
}

/// Encodes a locator record (tile 51) for paragraph path and printed page.
#[allow(clippy::too_many_arguments)]
pub fn encode_to_locator(
    source_id: [u8; 16],
    origin_ptr: [u8; 16],
    doc_id: [u8; 16],
    target_source_id: [u8; 16],
    target_tile_type: u32,
    page: u32,
    para_path: &[u8],
    printed_page: &[u8],
) -> Result<ToLocatorRecord, ToError> {
    require_source_id(source_id)?;
    require_origin_ptr(origin_ptr)?;
    require_doc_id(doc_id)?;
    if target_source_id == [0u8; 16] {
        return Err(ToError::MissingTargetSourceId);
    }
    if !is_to_target_tile(target_tile_type) {
        return Err(ToError::TargetTileNotTo {
            tile_type: target_tile_type,
        });
    }
    require_page(page)?;
    require_non_empty(para_path)?;
    require_non_empty(printed_page)?;
    Ok(ToLocatorRecord {
        source_id,
        content_hash: locator_content_hash(para_path, printed_page),
        origin_ptr,
        doc_id,
        target_source_id,
        target_tile_type,
        page,
        para_path_hash: sha256(para_path),
        printed_page_hash: first16(sha256(printed_page)),
        _pad: [0u8; 8],
    })
}

fn validate_locator(record: &ToLocatorRecord) -> Result<(), ToError> {
    require_source_id(record.source_id)?;
    require_origin_ptr(record.origin_ptr)?;
    require_doc_id(record.doc_id)?;
    if record.target_source_id == [0u8; 16] {
        return Err(ToError::MissingTargetSourceId);
    }
    if !is_to_target_tile(record.target_tile_type) {
        return Err(ToError::TargetTileNotTo {
            tile_type: record.target_tile_type,
        });
    }
    require_page(record.page)?;
    require_zero(&record._pad)
}

fn pack_bytes<const N: usize>(bytes: &[u8], empty: ToError) -> Result<[u8; N], ToError> {
    if bytes.is_empty() {
        return Err(empty);
    }
    if bytes.len() > N {
        return Err(ToError::EngineFieldTooLong);
    }
    let mut out = [0u8; N];
    out[..bytes.len()].copy_from_slice(bytes);
    Ok(out)
}

fn require_threshold(max_character_error_rate: f64) -> Result<u64, ToError> {
    if !max_character_error_rate.is_finite() || !(0.0..=1.0).contains(&max_character_error_rate) {
        Err(ToError::InvalidThreshold)
    } else {
        Ok(max_character_error_rate.to_bits())
    }
}

fn require_status(status: u8) -> Result<(), ToError> {
    match status {
        SCAN_STATUS_VERIFIED
        | SCAN_STATUS_HELD
        | SCAN_STATUS_SCAN_AGREEMENT
        | SCAN_STATUS_HUMAN_VERIFIED => Ok(()),
        other => Err(ToError::UnknownScanStatus { status: other }),
    }
}

fn scan_content_hash(
    engine_a_name: &[u8],
    engine_a_version: &[u8],
    engine_b_name: &[u8],
    engine_b_version: &[u8],
    ocr_a: [u8; 32],
    ocr_b: [u8; 32],
    crop: [u8; 32],
) -> [u8; 32] {
    let mut src = alloc_vec(engine_a_name);
    src.push(0x1f);
    src.extend_from_slice(engine_a_version);
    src.push(0x1f);
    src.extend_from_slice(engine_b_name);
    src.push(0x1f);
    src.extend_from_slice(engine_b_version);
    src.push(0x1f);
    src.extend_from_slice(&ocr_a);
    src.extend_from_slice(&ocr_b);
    src.extend_from_slice(&crop);
    sha256(&src)
}

fn alloc_vec(bytes: &[u8]) -> Vec<u8> {
    bytes.to_vec()
}

/// Encodes a scan-agreement adjunct record (tile 52) linked to an existing
/// Technical Order section, cell, or other TO record by id.
///
/// `max_character_error_rate` is required configuration in `[0, 1]`. There
/// is no silent default. Status `scan_agreement` requires a scorable CER at
/// or below that threshold and must not carry reviewer evidence.
/// `human_verified` requires a reviewer, a non-zero timestamp, and the
/// three evidence hashes.
#[allow(clippy::too_many_arguments)]
pub fn encode_to_scan_agreement(
    source_id: [u8; 16],
    origin_ptr: [u8; 16],
    doc_id: [u8; 16],
    target_source_id: [u8; 16],
    target_tile_type: u32,
    status: u8,
    engine_a_name: &[u8],
    engine_a_version: &[u8],
    engine_b_name: &[u8],
    engine_b_version: &[u8],
    ocr_a_sha256: [u8; 32],
    ocr_b_sha256: [u8; 32],
    crop_sha256: [u8; 32],
    cer: Option<f64>,
    max_character_error_rate: f64,
    reviewer: Option<&[u8]>,
    reviewed_at: Option<u64>,
) -> Result<ToScanAgreementRecord, ToError> {
    require_source_id(source_id)?;
    require_origin_ptr(origin_ptr)?;
    require_doc_id(doc_id)?;
    if target_source_id == [0u8; 16] {
        return Err(ToError::MissingTargetSourceId);
    }
    if !is_to_target_tile(target_tile_type) {
        return Err(ToError::TargetTileNotTo {
            tile_type: target_tile_type,
        });
    }
    require_status(status)?;
    let engine_a_name_f = pack_bytes::<32>(engine_a_name, ToError::MissingEngineName)?;
    let engine_a_version_f = pack_bytes::<16>(engine_a_version, ToError::MissingEngineVersion)?;
    let engine_b_name_f = pack_bytes::<32>(engine_b_name, ToError::MissingEngineName)?;
    let engine_b_version_f = pack_bytes::<16>(engine_b_version, ToError::MissingEngineVersion)?;
    if ocr_a_sha256 == [0u8; 32] || ocr_b_sha256 == [0u8; 32] {
        return Err(ToError::MissingOcrHash);
    }
    if crop_sha256 == [0u8; 32] {
        return Err(ToError::MissingCropHash);
    }
    let max_cer_bits = require_threshold(max_character_error_rate)?;
    let (flags, cer_bits) = match cer {
        Some(value) => {
            if !value.is_finite() {
                return Err(ToError::MissingCer);
            }
            if status == SCAN_STATUS_SCAN_AGREEMENT && value > max_character_error_rate {
                return Err(ToError::CerExceedsThreshold);
            }
            (SCAN_FLAG_CER_PRESENT, value.to_bits())
        }
        None => {
            if status == SCAN_STATUS_SCAN_AGREEMENT {
                return Err(ToError::MissingCer);
            }
            (0, 0)
        }
    };

    let (reviewer_f, reviewed_at_v) = match status {
        SCAN_STATUS_HUMAN_VERIFIED => {
            let reviewer_bytes = reviewer.unwrap_or(b"");
            if reviewer_bytes.is_empty() {
                return Err(ToError::MissingReviewer);
            }
            match reviewed_at {
                Some(ts) if ts != 0 => (
                    pack_bytes::<32>(reviewer_bytes, ToError::MissingReviewer)?,
                    ts,
                ),
                _ => return Err(ToError::MissingReviewedAt),
            }
        }
        _ => {
            if reviewer.is_some() || reviewed_at.is_some() {
                return Err(ToError::UnexpectedReviewEvidence);
            }
            ([0u8; 32], 0)
        }
    };

    Ok(ToScanAgreementRecord {
        source_id,
        content_hash: scan_content_hash(
            engine_a_name,
            engine_a_version,
            engine_b_name,
            engine_b_version,
            ocr_a_sha256,
            ocr_b_sha256,
            crop_sha256,
        ),
        origin_ptr,
        doc_id,
        target_source_id,
        target_tile_type,
        status,
        flags,
        _pad0: [0u8; 2],
        engine_a_name: engine_a_name_f,
        engine_a_version: engine_a_version_f,
        engine_b_name: engine_b_name_f,
        engine_b_version: engine_b_version_f,
        ocr_a_sha256,
        ocr_b_sha256,
        crop_sha256,
        cer_bits,
        max_cer_bits,
        reviewed_at: reviewed_at_v,
        reviewer: reviewer_f,
        _pad: [0u8; 32],
    })
}

fn validate_scan(record: &ToScanAgreementRecord) -> Result<(), ToError> {
    require_source_id(record.source_id)?;
    require_origin_ptr(record.origin_ptr)?;
    require_doc_id(record.doc_id)?;
    if record.target_source_id == [0u8; 16] {
        return Err(ToError::MissingTargetSourceId);
    }
    if !is_to_target_tile(record.target_tile_type) {
        return Err(ToError::TargetTileNotTo {
            tile_type: record.target_tile_type,
        });
    }
    require_status(record.status)?;
    if record.engine_a_name.iter().all(|b| *b == 0) || record.engine_b_name.iter().all(|b| *b == 0)
    {
        return Err(ToError::MissingEngineName);
    }
    if record.engine_a_version.iter().all(|b| *b == 0)
        || record.engine_b_version.iter().all(|b| *b == 0)
    {
        return Err(ToError::MissingEngineVersion);
    }
    if record.ocr_a_sha256 == [0u8; 32] || record.ocr_b_sha256 == [0u8; 32] {
        return Err(ToError::MissingOcrHash);
    }
    if record.crop_sha256 == [0u8; 32] {
        return Err(ToError::MissingCropHash);
    }
    let max_cer = f64::from_bits(record.max_cer_bits);
    require_threshold(max_cer)?;
    match record.flags {
        0 => {
            if record.cer_bits != 0 {
                return Err(ToError::MissingCer);
            }
            if record.status == SCAN_STATUS_SCAN_AGREEMENT {
                return Err(ToError::MissingCer);
            }
        }
        SCAN_FLAG_CER_PRESENT => {
            let cer = f64::from_bits(record.cer_bits);
            if !cer.is_finite() {
                return Err(ToError::MissingCer);
            }
            if record.status == SCAN_STATUS_SCAN_AGREEMENT && cer > max_cer {
                return Err(ToError::CerExceedsThreshold);
            }
        }
        _ => return Err(ToError::NonZeroPad),
    }
    match record.status {
        SCAN_STATUS_HUMAN_VERIFIED => {
            if record.reviewer.iter().all(|b| *b == 0) {
                return Err(ToError::MissingReviewer);
            }
            if record.reviewed_at == 0 {
                return Err(ToError::MissingReviewedAt);
            }
        }
        _ => {
            if record.reviewer.iter().any(|b| *b != 0) || record.reviewed_at != 0 {
                return Err(ToError::UnexpectedReviewEvidence);
            }
        }
    }
    require_zero(&record._pad0)?;
    require_zero(&record._pad)
}

pub fn decode_to_records(
    tile_type: u32,
    layout_version: u32,
    payload: &[u8],
) -> Result<ToDecoded, ToError> {
    match tile_type {
        ToTileType::TO_SECTION
        | ToTileType::TO_WCN
        | ToTileType::TO_FAULT_NODE
        | ToTileType::TO_FAULT_EDGE
        | ToTileType::TO_SPEC_CELL
        | ToTileType::TO_FIGURE_ANCHOR
        | ToTileType::TO_PROCEDURE_STEP
        | ToTileType::TO_REFERENCE
        | ToTileType::TO_LOCATOR
        | ToTileType::TO_SCAN_AGREEMENT => require_layout(tile_type, layout_version)?,
        other => return Err(ToError::UnknownTileType(other)),
    }

    match tile_type {
        ToTileType::TO_SECTION => decode_pod_slice(
            payload,
            ToSectionRecord::SIZE,
            validate_section,
            ToDecoded::Section,
        ),
        ToTileType::TO_WCN => {
            decode_pod_slice(payload, ToWcnRecord::SIZE, validate_wcn, ToDecoded::Wcn)
        }
        ToTileType::TO_FAULT_NODE => decode_pod_slice(
            payload,
            ToFaultNodeRecord::SIZE,
            validate_fault_node,
            ToDecoded::FaultNode,
        ),
        ToTileType::TO_FAULT_EDGE => decode_pod_slice(
            payload,
            ToFaultEdgeRecord::SIZE,
            validate_fault_edge,
            ToDecoded::FaultEdge,
        ),
        ToTileType::TO_SPEC_CELL => decode_pod_slice(
            payload,
            ToSpecCellRecord::SIZE,
            validate_spec_cell,
            ToDecoded::SpecCell,
        ),
        ToTileType::TO_FIGURE_ANCHOR => decode_pod_slice(
            payload,
            ToFigureAnchorRecord::SIZE,
            validate_figure,
            ToDecoded::FigureAnchor,
        ),
        ToTileType::TO_PROCEDURE_STEP => decode_pod_slice(
            payload,
            ToProcedureStepRecord::SIZE,
            validate_procedure_step,
            ToDecoded::ProcedureStep,
        ),
        ToTileType::TO_REFERENCE => decode_pod_slice(
            payload,
            ToReferenceRecord::SIZE,
            validate_reference,
            ToDecoded::Reference,
        ),
        ToTileType::TO_LOCATOR => decode_pod_slice(
            payload,
            ToLocatorRecord::SIZE,
            validate_locator,
            ToDecoded::Locator,
        ),
        ToTileType::TO_SCAN_AGREEMENT => decode_pod_slice(
            payload,
            ToScanAgreementRecord::SIZE,
            validate_scan,
            ToDecoded::ScanAgreement,
        ),
        other => Err(ToError::UnknownTileType(other)),
    }
}
