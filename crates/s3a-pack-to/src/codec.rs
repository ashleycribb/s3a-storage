//! Encode and decode Technical Order records. Fail closed on bad input.

use crate::pack::ToTileType;
use crate::records::{
    ToFaultEdgeRecord, ToFaultNodeRecord, ToFigureAnchorRecord, ToSectionRecord, ToSpecCellRecord,
    ToWcnRecord, NODE_KIND_ACTION, NODE_KIND_CONCLUSION, NODE_KIND_TEST, PAGE_THOUSANDTHS_MAX,
    WCN_KIND_CAUTION, WCN_KIND_NOTE, WCN_KIND_WARNING,
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
        | ToTileType::TO_FIGURE_ANCHOR => require_layout(tile_type, layout_version)?,
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
        other => Err(ToError::UnknownTileType(other)),
    }
}
