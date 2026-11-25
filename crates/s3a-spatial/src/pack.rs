use core::ops::RangeInclusive;
use s3a_core::{
    registry::{RegistryError, TileLifecycle, TileTypeDescriptor, TileTypeFlags, TileTypeRegistry},
    S3AAddon, ADDON_SPATIAL,
};

/// Tile type IDs allocated exclusively to the Spatial domain pack.
pub struct SpatialTileType;

impl SpatialTileType {
    /// Hyper-Tile 33: OGC GeoPose Basic-Quaternion record (GeoposeBasicQuatRecord, 128 bytes)
    pub const GEOPOSE_BASIC_QUAT: u32 = 33;
}

pub static SPATIAL_DESCRIPTORS: &[TileTypeDescriptor] = &[TileTypeDescriptor {
    tile_type: SpatialTileType::GEOPOSE_BASIC_QUAT,
    name: "geopose_basic_quat",
    owner_pack: "Spatial Pack",
    lifecycle: TileLifecycle::Active,
    required_addon: Some(ADDON_SPATIAL),
    record_size: 128,
    current_layout_version: 1,
    min_supported_layout_version: 1,
    flags: TileTypeFlags::from_bits_truncate(
        TileTypeFlags::FIXED_POD.bits() | TileTypeFlags::COPY_THROUGH_COMPACT.bits(),
    ),
}];

/// Registers tile types for the Spatial domain pack into a registry.
pub fn register_pack_types(reg: &mut TileTypeRegistry) -> Result<(), RegistryError> {
    for desc in SPATIAL_DESCRIPTORS {
        reg.register(desc.clone())?;
    }
    Ok(())
}

/// Domain pack implementation for Spatial (`s3a-spatial`).
pub struct SpatialPack;

impl S3AAddon for SpatialPack {
    fn addon_id(&self) -> u32 {
        ADDON_SPATIAL
    }

    fn name(&self) -> &'static str {
        "s3a-spatial"
    }

    fn version(&self) -> &'static str {
        "0.1.0"
    }

    fn tile_range(&self) -> RangeInclusive<u32> {
        33..=33
    }

    fn descriptors(&self) -> &'static [TileTypeDescriptor] {
        SPATIAL_DESCRIPTORS
    }
}
