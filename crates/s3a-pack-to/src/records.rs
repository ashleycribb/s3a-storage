//! Fixed-layout Technical Order records (step 1).
//!
//! Every record starts with the same 64-byte provenance prefix used by
//! `s3a-spatial`: `source_id` (16), `content_hash` (32, SHA-256 of the
//! source bytes), `origin_ptr` (16, opaque pointer to the original file).
//! Layout version 1. Padding bytes must be zero.

use bytemuck::{Pod, Zeroable};

/// WCN kind: Warning.
pub const WCN_KIND_WARNING: u8 = 1;
/// WCN kind: Caution.
pub const WCN_KIND_CAUTION: u8 = 2;
/// WCN kind: Note.
pub const WCN_KIND_NOTE: u8 = 3;

/// Fault-isolation node kind: a test or question.
pub const NODE_KIND_TEST: u16 = 1;
/// Fault-isolation node kind: a corrective action.
pub const NODE_KIND_ACTION: u16 = 2;
/// Fault-isolation node kind: a terminal conclusion.
pub const NODE_KIND_CONCLUSION: u16 = 3;

/// Page bounding boxes are stored in thousandths of the page.
pub const PAGE_THOUSANDTHS_MAX: i32 = 1000;

/// Technical Order section record (tile 41, `to_section`, 128 B).
///
/// | Off | Len | Field |
/// |---:|---:|---|
/// | 0 | 16 | `source_id` |
/// | 16 | 32 | `content_hash` (SHA-256 of the section source bytes) |
/// | 48 | 16 | `origin_ptr` |
/// | 64 | 16 | `doc_id` |
/// | 80 | 4 | `chapter` |
/// | 84 | 4 | `section` |
/// | 88 | 4 | `page` |
/// | 92 | 32 | `title_hash` (SHA-256 of the section title bytes) |
/// | 124 | 4 | `_pad` (must be zero) |
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq, Default)]
pub struct ToSectionRecord {
    pub source_id: [u8; 16],
    pub content_hash: [u8; 32],
    pub origin_ptr: [u8; 16],
    pub doc_id: [u8; 16],
    pub chapter: u32,
    pub section: u32,
    pub page: u32,
    pub title_hash: [u8; 32],
    pub _pad: [u8; 4],
}

impl ToSectionRecord {
    /// Wire size of the record in bytes.
    pub const SIZE: usize = 128;
}

/// Warning / Caution / Note record (tile 42, `to_wcn`, 128 B).
///
/// | Off | Len | Field |
/// |---:|---:|---|
/// | 0 | 16 | `source_id` |
/// | 16 | 32 | `content_hash` (SHA-256 of the WCN text) |
/// | 48 | 16 | `origin_ptr` |
/// | 64 | 16 | `doc_id` |
/// | 80 | 4 | `chapter` |
/// | 84 | 4 | `section` |
/// | 88 | 4 | `page` |
/// | 92 | 1 | `kind` (1=Warning, 2=Caution, 3=Note) |
/// | 93 | 3 | `_pad` (must be zero) |
/// | 96 | 32 | `text_hash` (SHA-256 of the WCN text) |
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq, Default)]
pub struct ToWcnRecord {
    pub source_id: [u8; 16],
    pub content_hash: [u8; 32],
    pub origin_ptr: [u8; 16],
    pub doc_id: [u8; 16],
    pub chapter: u32,
    pub section: u32,
    pub page: u32,
    pub kind: u8,
    pub _pad: [u8; 3],
    pub text_hash: [u8; 32],
}

impl ToWcnRecord {
    /// Wire size of the record in bytes.
    pub const SIZE: usize = 128;
}

/// Fault isolation tree node (tile 43, `to_fault_node`, 128 B).
///
/// | Off | Len | Field |
/// |---:|---:|---|
/// | 0 | 16 | `source_id` |
/// | 16 | 32 | `content_hash` (SHA-256 of the node label) |
/// | 48 | 16 | `origin_ptr` |
/// | 64 | 16 | `doc_id` |
/// | 80 | 8 | `node_id` |
/// | 88 | 2 | `kind` (1=Test, 2=Action, 3=Conclusion) |
/// | 90 | 2 | `_pad` (must be zero) |
/// | 92 | 32 | `label_hash` (SHA-256 of the node label) |
/// | 124 | 4 | `_pad2` (must be zero) |
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq, Default)]
pub struct ToFaultNodeRecord {
    pub source_id: [u8; 16],
    pub content_hash: [u8; 32],
    pub origin_ptr: [u8; 16],
    pub doc_id: [u8; 16],
    pub node_id: u64,
    pub kind: u16,
    pub _pad: [u8; 2],
    pub label_hash: [u8; 32],
    pub _pad2: [u8; 4],
}

