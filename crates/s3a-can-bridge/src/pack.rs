use core::ops::RangeInclusive;
use s3a_core::{
    registry::{RegistryError, TileLifecycle, TileTypeDescriptor, TileTypeFlags, TileTypeRegistry},
    S3AAddon, ADDON_CAN,
};

/// Tile type IDs allocated exclusively to the CAN bridge.
pub struct CanTileType;

impl CanTileType {
    /// Hyper-Tile 47: decoded diagnostic trouble code (`CanDtcRecord`, 128 bytes)
    pub const CAN_DTC: u32 = 47;
    /// Hyper-Tile 48: fail-closed trigger onto a pack-to node (`CanTriggerRecord`, 128 bytes)
    pub const CAN_TRIGGER: u32 = 48;
}

const FIXED_POD_COPY: TileTypeFlags = TileTypeFlags::from_bits_truncate(
    TileTypeFlags::FIXED_POD.bits() | TileTypeFlags::COPY_THROUGH_COMPACT.bits(),
);

pub static CAN_DESCRIPTORS: &[TileTypeDescriptor] = &[
    TileTypeDescriptor {
        tile_type: CanTileType::CAN_DTC,
        name: "can_dtc",
        owner_pack: "CAN Bridge",
        lifecycle: TileLifecycle::Active,
        required_addon: Some(ADDON_CAN),
        record_size: 128,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: FIXED_POD_COPY,
    },
    TileTypeDescriptor {
        tile_type: CanTileType::CAN_TRIGGER,
        name: "can_trigger",
        owner_pack: "CAN Bridge",
        lifecycle: TileLifecycle::Active,
        required_addon: Some(ADDON_CAN),
        record_size: 128,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: FIXED_POD_COPY,
    },
];

/// Registers tile types for the CAN bridge into a registry.
pub fn register_pack_types(reg: &mut TileTypeRegistry) -> Result<(), RegistryError> {
    for desc in CAN_DESCRIPTORS {
        reg.register(desc.clone())?;
    }
    Ok(())
}

/// Domain pack implementation for the CAN bridge (`s3a-can-bridge`).
pub struct CanBridgePack;

impl S3AAddon for CanBridgePack {
    fn addon_id(&self) -> u32 {
        ADDON_CAN
    }

    fn name(&self) -> &'static str {
        "s3a-can-bridge"
    }

    fn version(&self) -> &'static str {
        "0.1.0"
    }

    fn tile_range(&self) -> RangeInclusive<u32> {
        47..=48
    }

    fn descriptors(&self) -> &'static [TileTypeDescriptor] {
        CAN_DESCRIPTORS
    }
}
