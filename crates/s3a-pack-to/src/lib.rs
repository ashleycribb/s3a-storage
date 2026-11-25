//! S3A Domain Pack for Technical Order and OEM maintenance manual records
//! (`s3a-pack-to`).
//!
//! Step 1: fixed-layout records, tile registration, and synthetic-fixture
//! round-trips. ETIMS/S1000D parsing, CAN bridge, and SLM are out of scope.
//!
//! All sample data in this crate is synthetic. It is not Technical Order
//! or OEM content.
//!
//! Tiles 41..=46, 49..=51, and 52, addon bit `ADDON_TO` (`0x0100`).
//! Tiles 47..=48 are reserved for `s3a-can-bridge`.

pub mod codec;
pub mod ext;
pub mod pack;
pub mod records;

pub use codec::{
    decode_to_records, encode_to_fault_edge, encode_to_fault_node, encode_to_figure_anchor,
    encode_to_locator, encode_to_procedure_step, encode_to_reference, encode_to_scan_agreement,
    encode_to_section, encode_to_spec_cell, encode_to_wcn, ToDecoded, ToError,
};
pub use ext::ToCrudExt;
pub use pack::{
    register_pack_types, ToPack, ToTileType, TO_DESCRIPTORS, TO_LOCATOR_DESCRIPTORS,
    TO_SCAN_DESCRIPTORS,
};
pub use records::{
    scan_status_label, ToFaultEdgeRecord, ToFaultNodeRecord, ToFigureAnchorRecord, ToLocatorRecord,
    ToProcedureStepRecord, ToReferenceRecord, ToScanAgreementRecord, ToSectionRecord,
    ToSpecCellRecord, ToWcnRecord, NODE_KIND_ACTION, NODE_KIND_CONCLUSION, NODE_KIND_TEST,
    PAGE_THOUSANDTHS_MAX, SCAN_FLAG_CER_PRESENT, SCAN_STATUS_HELD, SCAN_STATUS_HUMAN_VERIFIED,
    SCAN_STATUS_SCAN_AGREEMENT, SCAN_STATUS_VERIFIED, WCN_KIND_CAUTION, WCN_KIND_NOTE,
    WCN_KIND_WARNING,
};
