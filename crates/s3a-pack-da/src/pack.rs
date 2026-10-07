use core::ops::RangeInclusive;
use s3a_core::{
    registry::{RegistryError, TileLifecycle, TileTypeDescriptor, TileTypeFlags, TileTypeRegistry},
    S3AAddon, ADDON_DA,
};

/// Tile type IDs allocated exclusively to the Data Availability domain pack.
pub struct DaTileType;

impl DaTileType {
    /// Hyper-Tile 6: Decentralized Data Availability & state commitment record (DaCommitmentRecord, 64 bytes)
    pub const DATA_AVAILABILITY: u32 = 6;
}

pub static DA_DESCRIPTORS: &[TileTypeDescriptor] = &[TileTypeDescriptor {
    tile_type: DaTileType::DATA_AVAILABILITY,
    name: "data_availability",
    owner_pack: "Data Availability Pack",
    lifecycle: TileLifecycle::Active,
    required_addon: Some(ADDON_DA),
    record_size: 64,
    current_layout_version: 1,
    min_supported_layout_version: 1,
    flags: TileTypeFlags::from_bits_truncate(
        TileTypeFlags::FIXED_POD.bits() | TileTypeFlags::COPY_THROUGH_COMPACT.bits(),
    ),
}];

/// Registers tile types for the Data Availability domain pack into a registry.
pub fn register_pack_types(reg: &mut TileTypeRegistry) -> Result<(), RegistryError> {
    for desc in DA_DESCRIPTORS {
        reg.register(desc.clone())?;
    }
    Ok(())
}

/// Domain pack implementation for Data Availability (`s3a-pack-da`).
pub struct DaPack;

impl S3AAddon for DaPack {
    fn addon_id(&self) -> u32 {
        ADDON_DA
    }

    fn name(&self) -> &'static str {
        "s3a-pack-da"
    }

    fn version(&self) -> &'static str {
        "0.1.0"
    }

    fn tile_range(&self) -> RangeInclusive<u32> {
        6..=6
    }

    fn descriptors(&self) -> &'static [TileTypeDescriptor] {
        DA_DESCRIPTORS
    }
}
