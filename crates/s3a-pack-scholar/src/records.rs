use bytemuck::{Pod, Zeroable};
use sha2::Digest;
use s3a_core::{S3ACoordinate, FLAG_ACTIVE, FLAG_TOMBSTONE};

/// HUMAN ACTIVITY SQL LRS RECORD (64 bytes, 8-byte aligned).
/// Captures human researcher reviews, approvals, rework decisions, and xAPI activity statements.
/// Contains a shared 128-bit `session_uuid` and direct O(1) coordinate pointer to the corresponding AI trace.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Default)]
pub struct HumanLrsRecord {
    pub session_uuid: [u64; 2],        // 128-bit shared session/registration UUID (offset 0..16)
    pub timestamp_sec: u64,            // Recorded timestamp (offset 16..24)
    pub actor_hash: u64,               // Hash of researcher / human actor (offset 24..32)
    pub verb_id: u32,                  // xAPI verb ID (e.g. accepted, reworked, decided) (offset 32..36)
    pub decision_score: f32,           // Feedback / rework score (offset 36..40)
    pub ai_trace_coord: S3ACoordinate, // Direct O(1) pointer to AI agent trace step (offset 40..56)
    pub object_hash: u32,              // Hash of target artifact / paper / claim (offset 56..60)
    pub flags: u32,                    // FLAG_ACTIVE, FLAG_TOMBSTONE, etc. (offset 60..64)
}

impl HumanLrsRecord {
    pub fn new(
        session_uuid: [u64; 2],
        timestamp_sec: u64,
        actor_hash: u64,
        verb_id: u32,
        decision_score: f32,
        ai_trace_coord: S3ACoordinate,
        object_hash: u32,
    ) -> Self {
        Self {
            session_uuid,
            timestamp_sec,
            actor_hash,
            verb_id,
            decision_score,
            ai_trace_coord,
            object_hash,
            flags: FLAG_ACTIVE as u32,
        }
    }

    pub fn is_tombstone(&self) -> bool {
        (self.flags & (FLAG_TOMBSTONE as u32)) != 0
    }

    pub fn mark_tombstone(&mut self) {
        self.flags |= FLAG_TOMBSTONE as u32;
    }
}

/// Step categories for AiTraceRecord.step_type
pub const STEP_REASONING: u32 = 1;
pub const STEP_TOOL_CALL: u32 = 2;
pub const STEP_CLAIM: u32 = 3;
pub const STEP_NOTE: u32 = 4;
pub const STEP_HALLUCINATION: u32 = 5;

/// Claim categories for ScholarNoteClaimRecord.kind
pub const CLAIM_KIND_EMPIRICAL: u16 = 1;
pub const CLAIM_KIND_THEORETICAL: u16 = 2;
pub const CLAIM_KIND_NOTE: u16 = 3;
pub const CLAIM_KIND_HYPOTHESIS: u16 = 4;

/// Computes a 32-bit token Bloom filter from text for sub-microsecond keyword rejection.
pub fn compute_keyword_bloom(text: &str) -> u32 {
    let mut bloom = 0u32;
    for word in text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()) {
        let mut lower = [0u8; 64];
        let bytes = word.as_bytes();
        let len = bytes.len().min(64);
        for i in 0..len {
            lower[i] = bytes[i].to_ascii_lowercase();
        }
        let h1 = (crc32c::crc32c(&lower[..len]) % 32) as u8;
        let h2 = ((crc32c::crc32c(&lower[..len]).rotate_left(5)) % 32) as u8;
        bloom |= (1 << h1) | (1 << h2);
    }
    bloom
}

/// Tests whether all query keywords match the record's 32-bit token Bloom filter.
pub fn matches_keyword_bloom(record_bloom: u32, query: &str) -> bool {
    let query_bloom = compute_keyword_bloom(query);
    (record_bloom & query_bloom) == query_bloom
}

/// SCHOLAR NOTE & CLAIM RECORD (64 bytes, 8-byte aligned, Pod/Zeroable).
/// Represents a structured claim, hypothesis, finding, or research note with
/// an integrated 32-bit token Bloom filter for instantaneous rejection,
/// text content hash, and optional embedding tile address pointer.
///
/// Phase C Extension: Links to CitationFingerprint, passage quote SHA-256 and span,
/// producer (human or AI agent id), and verification statement UUID (160 bytes total).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Default)]
pub struct ScholarNoteClaimRecord {
    pub claim_id: u64,                 // 64-bit ID or hash of claim (offset 0..8)
    pub timestamp_sec: u64,            // Assertion timestamp in epoch seconds (offset 8..16)
    pub session_uuid: [u64; 2],        // 128-bit session (offset 16..32)
    pub author_hash: u64,              // Author agent or researcher hash (offset 32..40)
    pub confidence: f32,               // Confidence / validation score (0.0..1.0) (offset 40..44)
    pub kind: u16,                     // CLAIM_KIND_EMPIRICAL, NOTE, HYPOTHESIS, etc. (offset 44..46)
    pub flags: u16,                    // FLAG_ACTIVE, etc. (offset 46..48)
    pub keyword_bloom: u32,            // 32-bit token bloom filter for rapid text rejection (offset 48..52)
    pub text_hash: u32,                // CRC32C hash of the full text content (offset 52..56)
    pub embedding_tile_id: u32,        // Tile ID of embedding if stored in archive (offset 56..60)
    pub embedding_offset: u32,         // Record offset in embedding tile (offset 60..64)
    pub citation_fingerprint_id: u64,  // 64-bit ID of CitationFingerprintRecord / paper_id (offset 64..72)
    pub passage_page: u32,             // Page number in source document (offset 72..76)
    pub passage_char_offset: u32,      // Character start offset within page (offset 76..80)
    pub passage_char_length: u32,      // Character length of passage (offset 80..84)
    pub _reserved_passage: u32,        // Alignment padding (offset 84..88)
    pub quote_sha256: [u8; 32],        // SHA-256 hash of the exact quoted passage text (offset 88..120)
    pub producer_id: u64,              // Human researcher or AI agent ID (offset 120..128)
    pub verification_statement_uuid: [u64; 2], // 128-bit statement UUID (verified-claim / flagged-hallucination) (offset 128..144)
    pub _reserved: [u64; 2],           // Reserved zero padding to 160 bytes (offset 144..160)
}