impl ToFaultNodeRecord {
    /// Wire size of the record in bytes.
    pub const SIZE: usize = 128;
}

/// Fault isolation tree edge (tile 44, `to_fault_edge`, 128 B).
///
/// | Off | Len | Field |
/// |---:|---:|---|
/// | 0 | 16 | `source_id` |
/// | 16 | 32 | `content_hash` (SHA-256 of the branch label) |
/// | 48 | 16 | `origin_ptr` |
/// | 64 | 16 | `doc_id` |
/// | 80 | 8 | `from_node_id` |
/// | 88 | 8 | `to_node_id` |
/// | 96 | 32 | `outcome_hash` (SHA-256 of the branch label) |
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq, Default)]
pub struct ToFaultEdgeRecord {
    pub source_id: [u8; 16],
    pub content_hash: [u8; 32],
    pub origin_ptr: [u8; 16],
    pub doc_id: [u8; 16],
    pub from_node_id: u64,
    pub to_node_id: u64,
    pub outcome_hash: [u8; 32],
}

impl ToFaultEdgeRecord {
    /// Wire size of the record in bytes.
    pub const SIZE: usize = 128;
}

/// Specification / tolerance table cell (tile 45, `to_spec_cell`, 160 B).
///
/// | Off | Len | Field |
/// |---:|---:|---|
/// | 0 | 16 | `source_id` |
/// | 16 | 32 | `content_hash` (SHA-256 of the cell value bytes) |
/// | 48 | 16 | `origin_ptr` |
/// | 64 | 8 | `table_id` |
/// | 72 | 4 | `row` (1-based) |
/// | 76 | 4 | `col` (1-based) |
/// | 80 | 32 | `value_hash` (SHA-256 of the cell value bytes) |
/// | 112 | 32 | `unit_hash` (SHA-256 of the unit bytes; SHA-256 of empty is allowed) |
/// | 144 | 16 | `_pad` (must be zero) |
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq, Default)]
pub struct ToSpecCellRecord {
    pub source_id: [u8; 16],
    pub content_hash: [u8; 32],
    pub origin_ptr: [u8; 16],
    pub table_id: u64,
    pub row: u32,
    pub col: u32,
    pub value_hash: [u8; 32],
    pub unit_hash: [u8; 32],
    pub _pad: [u8; 16],
}

impl ToSpecCellRecord {
    /// Wire size of the record in bytes.
    pub const SIZE: usize = 160;
}

/// Figure / schematic anchor (tile 46, `to_figure_anchor`, 160 B).
///
/// Bounding box values are signed 32-bit page thousandths
/// (`PAGE_THOUSANDTHS_MAX` = 1000).
///
/// | Off | Len | Field |
/// |---:|---:|---|
/// | 0 | 16 | `source_id` |
/// | 16 | 32 | `content_hash` (SHA-256 of the quote bytes) |
/// | 48 | 16 | `origin_ptr` |
/// | 64 | 16 | `doc_id` |
/// | 80 | 4 | `chapter` |
/// | 84 | 4 | `section` |
/// | 88 | 4 | `page` |
/// | 92 | 4 | `figure_id` |
/// | 96 | 4 | `bbox_x` |
/// | 100 | 4 | `bbox_y` |
/// | 104 | 4 | `bbox_w` |
/// | 108 | 4 | `bbox_h` |
/// | 112 | 32 | `quote_hash` (SHA-256 of the quote bytes) |
/// | 144 | 16 | `_pad` (must be zero) |
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq, Default)]
pub struct ToFigureAnchorRecord {
    pub source_id: [u8; 16],
    pub content_hash: [u8; 32],
    pub origin_ptr: [u8; 16],
    pub doc_id: [u8; 16],
    pub chapter: u32,
    pub section: u32,
    pub page: u32,
    pub figure_id: u32,
    pub bbox_x: i32,
    pub bbox_y: i32,
    pub bbox_w: i32,
    pub bbox_h: i32,
    pub quote_hash: [u8; 32],
    pub _pad: [u8; 16],
}

