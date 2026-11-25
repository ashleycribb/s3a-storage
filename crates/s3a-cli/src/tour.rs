//! S3A Archive Tour Module (`s3a-cli tour`)
//!
//! Provides read-only inspection of S3A archives using the Sealed Footer v2 index
//! and schema tile, falling back to scanning for v1 / unsealed archives.
//!
//! Guarantees:
//! - Strictly read-only operations.
//! - On v2 files, the lobby reads only the footer index; floor detail reads the
//!   footer plus the single requested tile.
//! - Evaluates and respects research participant consent (`purpose=research`),
//!   withholding and reporting records without active research consent.
//! - Supports plain text terminal output and machine-readable `--json`.
//!
//! Trust contract (B5/N5): the lobby seal status reports ONLY whether the seal
//! hash chain verifies. It never claims an archive is "verified": tour does not
//! re-check record payload checksums in the lobby path (that would turn every
//! lobby call into a full scan and break the read bound above). Run
//! `s3a-cli verify` for payload truth. Floor detail (`--floor N`) DOES verify
//! the payload checksums of the tile it displays and reports the result
//! per-floor (`payload_ok` / `payload_check`).
//!
//! Floor numbering (N4): the canonical floor index is the floor's position in
//! the archive (0..tile_count), matching the manifest's own tile directory.
//! The raw `tile_index` recorded in a sealed footer is NOT used for display or
//! addressing: the sealer writes the manifest tile as tile 0 without
//! renumbering the original tiles, so two floors can share one `tile_index`
//! (e.g. MANIFEST and ACADEMIC_PAPERS both 0). `--floor N`, the floor
//! directory, and `--json` all use the canonical position.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{self, Read, Seek, SeekFrom};

use bytemuck::{cast_slice, pod_read_unaligned};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use s3a_core::array::{ArrayDataType, ArrayTileHeader, ArrayTileView};
use s3a_core::consent::{ConsentPurpose, ConsentRecord};
use s3a_core::schema::{decode_schema_payload, SchemaFieldDefinition, SchemaTypeDefinition};
use s3a_core::{
    FileHeader, HyperTileHeader, MarkRecord, SealedDirectoryEntry, SealedEndCap, SealedEndCapV2,
    SealedIndexHeaderV2, SealedTileIndexEntry, TileType, FILE_HEADER_SIZE, HYPER_TILE_HEADER_SIZE,
    HYPER_TILE_SIZE, MARK_CONSENT_WITHDRAWN, MARK_SET, MARK_TARGET_ALL_RECORDS,
    SEALED_ANCHORS_MAGIC, SEALED_ENDCAP_MAGIC_V1, SEALED_ENDCAP_MAGIC_V2, SEALED_ENDCAP_SIZE,
    SEALED_INDEX_HEADER_SIZE_V2, SEALED_INDEX_MAGIC_V2,
};
use s3a_edge::{
    CompactEdgeSensorRecord, EdgeConsentRecord, EquipmentEventRecord, PURPOSE_RESEARCH_ANALYTICS,
};
use s3a_engine::anchor::{extract_anchors_from_footer, AnchorProof};
use s3a_engine::compute_tile_header_crc32;
use s3a_engine::sealed::{
    compute_endcap_crc32, compute_endcap_crc32_v2, compute_seal_hash_v2, verify_seal_chain,
};
use s3a_engine::standard_schema_for_type;
use s3a_pack_scholar::{
    AcademicPaperRecord, AgentRegistryRecord, AiTraceRecord, CitationFingerprintRecord,
    HumanLrsRecord, ResearchGraphEdgeRecord, ScholarNoteClaimRecord, ScholarTileType,
    SrsWorkspaceRecord, AGENT_FLAG_DISABLED, AGENT_FLAG_REQUIRES_HUMAN_GATE,
    AGENT_KIND_CITATION_GENERATOR, AGENT_KIND_CUSTODIAN, AGENT_KIND_ORCHESTRATOR,
    AGENT_KIND_REVIEW_COORDINATOR, AGENT_KIND_VERIFIER, CLAIM_KIND_EMPIRICAL,
    CLAIM_KIND_HYPOTHESIS, CLAIM_KIND_NOTE, CLAIM_KIND_THEORETICAL, CONTENT_KIND_ABSTRACT,
    CONTENT_KIND_HTML, CONTENT_KIND_PDF, STEP_CLAIM, STEP_HALLUCINATION, STEP_NOTE, STEP_REASONING,
    STEP_TOOL_CALL, TOOL_CREDENTIAL_EXPORT, TOOL_CROSSREF_QUERY, TOOL_EXPORT_SNAPSHOT,
    TOOL_EXTRACT_CLAIMS, TOOL_MUTATE_GRAPH, TOOL_OPENALEX_QUERY, TOOL_PUBLISH, TOOL_READ_FILE,
    TOOL_SYNTHESIZE_EVIDENCE, TOOL_TA_SUPERVISOR_CALL,
};

/// CLI Tour execution options.
#[derive(Debug, Clone, Default)]
pub struct TourOptions {
    pub json: bool,
    pub floor: Option<u32>,
    pub slice: Option<String>,
    pub cross: Option<String>,
    pub history: bool,
}

/// Authoritative serializable report for `s3a-cli tour`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TourReport {
    pub ok: bool,
    pub lobby: TourLobby,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub floors: Option<Vec<TourFloorItem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub floor_detail: Option<TourFloorDetail>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slice_result: Option<TourSliceResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cross_section: Option<TourCrossSectionResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history: Option<Vec<TourHistoryEntry>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// High-level archive lobby overview.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TourLobby {
    pub file: String,
    pub file_version: u32,
    pub footer_version: String,
    pub tile_count: u32,
    pub seal_status: TourSealStatus,
    pub consent_summary: TourConsentSummary,
    pub schema_names: BTreeMap<u32, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest: Option<String>,
}

/// Cryptographic seal and anchor verification state.
///
/// Trust contract: `hash_chain_verified` reports ONLY the seal hash chain.
/// `payloads_checked` is false for the lobby path — tour does not re-check
/// record payload checksums there (see module docs). Nothing in this struct
/// claims the payloads are verified; use `s3a-cli verify` for that, or the
/// per-floor `payload_ok` in `TourFloorDetail`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TourSealStatus {
    pub is_sealed: bool,
    pub hash_chain_verified: bool,
    pub anchors_verified: bool,
    pub anchor_status: String,
    /// Whether tour re-checked record payload checksums for this report.
    /// Always false for the lobby: the lobby reads the footer only.
    #[serde(default)]
    pub payloads_checked: bool,
    pub summary: String,
}

/// Consent governance summary for participant rights.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TourConsentSummary {
    pub total_consent_records: usize,
    pub active_research_count: usize,
    pub withdrawn_count: usize,
    pub status: String,
}

/// Item in the floor directory listing.
///
/// `index` is the canonical floor index: the floor's position in the archive
/// (0..tile_count), matching the manifest's tile directory. `min_key` /
/// `max_key` are the raw stored values; they are `u64::MAX..0` sentinels on
/// empty tiles, so use `key_range` (`None` = empty tile, no keys) for display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TourFloorItem {
    pub index: u32,
    pub type_id: u32,
    pub type_name: String,
    pub version: u16,
    pub record_count: u32,
    pub size_bytes: u64,
    pub min_key: u64,
    pub max_key: u64,
    /// Human-honest key range (`"min..max"`), or `None` when the tile holds
    /// no records (the raw `min_key`/`max_key` are sentinels then).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_range: Option<String>,
    pub payload_hash_short: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub array_info: Option<TourArrayInfo>,
}

/// Array tile dimensions, coordinates, and units.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TourArrayInfo {
    pub ndim: u16,
    pub dtype: String,
    pub shape: Vec<u64>,
    pub axis_names: String,
    pub units: String,
    pub chunk_coords: Vec<u64>,
    pub min_val: Option<f64>,
    pub max_val: Option<f64>,
}

/// Detail view for an individual floor (`--floor N`).
///
/// `index` is the canonical floor index (position in the archive). `payload_ok`
/// / `payload_check` report the payload checksum verification tour performed
/// on this tile's bytes (header CRC, data CRC, and — on v2 sealed archives —
/// the payload SHA-256 against the footer index). `payload_hash` is "—" when
/// no payload hash is recorded for the tile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TourFloorDetail {
    pub index: u32,
    pub type_id: u32,
    pub type_name: String,
    pub version: u16,
    pub record_count: u32,
    pub size_bytes: u64,
    pub min_key: u64,
    pub max_key: u64,
    /// Human-honest key range (`"min..max"`), or `None` when the tile holds
    /// no records (the raw `min_key`/`max_key` are sentinels then).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_range: Option<String>,
    pub payload_hash: String,
    /// True when this tile's payload checksums verified (see `payload_check`).
    #[serde(default)]
    pub payload_ok: bool,
    /// Human-readable payload checksum result for this tile.
    #[serde(default)]
    pub payload_check: String,
    pub fields: Vec<TourFieldInfo>,
    pub records: Vec<serde_json::Value>,
    pub withheld_count: usize,
    /// Honesty note for hash-only record types (Scholar): what the decoded
    /// fields cannot show (no title/author/DOI text is stored). `None` otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub array_stats: Option<TourArrayStats>,
}

/// Field description within a floor's schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TourFieldInfo {
    pub field_id: u16,
    pub name: String,
    pub field_type: String,
    pub byte_offset: u32,
    pub byte_len: u32,
    pub null_allowed: bool,
}

/// Numerical statistics for an Array floor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TourArrayStats {
    pub total_elements: usize,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub sum: f64,
}

/// Result of querying an Array slice across chunks (`--slice`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TourSliceResult {
    pub slice_spec: String,
    pub ndim: usize,
    pub chunks_evaluated: usize,
    pub chunks_covered: usize,
    pub chunks_skipped: usize,
    pub total_elements: usize,
    pub elements: Vec<serde_json::Value>,
}

/// Cross-section record match across floors (`--cross key=value`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TourCrossMatch {
    pub floor_index: u32,
    pub record_index: u32,
    pub type_id: u32,
    pub type_name: String,
    pub record_data: serde_json::Value,
}

/// Result of a cross-floor section query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TourCrossSectionResult {
    pub search_key: String,
    pub search_value: String,
    pub match_count: usize,
    pub withheld_count: usize,
    pub matches: Vec<TourCrossMatch>,
}

/// Individual entry in the historical seal chain (`--history`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TourHistoryEntry {
    pub generation: u64,
    pub time: u64,
    pub time_formatted: String,
    pub prev_seal_hash: String,
    pub seal_hash: String,
    pub anchor_status: String,
    pub hash_chain_valid: bool,
}

// ---------------------------------------------------------------------------
// Internal Models
// ---------------------------------------------------------------------------

struct ParsedFloorMeta {
    /// Raw tile index recorded in the footer / tile header. NOT used for
    /// display or addressing: it can collide across floors (see module docs).
    /// Kept because consent marks address records by this tile id.
    tile_index: u32,
    /// Canonical floor index: position in the archive (0..tile_count).
    /// This is what tour displays and what `--floor N` addresses.
    floor_index: u32,
    type_id: u32,
    type_version: u16,
    record_count: u32,
    offset: u64,
    length: u64,
    min_key: u64,
    max_key: u64,
    payload_hash: [u8; 32],
}

struct ParsedArchiveContext {
    file_version: u32,
    footer_version: String,
    tile_count: u32,
    seal_status: TourSealStatus,
    floors: Vec<ParsedFloorMeta>,
    schema_map: BTreeMap<u32, SchemaTypeDefinition>,
    _anchors: Vec<AnchorProof>,
    history_entries: Vec<TourHistoryEntry>,
}

fn type_enum_to_name(type_code: u16) -> &'static str {
    match type_code {
        1 => "u8",
        2 => "u16",
        3 => "u32",
        4 => "u64",
        5 => "i8",
        6 => "i16",
        7 => "i32",
        8 => "i64",
        9 => "f32",
        10 => "f64",
        11 => "string",
        12 => "uuid",
        13 => "bytes",
        _ => "unknown",
    }
}