impl ScholarNoteClaimRecord {
    pub fn new(
        claim_id: u64,
        timestamp_sec: u64,
        session_uuid: [u64; 2],
        author_hash: u64,
        confidence: f32,
        kind: u16,
        text: &str,
    ) -> Self {
        let keyword_bloom = compute_keyword_bloom(text);
        let text_hash = crc32c::crc32c(text.as_bytes());
        let mut quote_sha256 = [0u8; 32];
        if !text.is_empty() {
            let mut hasher = sha2::Sha256::new();
            hasher.update(text.as_bytes());
            quote_sha256 = hasher.finalize().into();
        }
        Self {
            claim_id,
            timestamp_sec,
            session_uuid,
            author_hash,
            confidence,
            kind,
            flags: FLAG_ACTIVE as u16,
            keyword_bloom,
            text_hash,
            embedding_tile_id: 0,
            embedding_offset: 0,
            citation_fingerprint_id: 0,
            passage_page: 0,
            passage_char_offset: 0,
            passage_char_length: 0,
            _reserved_passage: 0,
            quote_sha256,
            producer_id: author_hash,
            verification_statement_uuid: [0u64; 2],
            _reserved: [0u64; 2],
        }
    }

    pub fn with_evidence(
        mut self,
        citation_fingerprint_id: u64,
        passage_page: u32,
        passage_char_offset: u32,
        passage_char_length: u32,
        quoted_text: &str,
        producer_id: u64,
        verification_statement_uuid: [u64; 2],
    ) -> Self {
        self.citation_fingerprint_id = citation_fingerprint_id;
        self.passage_page = passage_page;
        self.passage_char_offset = passage_char_offset;
        self.passage_char_length = passage_char_length;
        if !quoted_text.is_empty() {
            let mut hasher = sha2::Sha256::new();
            hasher.update(quoted_text.as_bytes());
            self.quote_sha256 = hasher.finalize().into();
        }
        self.producer_id = producer_id;
        self.verification_statement_uuid = verification_statement_uuid;
        self
    }

    pub fn matches_query(&self, query: &str) -> bool {
        matches_keyword_bloom(self.keyword_bloom, query)
    }

    pub fn is_tombstone(&self) -> bool {
        (self.flags & (FLAG_TOMBSTONE as u16)) != 0
    }

    pub fn mark_tombstone(&mut self) {
        self.flags |= FLAG_TOMBSTONE as u16;
    }

    pub fn quote_sha256_hex(&self) -> String {
        hex::encode(self.quote_sha256)
    }

    pub fn verification_statement_uuid_string(&self) -> String {
        if self.verification_statement_uuid == [0u64; 2] {
            return String::new();
        }
        let b0 = self.verification_statement_uuid[0].to_be_bytes();
        let b1 = self.verification_statement_uuid[1].to_be_bytes();
        let mut bytes = [0u8; 16];
        bytes[0..8].copy_from_slice(&b0);
        bytes[8..16].copy_from_slice(&b1);
        format!(
            "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            bytes[0], bytes[1], bytes[2], bytes[3],
            bytes[4], bytes[5],
            bytes[6], bytes[7],
            bytes[8], bytes[9],
            bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
        )
    }

    pub fn kind_str(&self) -> &'static str {
        match self.kind {
            CLAIM_KIND_EMPIRICAL => "EMPIRICAL",
            CLAIM_KIND_THEORETICAL => "THEORETICAL",
            CLAIM_KIND_NOTE => "NOTE",
            CLAIM_KIND_HYPOTHESIS => "HYPOTHESIS",
            _ => "UNKNOWN",
        }
    }
}

/// AI AGENT TRACEABLE LOG METADATA RECORD (64 bytes, 8-byte aligned).
/// Captures agent reasoning steps, tool calls, hallucination events, and claim verifications.
/// Matches the human SQL LRS via shared 128-bit `session_uuid` and direct O(1) coordinate pointer.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Default)]
pub struct AiTraceRecord {
    pub session_uuid: [u64; 2],        // 128-bit shared session/registration UUID (offset 0..16)
    pub timestamp_sec: u64,            // Execution timestamp (offset 16..24)
    pub agent_id: u64,                 // AI Agent hash (offset 24..32)
    pub step_type: u32,                // Step category (reasoning, tool_call, claim, hallucination) (offset 32..36)
    pub confidence: f32,               // Model confidence / reward score (offset 36..40)
    pub human_coord: S3ACoordinate,    // Direct O(1) pointer to corresponding human decision (offset 40..56)
    pub hilbert_index: u32,            // 3D spatial cluster coordinate (offset 56..60)
    pub flags: u32,                    // Active, tombstone, warning flags (offset 60..64)
}

impl AiTraceRecord {
    pub fn new(
        session_uuid: [u64; 2],
        timestamp_sec: u64,
        agent_id: u64,
        step_type: u32,
        confidence: f32,
        human_coord: S3ACoordinate,
        hilbert_index: u32,
    ) -> Self {
        Self {
            session_uuid,
            timestamp_sec,
            agent_id,
            step_type,
            confidence,
            human_coord,
            hilbert_index,
            flags: FLAG_ACTIVE as u32,
        }
    }

    pub fn is_tombstone(&self) -> bool {
        (self.flags & (FLAG_TOMBSTONE as u32)) != 0
    }

    pub fn mark_tombstone(&mut self) {
        self.flags |= FLAG_TOMBSTONE as u32;
    }
}

/// ACADEMIC PAPER & RESEARCH METADATA RECORD (64 bytes, 8-byte aligned).
/// Captures literature metadata with 3D Skilling Hilbert curve spatial coordinates.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Default)]
pub struct AcademicPaperRecord {
    pub paper_id: u64,                 // 64-bit hash of DOI / URI (offset 0..8)
    pub timestamp_sec: u64,            // Publication timestamp (offset 8..16)
    pub hilbert_index: u64,            // 3D Skilling Hilbert curve index (offset 16..24)
    pub topic_x: f32,                  // 3D coordinate X (topic/cluster) (offset 24..28)
    pub topic_y: f32,                  // 3D coordinate Y (methodology/field) (offset 28..32)
    pub topic_z: f32,                  // 3D coordinate Z (year/recency) (offset 32..36)
    pub citation_count: u32,           // Citation count (offset 36..40)
    pub year: u16,                     // Publication year (offset 40..42)
    pub venue_id: u16,                 // Venue / service ID (offset 42..44)
    pub open_access_flag: u16,         // 1 = OA PDF, 0 = Paywalled (offset 44..46)
    pub warning_count: u16,            // Provenance warning count (offset 46..48)
    pub doi_prefix_hash: u64,          // Hash of publisher DOI prefix (offset 48..56)
    pub flags: u64,                    // Active, tombstone, reviewed (offset 56..64)
}

