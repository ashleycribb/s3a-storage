use core::fmt;
use alloc::vec::Vec;
use alloc::string::String;
use alloc::format;

/// Bitflags representing capabilities, constraints, and layout policies of a tile type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TileTypeFlags(pub u32);

impl TileTypeFlags {
    pub const NONE: Self = Self(0);
    /// Fixed-size Pod record array payload (record_count * record_size <= payload capacity).
    pub const FIXED_POD: Self = Self(1 << 0);
    /// Compactor can safely copy through without domain-specific re-encoding.
    pub const COPY_THROUGH_COMPACT: Self = Self(1 << 1);
    /// Micro-tile memory buffer format (4 KB SPI NOR page blocks).
    pub const MICRO_TILE: Self = Self(1 << 2);
    /// Variable-length or custom binary serialization format.
    pub const VARIABLE_PAYLOAD: Self = Self(1 << 3);
    /// Reserved boundary tile slot: no user data writes permitted.
    pub const RESERVED: Self = Self(1 << 4);
    /// Deprecated tile type (e.g. pre-Phase 2 ambiguous assignments).
    pub const DEPRECATED: Self = Self(1 << 5);

    pub const fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn from_bits_truncate(bits: u32) -> Self {
        Self(bits)
    }

    pub const fn from_bits(bits: u32) -> Option<Self> {
        Some(Self(bits))
    }

    pub const fn bits(&self) -> u32 {
        self.0
    }
}

impl core::ops::BitOr for TileTypeFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl core::ops::BitOrAssign for TileTypeFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// Lifecycle state of a registered tile type in the S3A schema registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TileLifecycle {
    /// Active tile type: open for both reading and writing.
    #[default]
    Active,
    /// Deprecated tile type: open for read-only querying, writes are prohibited.
    Deprecated,
    /// Retired tile type: obsolete, reads and writes are rejected or legacy-only.
    Retired,
}

impl TileLifecycle {
    pub const fn is_active(&self) -> bool {
        matches!(self, Self::Active)
    }

    pub const fn is_read_only(&self) -> bool {
        matches!(self, Self::Deprecated)
    }

    pub const fn allows_writes(&self) -> bool {
        matches!(self, Self::Active)
    }
}

/// Metadata descriptor defining the wire format, layout versioning, and authorization rules for a tile type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileTypeDescriptor {
    /// Wire tile identifier stored in `HyperTileHeader.tile_type`.
    pub tile_type: u32,
    /// Human-readable canonical name (e.g. "academic_papers", "telemetry").
    pub name: &'static str,
    /// Crate or subsystem owning the record layout ("core", "s3a-pack-scholar", etc.).
    pub owner_pack: &'static str,
    /// Lifecycle state (Active, Deprecated, Retired).
    pub lifecycle: TileLifecycle,
    /// Required capability bit in `FileHeader.addon_mask` (None for core storage).
    pub required_addon: Option<u32>,
    /// Byte size of each record for fixed Pod layouts; 0 if variable.
    pub record_size: usize,
    /// Current layout version produced by writers.
    pub current_layout_version: u32,
    /// Minimum layout version supported by readers (reader floor).
    pub min_supported_layout_version: u32,
    /// Behavioral and format flags.
    pub flags: TileTypeFlags,
}

