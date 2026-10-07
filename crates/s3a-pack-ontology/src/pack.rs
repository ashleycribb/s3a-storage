use core::ops::RangeInclusive;
use s3a_core::{
    registry::{RegistryError, TileLifecycle, TileTypeDescriptor, TileTypeFlags, TileTypeRegistry},
    S3AAddon, ADDON_ONTOLOGY,
};

/// Tile type IDs allocated exclusively to the Ontology domain pack.
pub struct OntologyTileType;

impl OntologyTileType {
    /// Hyper-Tile 16: Normalized Ontology Concept Record (OntologyConceptRecord, 64 bytes)
    pub const ONTOLOGY_CONCEPT: u32 = 16;
    /// Hyper-Tile 17: Ontology Axiom & Rule Record (OntologyAxiomRecord, 64 bytes)
    pub const ONTOLOGY_AXIOM: u32 = 17;
}

pub static ONTOLOGY_DESCRIPTORS: &[TileTypeDescriptor] = &[
    TileTypeDescriptor {
        tile_type: OntologyTileType::ONTOLOGY_CONCEPT,
        name: "ontology_concept",
        owner_pack: "Ontology Pack",
        lifecycle: TileLifecycle::Active,
        required_addon: Some(ADDON_ONTOLOGY),
        record_size: 64,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::from_bits_truncate(
            TileTypeFlags::FIXED_POD.bits() | TileTypeFlags::COPY_THROUGH_COMPACT.bits(),
        ),
    },
    TileTypeDescriptor {
        tile_type: OntologyTileType::ONTOLOGY_AXIOM,
        name: "ontology_axiom",
        owner_pack: "Ontology Pack",
        lifecycle: TileLifecycle::Active,
        required_addon: Some(ADDON_ONTOLOGY),
        record_size: 64,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::from_bits_truncate(
            TileTypeFlags::FIXED_POD.bits() | TileTypeFlags::COPY_THROUGH_COMPACT.bits(),
        ),
    },
];

/// Registers tile types for the Ontology domain pack into a registry.
pub fn register_pack_types(reg: &mut TileTypeRegistry) -> Result<(), RegistryError> {
    for desc in ONTOLOGY_DESCRIPTORS {
        reg.register(desc.clone())?;
    }
    Ok(())
}

/// Domain pack implementation for Ontology (`s3a-pack-ontology`).
pub struct OntologyPack;

impl S3AAddon for OntologyPack {
    fn addon_id(&self) -> u32 {
        ADDON_ONTOLOGY
    }

    fn name(&self) -> &'static str {
        "s3a-pack-ontology"
    }

    fn version(&self) -> &'static str {
        "0.1.0"
    }

    fn tile_range(&self) -> RangeInclusive<u32> {
        16..=17
    }

    fn descriptors(&self) -> &'static [TileTypeDescriptor] {
        ONTOLOGY_DESCRIPTORS
    }
}