fn parse_units_from_axis_names(axis_str: &str) -> String {
    let mut units = Vec::new();
    for part in axis_str.split(',') {
        if let Some((_, u)) = part.split_once(':') {
            units.push(u.trim());
        } else if let Some((_, rest)) = part.split_once('(') {
            if let Some((u, _)) = rest.split_once(')') {
                units.push(u.trim());
            } else {
                units.push("-");
            }
        } else {
            units.push("-");
        }
    }
    if units.iter().all(|&u| u == "-") {
        "-".to_string()
    } else {
        units.join(", ")
    }
}

fn format_unix_timestamp(ts: u64) -> String {
    if ts == 0 {
        return "genesis".to_string();
    }
    humantime::format_rfc3339_seconds(std::time::UNIX_EPOCH + std::time::Duration::from_secs(ts))
        .to_string()
}

/// Human-honest key range for a floor, or `None` when the tile holds no
/// records. The stored min/max are `u64::MAX..0` sentinels on empty tiles;
/// never display those raw.
fn key_range_display(min_key: u64, max_key: u64, record_count: u32) -> Option<String> {
    if record_count == 0 || min_key > max_key {
        None
    } else {
        Some(format!("{}..{}", min_key, max_key))
    }
}

/// Short payload hash for display, or "—" when no hash is recorded
/// (unsealed floors and v1 directory entries carry all zeros).
fn payload_hash_short_display(hash: &[u8; 32]) -> String {
    if *hash == [0u8; 32] {
        "—".to_string()
    } else {
        let full_hex = hex::encode(hash);
        format!("{}...", &full_hex[..8])
    }
}

fn payload_hash_display(hash: &[u8; 32]) -> String {
    if *hash == [0u8; 32] {
        "—".to_string()
    } else {
        hex::encode(hash)
    }
}

/// Verifies one tile's payload checksums against its own header and, on v2
/// sealed archives, the payload SHA-256 recorded in the footer index.
/// This is the same per-tile rule `s3a-cli verify` applies: header CRC (when
/// recorded), data CRC32C over `payload_bytes`, and payload SHA-256 when an
/// expected hash is recorded. Returns (ok, human-readable detail).
fn check_tile_payload(
    tile_bytes: &[u8],
    expected_payload_hash: Option<[u8; 32]>,
) -> (bool, String) {
    if tile_bytes.len() < HYPER_TILE_HEADER_SIZE {
        return (
            false,
            "tile truncated (shorter than a tile header)".to_string(),
        );
    }
    let hdr: HyperTileHeader = pod_read_unaligned(&tile_bytes[..HYPER_TILE_HEADER_SIZE]);
    if hdr.header_crc32 != 0 && hdr.header_crc32 != compute_tile_header_crc32(&hdr) {
        return (
            false,
            format!("header CRC mismatch (stored 0x{:08x})", hdr.header_crc32),
        );
    }
    let payload_len = hdr.payload_bytes as usize;
    if HYPER_TILE_HEADER_SIZE + payload_len > tile_bytes.len() {
        return (
            false,
            format!(
                "payload length {} exceeds the {} tile bytes read",
                payload_len,
                tile_bytes.len()
            ),
        );
    }
    let payload = &tile_bytes[HYPER_TILE_HEADER_SIZE..HYPER_TILE_HEADER_SIZE + payload_len];
    let actual_data_crc = crc32c::crc32c(payload);
    if hdr.data_crc32 != actual_data_crc {
        return (
            false,
            format!(
                "data CRC mismatch (stored 0x{:08x}, computed 0x{:08x})",
                hdr.data_crc32, actual_data_crc
            ),
        );
    }
    match expected_payload_hash {
        Some(expected) if expected != [0u8; 32] => {
            let actual: [u8; 32] = Sha256::digest(payload).into();
            if actual != expected {
                return (
                    false,
                    "payload SHA-256 mismatch against the footer index".to_string(),
                );
            }
            (
                true,
                "ok (header CRC ✓, data CRC ✓, payload SHA-256 ✓)".to_string(),
            )
        }
        _ => (
            true,
            "ok (header CRC ✓, data CRC ✓; no payload hash recorded for this tile)".to_string(),
        ),
    }
}

// ---------------------------------------------------------------------------
// Scholar record decoding (B3)
// ---------------------------------------------------------------------------
//
// Scholar tiles (pack-scholar) come in two record layouts per type. Layout
// v1 is the fixed POD record (`s3a-pack-scholar::records`:
// `AcademicPaperRecord`, `ResearchGraphEdgeRecord`,
// `ScholarNoteClaimRecord`, `CitationFingerprintRecord`); layout v2 (types
// 12/13 only) is the chunked-blob layout the engine schema describes. Tour
// selects the decoder from the tile's declared `record_layout_version` (see
// `is_scholar_pod_floor`): v1 tiles decode from the pack structs so a floor
// shows named fields instead of hex, v2 tiles decode through the embedded
// schema.
//
// Honesty limit: Scholar records store hashes only — no title, author, or
// DOI text exists anywhere in the archive. Hash fields are therefore shown
// as hex (what they are), never resolved into invented titles. See
// `scholar_record_note`.

fn fmt_u64_hex(v: u64) -> String {
    format!("0x{:x}", v)
}

/// Display rounding for f32 record fields: serde would print the exact f64
/// conversion (0.10000000149011612 for stored 0.1f32). Six decimals is exact
/// for the scores/coordinates these fields hold.
fn f32d(v: f32) -> f64 {
    (v as f64 * 1_000_000.0).round() / 1_000_000.0
}

fn fmt_u128_hex(v: [u64; 2]) -> String {
    format!("0x{:016x}{:016x}", v[0], v[1])
}

fn fmt_coord(c: s3a_core::S3ACoordinate) -> serde_json::Value {
    serde_json::json!({
        "level": c.level,
        "tile_id": c.tile_id,
        "record_offset": c.record_offset,
    })
}

/// "active" when the tombstone bit is clear, "tombstone" when set.
/// (`FLAG_ACTIVE` is 0 by definition; active is the absence of tombstone.)
fn record_status(flags: u64) -> &'static str {
    if flags & s3a_core::FLAG_TOMBSTONE != 0 {
        "tombstone"
    } else {
        "active"
    }
}

fn step_type_name(step_type: u32) -> &'static str {
    match step_type {
        STEP_REASONING => "reasoning",
        STEP_TOOL_CALL => "tool_call",
        STEP_CLAIM => "claim",
        STEP_NOTE => "note",
        STEP_HALLUCINATION => "hallucination",
        _ => "unknown",
    }
}

fn claim_kind_name(kind: u16) -> &'static str {
    match kind {
        CLAIM_KIND_EMPIRICAL => "empirical",
        CLAIM_KIND_THEORETICAL => "theoretical",
        CLAIM_KIND_NOTE => "note",
        CLAIM_KIND_HYPOTHESIS => "hypothesis",
        _ => "unknown",
    }
}

fn content_kind_name(kind: u32) -> &'static str {
    match kind {
        CONTENT_KIND_PDF => "PDF",
        CONTENT_KIND_HTML => "HTML",
        CONTENT_KIND_ABSTRACT => "abstract",
        _ => "unknown",
    }
}

fn agent_kind_name(kind: u16) -> &'static str {
    match kind {
        AGENT_KIND_ORCHESTRATOR => "orchestrator",
        AGENT_KIND_VERIFIER => "verifier",
        AGENT_KIND_CUSTODIAN => "custodian",
        AGENT_KIND_REVIEW_COORDINATOR => "review_coordinator",
        AGENT_KIND_CITATION_GENERATOR => "citation_generator",
        _ => "unknown",
    }
}

fn tool_names(mask: u32) -> Vec<&'static str> {
    let mut names = Vec::new();
    if mask & TOOL_READ_FILE != 0 {
        names.push("read_file");
    }
    if mask & TOOL_OPENALEX_QUERY != 0 {
        names.push("openalex_query");
    }
    if mask & TOOL_EXTRACT_CLAIMS != 0 {
        names.push("extract_claims");
    }
    if mask & TOOL_SYNTHESIZE_EVIDENCE != 0 {
        names.push("synthesize_evidence");
    }
    if mask & TOOL_CROSSREF_QUERY != 0 {
        names.push("crossref_query");
    }
    if mask & TOOL_EXPORT_SNAPSHOT != 0 {
        names.push("export_snapshot");
    }
    if mask & TOOL_TA_SUPERVISOR_CALL != 0 {
        names.push("ta_supervisor_call");
    }
    if mask & TOOL_MUTATE_GRAPH != 0 {
        names.push("mutate_graph");
    }
    if mask & TOOL_PUBLISH != 0 {
        names.push("publish");
    }
    if mask & TOOL_CREDENTIAL_EXPORT != 0 {
        names.push("credential_export");
    }
    names
}

/// Byte size of one record for a Scholar tile type, from the pack structs
/// (NOT the registry/schema `record_size`: the registry still says 64 for
/// note_claims while `ScholarNoteClaimRecord` is 160 bytes).
fn scholar_record_size(type_id: u32) -> Option<usize> {
    match type_id {
        ScholarTileType::HUMAN_LRS => Some(std::mem::size_of::<HumanLrsRecord>()),
        ScholarTileType::AI_TRACE_LOG => Some(std::mem::size_of::<AiTraceRecord>()),
        ScholarTileType::ACADEMIC_PAPERS => Some(std::mem::size_of::<AcademicPaperRecord>()),
        ScholarTileType::RESEARCH_GRAPH => Some(std::mem::size_of::<ResearchGraphEdgeRecord>()),
        ScholarTileType::SRS_WORKSPACE => Some(std::mem::size_of::<SrsWorkspaceRecord>()),
        ScholarTileType::AGENT_REGISTRY => Some(std::mem::size_of::<AgentRegistryRecord>()),
        ScholarTileType::NOTE_CLAIMS => Some(std::mem::size_of::<ScholarNoteClaimRecord>()),
        ScholarTileType::CITATION_FINGERPRINT => {
            Some(std::mem::size_of::<CitationFingerprintRecord>())
        }
        _ => None,
    }
}

/// B3 (corrected Nov 18, 2026): Scholar tile types 12/13 exist in two record
/// layouts. Layout v1 is the fixed POD record (`AcademicPaperRecord`,
/// `ResearchGraphEdgeRecord`, documented in docs/spec/legacy-layouts.md);
/// layout v2 is the chunked-blob layout (`ScholarPaperBlobHeader` + JSON
/// chunks, documented in docs/spec/registry.md) and is what the engine
/// schema describes. Types 20/32 have only the v1 POD layout. The tile's
/// declared `record_layout_version` (`floor.type_version`, taken from the
/// tile header / footer entry) selects the decoder: v1 tiles decode through
/// the pack decoder (`decode_scholar_record`), v2 tiles through the embedded
/// schema. Never select on the schema shape — one schema entry per type can
/// only describe the current (v2) layout, so it cannot tell a v1 tile from
/// a v2 tile.
fn scholar_pod_layout_version(type_id: u32) -> Option<u16> {
    match type_id {
        ScholarTileType::ACADEMIC_PAPERS
        | ScholarTileType::RESEARCH_GRAPH
        | ScholarTileType::NOTE_CLAIMS
        | ScholarTileType::CITATION_FINGERPRINT => Some(1),
        _ => None,
    }
}

/// True when this floor is a Scholar POD (layout v1) tile: decode it with
/// the pack decoder, not the embedded schema.
fn is_scholar_pod_floor(type_id: u32, type_version: u16) -> bool {
    scholar_record_size(type_id).is_some()
        && scholar_pod_layout_version(type_id) == Some(type_version)
}

/// Honesty note shown once per Scholar floor: what the decoded fields
/// cannot show, because the archive stores hashes, not identity text.
fn scholar_record_note(type_id: u32) -> Option<String> {
    match type_id {
        ScholarTileType::HUMAN_LRS
        | ScholarTileType::AI_TRACE_LOG
        | ScholarTileType::ACADEMIC_PAPERS
        | ScholarTileType::RESEARCH_GRAPH
        | ScholarTileType::SRS_WORKSPACE
        | ScholarTileType::AGENT_REGISTRY
        | ScholarTileType::NOTE_CLAIMS
        | ScholarTileType::CITATION_FINGERPRINT => Some(
            "Records store hashes only — no title, author, or DOI text is stored in \
            this archive, so hash fields are shown as hex and cannot be resolved to \
            names here. (Titles/DOIs live in the researcher's dossier, not in S3A.)"
                .to_string(),
        ),
        _ => None,
    }
}