impl AcademicPaperRecord {
    pub fn new(
        paper_id: u64,
        timestamp_sec: u64,
        hilbert_index: u64,
        topic_x: f32,
        topic_y: f32,
        topic_z: f32,
        citation_count: u32,
        year: u16,
        venue_id: u16,
        open_access_flag: u16,
        warning_count: u16,
        doi_prefix_hash: u64,
    ) -> Self {
        Self {
            paper_id,
            timestamp_sec,
            hilbert_index,
            topic_x,
            topic_y,
            topic_z,
            citation_count,
            year,
            venue_id,
            open_access_flag,
            warning_count,
            doi_prefix_hash,
            flags: FLAG_ACTIVE,
        }
    }

    pub fn is_tombstone(&self) -> bool {
        (self.flags & FLAG_TOMBSTONE) != 0
    }

    pub fn mark_tombstone(&mut self) {
        self.flags |= FLAG_TOMBSTONE;
    }

    // Provenance Source & Validation Flags (Offset 56..64)
    pub const SOURCE_OPENALEX: u64 = 1 << 3;
    pub const SOURCE_CROSSREF: u64 = 1 << 4;
    pub const RESEARCHER_APPROVED: u64 = 1 << 5;
    pub const HAS_OA_URL_REF: u64 = 1 << 6;

    pub fn with_openalex_source(mut self) -> Self {
        self.flags |= Self::SOURCE_OPENALEX;
        self
    }

    pub fn is_openalex_source(&self) -> bool {
        (self.flags & Self::SOURCE_OPENALEX) != 0
    }

    /// When SOURCE_OPENALEX is set, paper_id represents the 64-bit hash of the OpenAlex Work ID.
    pub fn openalex_work_id_hash(&self) -> u64 {
        self.paper_id
    }
}

/// RESEARCH KNOWLEDGE GRAPH EDGE RECORD (64 bytes, 8-byte aligned).
/// Captures subject-predicate-object knowledge graph relations.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Default)]
pub struct ResearchGraphEdgeRecord {
    pub subject_hash: u64,             // Node A identifier hash (offset 0..8)
    pub object_hash: u64,              // Node B identifier hash (offset 8..16)
    pub timestamp_sec: u64,            // Extraction timestamp (offset 16..24)
    pub hilbert_coord: u64,            // Spatial cluster coordinate (offset 24..32)
    pub predicate_id: u32,             // Predicate type (contains, authored_by, cites) (offset 32..36)
    pub weight: f32,                   // Edge weight / confidence (offset 36..40)
    pub flags: u64,                    // Active, tombstone (offset 40..48)
    pub _reserved: [u64; 2],           // Padding to 64 bytes (offset 48..64)
}

impl ResearchGraphEdgeRecord {
    pub fn new(
        subject_hash: u64,
        object_hash: u64,
        timestamp_sec: u64,
        hilbert_coord: u64,
        predicate_id: u32,
        weight: f32,
    ) -> Self {
        Self {
            subject_hash,
            object_hash,
            timestamp_sec,
            hilbert_coord,
            predicate_id,
            weight,
            flags: FLAG_ACTIVE,
            _reserved: [0u64; 2],
        }
    }

    pub fn is_tombstone(&self) -> bool {
        (self.flags & FLAG_TOMBSTONE) != 0
    }

    pub fn mark_tombstone(&mut self) {
        self.flags |= FLAG_TOMBSTONE;
    }
}

/// SCHOLAR RESEARCH SNAPSHOT (SRS) MANIFEST HEADER (512 bytes, exact sector-aligned, Pod/Zeroable).
/// Encapsulates portable research object identity, Project UUID, Snapshot UUID, parent lineage,
/// and hardware CRC32C checksums as defined in SECS-SRS-1.0.0.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct SrsManifestHeader {
    pub magic: [u8; 8],                // b"S3ASTOR1" (offset 0..8)
    pub schema_version: u32,           // SRS specification version (e.g. 1) (offset 8..12)
    pub archive_flags: u32,            // Bitmask: IS_IMMUTABLE, IS_SNAPSHOT, etc. (offset 12..16)
    pub project_uuid: [u64; 2],        // 128-bit persistent project identifier (offset 16..32)
    pub snapshot_uuid: [u64; 2],       // 128-bit unique checkpoint identifier (offset 32..48)
    pub parent_snapshot_uuid: [u64; 2],// 128-bit parent checkpoint (0 if initial root) (offset 48..64)
    pub origin_session_uuid: [u64; 2], // 128-bit active session identifier (offset 64..80)
    pub created_at_sec: u64,           // Unix epoch seconds (offset 80..88)
    pub tile_count: u32,               // Number of Hyper-Tiles in container (offset 88..92)
    pub crc32c_checksum: u32,          // Hardware CRC32C of entire payload (offset 92..96)
    pub total_records: u64,            // Total record count across all tiles (offset 96..104)
    pub _reserved1: [u64; 32],         // Zero-padded: 256 bytes (offset 104..360)
    pub _reserved2: [u64; 19],         // Zero-padded: 152 bytes (offset 360..512)
}

pub const SCHOLAR_LAYOUT_VERSION_V1: u32 = 1;

impl SrsManifestHeader {
    pub const MAGIC: [u8; 8] = *b"S3ASTOR1";
    pub const FLAG_IMMUTABLE: u32 = 1 << 0;
    pub const FLAG_SNAPSHOT: u32 = 1 << 1;
    pub const FLAG_WASM_EXPORTED: u32 = 1 << 2;

    pub fn new(
        project_uuid: [u64; 2],
        snapshot_uuid: [u64; 2],
        parent_snapshot_uuid: [u64; 2],
        origin_session_uuid: [u64; 2],
        created_at_sec: u64,
        tile_count: u32,
        total_records: u64,
    ) -> Self {
        let mut header = Self {
            magic: Self::MAGIC,
            schema_version: SCHOLAR_LAYOUT_VERSION_V1,
            archive_flags: Self::FLAG_IMMUTABLE | Self::FLAG_SNAPSHOT,
            project_uuid,
            snapshot_uuid,
            parent_snapshot_uuid,
            origin_session_uuid,
            created_at_sec,
            tile_count,
            crc32c_checksum: 0,
            total_records,
            _reserved1: [0u64; 32],
            _reserved2: [0u64; 19],
        };
        header.crc32c_checksum = header.compute_checksum();
        header
    }

    /// Computes hardware CRC32C over the 512-byte header with the crc32c_checksum field zeroed.
    pub fn compute_checksum(&self) -> u32 {
        let mut copy = *self;
        copy.crc32c_checksum = 0;
        crc32c::crc32c(bytemuck::bytes_of(&copy))
    }

    /// Verifies that the internal crc32c_checksum matches the computed hardware CRC32C.
    pub fn verify_checksum(&self) -> bool {
        self.crc32c_checksum == self.compute_checksum()
    }
}

