use core::ops::RangeInclusive;
use s3a_core::{
    registry::{RegistryError, TileLifecycle, TileTypeDescriptor, TileTypeFlags, TileTypeRegistry},
    S3AAddon, ADDON_EDU,
};

/// Tile type IDs allocated exclusively to the Scholar Explorer domain pack.
pub struct ScholarTileType;

impl ScholarTileType {
    /// Hyper-Tile 10: Human Researcher LRS Ledger (HumanLrsRecord, 64 bytes)
    pub const HUMAN_LRS: u32 = 10;
    /// Hyper-Tile 11: AI Agent Execution Trace Log (AiTraceRecord, 64 bytes)
    pub const AI_TRACE_LOG: u32 = 11;
    /// Hyper-Tile 12: Academic Paper Literature Corpus (AcademicPaperRecord, 64 bytes)
    pub const ACADEMIC_PAPERS: u32 = 12;
    /// Hyper-Tile 13: Research Knowledge Graph Edges (ResearchGraphEdgeRecord, 64 bytes)
    pub const RESEARCH_GRAPH: u32 = 13;
    /// Hyper-Tile 14: Scholar Research Workspace Context (SrsWorkspaceRecord, 64 bytes)
    pub const SRS_WORKSPACE: u32 = 14;
    /// Hyper-Tile 15: Scholar Research Snapshot Manifest Header (SrsManifestHeader, 512 bytes)
    pub const SRS_MANIFEST: u32 = 15;
    /// Hyper-Tile 18: Task Agent Registry (AgentRegistryRecord, 64 bytes)
    pub const AGENT_REGISTRY: u32 = 18;
    /// Hyper-Tile 20: Research Notes & Verified Claims (ScholarNoteClaimRecord, 160 bytes)
    pub const NOTE_CLAIMS: u32 = 20;
    /// Hyper-Tile 32: Cryptographic Citation Fingerprints (CitationFingerprintRecord, 96 bytes)
    pub const CITATION_FINGERPRINT: u32 = 32;
    /// Hyper-Tile 96: Signed Committee Decisions (CommitteeDecisionRecord, 160 bytes).
    /// Human judgments on submitted work; NOT a trace tile and NOT consent-gated.
    /// Allocated in the pack band (>= 64) because 10..=32 is full.
    pub const COMMITTEE_DECISION: u32 = 96;
}

