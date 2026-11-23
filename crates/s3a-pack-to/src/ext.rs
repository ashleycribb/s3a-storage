use crate::pack::ToTileType;
use crate::records::{
    ToFaultEdgeRecord, ToFaultNodeRecord, ToFigureAnchorRecord, ToLocatorRecord,
    ToProcedureStepRecord, ToReferenceRecord, ToSectionRecord, ToSpecCellRecord, ToWcnRecord,
};
use s3a_engine::S3ACrudEngine;
use std::io;

/// Append Technical Order records as hyper-tiles.
///
/// Fails with `PermissionDenied` when the archive does not have the
/// `ADDON_TO` bit enabled (the engine write gate enforces it).
pub trait ToCrudExt {
    fn create_to_section_records(&self, records: &[ToSectionRecord]) -> io::Result<u64>;
    fn create_to_wcn_records(&self, records: &[ToWcnRecord]) -> io::Result<u64>;
    fn create_to_fault_node_records(&self, records: &[ToFaultNodeRecord]) -> io::Result<u64>;
    fn create_to_fault_edge_records(&self, records: &[ToFaultEdgeRecord]) -> io::Result<u64>;
    fn create_to_spec_cell_records(&self, records: &[ToSpecCellRecord]) -> io::Result<u64>;
    fn create_to_figure_anchor_records(&self, records: &[ToFigureAnchorRecord]) -> io::Result<u64>;
    fn create_to_procedure_step_records(&self, records: &[ToProcedureStepRecord]) -> io::Result<u64>;
    fn create_to_reference_records(&self, records: &[ToReferenceRecord]) -> io::Result<u64>;
    fn create_to_locator_records(&self, records: &[ToLocatorRecord]) -> io::Result<u64>;
}

impl ToCrudExt for S3ACrudEngine {
    fn create_to_section_records(&self, records: &[ToSectionRecord]) -> io::Result<u64> {
        self.append_tile_records(ToTileType::TO_SECTION, records, None, None, None)
    }

    fn create_to_wcn_records(&self, records: &[ToWcnRecord]) -> io::Result<u64> {
        self.append_tile_records(ToTileType::TO_WCN, records, None, None, None)
    }

    fn create_to_fault_node_records(&self, records: &[ToFaultNodeRecord]) -> io::Result<u64> {
        self.append_tile_records(ToTileType::TO_FAULT_NODE, records, None, None, None)
    }

    fn create_to_fault_edge_records(&self, records: &[ToFaultEdgeRecord]) -> io::Result<u64> {
        self.append_tile_records(ToTileType::TO_FAULT_EDGE, records, None, None, None)
    }

    fn create_to_spec_cell_records(&self, records: &[ToSpecCellRecord]) -> io::Result<u64> {
        self.append_tile_records(ToTileType::TO_SPEC_CELL, records, None, None, None)
    }

    fn create_to_figure_anchor_records(&self, records: &[ToFigureAnchorRecord]) -> io::Result<u64> {
        self.append_tile_records(ToTileType::TO_FIGURE_ANCHOR, records, None, None, None)
    }

    fn create_to_procedure_step_records(&self, records: &[ToProcedureStepRecord]) -> io::Result<u64> {
        self.append_tile_records(ToTileType::TO_PROCEDURE_STEP, records, None, None, None)
    }

    fn create_to_reference_records(&self, records: &[ToReferenceRecord]) -> io::Result<u64> {
        self.append_tile_records(ToTileType::TO_REFERENCE, records, None, None, None)
    }

    fn create_to_locator_records(&self, records: &[ToLocatorRecord]) -> io::Result<u64> {
        self.append_tile_records(ToTileType::TO_LOCATOR, records, None, None, None)
    }
}