/// Decodes one Scholar record from its raw bytes into named fields.
/// Returns `None` for non-Scholar types or a short slice.
fn decode_scholar_record(type_id: u32, slice: &[u8]) -> Option<serde_json::Value> {
    match type_id {
        ScholarTileType::HUMAN_LRS => {
            if slice.len() < std::mem::size_of::<HumanLrsRecord>() {
                return None;
            }
            let r: HumanLrsRecord = pod_read_unaligned(slice);
            Some(serde_json::json!({
                "session_uuid": fmt_u128_hex(r.session_uuid),
                "timestamp": format_unix_timestamp(r.timestamp_sec),
                "timestamp_sec": r.timestamp_sec,
                "actor_hash": fmt_u64_hex(r.actor_hash),
                "verb_id": r.verb_id,
                "decision_score": f32d(r.decision_score),
                "ai_trace_coord": fmt_coord(r.ai_trace_coord),
                "object_hash": fmt_u64_hex(r.object_hash as u64),
                "flags": r.flags,
                "status": record_status(r.flags as u64),
            }))
        }
        ScholarTileType::AI_TRACE_LOG => {
            if slice.len() < std::mem::size_of::<AiTraceRecord>() {
                return None;
            }
            let r: AiTraceRecord = pod_read_unaligned(slice);
            Some(serde_json::json!({
                "session_uuid": fmt_u128_hex(r.session_uuid),
                "timestamp": format_unix_timestamp(r.timestamp_sec),
                "timestamp_sec": r.timestamp_sec,
                "agent_id": fmt_u64_hex(r.agent_id),
                "step_type": step_type_name(r.step_type),
                "step_type_id": r.step_type,
                "confidence": f32d(r.confidence),
                "human_coord": fmt_coord(r.human_coord),
                "hilbert_index": r.hilbert_index,
                "flags": r.flags,
                "status": record_status(r.flags as u64),
            }))
        }
        ScholarTileType::ACADEMIC_PAPERS => {
            if slice.len() < std::mem::size_of::<AcademicPaperRecord>() {
                return None;
            }
            let r: AcademicPaperRecord = pod_read_unaligned(slice);
            Some(serde_json::json!({
                "paper_id": r.paper_id,
                "paper_id_hex": fmt_u64_hex(r.paper_id),
                "published": format_unix_timestamp(r.timestamp_sec),
                "timestamp_sec": r.timestamp_sec,
                "hilbert_index": r.hilbert_index,
                "topic": { "x": f32d(r.topic_x), "y": f32d(r.topic_y), "z": f32d(r.topic_z) },
                "citation_count": r.citation_count,
                "year": r.year,
                "venue_id": r.venue_id,
                "open_access": r.open_access_flag != 0,
                "warning_count": r.warning_count,
                "doi_prefix_hash": fmt_u64_hex(r.doi_prefix_hash),
                "flags": r.flags,
                "status": record_status(r.flags),
            }))
        }
        ScholarTileType::RESEARCH_GRAPH => {
            if slice.len() < std::mem::size_of::<ResearchGraphEdgeRecord>() {
                return None;
            }
            let r: ResearchGraphEdgeRecord = pod_read_unaligned(slice);
            Some(serde_json::json!({
                "subject_hash": fmt_u64_hex(r.subject_hash),
                "object_hash": fmt_u64_hex(r.object_hash),
                "timestamp": format_unix_timestamp(r.timestamp_sec),
                "timestamp_sec": r.timestamp_sec,
                "hilbert_coord": r.hilbert_coord,
                "predicate_id": r.predicate_id,
                "weight": f32d(r.weight),
                "flags": r.flags,
                "status": record_status(r.flags),
            }))
        }
        ScholarTileType::SRS_WORKSPACE => {
            if slice.len() < std::mem::size_of::<SrsWorkspaceRecord>() {
                return None;
            }
            let r: SrsWorkspaceRecord = pod_read_unaligned(slice);
            Some(serde_json::json!({
                "project_uuid": fmt_u128_hex(r.project_uuid),
                "question_hash": fmt_u64_hex(r.question_hash),
                "collection_mask": fmt_u64_hex(r.collection_mask),
                "topic_centroid": { "x": f32d(r.topic_centroid_xyz[0]), "y": f32d(r.topic_centroid_xyz[1]), "z": f32d(r.topic_centroid_xyz[2]) },
                "status_id": r.status,
                "pinned_works_count": r.pinned_works_count,
                "claims_count": r.claims_count,
                "flags": r.flags,
                "status": record_status(r.flags),
            }))
        }
        ScholarTileType::AGENT_REGISTRY => {
            if slice.len() < std::mem::size_of::<AgentRegistryRecord>() {
                return None;
            }
            let r: AgentRegistryRecord = pod_read_unaligned(slice);
            let mut agent_flags = Vec::new();
            if r.flags & AGENT_FLAG_DISABLED != 0 {
                agent_flags.push("disabled");
            }
            if r.flags & AGENT_FLAG_REQUIRES_HUMAN_GATE != 0 {
                agent_flags.push("requires_human_gate");
            }
            if agent_flags.is_empty() {
                agent_flags.push("active");
            }
            Some(serde_json::json!({
                "agent_id": fmt_u64_hex(r.agent_id),
                "xapi_profile_iri_hash": fmt_u64_hex(r.xapi_profile_iri_hash),
                "owl_ontology_iri_hash": fmt_u64_hex(r.owl_ontology_iri_hash),
                "allowed_verb_mask": r.allowed_verb_mask,
                "allowed_tools": tool_names(r.allowed_tool_mask),
                "denied_tools": tool_names(r.denied_tool_mask),
                "agent_kind": agent_kind_name(r.agent_kind),
                "agent_kind_id": r.agent_kind,
                "agent_flags": agent_flags,
                "version": r.version,
            }))
        }
        ScholarTileType::NOTE_CLAIMS => {
            if slice.len() < std::mem::size_of::<ScholarNoteClaimRecord>() {
                return None;
            }
            let r: ScholarNoteClaimRecord = pod_read_unaligned(slice);
            Some(serde_json::json!({
                "claim_id": r.claim_id,
                "timestamp": format_unix_timestamp(r.timestamp_sec),
                "timestamp_sec": r.timestamp_sec,
                "session_uuid": fmt_u128_hex(r.session_uuid),
                "author_hash": fmt_u64_hex(r.author_hash),
                "confidence": f32d(r.confidence),
                "kind": claim_kind_name(r.kind),
                "kind_id": r.kind,
                "flags": r.flags,
                "status": record_status(r.flags as u64),
                "keyword_bloom": fmt_u64_hex(r.keyword_bloom as u64),
                "text_hash": fmt_u64_hex(r.text_hash as u64),
                "embedding_tile_id": r.embedding_tile_id,
                "embedding_offset": r.embedding_offset,
                "citation_fingerprint_id": r.citation_fingerprint_id,
                "passage_page": r.passage_page,
                "passage_char_offset": r.passage_char_offset,
                "passage_char_length": r.passage_char_length,
                "quote_sha256": hex::encode(r.quote_sha256),
                "producer_id": r.producer_id,
                "verification_statement_uuid": fmt_u128_hex(r.verification_statement_uuid),
            }))
        }
        ScholarTileType::CITATION_FINGERPRINT => {
            if slice.len() < std::mem::size_of::<CitationFingerprintRecord>() {
                return None;
            }
            let r: CitationFingerprintRecord = pod_read_unaligned(slice);
            Some(serde_json::json!({
                "paper_id": r.paper_id,
                "doi_hash": fmt_u64_hex(r.doi_hash),
                "content_sha256": hex::encode(r.content_sha256),
                "content_kind": content_kind_name(r.content_kind),
                "content_kind_id": r.content_kind,
                "crossref_version": r.crossref_version,
                "fetched_at": format_unix_timestamp(r.fetched_at),
                "fetched_at_sec": r.fetched_at,
                "source_url_hash": fmt_u64_hex(r.source_url_hash),
                "flags": r.flags,
                "status": record_status(r.flags),
            }))
        }
        _ => None,
    }
}

/// Field table for a Scholar floor's "Schema Fields" listing, mirroring the
/// pack struct layouts (name, type, byte offset, byte length).
fn scholar_record_fields(type_id: u32) -> Option<Vec<TourFieldInfo>> {
    let f = |field_id: u16, name: &str, field_type: &str, byte_offset: u32, byte_len: u32| {
        TourFieldInfo {
            field_id,
            name: name.to_string(),
            field_type: field_type.to_string(),
            byte_offset,
            byte_len,
            null_allowed: false,
        }
    };
    let fields = match type_id {
        ScholarTileType::HUMAN_LRS => vec![
            f(1, "session_uuid", "uuid128", 0, 16),
            f(2, "timestamp_sec", "u64", 16, 8),
            f(3, "actor_hash", "u64", 24, 8),
            f(4, "verb_id", "u32", 32, 4),
            f(5, "decision_score", "f32", 36, 4),
            f(6, "ai_trace_coord", "coord", 40, 16),
            f(7, "object_hash", "u32", 56, 4),
            f(8, "flags", "u32", 60, 4),
        ],
        ScholarTileType::AI_TRACE_LOG => vec![
            f(1, "session_uuid", "uuid128", 0, 16),
            f(2, "timestamp_sec", "u64", 16, 8),
            f(3, "agent_id", "u64", 24, 8),
            f(4, "step_type", "u32", 32, 4),
            f(5, "confidence", "f32", 36, 4),
            f(6, "human_coord", "coord", 40, 16),
            f(7, "hilbert_index", "u32", 56, 4),
            f(8, "flags", "u32", 60, 4),
        ],
        ScholarTileType::ACADEMIC_PAPERS => vec![
            f(1, "paper_id", "u64", 0, 8),
            f(2, "timestamp_sec", "u64", 8, 8),
            f(3, "hilbert_index", "u64", 16, 8),
            f(4, "topic_x", "f32", 24, 4),
            f(5, "topic_y", "f32", 28, 4),
            f(6, "topic_z", "f32", 32, 4),
            f(7, "citation_count", "u32", 36, 4),
            f(8, "year", "u16", 40, 2),
            f(9, "venue_id", "u16", 42, 2),
            f(10, "open_access_flag", "u16", 44, 2),
            f(11, "warning_count", "u16", 46, 2),
            f(12, "doi_prefix_hash", "u64", 48, 8),
            f(13, "flags", "u64", 56, 8),
        ],
        ScholarTileType::RESEARCH_GRAPH => vec![
            f(1, "subject_hash", "u64", 0, 8),
            f(2, "object_hash", "u64", 8, 8),
            f(3, "timestamp_sec", "u64", 16, 8),
            f(4, "hilbert_coord", "u64", 24, 8),
            f(5, "predicate_id", "u32", 32, 4),
            f(6, "weight", "f32", 36, 4),
            f(7, "flags", "u64", 40, 8),
        ],
        ScholarTileType::SRS_WORKSPACE => vec![
            f(1, "project_uuid", "uuid128", 0, 16),
            f(2, "question_hash", "u64", 16, 8),
            f(3, "collection_mask", "u64", 24, 8),
            f(4, "topic_centroid_xyz", "f32[3]", 32, 12),
            f(5, "status", "u32", 44, 4),
            f(6, "pinned_works_count", "u32", 48, 4),
            f(7, "claims_count", "u32", 52, 4),
            f(8, "flags", "u64", 56, 8),
        ],
        ScholarTileType::AGENT_REGISTRY => vec![
            f(1, "agent_id", "u64", 0, 8),
            f(2, "xapi_profile_iri_hash", "u64", 8, 8),
            f(3, "owl_ontology_iri_hash", "u64", 16, 8),
            f(4, "allowed_verb_mask", "u32", 24, 4),
            f(5, "allowed_tool_mask", "u32", 28, 4),
            f(6, "denied_tool_mask", "u32", 32, 4),
            f(7, "agent_kind", "u16", 36, 2),
            f(8, "flags", "u16", 38, 2),
            f(9, "version", "u64", 40, 8),
        ],
        ScholarTileType::NOTE_CLAIMS => vec![
            f(1, "claim_id", "u64", 0, 8),
            f(2, "timestamp_sec", "u64", 8, 8),
            f(3, "session_uuid", "uuid128", 16, 16),
            f(4, "author_hash", "u64", 32, 8),
            f(5, "confidence", "f32", 40, 4),
            f(6, "kind", "u16", 44, 2),
            f(7, "flags", "u16", 46, 2),
            f(8, "keyword_bloom", "u32", 48, 4),
            f(9, "text_hash", "u32", 52, 4),
            f(10, "embedding_tile_id", "u32", 56, 4),
            f(11, "embedding_offset", "u32", 60, 4),
            f(12, "citation_fingerprint_id", "u64", 64, 8),
            f(13, "passage_page", "u32", 72, 4),
            f(14, "passage_char_offset", "u32", 76, 4),
            f(15, "passage_char_length", "u32", 80, 4),
            f(16, "quote_sha256", "bytes", 88, 32),
            f(17, "producer_id", "u64", 120, 8),
            f(18, "verification_statement_uuid", "uuid128", 128, 16),
        ],
        ScholarTileType::CITATION_FINGERPRINT => vec![
            f(1, "paper_id", "u64", 0, 8),
            f(2, "doi_hash", "u64", 8, 8),
            f(3, "content_sha256", "bytes", 16, 32),
            f(4, "content_kind", "u32", 48, 4),
            f(5, "crossref_version", "u32", 52, 4),
            f(6, "fetched_at", "u64", 56, 8),
            f(7, "source_url_hash", "u64", 64, 8),
            f(8, "flags", "u64", 72, 8),
        ],
        _ => return None,
    };
    Some(fields)
}

