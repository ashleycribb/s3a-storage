use bytemuck::{Pod, Zeroable};
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use crate::pack::ScholarTileType;
use crate::records::ResearchGraphEdgeRecord;

/// 64-byte binary blob header preceding variable-length graph edge payloads
/// in `ScholarTileType::RESEARCH_GRAPH` (Tile 13, layout version 2).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct ScholarEdgeBlobHeader {
    pub source_hash: u64,
    pub target_hash: u64,
    pub total_bytes: u32,
    pub schema_version: u32,
    pub payload_crc32: u32,
    pub is_retraction: u32,
    pub _reserved: [u64; 4],
}

impl Default for ScholarEdgeBlobHeader {
    fn default() -> Self {
        Self {
            source_hash: 0,
            target_hash: 0,
            total_bytes: 0,
            schema_version: 2,
            payload_crc32: 0,
            is_retraction: 0,
            _reserved: [0u64; 4],
        }
    }
}

/// Formats Unix seconds as `YYYY-MM-DDTHH:MM:SSZ` (UTC). Used only for values really stored.
pub(crate) fn unix_secs_to_utc_iso(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Howard Hinnant's civil_from_days
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m,
        d,
        rem / 3_600,
        (rem % 3_600) / 60,
        rem % 60
    )
}

/// Rich knowledge graph edge representation with full provenance and append-only retraction support.
///
/// Fields a stored record does not hold are `None` and are omitted from JSON; readers never
/// invent placeholder values.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScholarGraphEdge {
    pub source_key: String,
    pub target_key: String,
    pub edge_type: String, // "cites", "cited-by", "alias-of", or registered extension
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance_actor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance_timestamp: Option<String>, // UTC ISO-8601
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<u32>,
    #[serde(default)]
    pub is_retraction: bool,
    /// For a retraction: the `edge_id()` of the single edge it retracts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retracted_edge_id: Option<String>,
}

fn invalid(msg: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, msg.into())
}

fn bad_data(msg: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, msg.into())
}

impl ScholarGraphEdge {
    pub fn new(
        source_key: &str,
        target_key: &str,
        edge_type: &str,
        provenance_actor: &str,
        provenance_timestamp: &str,
    ) -> Self {
        Self {
            source_key: source_key.nfc().collect(),
            target_key: target_key.nfc().collect(),
            edge_type: edge_type.nfc().collect(),
            provenance_actor: Some(provenance_actor.nfc().collect()),
            provenance_timestamp: Some(provenance_timestamp.nfc().collect()),
            schema_version: Some(2),
            is_retraction: false,
            retracted_edge_id: None,
        }
    }

    /// Append-only retraction of the currently visible edge with this relation
    /// (`source_key`, `target_key`, `edge_type`). When appended with [`Self::add_to_bytes`] it is
    /// bound to that one edge's [`Self::edge_id`]; an edge added later is a new edge and stays
    /// visible. Appending fails if no such edge is visible.
    pub fn new_retraction(
        source_key: &str,
        target_key: &str,
        edge_type: &str,
        provenance_actor: &str,
        provenance_timestamp: &str,
    ) -> Self {
        Self {
            source_key: source_key.nfc().collect(),
            target_key: target_key.nfc().collect(),
            edge_type: edge_type.nfc().collect(),
            provenance_actor: Some(provenance_actor.nfc().collect()),
            provenance_timestamp: Some(provenance_timestamp.nfc().collect()),
            schema_version: Some(2),
            is_retraction: true,
            retracted_edge_id: None,
        }
    }

    /// Append-only retraction of the single edge whose [`Self::edge_id`] is `target_edge_id`.
    /// The relation fields are copied from the target when appended with
    /// [`Self::add_to_bytes`], which fails if the target is not in the archive.
    pub fn retract(
        target_edge_id: &str,
        provenance_actor: &str,
        provenance_timestamp: &str,
    ) -> Self {
        Self {
            source_key: String::new(),
            target_key: String::new(),
            edge_type: String::new(),
            provenance_actor: Some(provenance_actor.nfc().collect()),
            provenance_timestamp: Some(provenance_timestamp.nfc().collect()),
            schema_version: Some(2),
            is_retraction: true,
            retracted_edge_id: Some(target_edge_id.to_string()),
        }
    }

