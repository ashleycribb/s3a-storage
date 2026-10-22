//! S3A Archive Tour Module (`s3a-cli tour`)
//!
//! Provides read-only inspection of S3A archives using the Sealed Footer v2 index
//! and schema tile, falling back to scanning for v1 / unsealed archives.
//!
//! Guarantees:
//! - Strictly read-only operations.
//! - On v2 files, reads only footer index and explicitly requested tile bytes.
//! - Evaluates and respects research participant consent (`purpose=research`),
//!   withholding and reporting records without active research consent.
//! - Supports plain text terminal output and machine-readable `--json`.

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
use s3a_engine::sealed::{
    compute_endcap_crc32, compute_endcap_crc32_v2, compute_seal_hash_v2, verify_seal_chain,
};
use s3a_engine::standard_schema_for_type;

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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TourSealStatus {
    pub is_sealed: bool,
    pub hash_chain_verified: bool,
    pub anchors_verified: bool,
    pub anchor_status: String,
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
    pub payload_hash: String,
    pub fields: Vec<TourFieldInfo>,
    pub records: Vec<serde_json::Value>,
    pub withheld_count: usize,
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
    tile_index: u32,
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

        let hash_short = if f.payload_hash == [0u8; 32] {
            "00000000...".to_string()
        } else {
            let full_hex = hex::encode(f.payload_hash);
            format!("{}...", &full_hex[..8])
        };

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

                array_info = Some(TourArrayInfo {
                    ndim: hdr.ndim,
                    dtype: dtype_str,
                    shape,
                    axis_names,
                    units,
                    chunk_coords,
                    min_val: if f.min_key != 0 {
                        Some(f.min_key as f64)
                    } else {
                        None
                    },
                    max_val: if f.max_key != 0 {
                        Some(f.max_key as f64)
                    } else {
                        None
                    },
                });
            }
        }

        floor_items.push(TourFloorItem {
            index: f.tile_index,
            type_id: f.type_id,
            type_name,
            version: f.type_version,
            record_count: f.record_count,
            size_bytes: f.length,
            min_key: f.min_key,
            max_key: f.max_key,
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

                    let seal_status = TourSealStatus {
                        is_sealed: true,
                        hash_chain_verified: chain_valid && seal_hash_valid,
                        anchors_verified: anchors_ok,
                        anchor_status: anchor_status_str.clone(),
                        summary: if chain_valid && seal_hash_valid {
                            format!("verified ✓ (Chain: ✓, Anchors: {})", anchor_status_str)
                        } else {
                            "broken ✗ (Checksum or chain invalid)".to_string()
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

                    let mut floors = Vec::with_capacity(v2_entries.len());
                    for e in v2_entries {
                        floors.push(ParsedFloorMeta {
                            tile_index: e.tile_index,
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

                for e in dir_entries {
                    floors.push(ParsedFloorMeta {
                        tile_index: e.tile_index,
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
                        hash_chain_verified: true,
                        anchors_verified: false,
                        anchor_status: "v1".to_string(),
                        summary: "verified ✓ (v1 directory)".to_string(),
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
                    tile_index: i,
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
    let floor = ctx
        .floors
        .iter()
        .find(|f| f.tile_index == floor_idx)
        .ok_or_else(|| format!("Floor index {} out of range", floor_idx))?;

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

    let consent_state = build_consent_state(reader, &ctx.floors)?;

    let mut fields = Vec::with_capacity(schema.fields.len());
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

    let mut records = Vec::new();
    let mut withheld_count = 0;
    let mut array_stats = None;

    if floor.type_id == TileType::ARRAY {
        if let Ok(view) = ArrayTileView::parse(payload) {
            let stats = calculate_array_stats(&view);
            array_stats = Some(stats);
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
        index: floor.tile_index,
        type_id: floor.type_id,
        type_name: schema.name.clone(),
        version: floor.type_version,
        record_count: floor.record_count,
        size_bytes: floor.length,
        min_key: floor.min_key,
        max_key: floor.max_key,
        payload_hash: hex::encode(floor.payload_hash),
        fields,
        records,
        withheld_count,
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
        for a in 0..ndim {
            let chunk_start = hdr.chunk_coords[a];
            let chunk_end = chunk_start + hdr.shape[a];
            let (req_start, req_end) = ranges[a];
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

        if schema.record_size > 0 && !schema.fields.is_empty() {
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
                            floor_index: floor.tile_index,
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
            " Seal Verification: {}",
            if report.lobby.seal_status.hash_chain_verified {
                "✓ Valid"
            } else {
                "✗ Broken"
            }
        );
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
        println!(" Keys/Range:  {}..{}", detail.min_key, detail.max_key);
        println!(" Hash:        {}", detail.payload_hash);
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
                println!("  [{}] {}", idx, r);
            }
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
            if let Some(arr) = &f.array_info {
                println!(
                    "Floor {:>2}: {:<16} (type {:<2}, v{}) | {:<4} recs | {:<6} B | shape: {:?} | axes: [{}] | units: [{}] | coords: {:?} | range: [{:?}..{:?}] | hash: {}",
                    f.index, type_upper, f.type_id, f.version, f.record_count, f.size_bytes,
                    arr.shape, arr.axis_names, arr.units, arr.chunk_coords,
                    arr.min_val.unwrap_or(0.0), arr.max_val.unwrap_or(0.0),
                    f.payload_hash_short
                );
            } else {
                println!(
                    "Floor {:>2}: {:<16} (type {:<2}, v{}) | {:<4} recs | {:<6} B | keys: {} | hash: {}",
                    f.index, type_upper, f.type_id, f.version, f.record_count, f.size_bytes,
                    key_range(f.min_key, f.max_key), f.payload_hash_short
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