/// SCHOLAR RESEARCH WORKSPACE RECORD (64 bytes, 8-byte aligned, Pod/Zeroable).
/// Encapsulates research question hash, collection masks, and topic centroid.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Default)]
pub struct SrsWorkspaceRecord {
    pub project_uuid: [u64; 2],        // 128-bit project UUID (offset 0..16)
    pub question_hash: u64,            // Hash of current research question (offset 16..24)
    pub collection_mask: u64,          // Active collection bitmask (offset 24..32)
    pub topic_centroid_xyz: [f32; 3],  // Topic cluster centroid coordinates (offset 32..44)
    pub status: u32,                   // Workspace status (open, investigating, synthesized) (offset 44..48)
    pub pinned_works_count: u32,       // Number of pinned works (offset 48..52)
    pub claims_count: u32,             // Number of extracted claims (offset 52..56)
    pub flags: u64,                    // Flags (offset 56..64)
}

impl SrsWorkspaceRecord {
    pub fn new(
        project_uuid: [u64; 2],
        question_hash: u64,
        collection_mask: u64,
        topic_centroid_xyz: [f32; 3],
        status: u32,
    ) -> Self {
        Self {
            project_uuid,
            question_hash,
            collection_mask,
            topic_centroid_xyz,
            status,
            pinned_works_count: 0,
            claims_count: 0,
            flags: FLAG_ACTIVE,
        }
    }
}

// Tool bitmask flags for AgentRegistryRecord.allowed_tool_mask and denied_tool_mask
pub const TOOL_READ_FILE: u32 = 1 << 0;
pub const TOOL_OPENALEX_QUERY: u32 = 1 << 1;
pub const TOOL_EXTRACT_CLAIMS: u32 = 1 << 2;
pub const TOOL_SYNTHESIZE_EVIDENCE: u32 = 1 << 3;
pub const TOOL_CROSSREF_QUERY: u32 = 1 << 4;
pub const TOOL_EXPORT_SNAPSHOT: u32 = 1 << 5;
pub const TOOL_TA_SUPERVISOR_CALL: u32 = 1 << 6;
pub const TOOL_MUTATE_GRAPH: u32 = 1 << 7;
pub const TOOL_PUBLISH: u32 = 1 << 16;
pub const TOOL_CREDENTIAL_EXPORT: u32 = 1 << 17;

// Agent kinds
pub const AGENT_KIND_ORCHESTRATOR: u16 = 1;
pub const AGENT_KIND_VERIFIER: u16 = 2;
pub const AGENT_KIND_CUSTODIAN: u16 = 3;
pub const AGENT_KIND_REVIEW_COORDINATOR: u16 = 4;
pub const AGENT_KIND_CITATION_GENERATOR: u16 = 5;

// Agent flags
pub const AGENT_FLAG_ACTIVE: u16 = 1 << 0;
pub const AGENT_FLAG_REQUIRES_HUMAN_GATE: u16 = 1 << 1;
pub const AGENT_FLAG_DISABLED: u16 = 1 << 2;

/// Hard denial error returned when an agent violates its allowlists or operational bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentDenied {
    AgentDisabled,
    ToolExplicitlyDenied { tool_bit: u32 },
    ToolNotAllowlisted { tool_bit: u32 },
    VerbNotAllowlisted { verb_bit: u32 },
    RequiresHumanGate,
    CorruptionDetected,
}

impl core::fmt::Display for AgentDenied {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            AgentDenied::AgentDisabled => write!(f, "Agent is disabled in registry"),
            AgentDenied::ToolExplicitlyDenied { tool_bit } => write!(f, "Tool bit 0x{:08X} is explicitly denied", tool_bit),
            AgentDenied::ToolNotAllowlisted { tool_bit } => write!(f, "Tool bit 0x{:08X} is not in allowed_tool_mask", tool_bit),
            AgentDenied::VerbNotAllowlisted { verb_bit } => write!(f, "Verb bit 0x{:08X} is not in allowed_verb_mask", verb_bit),
            AgentDenied::RequiresHumanGate => write!(f, "Agent action requires human/TA gate approval"),
            AgentDenied::CorruptionDetected => write!(f, "Corruption detected: redundant safety fields mismatch (possible bit flip)"),
        }
    }
}

/// AGENT REGISTRY RECORD (64 bytes, 8-byte aligned).
/// Wire-stable specification bounding task agents by xAPI profile IRI, OWL ontology IRI, and tool/verb allowlists.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq, Default)]
pub struct AgentRegistryRecord {
    pub agent_id: u64,              // 64-bit stable hash of agent slug (e.g. FNV-1a of "claim-checker") (offset 0..8)
    pub xapi_profile_iri_hash: u64, // 64-bit hash of bound xAPI Profile IRI (offset 8..16)
    pub owl_ontology_iri_hash: u64, // 64-bit hash of bound OWL Ontology IRI (offset 16..24)
    pub allowed_verb_mask: u32,     // Bitmask of permitted verbs (offset 24..28)
    pub allowed_tool_mask: u32,     // Bitmask of permitted tools (offset 28..32)
    pub denied_tool_mask: u32,      // Hard denials (e.g. publish, credential_export) (offset 32..36)
    pub agent_kind: u16,            // 1=orchestrator, 2=verifier, 3=custodian, 4=review_coord, 5=citation (offset 36..38)
    pub flags: u16,                 // Bit 0: ACTIVE, Bit 1: REQUIRES_HUMAN_GATE, Bit 2: DISABLED (offset 38..40)
    pub version: u64,               // Packed semver (major.minor.patch) or build id (offset 40..48)
    pub _reserved: [u8; 16],        // Must be zero (offset 48..64)
}

impl AgentRegistryRecord {
    pub fn new(
        agent_id: u64,
        xapi_profile_iri_hash: u64,
        owl_ontology_iri_hash: u64,
        allowed_verb_mask: u32,
        allowed_tool_mask: u32,
        denied_tool_mask: u32,
        agent_kind: u16,
        flags: u16,
        version: u64,
    ) -> Self {
        let mut rec = Self {
            agent_id,
            xapi_profile_iri_hash,
            owl_ontology_iri_hash,
            allowed_verb_mask,
            allowed_tool_mask,
            denied_tool_mask,
            agent_kind,
            flags,
            version,
            _reserved: [0u8; 16],
        };
        rec.sync_inverted_fields();
        rec
    }

    /// Creates an unstamped legacy v1 record with all-zero reserved bytes.
    pub fn new_legacy_unstamped(
        agent_id: u64,
        xapi_profile_iri_hash: u64,
        owl_ontology_iri_hash: u64,
        allowed_verb_mask: u32,
        allowed_tool_mask: u32,
        denied_tool_mask: u32,
        agent_kind: u16,
        flags: u16,
        version: u64,
    ) -> Self {
        Self {
            agent_id,
            xapi_profile_iri_hash,
            owl_ontology_iri_hash,
            allowed_verb_mask,
            allowed_tool_mask,
            denied_tool_mask,
            agent_kind,
            flags,
            version,
            _reserved: [0u8; 16],
        }
    }