// ---------------------------------------------------------------------------
// Core Tour Engine
// ---------------------------------------------------------------------------

/// Executes `s3a-cli tour` over an open reader.
pub fn tour_file<R: Read + Seek>(
    reader: &mut R,
    file_len: u64,
    path: &str,
    opts: &TourOptions,
) -> Result<TourReport, Box<dyn std::error::Error>> {
    let _ = opts.json;
    let ctx = inspect_archive_context(reader, file_len, path)?;

    // Build schemas map for lobby
    let mut schema_names = BTreeMap::new();
    for (t_id, def) in &ctx.schema_map {
        schema_names.insert(*t_id, def.name.clone());
    }

    // Evaluate consent state across the archive
    let consent_summary = evaluate_consent_summary(reader, &ctx.floors)?;

    let mut manifest = None;
    if let Some(m_floor) = ctx.floors.iter().find(|f| f.type_id == TileType::MANIFEST) {
        let mut hdr_buf = [0u8; HYPER_TILE_HEADER_SIZE];
        reader.seek(SeekFrom::Start(m_floor.offset))?;
        if reader.read_exact(&mut hdr_buf).is_ok() {
            let hdr: HyperTileHeader = pod_read_unaligned(&hdr_buf);
            let payload_len =
                (hdr.payload_bytes as usize).min(HYPER_TILE_SIZE - HYPER_TILE_HEADER_SIZE);
            if payload_len > 0 {
                let mut payload = vec![0u8; payload_len];
                reader.seek(SeekFrom::Start(
                    m_floor.offset + HYPER_TILE_HEADER_SIZE as u64,
                ))?;
                if reader.read_exact(&mut payload).is_ok() {
                    manifest = String::from_utf8(payload).ok();
                }
            }
        }
    }

    let lobby = TourLobby {
        file: path.to_string(),
        file_version: ctx.file_version,
        footer_version: ctx.footer_version.clone(),
        tile_count: ctx.tile_count,
        seal_status: ctx.seal_status.clone(),
        consent_summary,
        schema_names,
        manifest,
    };

    let mut report = TourReport {
        ok: true,
        lobby,
        floors: None,
        floor_detail: None,
        slice_result: None,
        cross_section: None,
        history: None,
        error: None,
    };

    // 1. History mode
    if opts.history {
        report.history = Some(ctx.history_entries.clone());
        return Ok(report);
    }

    // 2. Specific floor detail (`--floor N`)
    if let Some(target_floor) = opts.floor {
        let detail = inspect_floor_detail(reader, target_floor, &ctx)?;
        report.floor_detail = Some(detail);
        return Ok(report);
    }

    // 3. Array slice mode (`--slice spec`)
    if let Some(slice_spec) = &opts.slice {
        let slice_res = execute_array_slice(reader, slice_spec, &ctx)?;
        report.slice_result = Some(slice_res);
        return Ok(report);
    }

    // 4. Cross-section search (`--cross key=val`)
    if let Some(cross_expr) = &opts.cross {
        let cross_res = execute_cross_section(reader, cross_expr, &ctx)?;
        report.cross_section = Some(cross_res);
        return Ok(report);
    }

    // 5. Default mode: Lobby + Floor List
    let mut floor_items = Vec::with_capacity(ctx.floors.len());
    for f in &ctx.floors {
        let type_name = ctx
            .schema_map
            .get(&f.type_id)
            .map(|s| s.name.clone())
            .unwrap_or_else(|| format!("type_{}", f.type_id));

        // "—" when no payload hash is recorded (all unsealed floors, v1
        // entries): the old "00000000..." looked like a real hash.
        let hash_short = payload_hash_short_display(&f.payload_hash);
        // Honest key range: None on empty tiles (raw min/max are sentinels).
        let key_range = key_range_display(f.min_key, f.max_key, f.record_count);

        let mut array_info = None;
        if f.type_id == TileType::ARRAY {
            if let Ok(hdr) = read_array_header(reader, f.offset) {
                let ndim = hdr.ndim as usize;
                let shape = hdr.shape[..ndim].to_vec();
                let chunk_coords = hdr.chunk_coords[..ndim].to_vec();
                let axis_names = hdr.axis_names_as_str().to_string();
                let units = parse_units_from_axis_names(&axis_names);
                let dtype_str = ArrayDataType::from_u16(hdr.dtype)
                    .map(|d| format!("{:?}", d))
                    .unwrap_or_else(|| format!("dtype_{}", hdr.dtype));

                // Only report a value range when the stored bounds are
                // ordered; on empty tiles they are u64::MAX..0 sentinels.
                let (min_val, max_val) = if f.min_key <= f.max_key {
                    (Some(f.min_key as f64), Some(f.max_key as f64))
                } else {
                    (None, None)
                };

                array_info = Some(TourArrayInfo {
                    ndim: hdr.ndim,
                    dtype: dtype_str,
                    shape,
                    axis_names,
                    units,
                    chunk_coords,
                    min_val,
                    max_val,
                });
            }
        }

        floor_items.push(TourFloorItem {
            index: f.floor_index,
            type_id: f.type_id,
            type_name,
            version: f.type_version,
            record_count: f.record_count,
            size_bytes: f.length,
            min_key: f.min_key,
            max_key: f.max_key,
            key_range,
            payload_hash_short: hash_short,
            array_info,
        });
    }

    report.floors = Some(floor_items);
    Ok(report)
}

// ---------------------------------------------------------------------------
// Context Inspection: V2 vs V1 vs Unsealed
// ---------------------------------------------------------------------------

