//! Read-only display layouts for record types owned by the engine and the edge profile.
//! See [`s3a_core::display_layout`]: metadata only, never used for writing or sealing.
//!
//! Hyper-tile archives and edge archives reuse tile type numbers with different records
//! (e.g. type 64, 71, 75), so they are separate sets. Pick by `MmapReader::is_edge_archive`.

use s3a_core::display_layout;
use s3a_core::display_layout::{ft, DisplayLayoutRegistry};
use s3a_core::schema::SchemaTypeDefinition;
use s3a_core::TileType;
use s3a_edge::{
    CompactEdgeSensorRecord, ConfigEntryRecord, DeviceIdentityRecord, DeviceSealRecord,
    EdgeConsentRecord, EquipmentEventRecord,
};

use crate::record_roots::RecordRootEntry;

/// Record-root entry (tile 83, 64 bytes).
pub fn record_root_layout() -> SchemaTypeDefinition {
    display_layout!(
        RecordRootEntry,
        TileType::RECORD_ROOT,
        1,
        "record_root",
        "core",
        [
            (tile_index, ft::U32),
            (tile_type, ft::U32),
            (tile_id, ft::U64),
            (record_count, ft::U64),
            (seal_generation, ft::U64),
            (root, ft::BYTES),
        ]
    )
}

/// Layouts for hyper-tile archives from core and the engine: statement index (22) and record
/// roots (83). Domain packs and the vault add theirs (`s3a_pack_scholar::display_layouts`,
/// `s3a_vault::display_layouts`).
pub fn engine_display_layouts() -> Vec<SchemaTypeDefinition> {
    let mut v = s3a_core::display_layout::core_display_layouts();
    v.push(record_root_layout());
    v
}

/// Layouts for edge (micro-tile) archives. Type 64 holds either equipment events (32 bytes) or
/// the device identity (96 bytes): choose by record size (`payload_len / record_count`).
pub fn edge_display_layouts() -> Vec<SchemaTypeDefinition> {
    vec![
        display_layout!(
            EquipmentEventRecord,
            s3a_edge::TILE_TYPE_RECORD as u32,
            1,
            "equipment_event",
            "edge",
            [
                (timestamp_sec, ft::U64),
                (event_type, ft::U16),
                (severity, ft::U8),
                (subsystem_id, ft::U8),
                (event_code, ft::U32),
                (operator_id, ft::BYTES),
                (param1, ft::I32),
                (record_crc32, ft::U32),
            ]
        ),
        display_layout!(
            DeviceIdentityRecord,
            s3a_edge::TILE_TYPE_RECORD as u32,
            1,
            "device_identity",
            "edge",
            [
                (device_id, ft::UUID),
                (hardware_model, ft::STRING),
                (firmware_version, ft::BYTES),
                (public_key, ft::BYTES),
                (provisioned_at, ft::U64),
                (boot_count, ft::U64),
                (record_crc32, ft::U32),
            ]
        ),
        display_layout!(
            DeviceSealRecord,
            s3a_edge::TILE_TYPE_SEAL as u32,
            1,
            "seal_record",
            "edge",
            [
                (tile_index, ft::U32),
                (generation, ft::U64),
                (seal_hash, ft::BYTES),
                (prev_seal_hash, ft::BYTES),
                (signature, ft::BYTES),
                (record_crc32, ft::U32),
            ]
        ),
        display_layout!(
            EdgeConsentRecord,
            s3a_edge::TILE_TYPE_CONSENT as u32,
            1,
            "edge_consent",
            "edge",
            [
                (owner_uuid, ft::UUID),
                (allowed_purposes, ft::U32),
                (sharing_scope, ft::U32),
                (granted_at, ft::U64),
                (expires_at, ft::U64),
                (admin_actor_id, ft::U64),
                (consent_version, ft::U32),
                (record_crc32, ft::U32),
            ]
        ),
        display_layout!(
            CompactEdgeSensorRecord,
            s3a_edge::TILE_TYPE_TIME_SERIES as u32,
            1,
            "edge_sensor",
            "edge",
            [
                (delta_ts_ms, ft::U16),
                (sensor_id, ft::U8),
                (metric_id, ft::U8),
                (quantized_val, ft::I32),
                (quality_flags, ft::U16),
                (record_crc16, ft::U16),
            ]
        ),
        display_layout!(
            ConfigEntryRecord,
            s3a_edge::TILE_TYPE_CONFIG as u32,
            1,
            "config_entry",
            "edge",
            [
                (key, ft::STRING),
                (val_type, ft::U16),
                (val_bytes, ft::BYTES),
                (record_crc32, ft::U32),
            ]
        ),
    ]
}

/// A validated registry of [`engine_display_layouts`].
pub fn engine_display_registry() -> DisplayLayoutRegistry {
    DisplayLayoutRegistry::from_layouts(engine_display_layouts())
        .expect("built-in engine display layouts are valid")
}

/// A validated registry of [`edge_display_layouts`].
pub fn edge_display_registry() -> DisplayLayoutRegistry {
    DisplayLayoutRegistry::from_layouts(edge_display_layouts())
        .expect("built-in edge display layouts are valid")
}
