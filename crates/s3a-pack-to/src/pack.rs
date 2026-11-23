use core::ops::RangeInclusive;
use s3a_core::{
    registry::{RegistryError, TileLifecycle, TileTypeDescriptor, TileTypeFlags, TileTypeRegistry},
    S3AAddon, ADDON_TO,
};

/// Tile type IDs allocated exclusively to the Technical Order domain pack.
pub struct ToTileType;

impl ToTileType {
    /// Hyper-Tile 41: Technical Order section (`ToSectionRecord`, 128 bytes)
    pub const TO_SECTION: u32 = 41;
    /// Hyper-Tile 42: Warning / Caution / Note (`ToWcnRecord`, 128 bytes)
    pub const TO_WCN: u32 = 42;
    /// Hyper-Tile 43: Fault isolation tree node (`ToFaultNodeRecord`, 128 bytes)
    pub const TO_FAULT_NODE: u32 = 43;
    /// Hyper-Tile 44: Fault isolation tree edge (`ToFaultEdgeRecord`, 128 bytes)
    pub const TO_FAULT_EDGE: u32 = 44;
    /// Hyper-Tile 45: Spec / tolerance table cell (`ToSpecCellRecord`, 160 bytes)
    pub const TO_SPEC_CELL: u32 = 45;
    /// Hyper-Tile 46: Figure / schematic anchor (`ToFigureAnchorRecord`, 160 bytes)
    pub const TO_FIGURE_ANCHOR: u32 = 46;
}

const FIXED_POD_COPY: TileTypeFlags = TileTypeFlags::from_bits_truncate(
    TileTypeFlags::FIXED_POD.bits() | TileTypeFlags::COPY_THROUGH_COMPACT.bits(),
);

pub static TO_DESCRIPTORS: &[TileTypeDescriptor] = &[
    TileTypeDescriptor {
        tile_type: ToTileType::TO_SECTION,
        name: "to_section",
        owner_pack: "Technical Order Pack",
        lifecycle: TileLifecycle::Active,
        required_addon: Some(ADDON_TO),
        record_size: 128,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: FIXED_POD_COPY,
    },
    TileTypeDescriptor {
        tile_type: ToTileType::TO_WCN,
        name: "to_wcn",
        owner_pack: "Technical Order Pack",
        lifecycle: TileLifecycle::Active,
        required_addon: Some(ADDON_TO),
        record_size: 128,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: FIXED_POD_COPY,
    },
    TileTypeDescriptor {
        tile_type: ToTileType::TO_FAULT_NODE,
        name: "to_fault_node",
        owner_pack: "Technical Order Pack",
        lifecycle: TileLifecycle::Active,
        required_addon: Some(ADDON_TO),
        record_size: 128,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: FIXED_POD_COPY,
    },
    TileTypeDescriptor {
        tile_type: ToTileType::TO_FAULT_EDGE,
        name: "to_fault_edge",
        owner_pack: "Technical Order Pack",
        lifecycle: TileLifecycle::Active,
        required_addon: Some(ADDON_TO),
        record_size: 128,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: FIXED_POD_COPY,
    },
    TileTypeDescriptor {
        tile_type: ToTileType::TO_SPEC_CELL,
        name: "to_spec_cell",
        owner_pack: "Technical Order Pack",
        lifecycle: TileLifecycle::Active,
        required_addon: Some(ADDON_TO),
        record_size: 160,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: FIXED_POD_COPY,
    },
    TileTypeDescriptor {
        tile_type: ToTileType::TO_FIGURE_ANCHOR,
        name: "to_figure_anchor",
        owner_pack: "Technical Order Pack",
        lifecycle: TileLifecycle::Active,
        required_addon: Some(ADDON_TO),
        record_size: 160,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: FIXED_POD_COPY,
    },
];

/// Registers tile types for the Technical Order domain pack into a registry.
pub fn register_pack_types(reg: &mut TileTypeRegistry) -> Result<(), RegistryError> {
    for desc in TO_DESCRIPTORS {
        reg.register(desc.clone())?;
    }
    Ok(())
}

/// Domain pack implementation for Technical Orders (`s3a-pack-to`).
pub struct ToPack;

impl S3AAddon for ToPack {
    fn addon_id(&self) -> u32 {
        ADDON_TO
    }

    fn name(&self) -> &'static str {
        "s3a-pack-to"
    }

    fn version(&self) -> &'static str {
        "0.1.0"
    }

    fn tile_range(&self) -> RangeInclusive<u32> {
        41..=46
    }

    fn descriptors(&self) -> &'static [TileTypeDescriptor] {
        TO_DESCRIPTORS
    }
}