fn inspect_archive_context<R: Read + Seek>(
    reader: &mut R,
    file_len: u64,
    _path: &str,
) -> Result<ParsedArchiveContext, Box<dyn std::error::Error>> {
    // Attempt V2 fast path first
    if file_len >= (FILE_HEADER_SIZE + SEALED_ENDCAP_SIZE) as u64 {
        reader.seek(SeekFrom::Start(file_len - SEALED_ENDCAP_SIZE as u64))?;
        let mut endcap_buf = [0u8; SEALED_ENDCAP_SIZE];
        reader.read_exact(&mut endcap_buf)?;

        let endcap: SealedEndCapV2 = pod_read_unaligned(&endcap_buf);
        if endcap.magic == SEALED_ENDCAP_MAGIC_V2
            && endcap.endcap_crc32 == compute_endcap_crc32_v2(&endcap)
            && endcap.index_offset + endcap.index_len + (SEALED_ENDCAP_SIZE as u64) <= file_len
            && endcap.index_len >= SEALED_INDEX_HEADER_SIZE_V2 as u64
        {
            // Fast Path: read ONLY footer index
            reader.seek(SeekFrom::Start(endcap.index_offset))?;
            let mut index_bytes = vec![0u8; endcap.index_len as usize];
            reader.read_exact(&mut index_bytes)?;

            let actual_hash: [u8; 32] = Sha256::digest(&index_bytes).into();
            if actual_hash == endcap.index_hash {
                let index_hdr: SealedIndexHeaderV2 =
                    pod_read_unaligned(&index_bytes[..SEALED_INDEX_HEADER_SIZE_V2]);
                if index_hdr.magic == SEALED_INDEX_MAGIC_V2 {
                    let entries_slice = &index_bytes[SEALED_INDEX_HEADER_SIZE_V2..];
                    let v2_entries: &[SealedTileIndexEntry] = cast_slice(entries_slice);

                    let seal_hash_valid = compute_seal_hash_v2(
                        &index_hdr.prev_seal_hash,
                        index_hdr.seal_generation,
                        index_hdr.tile_count,
                        index_hdr.file_uuid,
                        v2_entries,
                    ) == index_hdr.seal_hash;

                    let chain_res = verify_seal_chain(&[index_hdr]);
                    let chain_valid = chain_res.is_ok();

                    // Check for embedded anchors in footer
                    let mut anchors = Vec::new();
                    let data_end_offset = FILE_HEADER_SIZE as u64
                        + (index_hdr.tile_count as u64) * (HYPER_TILE_SIZE as u64);
                    if endcap.index_offset > data_end_offset {
                        let potential_anchor_len = (endcap.index_offset - data_end_offset) as usize;
                        if potential_anchor_len >= 12 {
                            reader.seek(SeekFrom::Start(data_end_offset))?;
                            let mut anchor_buf = vec![0u8; potential_anchor_len];
                            if reader.read_exact(&mut anchor_buf).is_ok()
                                && anchor_buf[..8] == SEALED_ANCHORS_MAGIC
                            {
                                anchors = extract_anchors_from_footer(&anchor_buf, 0, 0);
                            }
                        }
                    }

                    let anchor_status_str = if anchors.is_empty() {
                        "none".to_string()
                    } else if anchors.iter().all(|a| a.status == "confirmed") {
                        format!("confirmed ({} proofs)", anchors.len())
                    } else {
                        format!("pending ({} proofs)", anchors.len())
                    };

                    let anchors_ok =
                        !anchors.is_empty() && anchors.iter().all(|a| a.status == "confirmed");

                    // B5/N5: the lobby reports ONLY the seal hash chain. It
                    // must not say "verified": tour does not re-check record
                    // payload checksums in the lobby path (that would be a
                    // full scan on every call). `payloads_checked` is false
                    // so a consumer can tell chain-valid from payload-valid.
                    let seal_status = TourSealStatus {
                        is_sealed: true,
                        hash_chain_verified: chain_valid && seal_hash_valid,
                        anchors_verified: anchors_ok,
                        anchor_status: anchor_status_str.clone(),
                        payloads_checked: false,
                        summary: if chain_valid && seal_hash_valid {
                            format!(
                                "chain valid ✓ (payloads not re-checked by tour — run `s3a-cli verify` for payload checksums; Anchors: {})",
                                anchor_status_str
                            )
                        } else {
                            "chain broken ✗ (checksum or chain invalid)".to_string()
                        },
                    };

                    let history_entry = TourHistoryEntry {
                        generation: index_hdr.seal_generation,
                        time: index_hdr.sealed_timestamp,
                        time_formatted: format_unix_timestamp(index_hdr.sealed_timestamp),
                        prev_seal_hash: hex::encode(index_hdr.prev_seal_hash),
                        seal_hash: hex::encode(index_hdr.seal_hash),
                        anchor_status: anchor_status_str,
                        hash_chain_valid: chain_valid && seal_hash_valid,
                    };

                    // N4: the canonical floor index is the floor's position
                    // in the footer index (0..tile_count), matching the
                    // manifest's tile directory. The raw `tile_index` from
                    // the entry is kept only for consent-mark matching: the
                    // sealer writes the manifest as tile 0 without
                    // renumbering the original tiles, so it can collide
                    // (MANIFEST and ACADEMIC_PAPERS both 0).
                    let mut floors = Vec::with_capacity(v2_entries.len());
                    for (pos, e) in v2_entries.iter().enumerate() {
                        floors.push(ParsedFloorMeta {
                            tile_index: e.tile_index,
                            floor_index: pos as u32,
                            type_id: e.type_id,
                            type_version: e.type_version,
                            record_count: e.record_count,
                            offset: e.offset,
                            length: e.length,
                            min_key: e.min_key,
                            max_key: e.max_key,
                            payload_hash: e.payload_hash,
                        });
                    }

                    // Build schema map
                    let mut schema_map = BTreeMap::new();
                    // First register standard schemas for all present types
                    for f in &floors {
                        schema_map
                            .entry(f.type_id)
                            .or_insert_with(|| standard_schema_for_type(f.type_id));
                    }

                    // If a schema tile (77) is present, read ONLY that tile and override schemas
                    for f in &floors {
                        if f.type_id == TileType::SCHEMA {
                            if let Ok(tile_bytes) =
                                read_exact_tile_bytes(reader, f.offset, f.length)
                            {
                                if tile_bytes.len() > HYPER_TILE_HEADER_SIZE {
                                    let payload = &tile_bytes[HYPER_TILE_HEADER_SIZE..];
                                    if let Ok(decoded_schemas) = decode_schema_payload(payload) {
                                        for s in decoded_schemas {
                                            schema_map.insert(s.type_id, s);
                                        }
                                    }
                                }
                            }
                            break;
                        }
                    }

                    return Ok(ParsedArchiveContext {
                        file_version: 2,
                        footer_version: "v2".to_string(),
                        tile_count: index_hdr.tile_count,
                        seal_status,
                        floors,
                        schema_map,
                        _anchors: anchors,
                        history_entries: vec![history_entry],
                    });
                }
            }
        }
    }

    // Attempt V1 fallback
    if file_len >= (FILE_HEADER_SIZE + SEALED_ENDCAP_SIZE) as u64 {
        reader.seek(SeekFrom::Start(file_len - SEALED_ENDCAP_SIZE as u64))?;
        let mut endcap_buf = [0u8; SEALED_ENDCAP_SIZE];
        reader.read_exact(&mut endcap_buf)?;
        let endcap_v1: SealedEndCap = pod_read_unaligned(&endcap_buf);

        if endcap_v1.magic == SEALED_ENDCAP_MAGIC_V1
            && endcap_v1.endcap_crc32 == compute_endcap_crc32(&endcap_v1)
        {
            let dir_offset = endcap_v1.footer_offset;
            let dir_len =
                (endcap_v1.tile_count as usize) * std::mem::size_of::<SealedDirectoryEntry>();

            if dir_offset + (dir_len as u64) + (SEALED_ENDCAP_SIZE as u64) <= file_len {
                reader.seek(SeekFrom::Start(dir_offset))?;
                let mut dir_bytes = vec![0u8; dir_len];
                reader.read_exact(&mut dir_bytes)?;
                let dir_entries: &[SealedDirectoryEntry] = cast_slice(&dir_bytes);

                let mut floors = Vec::with_capacity(dir_entries.len());
                let mut schema_map = BTreeMap::new();

                // N4: canonical floor index is the position in the directory.
                for (pos, e) in dir_entries.iter().enumerate() {
                    floors.push(ParsedFloorMeta {
                        tile_index: e.tile_index,
                        floor_index: pos as u32,
                        type_id: e.tile_type,
                        type_version: 1,
                        record_count: 0,
                        offset: e.offset,
                        length: e.length,
                        min_key: 0,
                        max_key: 0,
                        payload_hash: [0u8; 32],
                    });
                    schema_map
                        .entry(e.tile_type)
                        .or_insert_with(|| standard_schema_for_type(e.tile_type));
                }

                return Ok(ParsedArchiveContext {
                    file_version: 1,
                    footer_version: "v1".to_string(),
                    tile_count: endcap_v1.tile_count,
                    seal_status: TourSealStatus {
                        is_sealed: true,
                        // B5: v1 verifies nothing but the directory endcap
                        // CRC; it never checked a hash chain, so it must not
                        // claim the archive is verified either.
                        hash_chain_verified: false,
                        anchors_verified: false,
                        anchor_status: "v1".to_string(),
                        payloads_checked: false,
                        summary: "sealed v1 (directory endcap CRC ok; payloads not re-checked by tour — run `s3a-cli verify` for payload checksums)".to_string(),
                    },
                    floors,
                    schema_map,
                    _anchors: Vec::new(),
                    history_entries: Vec::new(),
                });
            }
        }
    }

    // Unsealed fallback: scan tile headers from file
    reader.seek(SeekFrom::Start(0))?;
    let mut file_hdr_buf = [0u8; FILE_HEADER_SIZE];
    reader.read_exact(&mut file_hdr_buf)?;
    // Use the arbitrated active slot (§2), not always slot A: slot B is newer after an
    // odd number of commits, and slot A then still lists the older tile count.
    let slot_a: FileHeader = pod_read_unaligned(&file_hdr_buf[..64]);
    let slot_b: FileHeader = pod_read_unaligned(&file_hdr_buf[64..128]);
    let active_hdr: FileHeader = match s3a_engine::resolve_active_header(&slot_a, &slot_b) {
        Ok((h, _)) => h,
        Err(_) => slot_a,
    };

    let tile_count = active_hdr.tile_count;
    let mut floors = Vec::with_capacity(tile_count as usize);
    let mut schema_map = BTreeMap::new();

    for i in 0..tile_count {
        let offset = FILE_HEADER_SIZE as u64 + (i as u64) * (HYPER_TILE_SIZE as u64);
        if offset + (HYPER_TILE_HEADER_SIZE as u64) <= file_len {
            reader.seek(SeekFrom::Start(offset))?;
            let mut tile_hdr_buf = [0u8; HYPER_TILE_HEADER_SIZE];
            if reader.read_exact(&mut tile_hdr_buf).is_ok() {
                let th: HyperTileHeader = pod_read_unaligned(&tile_hdr_buf);
                floors.push(ParsedFloorMeta {
                    // Unsealed scan: position and tile index agree.
                    tile_index: i,
                    floor_index: i,
                    type_id: th.tile_type,
                    type_version: th.record_layout_version as u16,
                    record_count: th.record_count,
                    offset,
                    length: HYPER_TILE_SIZE as u64,
                    min_key: th.min_timestamp,
                    max_key: th.max_timestamp,
                    payload_hash: [0u8; 32],
                });
                schema_map
                    .entry(th.tile_type)
                    .or_insert_with(|| standard_schema_for_type(th.tile_type));
            }
        }
    }

    Ok(ParsedArchiveContext {
        file_version: active_hdr.version as u32,
        footer_version: "unsealed".to_string(),
        tile_count,
        seal_status: TourSealStatus {
            is_sealed: false,
            hash_chain_verified: false,
            anchors_verified: false,
            anchor_status: "none".to_string(),
            payloads_checked: false,
            summary: "unsealed (live database)".to_string(),
        },
        floors,
        schema_map,
        _anchors: Vec::new(),
        history_entries: Vec::new(),
    })
}

// ---------------------------------------------------------------------------
// Helpers: Reading Tile Bytes & Array Headers
// ---------------------------------------------------------------------------

fn read_exact_tile_bytes<R: Read + Seek>(
    reader: &mut R,
    offset: u64,
    length: u64,
) -> io::Result<Vec<u8>> {
    reader.seek(SeekFrom::Start(offset))?;
    let mut buf = vec![0u8; length as usize];
    reader.read_exact(&mut buf)?;
    Ok(buf)
}

fn read_array_header<R: Read + Seek>(
    reader: &mut R,
    tile_offset: u64,
) -> io::Result<ArrayTileHeader> {
    reader.seek(SeekFrom::Start(tile_offset + HYPER_TILE_HEADER_SIZE as u64))?;
    let mut buf = [0u8; ArrayTileHeader::SIZE];
    reader.read_exact(&mut buf)?;
    let hdr: ArrayTileHeader = pod_read_unaligned(&buf);
    if hdr.is_valid() {
        Ok(hdr)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid ArrayTileHeader",
        ))
    }
}

// ---------------------------------------------------------------------------
// Consent Evaluation Engine (purpose=research)
// ---------------------------------------------------------------------------

struct ConsentGovernanceState {
    subject_research_status: HashMap<[u8; 16], bool>,
    marks_withdrawn: HashSet<(u32, u32, u32)>, // (tile_type, tile_id, record_index)
    edge_research_allowed: Option<bool>,
}

fn build_consent_state<R: Read + Seek>(
    reader: &mut R,
    floors: &[ParsedFloorMeta],
) -> io::Result<ConsentGovernanceState> {
    let mut subject_research_status = HashMap::new();
    let mut marks_withdrawn = HashSet::new();
    let mut edge_research_allowed = None;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    for f in floors {
        if f.type_id == TileType::CONSENT {
            if let Ok(bytes) = read_exact_tile_bytes(reader, f.offset, f.length) {
                if bytes.len() >= HYPER_TILE_HEADER_SIZE {
                    let payload = &bytes[HYPER_TILE_HEADER_SIZE..];
                    let rec_size = std::mem::size_of::<ConsentRecord>();
                    let count = payload.len() / rec_size;
                    let records: &[ConsentRecord] = cast_slice(&payload[..count * rec_size]);
                    for r in records {
                        if r.purpose == ConsentPurpose::Research as u32 {
                            let is_active =
                                r.withdrawn_at == 0 && (r.expires_at == 0 || now <= r.expires_at);
                            subject_research_status.insert(r.subject_id, is_active);
                        }
                    }
                }
            }
        } else if f.type_id == TileType::MARKS {
            if let Ok(bytes) = read_exact_tile_bytes(reader, f.offset, f.length) {
                if bytes.len() >= HYPER_TILE_HEADER_SIZE {
                    let payload = &bytes[HYPER_TILE_HEADER_SIZE..];
                    let rec_size = std::mem::size_of::<MarkRecord>();
                    let count = payload.len() / rec_size;
                    let records: &[MarkRecord] = cast_slice(&payload[..count * rec_size]);
                    for m in records {
                        if m.mark_kind == MARK_CONSENT_WITHDRAWN {
                            if m.set_or_clear == MARK_SET {
                                marks_withdrawn.insert((
                                    m.target_tile_type,
                                    m.target_tile_id as u32,
                                    m.record_index,
                                ));
                            } else {
                                marks_withdrawn.remove(&(
                                    m.target_tile_type,
                                    m.target_tile_id as u32,
                                    m.record_index,
                                ));
                            }
                        }
                    }
                }
            }
        } else if f.type_id == 82 {
            // EdgeConsentRecord
            if let Ok(bytes) = read_exact_tile_bytes(reader, f.offset, f.length) {
                if bytes.len() >= HYPER_TILE_HEADER_SIZE {
                    let payload = &bytes[HYPER_TILE_HEADER_SIZE..];
                    if payload.len() >= std::mem::size_of::<EdgeConsentRecord>() {
                        let rec: EdgeConsentRecord = pod_read_unaligned(payload);
                        let allowed = (rec.allowed_purposes & PURPOSE_RESEARCH_ANALYTICS) != 0;
                        edge_research_allowed = Some(allowed);
                    }
                }
            }
        }
    }

    Ok(ConsentGovernanceState {
        subject_research_status,
        marks_withdrawn,
        edge_research_allowed,
    })
}

fn evaluate_consent_summary<R: Read + Seek>(
    reader: &mut R,
    floors: &[ParsedFloorMeta],
) -> io::Result<TourConsentSummary> {
    let state = build_consent_state(reader, floors)?;

    if state.subject_research_status.is_empty()
        && state.marks_withdrawn.is_empty()
        && state.edge_research_allowed.is_none()
    {
        return Ok(TourConsentSummary {
            total_consent_records: 0,
            active_research_count: 0,
            withdrawn_count: 0,
            status: "open / unrestricted (no consent records)".to_string(),
        });
    }

    let mut active = 0;
    let mut withdrawn = 0;

    for &is_act in state.subject_research_status.values() {
        if is_act {
            active += 1;
        } else {
            withdrawn += 1;
        }
    }

    withdrawn += state.marks_withdrawn.len();

    let total = active + withdrawn;
    let status_str = if withdrawn > 0 {
        format!(
            "{} active, {} withdrawn (purpose=research)",
            active, withdrawn
        )
    } else {
        format!("{} active (purpose=research)", active)
    };

    Ok(TourConsentSummary {
        total_consent_records: total,
        active_research_count: active,
        withdrawn_count: withdrawn,
        status: status_str,
    })
}