    /// Stable id of this edge: lowercase hex SHA-256 of its canonical JSON (all fields, NFC).
    /// Two edges with the same relation but different provenance have different ids.
    pub fn edge_id(&self) -> String {
        use sha2::Digest;
        let mut n = self.clone();
        n.source_key = n.source_key.nfc().collect();
        n.target_key = n.target_key.nfc().collect();
        n.edge_type = n.edge_type.nfc().collect();
        n.provenance_actor = n.provenance_actor.map(|a| a.nfc().collect());
        n.provenance_timestamp = n.provenance_timestamp.map(|t| t.nfc().collect());
        let json = serde_json::to_vec(&n).expect("edge serializes");
        hex::encode(sha2::Sha256::digest(&json))
    }

    pub fn validate(&self) -> std::io::Result<()> {
        match &self.provenance_actor {
            Some(a) if !a.trim().is_empty() => {}
            _ => return Err(invalid("provenance actor cannot be empty")),
        }
        match &self.provenance_timestamp {
            Some(ts) => crate::paper::validate_utc_iso8601(ts)?,
            None => return Err(invalid("provenance timestamp is required")),
        }
        if self.is_retraction {
            match &self.retracted_edge_id {
                Some(id) if id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()) => {}
                Some(id) => {
                    return Err(invalid(format!(
                        "retracted_edge_id '{id}' is not an edge id"
                    )))
                }
                None => {}
            }
        } else if self.retracted_edge_id.is_some() {
            return Err(invalid("retracted_edge_id is only valid on a retraction"));
        }
        if !self.is_retraction || self.retracted_edge_id.is_none() {
            if self.source_key.trim().is_empty()
                || self.target_key.trim().is_empty()
                || self.edge_type.trim().is_empty()
            {
                return Err(invalid("source_key, target_key and edge_type are required"));
            }
        }
        Ok(())
    }

    /// Canonical identifier for deduplicating or targeting an edge relation.
    pub fn relation_key(&self) -> String {
        format!("{}:{}:{}", self.source_key, self.edge_type, self.target_key)
    }

    /// Binds a retraction to exactly one existing edge of `existing` (all stored edges, in
    /// archive order). Non-retractions are returned unchanged.
    pub fn resolve_against(&self, existing: &[ScholarGraphEdge]) -> std::io::Result<Self> {
        self.validate()?;
        if !self.is_retraction {
            return Ok(self.clone());
        }
        let visible = Self::visible(existing.to_vec());
        let target = match &self.retracted_edge_id {
            Some(id) => visible.iter().find(|e| &e.edge_id() == id).ok_or_else(|| {
                invalid(format!(
                    "retraction target edge {id} is not a visible edge in the archive"
                ))
            })?,
            None => visible
                .iter()
                .rev()
                .find(|e| {
                    e.source_key == self.source_key
                        && e.target_key == self.target_key
                        && e.edge_type == self.edge_type
                })
                .ok_or_else(|| {
                    invalid(format!(
                        "no visible edge {} to retract",
                        self.relation_key()
                    ))
                })?,
        };
        let mut r = self.clone();
        r.retracted_edge_id = Some(target.edge_id());
        r.source_key = target.source_key.clone();
        r.target_key = target.target_key.clone();
        r.edge_type = target.edge_type.clone();
        Ok(r)
    }

    fn encode_payload(&self) -> std::io::Result<Vec<u8>> {
        if self.is_retraction && (self.retracted_edge_id.is_none() || self.source_key.is_empty()) {
            return Err(invalid(
                "retraction is not bound to an edge; append it with add_to_bytes or resolve_against",
            ));
        }
        let json_bytes = serde_json::to_vec(self).map_err(|e| bad_data(e.to_string()))?;
        let blob_hdr = ScholarEdgeBlobHeader {
            source_hash: crc32c::crc32c(self.source_key.as_bytes()) as u64,
            target_hash: crc32c::crc32c(self.target_key.as_bytes()) as u64,
            total_bytes: json_bytes.len() as u32,
            // Encoding version of this blob (layout v2), not a value taken from the JSON.
            schema_version: 2,
            payload_crc32: crc32c::crc32c(&json_bytes),
            is_retraction: if self.is_retraction { 1 } else { 0 },
            _reserved: [0u64; 4],
        };
        let mut tile_payload =
            Vec::with_capacity(std::mem::size_of::<ScholarEdgeBlobHeader>() + json_bytes.len());
        tile_payload.extend_from_slice(bytemuck::bytes_of(&blob_hdr));
        tile_payload.extend_from_slice(&json_bytes);
        Ok(tile_payload)
    }

    /// Appends this edge to an archive in Tile 13 (`ScholarTileType::RESEARCH_GRAPH`).
    /// A retraction must already be bound to its target (see [`Self::resolve_against`]).
    pub fn add_to_writer(&self, writer: &mut s3a_engine::TileWriter) -> std::io::Result<()> {
        self.validate()?;
        let tile_payload = self.encode_payload()?;
        let mut hdr = s3a_core::HyperTileHeader::new(0, ScholarTileType::RESEARCH_GRAPH);
        hdr.record_layout_version = 2;
        hdr.payload_bytes = tile_payload.len() as u32;
        hdr.data_crc32 = crc32c::crc32c(&tile_payload);
        writer.write_raw_hyper_tile(&hdr, &tile_payload)?;
        Ok(())
    }

    /// Appends this edge to an in-memory archive byte buffer, returning the updated bytes.
    /// A retraction is bound to exactly one existing visible edge, or this returns Err.
    pub fn add_to_bytes(&self, archive_bytes: &[u8]) -> std::io::Result<Vec<u8>> {
        self.validate()?;
        let edge = if self.is_retraction {
            let existing = Self::all_from_bytes(archive_bytes)?;
            self.resolve_against(&existing)?
        } else {
            self.clone()
        };
        let tile_payload = edge.encode_payload()?;
        s3a_engine::append_tile_to_buffer(
            archive_bytes,
            ScholarTileType::RESEARCH_GRAPH,
            2,
            &tile_payload,
        )
    }

    fn tiles_from_bytes(
        archive_bytes: &[u8],
    ) -> std::io::Result<Vec<(s3a_core::HyperTileHeader, &[u8])>> {
        if archive_bytes.len() < s3a_core::FILE_HEADER_SIZE {
            return Ok(Vec::new());
        }
        let slot_a: s3a_core::FileHeader = bytemuck::pod_read_unaligned(
            &archive_bytes[s3a_core::FILE_HEADER_SLOT_A_OFFSET
                ..s3a_core::FILE_HEADER_SLOT_A_OFFSET + s3a_core::FILE_HEADER_SLOT_SIZE],
        );
        let slot_b: s3a_core::FileHeader = bytemuck::pod_read_unaligned(
            &archive_bytes[s3a_core::FILE_HEADER_SLOT_B_OFFSET
                ..s3a_core::FILE_HEADER_SLOT_B_OFFSET + s3a_core::FILE_HEADER_SLOT_SIZE],
        );
        let (active_hdr, _) = s3a_engine::resolve_active_header(&slot_a, &slot_b)
            .map_err(|e| bad_data(e.to_string()))?;
        let mut out = Vec::with_capacity(active_hdr.tile_count as usize);
        for i in 0..active_hdr.tile_count as usize {
            let off = s3a_core::FILE_HEADER_SIZE + i * s3a_core::HYPER_TILE_SIZE;
            if off + s3a_core::HYPER_TILE_SIZE > archive_bytes.len() {
                return Err(bad_data(format!("archive truncated at tile {i}")));
            }
            let hdr: s3a_core::HyperTileHeader = bytemuck::pod_read_unaligned(
                &archive_bytes[off..off + s3a_core::HYPER_TILE_HEADER_SIZE],
            );
            out.push((
                hdr,
                &archive_bytes
                    [off + s3a_core::HYPER_TILE_HEADER_SIZE..off + s3a_core::HYPER_TILE_SIZE],
            ));
        }
        Ok(out)
    }

    /// Every stored edge and retraction in archive order (no retraction resolution).
    pub fn all_from_bytes(archive_bytes: &[u8]) -> std::io::Result<Vec<ScholarGraphEdge>> {
        let tiles = Self::tiles_from_bytes(archive_bytes)?;
        Self::decode_tiles(tiles.iter().map(|(h, p)| (h, *p)))
    }

    /// Queries edges directly from raw archive bytes.
    pub fn list_from_bytes(
        archive_bytes: &[u8],
        node: &str,
        filter_type: Option<&str>,
    ) -> std::io::Result<Vec<ScholarGraphEdge>> {
        let tiles = Self::tiles_from_bytes(archive_bytes)?;
        Self::list_from_tiles(tiles.iter().map(|(h, p)| (h, *p)), node, filter_type)
    }

    /// Every stored edge and retraction readable through `reader`, in archive order.
    pub fn all_from_reader(
        reader: &s3a_engine::MmapReader,
    ) -> std::io::Result<Vec<ScholarGraphEdge>> {
        let tile_count = reader.tile_count() as usize;
        let mut tiles = Vec::with_capacity(tile_count);
        for i in 0..tile_count {
            if let Some((hdr, payload)) = reader.get_tile(i) {
                tiles.push((hdr, payload));
            }
        }
        Self::decode_tiles(tiles)
    }

    /// Queries edges in the archive matching `node` (as source or target) and optional `edge_type`.
    ///
    /// Resolves append-only retractions: a retraction hides exactly the one edge whose id it
    /// names, without deleting any history on disk.
    pub fn list_from_reader(
        reader: &s3a_engine::MmapReader,
        node: &str,
        filter_type: Option<&str>,
    ) -> std::io::Result<Vec<ScholarGraphEdge>> {
        Ok(Self::filter(
            Self::visible(Self::all_from_reader(reader)?),
            node,
            filter_type,
        ))
    }

    /// Decodes every Tile 13 record. Any inconsistent Tile 13 is an error, never skipped.
    pub fn decode_tiles<'a>(
        tiles: impl IntoIterator<Item = (&'a s3a_core::HyperTileHeader, &'a [u8])>,
    ) -> std::io::Result<Vec<ScholarGraphEdge>> {
        let mut edges_in_order: Vec<ScholarGraphEdge> = Vec::new();
        for (hdr, payload) in tiles {
            if hdr.tile_type != ScholarTileType::RESEARCH_GRAPH {
                continue;
            }
            if s3a_engine::check_tile_header_crc(hdr).is_err() {
                return Err(bad_data(format!(
                    "Tile 13 (id {}) header CRC mismatch",
                    hdr.tile_id
                )));
            }
            let plen = hdr.payload_bytes as usize;
            if plen > payload.len() {
                return Err(bad_data(format!(
                    "Tile 13 (id {}) payload_bytes {plen} exceeds tile payload",
                    hdr.tile_id
                )));
            }
            let payload_data = &payload[..plen];
            if !(s3a_engine::is_legacy_tile(hdr) && hdr.data_crc32 == 0)
                && crc32c::crc32c(payload_data) != hdr.data_crc32
            {
                return Err(bad_data(format!(
                    "Tile 13 (id {}) payload CRC mismatch",
                    hdr.tile_id
                )));
            }

            if hdr.record_layout_version <= 1 {
                // Legacy 64-byte ResearchGraphEdgeRecord (wire version 0 is an alias for 1): no actor, no schema_version.
                let records = bytemuck::try_cast_slice::<u8, ResearchGraphEdgeRecord>(payload_data)
                    .map_err(|e| {
                        bad_data(format!("Tile 13 v1 payload is not 64-byte records: {e}"))
                    })?;
                for r in records {
                    if r.is_tombstone() {
                        continue;
                    }
                    let generic = r.to_generic_edge();
                    edges_in_order.push(ScholarGraphEdge {
                        source_key: generic.from,
                        target_key: generic.to,
                        edge_type: generic.edge_type,
                        provenance_actor: None,
                        provenance_timestamp: if r.timestamp_sec != 0 {
                            Some(unix_secs_to_utc_iso(r.timestamp_sec))
                        } else {
                            None
                        },
                        schema_version: None,
                        is_retraction: false,
                        retracted_edge_id: None,
                    });
                }
            } else if hdr.record_layout_version == 2 {
                let bsz = std::mem::size_of::<ScholarEdgeBlobHeader>();
                if payload_data.len() < bsz {
                    return Err(bad_data(format!(
                        "Tile 13 (id {}) shorter than blob header",
                        hdr.tile_id
                    )));
                }
                let blob_hdr: ScholarEdgeBlobHeader =
                    bytemuck::pod_read_unaligned(&payload_data[..bsz]);
                let total_len = blob_hdr.total_bytes as usize;
                if bsz.checked_add(total_len) != Some(payload_data.len()) {
                    return Err(bad_data(format!(
                        "Tile 13 (id {}) total_bytes {total_len} inconsistent with payload_bytes {plen}",
                        hdr.tile_id
                    )));
                }
                let edge_bytes = &payload_data[bsz..];
                if crc32c::crc32c(edge_bytes) != blob_hdr.payload_crc32 {
                    return Err(bad_data(format!(
                        "Tile 13 (id {}) edge CRC mismatch",
                        hdr.tile_id
                    )));
                }
                let edge = serde_json::from_slice::<ScholarGraphEdge>(edge_bytes).map_err(|e| {
                    bad_data(format!(
                        "Tile 13 (id {}) edge JSON invalid: {e}",
                        hdr.tile_id
                    ))
                })?;
                edges_in_order.push(edge);
            } else {
                return Err(bad_data(format!(
                    "Tile 13 (id {}) has unsupported layout version {}",
                    hdr.tile_id, hdr.record_layout_version
                )));
            }
        }
        Ok(edges_in_order)
    }

    /// Applies retractions: drops retraction records and every edge whose id was retracted.
    pub fn visible(edges_in_order: Vec<ScholarGraphEdge>) -> Vec<ScholarGraphEdge> {
        let retracted: std::collections::HashSet<String> = edges_in_order
            .iter()
            .filter(|e| e.is_retraction)
            .filter_map(|e| e.retracted_edge_id.clone())
            .collect();
        edges_in_order
            .into_iter()
            .filter(|e| !e.is_retraction && !retracted.contains(&e.edge_id()))
            .collect()
    }

    fn filter(
        edges: Vec<ScholarGraphEdge>,
        node: &str,
        filter_type: Option<&str>,
    ) -> Vec<ScholarGraphEdge> {
        let norm_node: String = node.nfc().collect();
        let norm_type: Option<String> = filter_type.map(|t| t.nfc().collect());
        edges
            .into_iter()
            .filter(|e| e.source_key == norm_node || e.target_key == norm_node)
            .filter(|e| norm_type.as_ref().is_none_or(|t| &e.edge_type == t))
            .collect()
    }

    /// Resolves edges from a sequence of tiles.
    pub fn list_from_tiles<'a>(
        tiles: impl IntoIterator<Item = (&'a s3a_core::HyperTileHeader, &'a [u8])>,
        node: &str,
        filter_type: Option<&str>,
    ) -> std::io::Result<Vec<ScholarGraphEdge>> {
        Ok(Self::filter(
            Self::visible(Self::decode_tiles(tiles)?),
            node,
            filter_type,
        ))
    }
}