    /// Detects if this record is an unstamped legacy v1 record (all 16 reserved bytes are zero).
    pub fn is_legacy_unstamped(&self) -> bool {
        self._reserved == [0u8; 16]
    }

    /// Stores redundant bitwise-inverted copies of safety-critical fields (denied_tool_mask and flags)
    /// in reserved bytes to guarantee single-bit flip detection.
    pub fn sync_inverted_fields(&mut self) {
        self._reserved[0..4].copy_from_slice(&(!self.denied_tool_mask).to_le_bytes());
        self._reserved[4..6].copy_from_slice(&(!self.flags).to_le_bytes());
    }

    pub fn denied_tool_mask_inv(&self) -> u32 {
        u32::from_le_bytes(self._reserved[0..4].try_into().unwrap())
    }

    pub fn flags_inv(&self) -> u16 {
        u16::from_le_bytes(self._reserved[4..6].try_into().unwrap())
    }

    /// Static capability verification bounding agent actions (tools and verbs).
    /// Never warns; returns Err(AgentDenied) on any violation.
    ///
    /// Note: To simultaneously enforce both static allowlists and dynamic human-in-the-loop
    /// supervisory gating for agents with AGENT_FLAG_REQUIRES_HUMAN_GATE, call `assert_allows_gated`.
    pub fn assert_allows(&self, verb_bit: Option<u32>, tool_bit: Option<u32>) -> Result<(), AgentDenied> {
        self.assert_allows_gated(verb_bit, tool_bit, true)
    }

    /// Complete bounding verification enforcing both static allowlists/denials
    /// AND mandatory human supervisory clearance (AGENT_FLAG_REQUIRES_HUMAN_GATE).
    ///
    /// If the agent has `AGENT_FLAG_REQUIRES_HUMAN_GATE` and `is_human_cleared` is false,
    /// returns `Err(AgentDenied::RequiresHumanGate)`.
    pub fn assert_allows_gated(
        &self,
        verb_bit: Option<u32>,
        tool_bit: Option<u32>,
        is_human_cleared: bool,
    ) -> Result<(), AgentDenied> {
        // Redundant bit-flip safety check on denied_tool_mask and flags:
        // If _reserved is all zeroes, treat as an unstamped legacy v1 record and skip inverted checks.
        // Otherwise, verify that inverted copies match; on mismatch return AgentDenied::CorruptionDetected.
        if !self.is_legacy_unstamped() {
            let inv_denied = self.denied_tool_mask_inv();
            if inv_denied != !self.denied_tool_mask {
                return Err(AgentDenied::CorruptionDetected);
            }

            let inv_flags = self.flags_inv();
            if inv_flags != !self.flags {
                return Err(AgentDenied::CorruptionDetected);
            }
        }

        if (self.flags & AGENT_FLAG_DISABLED) != 0 {
            return Err(AgentDenied::AgentDisabled);
        }

        if self.requires_human_gate() && !is_human_cleared {
            return Err(AgentDenied::RequiresHumanGate);
        }

        if let Some(t) = tool_bit {
            if (self.denied_tool_mask & t) != 0 {
                return Err(AgentDenied::ToolExplicitlyDenied { tool_bit: t });
            }
            if (self.allowed_tool_mask & t) != t {
                return Err(AgentDenied::ToolNotAllowlisted { tool_bit: t });
            }
        }

        if let Some(v) = verb_bit {
            if (self.allowed_verb_mask & v) != v {
                return Err(AgentDenied::VerbNotAllowlisted { verb_bit: v });
            }
        }

        Ok(())
    }

    pub fn is_active(&self) -> bool {
        if !self.is_legacy_unstamped() {
            let inv_flags = self.flags_inv();
            if inv_flags != !self.flags {
                return false;
            }
        }
        (self.flags & AGENT_FLAG_ACTIVE) != 0 && (self.flags & AGENT_FLAG_DISABLED) == 0
    }

    pub fn requires_human_gate(&self) -> bool {
        if !self.is_legacy_unstamped() {
            let inv_flags = self.flags_inv();
            if inv_flags != !self.flags {
                return true; // Fail closed if corrupted
            }
        }
        (self.flags & AGENT_FLAG_REQUIRES_HUMAN_GATE) != 0
    }
}