impl ToFigureAnchorRecord {
    /// Wire size of the record in bytes.
    pub const SIZE: usize = 160;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_sizes_and_alignments() {
        assert_eq!(std::mem::size_of::<ToSectionRecord>(), 128);
        assert_eq!(std::mem::align_of::<ToSectionRecord>(), 8);
        assert_eq!(std::mem::size_of::<ToWcnRecord>(), 128);
        assert_eq!(std::mem::align_of::<ToWcnRecord>(), 8);
        assert_eq!(std::mem::size_of::<ToFaultNodeRecord>(), 128);
        assert_eq!(std::mem::align_of::<ToFaultNodeRecord>(), 8);
        assert_eq!(std::mem::size_of::<ToFaultEdgeRecord>(), 128);
        assert_eq!(std::mem::align_of::<ToFaultEdgeRecord>(), 8);
        assert_eq!(std::mem::size_of::<ToSpecCellRecord>(), 160);
        assert_eq!(std::mem::align_of::<ToSpecCellRecord>(), 8);
        assert_eq!(std::mem::size_of::<ToFigureAnchorRecord>(), 160);
        assert_eq!(std::mem::align_of::<ToFigureAnchorRecord>(), 8);
    }
}

/// Procedure step record (tile 49, `to_procedure_step`, 160 B).
///
/// | Off | Len | Field |
/// |---:|---:|---|
/// | 0 | 16 | `source_id` |
/// | 16 | 32 | `content_hash` (SHA-256 of the step text) |
/// | 48 | 16 | `origin_ptr` |
/// | 64 | 16 | `doc_id` |
/// | 80 | 32 | `para_path_hash` (SHA-256 of the paragraph path bytes) |
/// | 112 | 4 | `step_no` (1-based) |
/// | 116 | 4 | `page` (1-based PDF page) |
/// | 120 | 32 | `text_hash` (SHA-256 of the step text) |
/// | 152 | 8 | `_pad` (must be zero) |
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq, Default)]
pub struct ToProcedureStepRecord {
    pub source_id: [u8; 16],
    pub content_hash: [u8; 32],
    pub origin_ptr: [u8; 16],
    pub doc_id: [u8; 16],
    pub para_path_hash: [u8; 32],
    pub step_no: u32,
    pub page: u32,
    pub text_hash: [u8; 32],
    pub _pad: [u8; 8],
}

impl ToProcedureStepRecord {
    /// Wire size of the record in bytes.
    pub const SIZE: usize = 160;
}

/// Related-publication record (tile 50, `to_reference`, 128 B).
///
/// | Off | Len | Field |
/// |---:|---:|---|
/// | 0 | 16 | `source_id` |
/// | 16 | 32 | `content_hash` (SHA-256 of the title bytes) |
/// | 48 | 16 | `origin_ptr` |
/// | 64 | 16 | `doc_id` |
/// | 80 | 32 | `pub_id_hash` (SHA-256 of the publication identifier bytes) |
/// | 112 | 4 | `page` (1-based PDF page of the citation) |
/// | 116 | 12 | `_pad` (must be zero) |
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq, Default)]
pub struct ToReferenceRecord {
    pub source_id: [u8; 16],
    pub content_hash: [u8; 32],
    pub origin_ptr: [u8; 16],
    pub doc_id: [u8; 16],
    pub pub_id_hash: [u8; 32],
    pub page: u32,
    pub _pad: [u8; 12],
}

impl ToReferenceRecord {
    /// Wire size of the record in bytes.
    pub const SIZE: usize = 128;
}

/// Locator record (tile 51, `to_locator`, 160 B).
///
/// Carries the full paragraph path and printed page for a Technical Order
/// record (including tiles 41..=46, which have no room for those fields).
/// `content_hash` is SHA-256 of `para_path || 0x1F || printed_page`.
/// `printed_page_hash` is the first 16 bytes of SHA-256 of the printed
/// page string (for example `4-2` or `xi`).
///
/// | Off | Len | Field |
/// |---:|---:|---|
/// | 0 | 16 | `source_id` |
/// | 16 | 32 | `content_hash` |
/// | 48 | 16 | `origin_ptr` |
/// | 64 | 16 | `doc_id` |
/// | 80 | 16 | `target_source_id` |
/// | 96 | 4 | `target_tile_type` |
/// | 100 | 4 | `page` (1-based PDF page) |
/// | 104 | 32 | `para_path_hash` |
/// | 136 | 16 | `printed_page_hash` |
/// | 152 | 8 | `_pad` (must be zero) |
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq, Default)]
pub struct ToLocatorRecord {
    pub source_id: [u8; 16],
    pub content_hash: [u8; 32],
    pub origin_ptr: [u8; 16],
    pub doc_id: [u8; 16],
    pub target_source_id: [u8; 16],
    pub target_tile_type: u32,
    pub page: u32,
    pub para_path_hash: [u8; 32],
    pub printed_page_hash: [u8; 16],
    pub _pad: [u8; 8],
}

impl ToLocatorRecord {
    /// Wire size of the record in bytes.
    pub const SIZE: usize = 160;
}

