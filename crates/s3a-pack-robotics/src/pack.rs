use core::ops::RangeInclusive;
use s3a_core::{
    registry::{RegistryError, TileLifecycle, TileTypeDescriptor, TileTypeFlags, TileTypeRegistry},
    S3AAddon, ADDON_ROBOTICS,
};

/// Tile type IDs allocated exclusively to the Robotics domain pack.
pub struct RoboticsTileType;

impl RoboticsTileType {
    /// Hyper-Tile 4: Robotics platform kinematic & IMU state record (RoboticsKinematicRecord, 72 bytes)
    pub const ROBOTICS_KINEMATIC: u32 = 4;
}

pub static ROBOTICS_DESCRIPTORS: &[TileTypeDescriptor] = &[TileTypeDescriptor {
    tile_type: RoboticsTileType::ROBOTICS_KINEMATIC,
    name: "robotics_kinematic",
    owner_pack: "Robotics Pack",
    lifecycle: TileLifecycle::Active,
    required_addon: Some(ADDON_ROBOTICS),
    record_size: 72,
    current_layout_version: 1,
    min_supported_layout_version: 1,
    flags: TileTypeFlags::from_bits_truncate(
        TileTypeFlags::FIXED_POD.bits() | TileTypeFlags::COPY_THROUGH_COMPACT.bits(),
    ),
}];

/// Registers tile types for the Robotics domain pack into a registry.
pub fn register_pack_types(reg: &mut TileTypeRegistry) -> Result<(), RegistryError> {
    for desc in ROBOTICS_DESCRIPTORS {
        reg.register(desc.clone())?;
    }
    Ok(())
}

/// Domain pack implementation for Robotics (`s3a-pack-robotics`).
pub struct RoboticsPack;

impl S3AAddon for RoboticsPack {
    fn addon_id(&self) -> u32 {
        ADDON_ROBOTICS
    }

    fn name(&self) -> &'static str {
        "s3a-pack-robotics"
    }

    fn version(&self) -> &'static str {
        "0.1.0"
    }

    fn tile_range(&self) -> RangeInclusive<u32> {
        4..=4
    }

    fn descriptors(&self) -> &'static [TileTypeDescriptor] {
        ROBOTICS_DESCRIPTORS
    }
}
