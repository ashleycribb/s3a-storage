use crate::pack::CanTileType;
use crate::records::{CanDtcRecord, CanTriggerRecord};
use s3a_engine::S3ACrudEngine;
use s3a_pack_to::ToFaultNodeRecord;
use std::io;

/// Append CAN bridge records as hyper-tiles.
///
/// Fails with `PermissionDenied` when the archive does not have the
/// `ADDON_CAN` bit enabled (the engine write gate enforces it).
pub trait CanCrudExt {
    fn create_can_dtc_records(&self, records: &[CanDtcRecord]) -> io::Result<u64>;
    fn create_can_trigger_records(&self, records: &[CanTriggerRecord]) -> io::Result<u64>;
}

impl CanCrudExt for S3ACrudEngine {
    fn create_can_dtc_records(&self, records: &[CanDtcRecord]) -> io::Result<u64> {
        self.append_tile_records(CanTileType::CAN_DTC, records, None, None, None)
    }

    fn create_can_trigger_records(&self, records: &[CanTriggerRecord]) -> io::Result<u64> {
        self.append_tile_records(CanTileType::CAN_TRIGGER, records, None, None, None)
    }
}

/// True when a trigger record points at the given pack-to fault node.
pub fn trigger_matches_node(trigger: &CanTriggerRecord, node: &ToFaultNodeRecord) -> bool {
    trigger.fault_node_id != 0 && trigger.fault_node_id == node.node_id
}