/// Generates the canonical seed task agents bounded by Scholar xAPI Profile & OWL ontologies.
pub fn seed_default_agents() -> Vec<AgentRegistryRecord> {
    let profile_hash = crate::xapi::hash_iri(crate::xapi::SCHOLAR_XAPI_PROFILE_IRI);
    let owl_hash = crate::xapi::hash_iri(crate::xapi::SCHOLAR_OWL_ONTOLOGY_IRI);
    let v1 = 0x0001_0000_0000_0000; // v1.0.0

    vec![
        // 1. lab-assistant: General literature explorer & question formulator
        AgentRegistryRecord::new(
            crate::xapi::hash_iri("lab-assistant"),
            profile_hash,
            owl_hash,
            (1 << 4) | (1 << 5) | (1 << 11), // experienced, reviewed, imported
            TOOL_READ_FILE | TOOL_OPENALEX_QUERY | TOOL_CROSSREF_QUERY | TOOL_TA_SUPERVISOR_CALL,
            TOOL_PUBLISH | TOOL_CREDENTIAL_EXPORT,
            AGENT_KIND_ORCHESTRATOR,
            AGENT_FLAG_ACTIVE,
            v1,
        ),
        // 2. academic-custodian: Workspace & snapshot maintainer
        AgentRegistryRecord::new(
            crate::xapi::hash_iri("academic-custodian"),
            profile_hash,
            owl_hash,
            (1 << 11) | (1 << 12), // imported, exported-snapshot
            TOOL_READ_FILE | TOOL_EXPORT_SNAPSHOT,
            TOOL_PUBLISH | TOOL_CREDENTIAL_EXPORT,
            AGENT_KIND_CUSTODIAN,
            AGENT_FLAG_ACTIVE,
            v1,
        ),
        // 3. claim-checker / hallucination-review: Empirical claim verifier
        AgentRegistryRecord::new(
            crate::xapi::hash_iri("claim-checker"),
            profile_hash,
            owl_hash,
            (1 << 5) | (1 << 8) | (1 << 9), // reviewed, requested-human-review, flagged-hallucination
            TOOL_READ_FILE | TOOL_EXTRACT_CLAIMS | TOOL_TA_SUPERVISOR_CALL,
            TOOL_PUBLISH | TOOL_CREDENTIAL_EXPORT,
            AGENT_KIND_VERIFIER,
            AGENT_FLAG_ACTIVE | AGENT_FLAG_REQUIRES_HUMAN_GATE,
            v1,
        ),
        // 4. source-verifier: OpenAlex & Crossref metadata auditor
        AgentRegistryRecord::new(
            crate::xapi::hash_iri("source-verifier"),
            profile_hash,
            owl_hash,
            (1 << 5) | (1 << 8), // reviewed, requested-human-review
            TOOL_OPENALEX_QUERY | TOOL_CROSSREF_QUERY,
            TOOL_PUBLISH | TOOL_CREDENTIAL_EXPORT,
            AGENT_KIND_VERIFIER,
            AGENT_FLAG_ACTIVE,
            v1,
        ),
        // 5. methodology-auditor: In-depth research design evaluator
        AgentRegistryRecord::new(
            crate::xapi::hash_iri("methodology-auditor"),
            profile_hash,
            owl_hash,
            (1 << 5) | (1 << 6) | (1 << 7) | (1 << 8), // reviewed, approved, rejected, requested-human-review
            TOOL_READ_FILE | TOOL_SYNTHESIZE_EVIDENCE | TOOL_TA_SUPERVISOR_CALL,
            TOOL_PUBLISH | TOOL_CREDENTIAL_EXPORT,
            AGENT_KIND_VERIFIER,
            AGENT_FLAG_ACTIVE | AGENT_FLAG_REQUIRES_HUMAN_GATE,
            v1,
        ),
        // 6. human-review-coordinator: Gating coordinator for TA / Researcher approvals
        AgentRegistryRecord::new(
            crate::xapi::hash_iri("human-review-coordinator"),
            profile_hash,
            owl_hash,
            0x000F | (1 << 8), // TA verbs 0..3 + requested-human-review
            TOOL_TA_SUPERVISOR_CALL,
            TOOL_PUBLISH | TOOL_CREDENTIAL_EXPORT,
            AGENT_KIND_REVIEW_COORDINATOR,
            AGENT_FLAG_ACTIVE,
            v1,
        ),
        // 7. citation-generator: Citations & evidence graph linking
        AgentRegistryRecord::new(
            crate::xapi::hash_iri("citation-generator"),
            profile_hash,
            owl_hash,
            1 << 10, // cited
            TOOL_READ_FILE | TOOL_MUTATE_GRAPH,
            TOOL_PUBLISH | TOOL_CREDENTIAL_EXPORT,
            AGENT_KIND_CITATION_GENERATOR,
            AGENT_FLAG_ACTIVE,
            v1,
        ),
    ]
}

/// Content format categories for CitationFingerprintRecord.content_kind
pub const CONTENT_KIND_PDF: u32 = 1;
pub const CONTENT_KIND_HTML: u32 = 2;
pub const CONTENT_KIND_ABSTRACT: u32 = 3;

/// CRYPTOGRAPHIC CITATION FINGERPRINT RECORD (96 bytes, 8-byte aligned, Pod/Zeroable).
/// Encapsulates verifiable provenance for academic literature without storing copyright payload content.
/// Tracks SHA-256 hash of fetched bytes, content format, fetch timestamp, source URL hash, and Crossref version.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq, Default)]
pub struct CitationFingerprintRecord {
    pub paper_id: u64,                 // 64-bit ID or hash of paper / DOI (offset 0..8)
    pub doi_hash: u64,                 // 64-bit hash of normalized DOI string (offset 8..16)
    pub content_sha256: [u8; 32],      // SHA-256 cryptographic digest of fetched content bytes (offset 16..48)
    pub content_kind: u32,             // CONTENT_KIND_PDF (1), HTML (2), ABSTRACT (3) (offset 48..52)
    pub crossref_version: u32,         // Crossref work update sequence / version number (offset 52..56)
    pub fetched_at: u64,               // Timestamp when content was fetched in epoch seconds (offset 56..64)
    pub source_url_hash: u64,          // 64-bit hash of source URL (offset 64..72)
    pub flags: u64,                    // FLAG_ACTIVE, FLAG_TOMBSTONE, etc. (offset 72..80)
    pub _reserved: [u64; 2],           // Reserved zero padding to 96 bytes (offset 80..96)
}

impl CitationFingerprintRecord {
    pub fn new(
        paper_id: u64,
        doi_hash: u64,
        content_sha256: [u8; 32],
        content_kind: u32,
        crossref_version: u32,
        fetched_at: u64,
        source_url_hash: u64,
    ) -> Self {
        Self {
            paper_id,
            doi_hash,
            content_sha256,
            content_kind,
            crossref_version,
            fetched_at,
            source_url_hash,
            flags: FLAG_ACTIVE,
            _reserved: [0u64; 2],
        }
    }

    pub fn from_content(
        paper_id: u64,
        doi: &str,
        content_bytes: &[u8],
        content_kind: u32,
        source_url: &str,
        crossref_version: u32,
        fetched_at: u64,
    ) -> Self {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(content_bytes);
        let digest: [u8; 32] = hasher.finalize().into();

        let doi_hash = crate::xapi::hash_iri(doi);
        let source_url_hash = crate::xapi::hash_iri(source_url);

        Self::new(
            paper_id,
            doi_hash,
            digest,
            content_kind,
            crossref_version,
            fetched_at,
            source_url_hash,
        )
    }

    pub fn is_active(&self) -> bool {
        (self.flags & FLAG_TOMBSTONE) == 0
    }

    pub fn is_tombstone(&self) -> bool {
        (self.flags & FLAG_TOMBSTONE) != 0
    }

    pub fn mark_tombstone(&mut self) {
        self.flags |= FLAG_TOMBSTONE;
    }