fn is_record_withheld(
    state: &ConsentGovernanceState,
    floor: &ParsedFloorMeta,
    record_idx: u32,
    record_value: &serde_json::Value,
) -> bool {
    // 1. Edge profile consent
    if let Some(allowed) = state.edge_research_allowed {
        if !allowed {
            return true;
        }
    }

    // 2. Tile 19 Mark check
    if state
        .marks_withdrawn
        .contains(&(floor.type_id, floor.tile_index, record_idx))
        || state.marks_withdrawn.contains(&(
            floor.type_id,
            floor.tile_index,
            MARK_TARGET_ALL_RECORDS,
        ))
    {
        return true;
    }

    // 3. Subject-level ConsentRecord check
    if !state.subject_research_status.is_empty() {
        // Look for subject_id in record JSON
        if let Some(sub_val) = record_value.get("subject_id") {
            if let Some(hex_str) = sub_val.as_str() {
                if let Ok(uuid_bytes) = hex::decode(hex_str) {
                    if uuid_bytes.len() == 16 {
                        let mut arr = [0u8; 16];
                        arr.copy_from_slice(&uuid_bytes);
                        if let Some(&is_active) = state.subject_research_status.get(&arr) {
                            if !is_active {
                                return true;
                            }
                        } else {
                            // Consent governance active for archive, but subject has no research grant -> withheld
                            return true;
                        }
                    }
                }
            }
        }
    }

    false
}

// ---------------------------------------------------------------------------
// Floor Detail (`--floor N`)
// ---------------------------------------------------------------------------

fn inspect_floor_detail<R: Read + Seek>(
    reader: &mut R,
    floor_idx: u32,
    ctx: &ParsedArchiveContext,
) -> Result<TourFloorDetail, Box<dyn std::error::Error>> {
    // N4: address by canonical floor index (position), not the raw
    // tile_index recorded in the footer, which can collide across floors.
    let floor = ctx
        .floors
        .iter()
        .find(|f| f.floor_index == floor_idx)
        .ok_or_else(|| {
            format!(
                "Floor index {} out of range (floors 0..{}; run `s3a-cli tour <file>` to list them)",
                floor_idx,
                ctx.floors.len().saturating_sub(1)
            )
        })?;

    let schema = ctx
        .schema_map
        .get(&floor.type_id)
        .cloned()
        .unwrap_or_else(|| standard_schema_for_type(floor.type_id));

    let tile_bytes = read_exact_tile_bytes(reader, floor.offset, floor.length)?;
    let payload = if tile_bytes.len() > HYPER_TILE_HEADER_SIZE {
        &tile_bytes[HYPER_TILE_HEADER_SIZE..]
    } else {
        &[]
    };

    // B5: verify this tile's payload checksums (header CRC, data CRC, and
    // the payload SHA-256 against the footer index on v2 sealed archives).
    // This reuses the bytes already read for the detail view, so the lobby
    // read bound is unaffected.
    let expected_hash = if ctx.footer_version == "v2" {
        Some(floor.payload_hash)
    } else {
        None
    };
    let (payload_ok, payload_check) = check_tile_payload(&tile_bytes, expected_hash);

    let consent_state = build_consent_state(reader, &ctx.floors)?;

    // B3: layout-v1 Scholar POD tiles decode through the pack decoder (see
    // `is_scholar_pod_floor`); v2 blob tiles use the embedded schema.
    let is_scholar_pod = is_scholar_pod_floor(floor.type_id, floor.type_version);
    let scholar_size = if is_scholar_pod {
        scholar_record_size(floor.type_id)
    } else {
        None
    };
    let mut fields = if is_scholar_pod {
        scholar_record_fields(floor.type_id).unwrap_or_default()
    } else {
        Vec::with_capacity(schema.fields.len())
    };
    if !is_scholar_pod {
        for f in &schema.fields {
            fields.push(TourFieldInfo {
                field_id: f.field_id,
                name: f.name.clone(),
                field_type: type_enum_to_name(f.field_type).to_string(),
                byte_offset: f.byte_offset,
                byte_len: f.byte_len,
                null_allowed: f.null_allowed,
            });
        }
    }

    let mut records = Vec::new();
    let mut withheld_count = 0;
    let mut array_stats = None;

    if floor.type_id == TileType::ARRAY {
        if let Ok(view) = ArrayTileView::parse(payload) {
            let stats = calculate_array_stats(&view);
            array_stats = Some(stats);
        }
    } else if let Some(scholar_size) = scholar_size {
        // B3: decode Scholar POD records into named fields. Use the pack
        // struct size, not the schema `record_size`: the schema describes
        // the current (v2 blob) layout for types 12/13, not the v1 POD
        // records on this tile.
        let count = (payload.len() / scholar_size).min(floor.record_count as usize);
        for i in 0..count {
            let start = i * scholar_size;
            let slice = &payload[start..start + scholar_size];
            let obj = match decode_scholar_record(floor.type_id, slice) {
                Some(obj) => obj,
                None => continue,
            };

            if is_record_withheld(&consent_state, floor, i as u32, &obj) {
                withheld_count += 1;
            } else if records.len() < 10 {
                // Keep first 10 allowed records
                records.push(obj);
            }
        }
    } else if schema.record_size > 0 && !schema.fields.is_empty() {
        let rec_size = schema.record_size as usize;
        let count = (payload.len() / rec_size).min(floor.record_count as usize);

        for i in 0..count {
            let start = i * rec_size;
            let slice = &payload[start..start + rec_size];
            let obj = unpack_record_json(&schema.fields, slice);

            if is_record_withheld(&consent_state, floor, i as u32, &obj) {
                withheld_count += 1;
            } else if records.len() < 10 {
                // Keep first 10 allowed records
                records.push(obj);
            }
        }
    } else {
        // Known fallback unpacking if record_size == 0
        unpack_fallback_records(
            floor,
            payload,
            &consent_state,
            &mut records,
            &mut withheld_count,
        );
    }

    Ok(TourFloorDetail {
        index: floor.floor_index,
        type_id: floor.type_id,
        type_name: schema.name.clone(),
        version: floor.type_version,
        record_count: floor.record_count,
        size_bytes: floor.length,
        min_key: floor.min_key,
        max_key: floor.max_key,
        key_range: key_range_display(floor.min_key, floor.max_key, floor.record_count),
        payload_hash: payload_hash_display(&floor.payload_hash),
        payload_ok,
        payload_check,
        fields,
        records,
        withheld_count,
        record_note: scholar_record_note(floor.type_id),
        array_stats,
    })
}

fn calculate_array_stats(view: &ArrayTileView) -> TourArrayStats {
    let dtype = ArrayDataType::from_u16(view.header.dtype);
    let mut total_elements = view.header.total_elements();
    let mut min = f64::MAX;
    let mut max = f64::MIN;
    let mut sum = 0.0;

    match dtype {
        Some(ArrayDataType::Float32) => {
            if let Ok(slice) = view.read_elements::<f32>() {
                total_elements = slice.len();
                for &v in slice {
                    let vf = v as f64;
                    if vf < min {
                        min = vf;
                    }
                    if vf > max {
                        max = vf;
                    }
                    sum += vf;
                }
            }
        }
        Some(ArrayDataType::Float64) => {
            if let Ok(slice) = view.read_elements::<f64>() {
                total_elements = slice.len();
                for &v in slice {
                    if v < min {
                        min = v;
                    }
                    if v > max {
                        max = v;
                    }
                    sum += v;
                }
            }
        }
        Some(ArrayDataType::Int32) => {
            if let Ok(slice) = view.read_elements::<i32>() {
                total_elements = slice.len();
                for &v in slice {
                    let vf = v as f64;
                    if vf < min {
                        min = vf;
                    }
                    if vf > max {
                        max = vf;
                    }
                    sum += vf;
                }
            }
        }
        Some(ArrayDataType::UInt32) => {
            if let Ok(slice) = view.read_elements::<u32>() {
                total_elements = slice.len();
                for &v in slice {
                    let vf = v as f64;
                    if vf < min {
                        min = vf;
                    }
                    if vf > max {
                        max = vf;
                    }
                    sum += vf;
                }
            }
        }
        _ => {
            // General byte elements
            for &b in view.data {
                let vf = b as f64;
                if vf < min {
                    min = vf;
                }
                if vf > max {
                    max = vf;
                }
                sum += vf;
            }
        }
    }

    if min == f64::MAX {
        min = 0.0;
    }
    if max == f64::MIN {
        max = 0.0;
    }
    let mean = if total_elements > 0 {
        sum / total_elements as f64
    } else {
        0.0
    };

    TourArrayStats {
        total_elements,
        min,
        max,
        mean,
        sum,
    }
}

fn unpack_record_json(fields: &[SchemaFieldDefinition], slice: &[u8]) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for f in fields {
        let offset = f.byte_offset as usize;
        let len = f.byte_len as usize;
        if offset + len <= slice.len() {
            let field_slice = &slice[offset..offset + len];
            let val = match f.field_type {
                1 => serde_json::json!(field_slice[0]),
                2 => {
                    let bytes: [u8; 2] = field_slice.try_into().unwrap_or([0, 0]);
                    serde_json::json!(u16::from_le_bytes(bytes))
                }
                3 => {
                    let bytes: [u8; 4] = field_slice.try_into().unwrap_or([0; 4]);
                    serde_json::json!(u32::from_le_bytes(bytes))
                }
                4 => {
                    let bytes: [u8; 8] = field_slice.try_into().unwrap_or([0; 8]);
                    serde_json::json!(u64::from_le_bytes(bytes))
                }
                5 => serde_json::json!(field_slice[0] as i8),
                6 => {
                    let bytes: [u8; 2] = field_slice.try_into().unwrap_or([0, 0]);
                    serde_json::json!(i16::from_le_bytes(bytes))
                }
                7 => {
                    let bytes: [u8; 4] = field_slice.try_into().unwrap_or([0; 4]);
                    serde_json::json!(i32::from_le_bytes(bytes))
                }
                8 => {
                    let bytes: [u8; 8] = field_slice.try_into().unwrap_or([0; 8]);
                    serde_json::json!(i64::from_le_bytes(bytes))
                }
                9 => {
                    let bytes: [u8; 4] = field_slice.try_into().unwrap_or([0; 4]);
                    serde_json::json!(f32::from_le_bytes(bytes))
                }
                10 => {
                    let bytes: [u8; 8] = field_slice.try_into().unwrap_or([0; 8]);
                    serde_json::json!(f64::from_le_bytes(bytes))
                }
                11 => {
                    let nul = field_slice
                        .iter()
                        .position(|&b| b == 0)
                        .unwrap_or(field_slice.len());
                    let s = std::str::from_utf8(&field_slice[..nul]).unwrap_or("");
                    serde_json::json!(s)
                }
                12 => serde_json::json!(hex::encode(field_slice)),
                13 => serde_json::json!(hex::encode(field_slice)),
                _ => serde_json::json!(hex::encode(field_slice)),
            };
            map.insert(f.name.clone(), val);
        }
    }
    serde_json::Value::Object(map)
}

