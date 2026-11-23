//! S3A CAN bus bridge (`s3a-can-bridge`).
//!
//! Decodes J1939 (including DM1 and BAM) and OBD-II Mode 03 frames, ingests
//! replayable logs, and maps decoded faults to `s3a-pack-to` fault-isolation
//! node identifiers through a checked-in mapping table. Unmapped faults
//! return `NoMatch`. The crate never guesses.
//!
//! All sample data in this crate is synthetic. It is not a vehicle capture,
//! not a Technical Order, and not ETIMS or OEM content.
//!
//! Tiles 47..=48, addon bit `ADDON_CAN` (`0x0200`).
//!
//! Live SocketCAN is feature-gated (`socketcan`) and is not a fallback.

pub mod codec;
pub mod error;
pub mod ext;
pub mod frame;
pub mod ingest;
pub mod j1939;
pub mod log;
pub mod obd;
pub mod pack;
pub mod records;
#[cfg(feature = "socketcan")]
pub mod socketcan;
pub mod trigger;

pub use codec::{decode_can_records, encode_can_dtc, encode_can_trigger, CanDecoded};
pub use error::{CanError, TriggerResult};
pub use ext::{trigger_matches_node, CanCrudExt};
pub use frame::CanFrame;
pub use ingest::{ingest_candump, IngestReport};
pub use j1939::{
    decode_dm1, decode_j1939_dtcs, parse_j1939_id, DecodedJ1939Dtc, J1939Dtc, J1939Id,
};
pub use log::{parse_can_csv, parse_candump, parse_vector_asc};
pub use obd::{format_obd_dtc, parse_obd_mode03};
pub use pack::{register_pack_types, CanBridgePack, CanTileType, CAN_DESCRIPTORS};
pub use records::{
    CanDtcRecord, CanTriggerRecord, DTC_KIND_J1939_DM1, DTC_KIND_OBD_MODE03, TRIGGER_KIND_J1939,
    TRIGGER_KIND_OBD, TRIGGER_KIND_PARAM,
};
#[cfg(feature = "socketcan")]
pub use socketcan::SocketCanReader;
pub use trigger::{MappingTable, ParamEvaluation, ThresholdTable};

/// `true` only when the crate is built with the `socketcan` feature.
#[cfg(feature = "socketcan")]
pub const SOCKETCAN_ENABLED: bool = true;
/// `true` only when the crate is built with the `socketcan` feature.
#[cfg(not(feature = "socketcan"))]
pub const SOCKETCAN_ENABLED: bool = false;