    pub fn content_kind_str(&self) -> &'static str {
        match self.content_kind {
            CONTENT_KIND_PDF => "pdf",
            CONTENT_KIND_HTML => "html",
            CONTENT_KIND_ABSTRACT => "abstract",
            _ => "unknown",
        }
    }

    pub fn parse_content_kind(s: &str) -> u32 {
        match s.to_ascii_lowercase().as_str() {
            "pdf" => CONTENT_KIND_PDF,
            "html" => CONTENT_KIND_HTML,
            "abstract" => CONTENT_KIND_ABSTRACT,
            _ => 0,
        }
    }

    pub fn content_sha256_hex(&self) -> String {
        hex::encode(self.content_sha256)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::size_of;

    #[test]
    fn test_record_sizes_and_alignments() {
        assert_eq!(size_of::<HumanLrsRecord>(), 64);
        assert_eq!(std::mem::align_of::<HumanLrsRecord>(), 8);

        assert_eq!(size_of::<AiTraceRecord>(), 64);
        assert_eq!(std::mem::align_of::<AiTraceRecord>(), 8);

        assert_eq!(size_of::<AcademicPaperRecord>(), 64);
        assert_eq!(std::mem::align_of::<AcademicPaperRecord>(), 8);

        assert_eq!(size_of::<ResearchGraphEdgeRecord>(), 64);
        assert_eq!(std::mem::align_of::<ResearchGraphEdgeRecord>(), 8);

        assert_eq!(size_of::<SrsWorkspaceRecord>(), 64);
        assert_eq!(std::mem::align_of::<SrsWorkspaceRecord>(), 8);

        assert_eq!(size_of::<ScholarNoteClaimRecord>(), 160);
        assert_eq!(std::mem::align_of::<ScholarNoteClaimRecord>(), 8);

        assert_eq!(size_of::<CitationFingerprintRecord>(), 96);
        assert_eq!(std::mem::align_of::<CitationFingerprintRecord>(), 8);

        assert_eq!(size_of::<AgentRegistryRecord>(), 64);
        assert_eq!(std::mem::align_of::<AgentRegistryRecord>(), 8);

        assert_eq!(size_of::<SrsManifestHeader>(), 512);
        assert_eq!(std::mem::align_of::<SrsManifestHeader>(), 8);
    }

    #[test]
    fn test_mismatched_denied_tool_mask_denies_everything() {
        let agent = AgentRegistryRecord::new(
            12345,
            0,
            0,
            0xFFFF,
            TOOL_READ_FILE | TOOL_OPENALEX_QUERY,
            TOOL_PUBLISH,
            AGENT_KIND_VERIFIER,
            AGENT_FLAG_ACTIVE,
            1,
        );

        // Normally, allowed tool succeeds
        assert!(agent.assert_allows(None, Some(TOOL_OPENALEX_QUERY)).is_ok());

        // Corrupt inverted copy in _reserved[0..4]
        let mut corrupted = agent;
        corrupted._reserved[0] ^= 0x01; // flip 1 bit in redundant copy

        // Denies allowed tool with CorruptionDetected
        assert_eq!(
            corrupted.assert_allows(None, Some(TOOL_OPENALEX_QUERY)),
            Err(AgentDenied::CorruptionDetected)
        );
        // Denies verb query with CorruptionDetected
        assert_eq!(
            corrupted.assert_allows(Some(1), None),
            Err(AgentDenied::CorruptionDetected)
        );

        // Corrupt primary denied_tool_mask
        let mut corrupted2 = agent;
        corrupted2.denied_tool_mask ^= 0x02;
        assert_eq!(
            corrupted2.assert_allows(None, Some(TOOL_OPENALEX_QUERY)),
            Err(AgentDenied::CorruptionDetected)
        );
    }

    #[test]
    fn test_legacy_unstamped_record_allows_permitted_actions() {
        let legacy = AgentRegistryRecord::new_legacy_unstamped(
            777,
            0x1111,
            0x2222,
            0xFFFF,
            TOOL_READ_FILE | TOOL_OPENALEX_QUERY,
            TOOL_PUBLISH,
            AGENT_KIND_VERIFIER,
            AGENT_FLAG_ACTIVE,
            1,
        );

        assert!(legacy.is_legacy_unstamped());
        assert!(legacy.is_active());
        // Permitted action is allowed without inverted-check failure
        assert!(legacy.assert_allows(None, Some(TOOL_OPENALEX_QUERY)).is_ok());
        // Explicitly denied action is still denied
        assert_eq!(
            legacy.assert_allows(None, Some(TOOL_PUBLISH)),
            Err(AgentDenied::ToolExplicitlyDenied { tool_bit: TOOL_PUBLISH })
        );
        // Unallowlisted tool is denied
        assert_eq!(
            legacy.assert_allows(None, Some(TOOL_MUTATE_GRAPH)),
            Err(AgentDenied::ToolNotAllowlisted { tool_bit: TOOL_MUTATE_GRAPH })
        );
    }

    #[test]
    fn test_bit_flip_safety_exhaustive() {
        let agent = AgentRegistryRecord::new(
            999,
            0xAAAA,
            0xBBBB,
            0xFFFFFFFF,
            TOOL_READ_FILE | TOOL_OPENALEX_QUERY | TOOL_EXTRACT_CLAIMS,
            TOOL_PUBLISH | TOOL_CREDENTIAL_EXPORT,
            AGENT_KIND_ORCHESTRATOR,
            AGENT_FLAG_ACTIVE,
            0x1000,
        );

        // Baseline: valid agent allows permitted tool
        assert!(agent.assert_allows(None, Some(TOOL_OPENALEX_QUERY)).is_ok());
        assert!(agent.is_active());

        // 1. Flip every bit of denied_tool_mask (bits 0..32)
        for bit in 0..32 {
            let mut flipped = agent;
            flipped.denied_tool_mask ^= 1 << bit;
            assert_eq!(
                flipped.assert_allows(None, Some(TOOL_OPENALEX_QUERY)),
                Err(AgentDenied::CorruptionDetected),
                "Flipping bit {} of denied_tool_mask must return CorruptionDetected", bit
            );
        }

        // 2. Flip every bit of the inverted copy in _reserved[0..4]
        for byte_idx in 0..4 {
            for bit in 0..8 {
                let mut flipped = agent;
                flipped._reserved[byte_idx] ^= 1 << bit;
                assert_eq!(
                    flipped.assert_allows(None, Some(TOOL_OPENALEX_QUERY)),
                    Err(AgentDenied::CorruptionDetected),
                    "Flipping byte {} bit {} of denied_tool_mask_inv must return CorruptionDetected", byte_idx, bit
                );
            }
        }

        // 3. Flip every bit of flags (agent DISABLED / ACTIVE)
        for bit in 0..16 {
            let mut flipped = agent;
            flipped.flags ^= 1 << bit;
            assert_eq!(
                flipped.assert_allows(None, Some(TOOL_OPENALEX_QUERY)),
                Err(AgentDenied::CorruptionDetected),
                "Flipping bit {} of flags must return CorruptionDetected", bit
            );
            assert!(
                !flipped.is_active(),
                "Flipping bit {} of flags must result in inactive agent", bit
            );
        }

        // 4. Flip every bit of flags_inv in _reserved[4..6]
        for byte_idx in 4..6 {
            for bit in 0..8 {
                let mut flipped = agent;
                flipped._reserved[byte_idx] ^= 1 << bit;
                assert_eq!(
                    flipped.assert_allows(None, Some(TOOL_OPENALEX_QUERY)),
                    Err(AgentDenied::CorruptionDetected),
                    "Flipping byte {} bit {} of flags_inv must return CorruptionDetected", byte_idx, bit
                );
                assert!(
                    !flipped.is_active(),
                    "Flipping byte {} bit {} of flags_inv must result in inactive agent", byte_idx, bit
                );
            }
        }

        // 5. Test ACTIVE vs TOMBSTONE single bit flips on records
        let mut active_paper = AcademicPaperRecord::new(
            1, 100, 0, 0.0, 0.0, 0.0, 10, 2024, 1, 1, 0, 0,
        );
        assert_eq!(active_paper.flags, FLAG_ACTIVE as u64);
        assert!(!active_paper.is_tombstone());

        // Flip bit 0: becomes tombstone (safely pruned/hidden)
        active_paper.flags ^= 1 << 0;
        assert!(active_paper.is_tombstone());

        // For all other bits (1..64), flipping the bit departs from FLAG_ACTIVE (0)
        for bit in 1..64 {
            let mut p = AcademicPaperRecord::new(
                1, 100, 0, 0.0, 0.0, 0.0, 10, 2024, 1, 1, 0, 0,
            );
            p.flags ^= 1 << bit;
            assert_ne!(p.flags, FLAG_ACTIVE as u64, "Bit {} flip cannot leave FLAG_ACTIVE unchanged", bit);
        }
    }

    #[test]
    fn test_cross_trace_human_and_ai_with_shared_uuid() {
        let session_uuid = [0x1234_5678_9ABC_DEF0, 0x0FED_CBA9_8765_4321];
        let ai_coord = S3ACoordinate::new(0, 1, 5);
        let human_coord = S3ACoordinate::new(0, 2, 8);

        let human = HumanLrsRecord::new(session_uuid, 1600000000, 42, 1, 1.0, ai_coord, 100);
        let ai = AiTraceRecord::new(session_uuid, 1600000005, 99, 2, 0.95, human_coord, 500);

        assert_eq!(human.session_uuid, ai.session_uuid);
        assert_eq!(human.ai_trace_coord, ai_coord);
        assert_eq!(ai.human_coord, human_coord);
    }

    #[test]
    fn test_srs_manifest_header_layout() {
        let project_uuid = [0x123456789abcdef0, 0x0fedcba987654321];
        let snapshot_uuid = [0x1111222233334444, 0x5555666677778888];
        let parent_uuid = [0, 0];
        let session_uuid = [0xaaaabbbbccccdddd, 0xeeeeffff00001111];

        let manifest = SrsManifestHeader::new(
            project_uuid,
            snapshot_uuid,
            parent_uuid,
            session_uuid,
            1600000000,
            5,
            1250,
        );

        assert_eq!(manifest.magic, SrsManifestHeader::MAGIC);
        assert_eq!(manifest.schema_version, SCHOLAR_LAYOUT_VERSION_V1);
        assert!(manifest.verify_checksum());
        assert_eq!(manifest.project_uuid, project_uuid);
        assert_eq!(manifest.snapshot_uuid, snapshot_uuid);
        assert_eq!(manifest.tile_count, 5);
        assert_eq!(manifest.total_records, 1250);
        assert_eq!(manifest.archive_flags & SrsManifestHeader::FLAG_SNAPSHOT, SrsManifestHeader::FLAG_SNAPSHOT);
    }

    #[test]
    fn test_srs_workspace_record_layout() {
        let project_uuid = [0x123456789abcdef0, 0x0fedcba987654321];
        let ws = SrsWorkspaceRecord::new(
            project_uuid,
            999888,
            0b1011,
            [0.25, 0.5, 0.75],
            1,
        );

        assert_eq!(ws.project_uuid, project_uuid);
        assert_eq!(ws.question_hash, 999888);
        assert_eq!(ws.collection_mask, 0b1011);
        assert_eq!(ws.topic_centroid_xyz, [0.25, 0.5, 0.75]);
        assert_eq!(ws.status, 1);
    }

    #[test]
    fn test_scholar_note_claim_record_and_bloom() {
        assert_eq!(std::mem::size_of::<ScholarNoteClaimRecord>(), 160);
        assert_eq!(std::mem::align_of::<ScholarNoteClaimRecord>(), 8);

        let session = [123, 456];
        let claim = ScholarNoteClaimRecord::new(
            1,
            1600000000,
            session,
            0xCAFE,
            0.92,
            CLAIM_KIND_EMPIRICAL,
            "Majorana zero modes observed in semiconductor nanowire junctions",
        ).with_evidence(
            101,
            12,
            340,
            88,
            "Majorana zero modes observed in semiconductor nanowire junctions",
            0xCAFE,
            [0x1111222233334444, 0x5555666677778888],
        );

        assert_eq!(claim.claim_id, 1);
        assert_eq!(claim.confidence, 0.92);
        assert_eq!(claim.kind, CLAIM_KIND_EMPIRICAL);
        assert_eq!(claim.citation_fingerprint_id, 101);
        assert_eq!(claim.passage_page, 12);
        assert_eq!(claim.passage_char_offset, 340);
        assert_eq!(claim.passage_char_length, 88);
        assert_eq!(claim.producer_id, 0xCAFE);
        assert_eq!(claim.quote_sha256_hex().len(), 64);
        assert_eq!(claim.verification_statement_uuid_string(), "11112222-3333-4444-5555-666677778888");
        assert!(claim.matches_query("Majorana"));
        assert!(claim.matches_query("nanowire"));
        assert!(claim.matches_query("semiconductor junctions"));
        assert!(!claim.matches_query("black hole hawking radiation"));
    }

    #[test]
    fn test_citation_fingerprint_record() {
        let content = b"%PDF-1.4 Mock paper full-text content with equations and citations...";
        let rec = CitationFingerprintRecord::from_content(
            0x1000182,
            "10.1000/182",
            content,
            CONTENT_KIND_PDF,
            "https://doi.org/10.1000/182",
            1,
            1700000000,
        );

        assert_eq!(rec.paper_id, 0x1000182);
        assert_eq!(rec.content_kind, CONTENT_KIND_PDF);
        assert_eq!(rec.content_kind_str(), "pdf");
        assert_eq!(rec.crossref_version, 1);
        assert_eq!(rec.fetched_at, 1700000000);
        assert!(rec.is_active());
        assert!(!rec.is_tombstone());
        assert_eq!(rec.content_sha256.len(), 32);
        assert_eq!(rec.content_sha256_hex().len(), 64);

        let mut rec2 = rec;
        rec2.mark_tombstone();
        assert!(rec2.is_tombstone());
        assert!(!rec2.is_active());
    }
}