fn unpack_fallback_records(
    floor: &ParsedFloorMeta,
    payload: &[u8],
    consent_state: &ConsentGovernanceState,
    records: &mut Vec<serde_json::Value>,
    withheld_count: &mut usize,
) {
    if floor.type_id == 80 {
        // CompactEdgeSensorRecord
        let rec_size = std::mem::size_of::<CompactEdgeSensorRecord>();
        let count = payload.len() / rec_size;
        let slice: &[CompactEdgeSensorRecord] = cast_slice(&payload[..count * rec_size]);
        for (i, r) in slice.iter().enumerate() {
            let obj = serde_json::json!({
                "delta_ts_ms": r.delta_ts_ms,
                "sensor_id": r.sensor_id,
                "metric_id": r.metric_id,
                "quantized_val": r.quantized_val,
                "quality_flags": r.quality_flags,
            });
            if is_record_withheld(consent_state, floor, i as u32, &obj) {
                *withheld_count += 1;
            } else if records.len() < 10 {
                records.push(obj);
            }
        }
    } else if floor.type_id == 81 {
        // EquipmentEventRecord
        let rec_size = std::mem::size_of::<EquipmentEventRecord>();
        let count = payload.len() / rec_size;
        let slice: &[EquipmentEventRecord] = cast_slice(&payload[..count * rec_size]);
        for (i, r) in slice.iter().enumerate() {
            let obj = serde_json::json!({
                "timestamp_sec": r.timestamp_sec,
                "event_type": r.event_type,
                "severity": r.severity,
                "subsystem_id": r.subsystem_id,
                "event_code": r.event_code,
                "operator_id": hex::encode(r.operator_id),
                "param1": r.param1,
            });
            if is_record_withheld(consent_state, floor, i as u32, &obj) {
                *withheld_count += 1;
            } else if records.len() < 10 {
                records.push(obj);
            }
        }
    } else if floor.type_id == TileType::MANIFEST {
        if let Ok(text) = String::from_utf8(payload.to_vec()) {
            let obj = serde_json::json!({
                "manifest": text.trim(),
            });
            records.push(obj);
        }
    } else if floor.type_id == TileType::SPECIFICATION {
        let obj = serde_json::json!({
            "specification": "Embedded compressed S3A Specification (Markdown)",
            "size_bytes": payload.len(),
        });
        records.push(obj);
    }
}

// ---------------------------------------------------------------------------
// Array Slicing (`--slice axis=range,...`)
// ---------------------------------------------------------------------------

fn execute_array_slice<R: Read + Seek>(
    reader: &mut R,
    slice_spec: &str,
    ctx: &ParsedArchiveContext,
) -> Result<TourSliceResult, Box<dyn std::error::Error>> {
    // Collect all Array floors
    let array_floors: Vec<&ParsedFloorMeta> = ctx
        .floors
        .iter()
        .filter(|f| f.type_id == TileType::ARRAY)
        .collect();

    if array_floors.is_empty() {
        return Err("No Array floors found in archive".into());
    }

    // Read header of first array floor to get ndim and axis names
    let first_hdr = read_array_header(reader, array_floors[0].offset)?;
    let ndim = first_hdr.ndim as usize;
    let axis_names_str = first_hdr.axis_names_as_str().to_string();
    let axis_names: Vec<&str> = axis_names_str.split(',').map(|s| s.trim()).collect();

    let ranges = parse_slice_ranges(slice_spec, ndim, &axis_names)?;

    let mut chunks_evaluated = 0;
    let mut chunks_covered = 0;
    let mut chunks_skipped = 0;
    let mut elements = Vec::new();

    for f in array_floors {
        chunks_evaluated += 1;
        let hdr = read_array_header(reader, f.offset)?;
        let chunk_ndim = hdr.ndim as usize;
        if chunk_ndim != ndim {
            chunks_skipped += 1;
            continue;
        }

        // Bounding box overlap test
        let mut overlaps = true;
        for (a, &(req_start, req_end)) in ranges.iter().enumerate().take(ndim) {
            let chunk_start = hdr.chunk_coords[a];
            let chunk_end = chunk_start + hdr.shape[a];
            if req_end <= chunk_start || req_start >= chunk_end {
                overlaps = false;
                break;
            }
        }

        if !overlaps {
            chunks_skipped += 1;
            continue;
        }

        chunks_covered += 1;

        // ONLY read the chunk payload if it is covered!
        let tile_bytes = read_exact_tile_bytes(reader, f.offset, f.length)?;
        if tile_bytes.len() <= HYPER_TILE_HEADER_SIZE {
            continue;
        }
        let payload = &tile_bytes[HYPER_TILE_HEADER_SIZE..];
        let view = match ArrayTileView::parse(payload) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Extract elements overlapping the slice range
        let dtype = ArrayDataType::from_u16(view.header.dtype);
        match dtype {
            Some(ArrayDataType::Float32) => {
                if let Ok(slice) = view.read_elements::<f32>() {
                    extract_chunk_elements_32(&hdr, &ranges, slice, |coords, val| {
                        elements.push(serde_json::json!({
                            "coords": coords,
                            "value": val,
                        }));
                    });
                }
            }
            Some(ArrayDataType::Float64) => {
                if let Ok(slice) = view.read_elements::<f64>() {
                    extract_chunk_elements_64(&hdr, &ranges, slice, |coords, val| {
                        elements.push(serde_json::json!({
                            "coords": coords,
                            "value": val,
                        }));
                    });
                }
            }
            Some(ArrayDataType::Int32) => {
                if let Ok(slice) = view.read_elements::<i32>() {
                    extract_chunk_elements_i32(&hdr, &ranges, slice, |coords, val| {
                        elements.push(serde_json::json!({
                            "coords": coords,
                            "value": val,
                        }));
                    });
                }
            }
            _ => {
                // Byte fallback
                extract_chunk_elements_u8(&hdr, &ranges, view.data, |coords, val| {
                    elements.push(serde_json::json!({
                        "coords": coords,
                        "value": val,
                    }));
                });
            }
        }
    }

    Ok(TourSliceResult {
        slice_spec: slice_spec.to_string(),
        ndim,
        chunks_evaluated,
        chunks_covered,
        chunks_skipped,
        total_elements: elements.len(),
        elements,
    })
}

fn parse_slice_ranges(
    spec: &str,
    ndim: usize,
    axis_names: &[&str],
) -> Result<Vec<(u64, u64)>, Box<dyn std::error::Error>> {
    let mut ranges = vec![(0u64, u64::MAX); ndim];
    let parts: Vec<&str> = spec.split(',').map(|s| s.trim()).collect();

    for (pos, part) in parts.iter().enumerate() {
        let (axis_idx, range_str) = if let Some((axis_id, r_str)) = part.split_once('=') {
            let axis_id = axis_id.trim();
            let idx = if let Ok(num) = axis_id.parse::<usize>() {
                num
            } else if let Some(found) = axis_names
                .iter()
                .position(|&n| n.eq_ignore_ascii_case(axis_id))
            {
                found
            } else {
                return Err(format!("Unknown axis name '{}'", axis_id).into());
            };
            (idx, r_str.trim())
        } else {
            (pos, *part)
        };

        if axis_idx >= ndim {
            return Err(format!("Axis index {} exceeds array ndim {}", axis_idx, ndim).into());
        }

        let parsed_range = if let Some((s_str, e_str)) = range_str.split_once("..=") {
            let s = s_str.trim().parse::<u64>()?;
            let e = e_str.trim().parse::<u64>()? + 1;
            (s, e)
        } else if let Some((s_str, e_str)) = range_str.split_once("..") {
            let s = s_str.trim().parse::<u64>()?;
            let e = e_str.trim().parse::<u64>()?;
            (s, e)
        } else if let Some((s_str, e_str)) = range_str.split_once(':') {
            let s = s_str.trim().parse::<u64>()?;
            let e = e_str.trim().parse::<u64>()?;
            (s, e)
        } else {
            let point = range_str.trim().parse::<u64>()?;
            (point, point + 1)
        };

        ranges[axis_idx] = parsed_range;
    }

    Ok(ranges)
}

fn extract_chunk_elements_i32<F>(
    hdr: &ArrayTileHeader,
    ranges: &[(u64, u64)],
    data: &[i32],
    mut cb: F,
) where
    F: FnMut(Vec<u64>, i32),
{
    let ndim = hdr.ndim as usize;
    if ndim == 2 {
        let (r_start, r_end) = ranges[0];
        let (c_start, c_end) = ranges[1];

        let chunk_r = hdr.chunk_coords[0];
        let chunk_c = hdr.chunk_coords[1];
        let shape_r = hdr.shape[0];
        let shape_c = hdr.shape[1];

        let min_r = chunk_r.max(r_start);
        let max_r = (chunk_r + shape_r).min(r_end);
        let min_c = chunk_c.max(c_start);
        let max_c = (chunk_c + shape_c).min(c_end);

        for r in min_r..max_r {
            for c in min_c..max_c {
                let local_r = (r - chunk_r) as usize;
                let local_c = (c - chunk_c) as usize;
                let idx = local_r * (shape_c as usize) + local_c;
                if idx < data.len() {
                    cb(vec![r, c], data[idx]);
                }
            }
        }
    } else {
        // 1D or general
        let (start, end) = ranges[0];
        let chunk_0 = hdr.chunk_coords[0];
        let shape_0 = hdr.shape[0];
        let min_0 = chunk_0.max(start);
        let max_0 = (chunk_0 + shape_0).min(end);
        for x in min_0..max_0 {
            let idx = (x - chunk_0) as usize;
            if idx < data.len() {
                cb(vec![x], data[idx]);
            }
        }
    }
}

fn extract_chunk_elements_32<F>(
    hdr: &ArrayTileHeader,
    ranges: &[(u64, u64)],
    data: &[f32],
    mut cb: F,
) where
    F: FnMut(Vec<u64>, f32),
{
    let ndim = hdr.ndim as usize;
    if ndim == 2 {
        let (r_start, r_end) = ranges[0];
        let (c_start, c_end) = ranges[1];
        let chunk_r = hdr.chunk_coords[0];
        let chunk_c = hdr.chunk_coords[1];
        let shape_r = hdr.shape[0];
        let shape_c = hdr.shape[1];
        let min_r = chunk_r.max(r_start);
        let max_r = (chunk_r + shape_r).min(r_end);
        let min_c = chunk_c.max(c_start);
        let max_c = (chunk_c + shape_c).min(c_end);

        for r in min_r..max_r {
            for c in min_c..max_c {
                let local_r = (r - chunk_r) as usize;
                let local_c = (c - chunk_c) as usize;
                let idx = local_r * (shape_c as usize) + local_c;
                if idx < data.len() {
                    cb(vec![r, c], data[idx]);
                }
            }
        }
    }
}

fn extract_chunk_elements_64<F>(
    hdr: &ArrayTileHeader,
    ranges: &[(u64, u64)],
    data: &[f64],
    mut cb: F,
) where
    F: FnMut(Vec<u64>, f64),
{
    let ndim = hdr.ndim as usize;
    if ndim == 2 {
        let (r_start, r_end) = ranges[0];
        let (c_start, c_end) = ranges[1];
        let chunk_r = hdr.chunk_coords[0];
        let chunk_c = hdr.chunk_coords[1];
        let shape_r = hdr.shape[0];
        let shape_c = hdr.shape[1];
        let min_r = chunk_r.max(r_start);
        let max_r = (chunk_r + shape_r).min(r_end);
        let min_c = chunk_c.max(c_start);
        let max_c = (chunk_c + shape_c).min(c_end);

        for r in min_r..max_r {
            for c in min_c..max_c {
                let local_r = (r - chunk_r) as usize;
                let local_c = (c - chunk_c) as usize;
                let idx = local_r * (shape_c as usize) + local_c;
                if idx < data.len() {
                    cb(vec![r, c], data[idx]);
                }
            }
        }
    }
}