/// Errors returned by the tile type registry during registration or validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    DuplicateTileType {
        tile_type: u32,
        existing_name: &'static str,
        new_name: &'static str,
    },
    UnregisteredTileType(u32),
    ReservedTile(u32),
    DeprecatedTile(u32),
    RetiredTile(u32),
    MissingAddon {
        tile_type: u32,
        pack: &'static str,
        required_mask: u32,
        actual_mask: u32,
    },
    UnsupportedLayoutVersionTooOld {
        tile_type: u32,
        version: u32,
        min_supported: u32,
    },
    UnsupportedLayoutVersionTooNew {
        tile_type: u32,
        version: u32,
        current_supported: u32,
    },
    MigrationFailed(String),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistryError::DuplicateTileType { tile_type, existing_name, new_name } => {
                write!(f, "Duplicate registration for tile type {}: existing '{}' conflicts with new '{}'", tile_type, existing_name, new_name)
            }
            RegistryError::UnregisteredTileType(tt) => {
                write!(f, "Tile type {} is not registered", tt)
            }
            RegistryError::ReservedTile(tt) => {
                write!(f, "Tile type {} is marked RESERVED (no writes permitted; see docs/schema-evolution.md)", tt)
            }
            RegistryError::DeprecatedTile(tt) => {
                write!(f, "Tile type {} is DEPRECATED (read-only; no writes permitted)", tt)
            }
            RegistryError::RetiredTile(tt) => {
                write!(f, "Tile type {} is RETIRED (no reads or writes permitted)", tt)
            }
            RegistryError::MissingAddon { tile_type, pack, required_mask, actual_mask } => {
                write!(f, "Tile type {} belongs to {} but addon bit 0x{:04x} is not enabled on archive (addon_mask: 0x{:04x})", tile_type, pack, required_mask, actual_mask)
            }
            RegistryError::UnsupportedLayoutVersionTooOld { tile_type, version, min_supported } => {
                write!(f, "Tile type {} layout version {} is too old (minimum supported: {})", tile_type, version, min_supported)
            }
            RegistryError::UnsupportedLayoutVersionTooNew { tile_type, version, current_supported } => {
                write!(f, "Tile type {} layout version {} is too new (current supported: {})", tile_type, version, current_supported)
            }
            RegistryError::MigrationFailed(msg) => {
                write!(f, "Tile payload migration failed: {}", msg)
            }
        }
    }
}

/// In-memory registry of known tile types, enforcing layout version constraints and capability write gates.
#[derive(Debug, Clone)]
pub struct TileTypeRegistry {
    descriptors: Vec<TileTypeDescriptor>,
}

impl Default for TileTypeRegistry {
    fn default() -> Self {
        Self::with_standard_types()
    }
}

impl TileTypeRegistry {
    pub fn new() -> Self {
        Self {
            descriptors: Vec::new(),
        }
    }

    /// Initializes a registry containing only core domain-agnostic storage primitives (tiles 0, 1, 2, 7, 8, 9).
    pub fn with_core_types() -> Self {
        let mut reg = Self::new();
        register_core_types(&mut reg).expect("Core types must register cleanly");
        reg
    }

    /// Initializes a registry containing core storage primitives and standard domain pack descriptors (tiles 0..=15).
    pub fn with_standard_types() -> Self {
        let mut reg = Self::new();
        register_core_types(&mut reg).expect("Core types must register cleanly");
        register_standard_pack_types(&mut reg).expect("Standard pack types must register cleanly");
        reg
    }

    /// Registers a new tile type descriptor.
    /// Re-registering an identical descriptor is an idempotent no-op.
    /// Conflicting duplicate tile IDs return `RegistryError::DuplicateTileType`.
    pub fn register(&mut self, descriptor: TileTypeDescriptor) -> Result<(), RegistryError> {
        if let Some(existing) = self.get(descriptor.tile_type) {
            if existing == &descriptor {
                return Ok(());
            }
            return Err(RegistryError::DuplicateTileType {
                tile_type: descriptor.tile_type,
                existing_name: existing.name,
                new_name: descriptor.name,
            });
        }
        self.descriptors.push(descriptor);
        Ok(())
    }

    /// Looks up a descriptor by wire tile ID.
    pub fn get(&self, tile_type: u32) -> Option<&TileTypeDescriptor> {
        self.descriptors.iter().find(|d| d.tile_type == tile_type)
    }

    /// Returns the required capability bitmask for a given tile type, if any.
    pub fn required_addon(&self, tile_type: u32) -> Option<u32> {
        self.get(tile_type).and_then(|d| d.required_addon)
    }

