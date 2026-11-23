//! S3A Domain Pack for Technical Order and OEM maintenance manual records
//! (`s3a-pack-to`).
//!
//! Step 1: fixed-layout records, tile registration, and synthetic-fixture
//! round-trips. ETIMS/S1000D parsing, CAN bridge, and SLM are out of scope.
//!
//! All sample data in this crate is synthetic. It is not Technical Order
//! or OEM content.
//!
//! Tiles 41..=46, addon bit `ADDON_TO` (`0x0100`).

pub mod codec;
pub mod ext;
pub mod pack;
pub mod records;

pub use codec::{
    decode_to_records, encode_to_fault_edge, encode_to_fault_node, encode_to_figure_anchor,
    encode_to_section, encode_to_spec_cell, encode_to_wcn, ToDecoded, ToError,
};
pub use ext::ToCrudExt;
pub use pack::{register_pack_types, ToPack, ToTileType, TO_DESCRIPTORS};
pub use records::{
    ToFaultEdgeRecord, ToFaultNodeRecord, ToFigureAnchorRecord, ToSectionRecord, ToSpecCellRecord,
    ToWcnRecord, NODE_KIND_ACTION, NODE_KIND_CONCLUSION, NODE_KIND_TEST, PAGE_THOUSANDTHS_MAX,
    WCN_KIND_CAUTION, WCN_KIND_NOTE, WCN_KIND_WARNING,
};