fn extract_chunk_elements_u8<F>(
    hdr: &ArrayTileHeader,
    ranges: &[(u64, u64)],
    data: &[u8],
    mut cb: F,
) where
    F: FnMut(Vec<u64>, u8),
{
    let ndim = hdr.ndim as usize;
    if ndim == 2 {
        let (r_start, r_end) = ranges[0];
        let (c_start, c_end) = ranges[1];
        let chunk_r = hdr.chunk_coords[0];
        let chunk_c = hdr.chunk_coords[1];
        let shape_r = hdr.shape[0];
        let shape_c = hdr.shape[1];
        let min_r = chunk_r.max(r_start);
        let max_r = (chunk_r + shape_r).min(r_end);
        let min_c = chunk_c.max(c_start);
        let max_c = (chunk_c + shape_c).min(c_end);

        for r in min_r..max_r {
            for c in min_c..max_c {
                let local_r = (r - chunk_r) as usize;
                let local_c = (c - chunk_c) as usize;
                let idx = local_r * (shape_c as usize) + local_c;
                if idx < data.len() {
                    cb(vec![r, c], data[idx]);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Cross-Section Search (`--cross key=value`)
// ---------------------------------------------------------------------------

fn execute_cross_section<R: Read + Seek>(
    reader: &mut R,
    cross_expr: &str,
    ctx: &ParsedArchiveContext,
) -> Result<TourCrossSectionResult, Box<dyn std::error::Error>> {
    let (search_key, search_val) = if let Some((k, v)) = cross_expr.split_once('=') {
        (k.trim().to_lowercase(), v.trim())
    } else {
        ("key".to_string(), cross_expr.trim())
    };

    let consent_state = build_consent_state(reader, &ctx.floors)?;

    let mut matches = Vec::new();
    let mut withheld_count = 0;

    for floor in &ctx.floors {
        let schema = ctx
            .schema_map
            .get(&floor.type_id)
            .cloned()
            .unwrap_or_else(|| standard_schema_for_type(floor.type_id));

        // Skip array tiles for cross-section entity lookup
        if floor.type_id == TileType::ARRAY {
            continue;
        }

        let tile_bytes = read_exact_tile_bytes(reader, floor.offset, floor.length)?;
        if tile_bytes.len() <= HYPER_TILE_HEADER_SIZE {
            continue;
        }
        let payload = &tile_bytes[HYPER_TILE_HEADER_SIZE..];

        // B3: layout-v1 Scholar POD tiles decode through the pack decoder
        // (see `is_scholar_pod_floor`) so `--cross paper_id=1001` can match;
        // v2 blob tiles use the embedded schema below.
        let is_scholar_blob = is_scholar_pod_floor(floor.type_id, floor.type_version);
        let scholar_size = if is_scholar_blob {
            scholar_record_size(floor.type_id)
        } else {
            None
        };

        if let Some(scholar_size) = scholar_size {
            let count = (payload.len() / scholar_size).min(floor.record_count as usize);
            for i in 0..count {
                let start = i * scholar_size;
                let slice = &payload[start..start + scholar_size];
                let obj = match decode_scholar_record(floor.type_id, slice) {
                    Some(obj) => obj,
                    None => continue,
                };

                if record_matches_filter(
                    &obj,
                    &search_key,
                    search_val,
                    floor.min_key,
                    floor.max_key,
                ) {
                    if is_record_withheld(&consent_state, floor, i as u32, &obj) {
                        withheld_count += 1;
                    } else {
                        matches.push(TourCrossMatch {
                            floor_index: floor.floor_index,
                            record_index: i as u32,
                            type_id: floor.type_id,
                            type_name: schema.name.clone(),
                            record_data: obj,
                        });
                    }
                }
            }
        } else if schema.record_size > 0 && !schema.fields.is_empty() {
            let rec_size = schema.record_size as usize;
            let count = (payload.len() / rec_size).min(floor.record_count as usize);

            for i in 0..count {
                let start = i * rec_size;
                let slice = &payload[start..start + rec_size];
                let obj = unpack_record_json(&schema.fields, slice);

                if record_matches_filter(
                    &obj,
                    &search_key,
                    search_val,
                    floor.min_key,
                    floor.max_key,
                ) {
                    if is_record_withheld(&consent_state, floor, i as u32, &obj) {
                        withheld_count += 1;
                    } else {
                        matches.push(TourCrossMatch {
                            floor_index: floor.floor_index,
                            record_index: i as u32,
                            type_id: floor.type_id,
                            type_name: schema.name.clone(),
                            record_data: obj,
                        });
                    }
                }
            }
        }
    }

    Ok(TourCrossSectionResult {
        search_key,
        search_value: search_val.to_string(),
        match_count: matches.len(),
        withheld_count,
        matches,
    })
}

fn record_matches_filter(
    obj: &serde_json::Value,
    key_name: &str,
    val_str: &str,
    min_key: u64,
    max_key: u64,
) -> bool {
    let map = match obj.as_object() {
        Some(m) => m,
        None => return false,
    };

    if key_name == "key" {
        // Generic key: check min/max or matching timestamp/id
        if let Ok(num) = val_str.parse::<u64>() {
            if num >= min_key && num <= max_key {
                return true;
            }
        }
        for (k, v) in map {
            if (k == "timestamp" || k == "id" || k == "sensor_id" || k == "subject_id")
                && format!("{}", v).trim_matches('"') == val_str
            {
                return true;
            }
        }
        return false;
    }

    for (k, v) in map {
        if k.eq_ignore_ascii_case(key_name) {
            let formatted = format!("{}", v).trim_matches('"').to_string();
            if formatted.eq_ignore_ascii_case(val_str) {
                return true;
            }
        }
    }

    false
}

// ---------------------------------------------------------------------------
// Output Formatting: Plain Text & Terminal Printing
// ---------------------------------------------------------------------------

/// Prints a TourReport to stdout in plain text or JSON format.
pub fn print_tour_report(report: &TourReport, json_mode: bool) {
    if json_mode {
        println!("{}", serde_json::to_string_pretty(report).unwrap());
        return;
    }

    // 1. History mode
    if let Some(history) = &report.history {
        println!("============================================================");
        println!(" S3A SEAL CHAIN HISTORY");
        println!("============================================================");
        println!(
            " Seal Chain: {}",
            if report.lobby.seal_status.hash_chain_verified {
                "✓ Valid"
            } else {
                "✗ Broken"
            }
        );
        println!(" (Chain only — payload checksums are checked by `s3a-cli verify`, not here.)");
        println!(" Total Epochs:      {}", history.len());
        println!();
        println!(
            "{:<4} {:<20} {:<34} {:<34} Anchor Status",
            "Gen", "Sealed Time", "Prev Seal Hash", "Seal Hash"
        );
        println!(
            "{:<4} {:<20} {:<34} {:<34} -------------",
            "---",
            "-------------------",
            "--------------------------------",
            "--------------------------------"
        );
        for h in history {
            println!(
                "{:<4} {:<20} {:<34} {:<34} {}",
                h.generation,
                h.time_formatted,
                &h.prev_seal_hash[..32.min(h.prev_seal_hash.len())],
                &h.seal_hash[..32.min(h.seal_hash.len())],
                h.anchor_status
            );
        }
        return;
    }

    // 2. Specific floor detail (`--floor N`)
    if let Some(detail) = &report.floor_detail {
        println!("============================================================");
        println!(" S3A FLOOR DETAIL: Floor {}", detail.index);
        println!("============================================================");
        println!(
            " Type:        {} (Type ID {}, Layout v{})",
            detail.type_name, detail.type_id, detail.version
        );
        println!(
            " Records:     {} ({} bytes)",
            detail.record_count, detail.size_bytes
        );
        println!(
            " Keys/Range:  {}",
            detail
                .key_range
                .clone()
                .unwrap_or_else(|| "— (empty tile)".to_string())
        );
        println!(" Hash:        {}", detail.payload_hash);
        println!(
            " Payload:     {}",
            if detail.payload_check.is_empty() {
                "(not checked)".to_string()
            } else {
                detail.payload_check.clone()
            }
        );
        println!();

        if let Some(stats) = &detail.array_stats {
            println!("Array Tensor Statistics:");
            println!("  Total Elements: {}", stats.total_elements);
            println!("  Min Value:      {:.4}", stats.min);
            println!("  Max Value:      {:.4}", stats.max);
            println!("  Mean Value:     {:.4}", stats.mean);
            println!("  Sum:            {:.4}", stats.sum);
        }

        if !detail.fields.is_empty() {
            println!("Schema Fields:");
            for f in &detail.fields {
                println!(
                    "  #{:<2} {:<20} ({}, offset {}, len {})",
                    f.field_id, f.name, f.field_type, f.byte_offset, f.byte_len
                );
            }
            println!();
        }

        if !detail.records.is_empty() {
            println!("Records (First {}):", detail.records.len());
            for (idx, r) in detail.records.iter().enumerate() {
                let pretty = serde_json::to_string_pretty(r).unwrap_or_else(|_| r.to_string());
                let mut lines = pretty.lines();
                if let Some(first) = lines.next() {
                    println!("  [{}] {}", idx, first);
                    for line in lines {
                        println!("       {}", line);
                    }
                }
            }
        }

        if let Some(note) = &detail.record_note {
            println!();
            println!("Note: {}", note);
        }

        if detail.withheld_count > 0 {
            println!(
                "({} records displayed, {} withheld due to research consent policy)",
                detail.records.len(),
                detail.withheld_count
            );
        } else {
            println!("({} records displayed, 0 withheld)", detail.records.len());
        }
        return;
    }

    // 3. Array slice mode (`--slice spec`)
    if let Some(slice) = &report.slice_result {
        println!("============================================================");
        println!(" S3A ARRAY SLICE: {}", slice.slice_spec);
        println!("============================================================");
        println!(" Dimensionality:   {}D", slice.ndim);
        println!(" Chunks Evaluated: {}", slice.chunks_evaluated);
        println!(" Chunks Covered:   {}", slice.chunks_covered);
        println!(" Chunks Skipped:   {}", slice.chunks_skipped);
        println!(" Sliced Elements:  {}", slice.total_elements);
        println!();
        if !slice.elements.is_empty() {
            println!("Elements (First {}):", slice.elements.len().min(20));
            for el in slice.elements.iter().take(20) {
                println!("  {}", el);
            }
        }
        return;
    }

    // 4. Cross-section search (`--cross key=val`)
    if let Some(cross) = &report.cross_section {
        println!("============================================================");
        println!(
            " S3A CROSS-SECTION: {}={}",
            cross.search_key, cross.search_value
        );
        println!("============================================================");
        println!(" Total Matches:    {}", cross.match_count);
        println!(
            " Withheld Count:   {} (consent: purpose=research)",
            cross.withheld_count
        );
        println!();
        if !cross.matches.is_empty() {
            println!("Matches:");
            for m in &cross.matches {
                println!(
                    "  Floor {:<2} [rec {:<3}] ({}): {}",
                    m.floor_index, m.record_index, m.type_name, m.record_data
                );
            }
        }
        return;
    }

    // 5. Default mode: Lobby + Floor List
    println!("============================================================");
    println!(" S3A ARCHIVE TOUR: {}", report.lobby.file);
    println!("============================================================");
    println!(" File Version:    {}", report.lobby.file_version);
    println!(" Footer Version:  {}", report.lobby.footer_version);
    println!(" Total Floors:    {}", report.lobby.tile_count);
    println!(" Seal Status:     {}", report.lobby.seal_status.summary);
    println!(" Consent Summary: {}", report.lobby.consent_summary.status);
    println!(
        " Schema Registry: {} registered types",
        report.lobby.schema_names.len()
    );
    for (t_id, name) in &report.lobby.schema_names {
        println!("   - Type {}: {}", t_id, name);
    }
    if let Some(manifest) = &report.lobby.manifest {
        println!();
        println!("--- PLAIN-TEXT MANIFEST ---");
        println!("{}", manifest.trim());
    }
    println!();

    if let Some(floors) = &report.floors {
        println!("--- FLOOR DIRECTORY ---");
        for f in floors {
            let type_upper = f.type_name.to_uppercase();
            let keys = f.key_range.clone().unwrap_or_else(|| "—".to_string());
            if let Some(arr) = &f.array_info {
                let range = match (arr.min_val, arr.max_val) {
                    (Some(lo), Some(hi)) => format!("{}..{}", lo, hi),
                    _ => "—".to_string(),
                };
                println!(
                    "Floor {:>2}: {:<16} (type {:<2}, v{}) | {:<4} recs | {:<6} B | shape: {:?} | axes: [{}] | units: [{}] | coords: {:?} | range: [{}] | hash: {}",
                    f.index, type_upper, f.type_id, f.version, f.record_count, f.size_bytes,
                    arr.shape, arr.axis_names, arr.units, arr.chunk_coords,
                    range,
                    f.payload_hash_short
                );
            } else {
                println!(
                    "Floor {:>2}: {:<16} (type {:<2}, v{}) | {:<4} recs | {:<6} B | keys: {} | hash: {}",
                    f.index, type_upper, f.type_id, f.version, f.record_count, f.size_bytes,
                    keys, f.payload_hash_short
                );
            }
        }
    }
}

/// Text form of a tile's key range. Tiles without timestamps carry the empty-range
/// sentinel `min = u64::MAX, max = 0`, printed as `none`.
pub fn key_range(min: u64, max: u64) -> String {
    if min > max {
        "none".to_string()
    } else {
        format!("{min}..{max}")
    }
}

#[cfg(test)]
mod key_range_tests {
    #[test]
    fn empty_sentinel_prints_none() {
        assert_eq!(super::key_range(u64::MAX, 0), "none");
        assert_eq!(super::key_range(5, 9), "5..9");
        assert_eq!(super::key_range(7, 7), "7..7");
    }
}