pub static SCHOLAR_DESCRIPTORS: &[TileTypeDescriptor] = &[
    TileTypeDescriptor {
        tile_type: ScholarTileType::HUMAN_LRS,
        name: "human_lrs",
        owner_pack: "Scholar Pack",
        lifecycle: TileLifecycle::Active,
        required_addon: Some(ADDON_EDU),
        record_size: 64,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::from_bits_truncate(
            TileTypeFlags::FIXED_POD.bits() | TileTypeFlags::COPY_THROUGH_COMPACT.bits(),
        ),
    },
    TileTypeDescriptor {
        tile_type: ScholarTileType::AI_TRACE_LOG,
        name: "ai_trace_log",
        owner_pack: "Scholar Pack",
        lifecycle: TileLifecycle::Active,
        required_addon: Some(ADDON_EDU),
        record_size: 64,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::from_bits_truncate(
            TileTypeFlags::FIXED_POD.bits() | TileTypeFlags::COPY_THROUGH_COMPACT.bits(),
        ),
    },
    TileTypeDescriptor {
        tile_type: ScholarTileType::ACADEMIC_PAPERS,
        name: "academic_papers",
        owner_pack: "Scholar Pack",
        lifecycle: TileLifecycle::Active,
        required_addon: Some(ADDON_EDU),
        record_size: 64,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::from_bits_truncate(
            TileTypeFlags::FIXED_POD.bits() | TileTypeFlags::COPY_THROUGH_COMPACT.bits(),
        ),
    },
    TileTypeDescriptor {
        tile_type: ScholarTileType::RESEARCH_GRAPH,
        name: "research_graph",
        owner_pack: "Scholar Pack",
        lifecycle: TileLifecycle::Active,
        required_addon: Some(ADDON_EDU),
        record_size: 64,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::from_bits_truncate(
            TileTypeFlags::FIXED_POD.bits() | TileTypeFlags::COPY_THROUGH_COMPACT.bits(),
        ),
    },
    TileTypeDescriptor {
        tile_type: ScholarTileType::SRS_WORKSPACE,
        name: "srs_workspace",
        owner_pack: "Scholar Pack",
        lifecycle: TileLifecycle::Active,
        required_addon: Some(ADDON_EDU),
        record_size: 64,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::from_bits_truncate(
            TileTypeFlags::FIXED_POD.bits() | TileTypeFlags::COPY_THROUGH_COMPACT.bits(),
        ),
    },
    TileTypeDescriptor {
        tile_type: ScholarTileType::SRS_MANIFEST,
        name: "srs_manifest",
        owner_pack: "Scholar Pack",
        lifecycle: TileLifecycle::Active,
        required_addon: Some(ADDON_EDU),
        record_size: 64,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::from_bits_truncate(
            TileTypeFlags::FIXED_POD.bits() | TileTypeFlags::COPY_THROUGH_COMPACT.bits(),
        ),
    },
    TileTypeDescriptor {
        tile_type: ScholarTileType::AGENT_REGISTRY,
        name: "agent_registry",
        owner_pack: "Scholar Pack",
        lifecycle: TileLifecycle::Active,
        required_addon: Some(ADDON_EDU),
        record_size: 64,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::from_bits_truncate(
            TileTypeFlags::FIXED_POD.bits() | TileTypeFlags::COPY_THROUGH_COMPACT.bits(),
        ),
    },
    TileTypeDescriptor {
        tile_type: ScholarTileType::NOTE_CLAIMS,
        name: "note_claims",
        owner_pack: "Scholar Pack",
        lifecycle: TileLifecycle::Active,
        required_addon: Some(ADDON_EDU),
        record_size: 64,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::from_bits_truncate(
            TileTypeFlags::FIXED_POD.bits() | TileTypeFlags::COPY_THROUGH_COMPACT.bits(),
        ),
    },
    TileTypeDescriptor {
        tile_type: ScholarTileType::CITATION_FINGERPRINT,
        name: "citation_fingerprint",
        owner_pack: "Scholar Pack",
        lifecycle: TileLifecycle::Active,
        required_addon: Some(ADDON_EDU),
        record_size: 96,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::from_bits_truncate(
            TileTypeFlags::FIXED_POD.bits() | TileTypeFlags::COPY_THROUGH_COMPACT.bits(),
        ),
    },
    TileTypeDescriptor {
        tile_type: ScholarTileType::COMMITTEE_DECISION,
        name: "committee_decision",
        owner_pack: "Scholar Pack",
        lifecycle: TileLifecycle::Active,
        required_addon: Some(ADDON_EDU),
        record_size: 160,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::from_bits_truncate(
            TileTypeFlags::FIXED_POD.bits() | TileTypeFlags::COPY_THROUGH_COMPACT.bits(),
        ),
    },
];

/// Registers tile types for the Scholar Explorer domain pack into a registry.
pub fn register_pack_types(reg: &mut TileTypeRegistry) -> Result<(), RegistryError> {
    for desc in SCHOLAR_DESCRIPTORS {
        reg.register(desc.clone())?;
    }
    Ok(())
}

/// Domain pack implementation for Scholar Explorer (`s3a-pack-scholar`).
pub struct ScholarPack;

impl S3AAddon for ScholarPack {
    fn addon_id(&self) -> u32 {
        ADDON_EDU
    }

    fn name(&self) -> &'static str {
        "s3a-pack-scholar"
    }

    fn version(&self) -> &'static str {
        "0.1.0"
    }

    fn tile_range(&self) -> RangeInclusive<u32> {
        10..=32
    }

    fn descriptors(&self) -> &'static [TileTypeDescriptor] {
        SCHOLAR_DESCRIPTORS
    }
}
