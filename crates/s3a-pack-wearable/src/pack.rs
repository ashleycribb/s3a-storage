use core::ops::RangeInclusive;
use s3a_core::{
    registry::{RegistryError, TileLifecycle, TileTypeDescriptor, TileTypeFlags, TileTypeRegistry},
    S3AAddon, ADDON_WEARABLE,
};

/// Tile type IDs allocated exclusively to the Wearable domain pack.
pub struct WearableTileType;

impl WearableTileType {
    /// Micro-Tile 3: Ultra-compact wearable telemetry (PPG, IMU, heart-rate, gyro)
    pub const WEARABLE_MICRO: u32 = 3;
}

pub static WEARABLE_DESCRIPTORS: &[TileTypeDescriptor] = &[TileTypeDescriptor {
    tile_type: WearableTileType::WEARABLE_MICRO,
    name: "wearable_micro",
    owner_pack: "Wearable Pack",
    lifecycle: TileLifecycle::Active,
    required_addon: Some(ADDON_WEARABLE),
    record_size: 16,
    current_layout_version: 1,
    min_supported_layout_version: 1,
    flags: TileTypeFlags::from_bits_truncate(
        TileTypeFlags::FIXED_POD.bits()
            | TileTypeFlags::COPY_THROUGH_COMPACT.bits()
            | TileTypeFlags::MICRO_TILE.bits(),
    ),
}];

/// Registers tile types for the Wearable domain pack into a registry.
pub fn register_pack_types(reg: &mut TileTypeRegistry) -> Result<(), RegistryError> {
    for desc in WEARABLE_DESCRIPTORS {
        reg.register(desc.clone())?;
    }
    Ok(())
}

/// Domain pack implementation for Wearable Micro-Telemetry (`s3a-pack-wearable`).
pub struct WearablePack;

impl S3AAddon for WearablePack {
    fn addon_id(&self) -> u32 {
        ADDON_WEARABLE
    }

    fn name(&self) -> &'static str {
        "s3a-pack-wearable"
    }

    fn version(&self) -> &'static str {
        "0.1.0"
    }

    fn tile_range(&self) -> RangeInclusive<u32> {
        3..=3
    }

    fn descriptors(&self) -> &'static [TileTypeDescriptor] {
        WEARABLE_DESCRIPTORS
    }
}