    /// Validates an intended tile write against capability authorization and layout version boundaries.
    pub fn validate_write(&self, tile_type: u32, layout_version: u32, addon_mask: u32) -> Result<(), RegistryError> {
        let desc = self.get(tile_type).ok_or(RegistryError::UnregisteredTileType(tile_type))?;

        // Fail-closed on lifecycle state (Deprecated is read-only, Retired is inaccessible)
        if desc.lifecycle == TileLifecycle::Deprecated {
            return Err(RegistryError::DeprecatedTile(tile_type));
        }
        if desc.lifecycle == TileLifecycle::Retired {
            return Err(RegistryError::RetiredTile(tile_type));
        }

        // Fail-closed on reserved slots
        if desc.flags.contains(TileTypeFlags::RESERVED) {
            return Err(RegistryError::ReservedTile(tile_type));
        }

        // Enforce required capability bit
        if let Some(mask) = desc.required_addon {
            if (addon_mask & mask) == 0 {
                return Err(RegistryError::MissingAddon {
                    tile_type,
                    pack: desc.owner_pack,
                    required_mask: mask,
                    actual_mask: addon_mask,
                });
            }
        }

        // Layout version bounds
        if layout_version < desc.min_supported_layout_version {
            return Err(RegistryError::UnsupportedLayoutVersionTooOld {
                tile_type,
                version: layout_version,
                min_supported: desc.min_supported_layout_version,
            });
        }
        if layout_version > desc.current_layout_version {
            return Err(RegistryError::UnsupportedLayoutVersionTooNew {
                tile_type,
                version: layout_version,
                current_supported: desc.current_layout_version,
            });
        }

        Ok(())
    }

    /// Validates an incoming tile read against supported layout versions.
    /// Wire layout version 0 is mapped to version 1 as a legacy compatibility alias.
    pub fn validate_read(&self, tile_type: u32, layout_version: u32) -> Result<(), RegistryError> {
        let desc = self.get(tile_type).ok_or(RegistryError::UnregisteredTileType(tile_type))?;

        if desc.lifecycle == TileLifecycle::Retired {
            return Err(RegistryError::RetiredTile(tile_type));
        }

        // Legacy compatibility alias: 0 means pre-Phase 2 unversioned header
        let effective_version = if layout_version == 0 { 1 } else { layout_version };

        if effective_version < desc.min_supported_layout_version {
            return Err(RegistryError::UnsupportedLayoutVersionTooOld {
                tile_type,
                version: effective_version,
                min_supported: desc.min_supported_layout_version,
            });
        }
        if effective_version > desc.current_layout_version {
            return Err(RegistryError::UnsupportedLayoutVersionTooNew {
                tile_type,
                version: effective_version,
                current_supported: desc.current_layout_version,
            });
        }

        Ok(())
    }

    pub fn descriptors(&self) -> &[TileTypeDescriptor] {
        &self.descriptors
    }
}

