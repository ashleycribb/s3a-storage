use core::ops::RangeInclusive;
use s3a_core::{
    registry::{RegistryError, TileLifecycle, TileTypeDescriptor, TileTypeFlags, TileTypeRegistry},
    S3AAddon, ADDON_GIS,
};

/// Tile type IDs allocated exclusively to the GIS domain pack.
pub struct GisTileType;

impl GisTileType {
    /// Hyper-Tile 5: GIS surveying & subsurface point cloud / mesh record (GisSurveyPointRecord, 32 bytes)
    pub const GIS_SURVEY_MESH: u32 = 5;
}

pub static GIS_DESCRIPTORS: &[TileTypeDescriptor] = &[TileTypeDescriptor {
    tile_type: GisTileType::GIS_SURVEY_MESH,
    name: "gis_survey_mesh",
    owner_pack: "GIS Pack",
    lifecycle: TileLifecycle::Active,
    required_addon: Some(ADDON_GIS),
    record_size: 32,
    current_layout_version: 1,
    min_supported_layout_version: 1,
    flags: TileTypeFlags::from_bits_truncate(
        TileTypeFlags::FIXED_POD.bits() | TileTypeFlags::COPY_THROUGH_COMPACT.bits(),
    ),
}];

/// Registers tile types for the GIS domain pack into a registry.
pub fn register_pack_types(reg: &mut TileTypeRegistry) -> Result<(), RegistryError> {
    for desc in GIS_DESCRIPTORS {
        reg.register(desc.clone())?;
    }
    Ok(())
}

/// Domain pack implementation for GIS (`s3a-pack-gis`).
pub struct GisPack;

impl S3AAddon for GisPack {
    fn addon_id(&self) -> u32 {
        ADDON_GIS
    }

    fn name(&self) -> &'static str {
        "s3a-pack-gis"
    }

    fn version(&self) -> &'static str {
        "0.1.0"
    }

    fn tile_range(&self) -> RangeInclusive<u32> {
        5..=5
    }

    fn descriptors(&self) -> &'static [TileTypeDescriptor] {
        GIS_DESCRIPTORS
    }
}