/// Registers core domain-agnostic storage primitives (tiles 0, 1, 2, 7, 8, and reserved 9).
pub fn register_core_types(reg: &mut TileTypeRegistry) -> Result<(), RegistryError> {
    // Tile 0: Telemetry
    reg.register(TileTypeDescriptor {
        tile_type: 0,
        name: "telemetry",
        owner_pack: "core",
        lifecycle: TileLifecycle::Active,
        required_addon: None,
        record_size: 32,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::FIXED_POD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    // Tile 1: High-dim Embedding (128-float)
    reg.register(TileTypeDescriptor {
        tile_type: 1,
        name: "embedding_128",
        owner_pack: "core",
        lifecycle: TileLifecycle::Active,
        required_addon: None,
        record_size: 544,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::FIXED_POD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    // Tile 2: Hybrid Inverted / Vector Index
    reg.register(TileTypeDescriptor {
        tile_type: 2,
        name: "hybrid_index",
        owner_pack: "core",
        lifecycle: TileLifecycle::Active,
        required_addon: None,
        record_size: 0,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::VARIABLE_PAYLOAD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    // Tile 7: Quantized Embeddings (INT8)
    reg.register(TileTypeDescriptor {
        tile_type: 7,
        name: "quantized_embedding_int8",
        owner_pack: "core",
        lifecycle: TileLifecycle::Active,
        required_addon: None,
        record_size: 40,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::FIXED_POD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    // Tile 8: Quantized Embeddings (4-bit)
    reg.register(TileTypeDescriptor {
        tile_type: 8,
        name: "quantized_embedding_4bit",
        owner_pack: "core",
        lifecycle: TileLifecycle::Active,
        required_addon: None,
        record_size: 24,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::FIXED_POD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    // Tile 9: Reserved Boundary Slot (no user writes permitted)
    reg.register(TileTypeDescriptor {
        tile_type: 9,
        name: "reserved_boundary",
        owner_pack: "core",
        lifecycle: TileLifecycle::Active,
        required_addon: None,
        record_size: 0,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::RESERVED,
    })?;

    // Tile 19: Changeable Metadata Marks
    reg.register(TileTypeDescriptor {
        tile_type: 19,
        name: "marks",
        owner_pack: "core",
        lifecycle: TileLifecycle::Active,
        required_addon: None,
        record_size: 64,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::FIXED_POD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    // Tile 21: Core Statement Body Blobs (variable payload)
    reg.register(TileTypeDescriptor {
        tile_type: 21,
        name: "statement_body",
        owner_pack: "core",
        lifecycle: TileLifecycle::Active,
        required_addon: None,
        record_size: 0,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::VARIABLE_PAYLOAD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    // Tile 22: Core Statement Index (64B Pod records)
    reg.register(TileTypeDescriptor {
        tile_type: 22,
        name: "statement_index",
        owner_pack: "core",
        lifecycle: TileLifecycle::Active,
        required_addon: None,
        record_size: 64,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::FIXED_POD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    // Tile 23: Core Document Store (State / Profile APIs, variable payload)
    reg.register(TileTypeDescriptor {
        tile_type: 23,
        name: "document_store",
        owner_pack: "core",
        lifecycle: TileLifecycle::Active,
        required_addon: None,
        record_size: 0,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::VARIABLE_PAYLOAD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    // Tile 24: Core Attachment Blobs (variable payload)
    reg.register(TileTypeDescriptor {
        tile_type: 24,
        name: "attachment_blob",
        owner_pack: "core",
        lifecycle: TileLifecycle::Active,
        required_addon: None,
        record_size: 0,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::VARIABLE_PAYLOAD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    // Tile 25: ELR Person (P2997 subset)
    reg.register(TileTypeDescriptor {
        tile_type: crate::lrs::TILE_ELR_PERSON,
        name: "elr_person",
        owner_pack: "core",
        lifecycle: TileLifecycle::Active,
        required_addon: None,
        record_size: 0,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::VARIABLE_PAYLOAD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    // Tile 26: ELR Organization (P2997 subset)
    reg.register(TileTypeDescriptor {
        tile_type: crate::lrs::TILE_ELR_ORGANIZATION,
        name: "elr_organization",
        owner_pack: "core",
        lifecycle: TileLifecycle::Active,
        required_addon: None,
        record_size: 0,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::VARIABLE_PAYLOAD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    // Tile 27: ELR Learning Resource (P2997 subset)
    reg.register(TileTypeDescriptor {
        tile_type: crate::lrs::TILE_ELR_LEARNING_RESOURCE,
        name: "elr_learning_resource",
        owner_pack: "core",
        lifecycle: TileLifecycle::Active,
        required_addon: None,
        record_size: 0,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::VARIABLE_PAYLOAD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    // Tile 28: ELR Learning Event (P2997 subset)
    reg.register(TileTypeDescriptor {
        tile_type: crate::lrs::TILE_ELR_LEARNING_EVENT,
        name: "elr_learning_event",
        owner_pack: "core",
        lifecycle: TileLifecycle::Active,
        required_addon: None,
        record_size: 0,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::VARIABLE_PAYLOAD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    // Tile 29: ELR Competency Assertion (P2997 subset)
    reg.register(TileTypeDescriptor {
        tile_type: crate::lrs::TILE_ELR_COMPETENCY_ASSERTION,
        name: "elr_competency_assertion",
        owner_pack: "core",
        lifecycle: TileLifecycle::Active,
        required_addon: None,
        record_size: 0,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::VARIABLE_PAYLOAD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    // Tile 30: ELR Credential Assertion (P2997 subset)
    reg.register(TileTypeDescriptor {
        tile_type: crate::lrs::TILE_ELR_CREDENTIAL_ASSERTION,
        name: "elr_credential_assertion",
        owner_pack: "core",
        lifecycle: TileLifecycle::Active,
        required_addon: None,
        record_size: 0,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::VARIABLE_PAYLOAD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    // Tile 31: ELR Goal (P2997 subset)
    reg.register(TileTypeDescriptor {
        tile_type: crate::lrs::TILE_ELR_GOAL,
        name: "elr_goal",
        owner_pack: "core",
        lifecycle: TileLifecycle::Active,
        required_addon: None,
        record_size: 0,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::VARIABLE_PAYLOAD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    // Tile 65: Vertically Partitioned Columnar Tile (variable payload)
    reg.register(TileTypeDescriptor {
        tile_type: crate::TileType::COLUMNAR,
        name: "columnar",
        owner_pack: "core",
        lifecycle: TileLifecycle::Active,
        required_addon: None,
        record_size: 0,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::VARIABLE_PAYLOAD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    // Tile 66: N-Dimensional Array Tile (variable payload)
    reg.register(TileTypeDescriptor {
        tile_type: crate::TileType::ARRAY,
        name: "array",
        owner_pack: "core",
        lifecycle: TileLifecycle::Active,
        required_addon: None,
        record_size: 0,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::VARIABLE_PAYLOAD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    // Tile 71: Participant Consent & Governance (136B Pod records)
    reg.register(TileTypeDescriptor {
        tile_type: crate::TileType::CONSENT,
        name: "consent",
        owner_pack: "core",
        lifecycle: TileLifecycle::Active,
        required_addon: None,
        record_size: 136,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::FIXED_POD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    // Tile 73: Right-to-be-Forgotten Erasure Tile (144B Pod records)
    reg.register(TileTypeDescriptor {
        tile_type: crate::TileType::ERASURE,
        name: "erasure",
        owner_pack: "core",
        lifecycle: TileLifecycle::Active,
        required_addon: None,
        record_size: 144,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::FIXED_POD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    // Tile 77: Embedded Schema Registry (self-describing, variable payload)
    reg.register(TileTypeDescriptor {
        tile_type: crate::TileType::SCHEMA,
        name: "schema",
        owner_pack: "core",
        lifecycle: TileLifecycle::Active,
        required_addon: None,
        record_size: 0,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::VARIABLE_PAYLOAD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    // Tile 78: Plain-Text UTF-8 Manifest Tile (self-describing, variable payload)
    reg.register(TileTypeDescriptor {
        tile_type: crate::TileType::MANIFEST,
        name: "manifest",
        owner_pack: "core",
        lifecycle: TileLifecycle::Active,
        required_addon: None,
        record_size: 0,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::VARIABLE_PAYLOAD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    // Tile 79: Compressed S3A Specification Tile (variable payload)
    reg.register(TileTypeDescriptor {
        tile_type: crate::TileType::SPECIFICATION,
        name: "specification",
        owner_pack: "core",
        lifecycle: TileLifecycle::Active,
        required_addon: None,
        record_size: 0,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::VARIABLE_PAYLOAD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    // Tile 80: Version Control Commit Metadata Tile
    reg.register(TileTypeDescriptor {
        tile_type: crate::TileType::VCS_COMMIT,
        name: "vcs_commit",
        owner_pack: "vcs",
        lifecycle: TileLifecycle::Active,
        required_addon: None,
        record_size: 0,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::VARIABLE_PAYLOAD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    // Tile 81: Version Control Reference & Branch Tile
    reg.register(TileTypeDescriptor {
        tile_type: crate::TileType::VCS_REF,
        name: "vcs_ref",
        owner_pack: "vcs",
        lifecycle: TileLifecycle::Active,
        required_addon: None,
        record_size: 0,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::VARIABLE_PAYLOAD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    // Tile 82: Version Control Conflict Resolution Tile
    reg.register(TileTypeDescriptor {
        tile_type: crate::TileType::VCS_RESOLUTION,
        name: "vcs_resolution",
        owner_pack: "vcs",
        lifecycle: TileLifecycle::Active,
        required_addon: None,
        record_size: 0,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::VARIABLE_PAYLOAD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    // Tile 85: Gate Network Directory Registration Tile
    reg.register(TileTypeDescriptor {
        tile_type: crate::TileType::GATE_DIRECTORY,
        name: "gate_directory",
        owner_pack: "vault",
        lifecycle: TileLifecycle::Active,
        required_addon: None,
        record_size: 0,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::VARIABLE_PAYLOAD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    // Tile 86: Gate Network Relay Mailbox Item Tile
    reg.register(TileTypeDescriptor {
        tile_type: crate::TileType::GATE_MAILBOX,
        name: "gate_mailbox",
        owner_pack: "vault",
        lifecycle: TileLifecycle::Active,
        required_addon: None,
        record_size: 0,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::VARIABLE_PAYLOAD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    Ok(())
}

/// Registers standard first-class domain pack types (wearable 3, robotics 4, GIS 5, DA 6, Scholar 10..=15).
pub fn register_standard_pack_types(reg: &mut TileTypeRegistry) -> Result<(), RegistryError> {
    // Tile 3: Wearable Micro-Telemetry
    reg.register(TileTypeDescriptor {
        tile_type: 3,
        name: "wearable_micro",
        owner_pack: "Wearable Pack",
        lifecycle: TileLifecycle::Active,
        required_addon: Some(crate::ADDON_WEARABLE),
        record_size: 16,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::FIXED_POD | TileTypeFlags::COPY_THROUGH_COMPACT | TileTypeFlags::MICRO_TILE,
    })?;

    // Tile 4: Robotics Kinematic Trajectories
    reg.register(TileTypeDescriptor {
        tile_type: 4,
        name: "robotics_kinematic",
        owner_pack: "Robotics Pack",
        lifecycle: TileLifecycle::Active,
        required_addon: Some(crate::ADDON_ROBOTICS),
        record_size: 72,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::FIXED_POD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    // Tile 5: GIS Topographic Mesh
    reg.register(TileTypeDescriptor {
        tile_type: 5,
        name: "gis_survey_mesh",
        owner_pack: "GIS Pack",
        lifecycle: TileLifecycle::Active,
        required_addon: Some(crate::ADDON_GIS),
        record_size: 32,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::FIXED_POD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    // Tile 6: Data Availability State Commitments
    reg.register(TileTypeDescriptor {
        tile_type: 6,
        name: "data_availability",
        owner_pack: "Data Availability Pack",
        lifecycle: TileLifecycle::Active,
        required_addon: Some(crate::ADDON_DA),
        record_size: 64,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::FIXED_POD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    // Tiles 10..=15: Scholar Explorer Pack
    reg.register(TileTypeDescriptor {
        tile_type: 10,
        name: "human_lrs",
        owner_pack: "Scholar Pack",
        lifecycle: TileLifecycle::Active,
        required_addon: Some(crate::ADDON_EDU),
        record_size: 64,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::FIXED_POD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;
    reg.register(TileTypeDescriptor {
        tile_type: 11,
        name: "ai_trace_log",
        owner_pack: "Scholar Pack",
        lifecycle: TileLifecycle::Active,
        required_addon: Some(crate::ADDON_EDU),
        record_size: 64,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::FIXED_POD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;
    reg.register(TileTypeDescriptor {
        tile_type: 12,
        name: "academic_papers",
        owner_pack: "Scholar Pack",
        lifecycle: TileLifecycle::Active,
        required_addon: Some(crate::ADDON_EDU),
        record_size: 64,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::FIXED_POD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;
    reg.register(TileTypeDescriptor {
        tile_type: 13,
        name: "research_graph",
        owner_pack: "Scholar Pack",
        lifecycle: TileLifecycle::Active,
        required_addon: Some(crate::ADDON_EDU),
        record_size: 64,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::FIXED_POD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;
    reg.register(TileTypeDescriptor {
        tile_type: 14,
        name: "srs_workspace",
        owner_pack: "Scholar Pack",
        lifecycle: TileLifecycle::Active,
        required_addon: Some(crate::ADDON_EDU),
        record_size: 64,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::FIXED_POD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;
    reg.register(TileTypeDescriptor {
        tile_type: 15,
        name: "srs_manifest",
        owner_pack: "Scholar Pack",
        lifecycle: TileLifecycle::Active,
        required_addon: Some(crate::ADDON_EDU),
        record_size: 64,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::FIXED_POD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    // Tiles 16..=17: Formal Ontology & Axiom Registry Pack
    reg.register(TileTypeDescriptor {
        tile_type: 16,
        name: "ontology_concept",
        owner_pack: "Ontology Pack",
        lifecycle: TileLifecycle::Active,
        required_addon: Some(crate::ADDON_ONTOLOGY),
        record_size: 64,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::FIXED_POD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;
    reg.register(TileTypeDescriptor {
        tile_type: 17,
        name: "ontology_axiom",
        owner_pack: "Ontology Pack",
        lifecycle: TileLifecycle::Active,
        required_addon: Some(crate::ADDON_ONTOLOGY),
        record_size: 64,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::FIXED_POD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    // Tile 18: Scholar Explorer Task Agent Registry
    reg.register(TileTypeDescriptor {
        tile_type: 18,
        name: "agent_registry",
        owner_pack: "Scholar Pack",
        lifecycle: TileLifecycle::Active,
        required_addon: Some(crate::ADDON_EDU),
        record_size: 64,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::FIXED_POD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    // Tile 20: Scholar Explorer Research Notes & Verified Claims
    reg.register(TileTypeDescriptor {
        tile_type: 20,
        name: "note_claims",
        owner_pack: "Scholar Pack",
        lifecycle: TileLifecycle::Active,
        required_addon: Some(crate::ADDON_EDU),
        record_size: 64,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::FIXED_POD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    // Tile 32: Scholar Explorer Cryptographic Citation Fingerprints
    reg.register(TileTypeDescriptor {
        tile_type: 32,
        name: "citation_fingerprint",
        owner_pack: "Scholar Pack",
        lifecycle: TileLifecycle::Active,
        required_addon: Some(crate::ADDON_EDU),
        record_size: 96,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::FIXED_POD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    Ok(())
}

/// Trait hook for migrating raw tile payload bytes between record layout versions.
pub trait TilePayloadMigrator: Send + Sync {
    fn migrate_payload(
        &self,
        tile_type: u32,
        from_version: u32,
        to_version: u32,
        payload: &[u8],
    ) -> Result<Vec<u8>, RegistryError>;
}

/// Demonstration payload migrator hook for converting between layout versions.
pub fn migrate_tile_payload(
    tile_type: u32,
    from_version: u32,
    to_version: u32,
    payload: &[u8],
) -> Result<Vec<u8>, RegistryError> {
    if from_version == to_version {
        return Ok(payload.to_vec());
    }
    if from_version == 0 && to_version == 1 {
        // Legacy v0 maps transparently to v1
        return Ok(payload.to_vec());
    }

    // Demo migrator: e.g. for synthetic test tile 99
    if tile_type == 99 && from_version == 1 && to_version == 2 {
        // Expand 16B records to 20B records by appending 4 bytes of zeroes
        let chunk_size = 16;
        let mut new_payload = Vec::new();
        for chunk in payload.chunks_exact(chunk_size) {
            new_payload.extend_from_slice(chunk);
            new_payload.extend_from_slice(&[0u8; 4]);
        }
        return Ok(new_payload);
    }

    Err(RegistryError::MigrationFailed(format!(
        "No migrator registered for tile type {} from v{} to v{}",
        tile_type, from_version, to_version
    )))
}

#[cfg(feature = "std")]
static GLOBAL_REGISTRY: std::sync::OnceLock<std::sync::RwLock<TileTypeRegistry>> = std::sync::OnceLock::new();

#[cfg(feature = "std")]
fn get_registry_lock() -> &'static std::sync::RwLock<TileTypeRegistry> {
    GLOBAL_REGISTRY.get_or_init(|| {
        std::sync::RwLock::new(TileTypeRegistry::with_standard_types())
    })
}

/// Returns a read guard to the process-global tile type registry.
#[cfg(feature = "std")]
pub fn global_registry() -> std::sync::RwLockReadGuard<'static, TileTypeRegistry> {
    get_registry_lock().read().unwrap()
}

/// Allows mutating the process-global tile type registry (e.g. to register custom/third-party packs).
#[cfg(feature = "std")]
pub fn with_global_registry_mut<F, R>(f: F) -> R
where
    F: FnOnce(&mut TileTypeRegistry) -> R,
{
    let mut w = get_registry_lock().write().unwrap();
    f(&mut *w)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ADDON_WEARABLE;

    #[test]
    fn test_registry_registration_and_lookup() {
        let mut reg = TileTypeRegistry::new();
        register_core_types(&mut reg).unwrap();

        let desc = reg.get(0).unwrap();
        assert_eq!(desc.name, "telemetry");
        assert_eq!(desc.record_size, 32);
        assert_eq!(desc.current_layout_version, 1);
        assert!(desc.flags.contains(TileTypeFlags::FIXED_POD));
    }

    #[test]
    fn test_duplicate_registration_rejected_unless_identical() {
        let mut reg = TileTypeRegistry::new();
        register_core_types(&mut reg).unwrap();

        // Idempotent re-registration succeeds
        let desc0 = reg.get(0).cloned().unwrap();
        assert!(reg.register(desc0).is_ok());

        // Conflicting duplicate fails
        let conflict = TileTypeDescriptor {
            tile_type: 0,
            name: "telemetry_conflict",
            owner_pack: "custom",
            lifecycle: TileLifecycle::Active,
            required_addon: None,
            record_size: 64,
            current_layout_version: 2,
            min_supported_layout_version: 2,
            flags: TileTypeFlags::NONE,
        };
        assert!(matches!(reg.register(conflict), Err(RegistryError::DuplicateTileType { .. })));
    }

    #[test]
    fn test_lifecycle_rules() {
        let mut reg = TileTypeRegistry::new();
        // Register active tile
        reg.register(TileTypeDescriptor {
            tile_type: 100,
            name: "active_tile",
            owner_pack: "test",
            lifecycle: TileLifecycle::Active,
            required_addon: None,
            record_size: 16,
            current_layout_version: 1,
            min_supported_layout_version: 1,
            flags: TileTypeFlags::NONE,
        }).unwrap();

        // Register deprecated (read-only) tile
        reg.register(TileTypeDescriptor {
            tile_type: 101,
            name: "deprecated_tile",
            owner_pack: "test",
            lifecycle: TileLifecycle::Deprecated,
            required_addon: None,
            record_size: 16,
            current_layout_version: 1,
            min_supported_layout_version: 1,
            flags: TileTypeFlags::NONE,
        }).unwrap();

        // Register retired tile
        reg.register(TileTypeDescriptor {
            tile_type: 102,
            name: "retired_tile",
            owner_pack: "test",
            lifecycle: TileLifecycle::Retired,
            required_addon: None,
            record_size: 16,
            current_layout_version: 1,
            min_supported_layout_version: 1,
            flags: TileTypeFlags::NONE,
        }).unwrap();

        // Active: write and read ok
        assert!(reg.validate_write(100, 1, 0).is_ok());
        assert!(reg.validate_read(100, 1).is_ok());

        // Deprecated: write denied, read ok
        assert!(matches!(
            reg.validate_write(101, 1, 0),
            Err(RegistryError::DeprecatedTile(101))
        ));
        assert!(reg.validate_read(101, 1).is_ok());

        // Retired: both write and read denied
        assert!(matches!(
            reg.validate_write(102, 1, 0),
            Err(RegistryError::RetiredTile(102))
        ));
        assert!(matches!(
            reg.validate_read(102, 1),
            Err(RegistryError::RetiredTile(102))
        ));
    }

    #[test]
    fn test_validate_write_rules() {
        let reg = TileTypeRegistry::with_standard_types();

        // Core tile 0 requires no addon
        assert!(reg.validate_write(0, 1, 0).is_ok());

        // Wearable tile 3 fails without ADDON_WEARABLE
        assert!(matches!(
            reg.validate_write(3, 1, 0),
            Err(RegistryError::MissingAddon { required_mask, .. }) if required_mask == ADDON_WEARABLE
        ));
        assert!(reg.validate_write(3, 1, ADDON_WEARABLE).is_ok());

        // Reserved tile 9 fails closed even with mask
        assert!(matches!(
            reg.validate_write(9, 1, 0xFFFFFFFF),
            Err(RegistryError::ReservedTile(9))
        ));

        // Layout version too old / too new
        assert!(matches!(
            reg.validate_write(0, 0, 0),
            Err(RegistryError::UnsupportedLayoutVersionTooOld { .. })
        ));
        assert!(matches!(
            reg.validate_write(0, 999, 0),
            Err(RegistryError::UnsupportedLayoutVersionTooNew { .. })
        ));
    }

    #[test]
    fn test_validate_read_and_legacy_v0_alias() {
        let reg = TileTypeRegistry::with_standard_types();

        // Legacy layout 0 maps to layout 1
        assert!(reg.validate_read(0, 0).is_ok());
        assert!(reg.validate_read(10, 0).is_ok());

        // Layout 1 succeeds
        assert!(reg.validate_read(0, 1).is_ok());

        // Layout 999 fails closed
        assert!(matches!(
            reg.validate_read(0, 999),
            Err(RegistryError::UnsupportedLayoutVersionTooNew { .. })
        ));
    }

    #[test]
    fn test_demo_payload_migration() {
        let original_data = vec![1u8; 32]; // two 16B records
        let migrated = migrate_tile_payload(99, 1, 2, &original_data).unwrap();
        assert_eq!(migrated.len(), 40); // two 20B records
        assert_eq!(&migrated[0..16], &original_data[0..16]);
        assert_eq!(&migrated[16..20], &[0, 0, 0, 0]);
    }
}
