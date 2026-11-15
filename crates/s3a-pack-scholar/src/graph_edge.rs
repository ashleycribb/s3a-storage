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

fn default_edge_schema_version() -> u32 {
    2
}

/// Rich knowledge graph edge representation with full provenance and append-only retraction support.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScholarGraphEdge {
    pub source_key: String,
    pub target_key: String,
    pub edge_type: String, // "cites", "cited-by", "alias-of", or registered extension
    pub provenance_actor: String,
    pub provenance_timestamp: String, // UTC ISO-8601
    #[serde(default = "default_edge_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub is_retraction: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retracted_edge_id: Option<String>,
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
            provenance_actor: provenance_actor.nfc().collect(),
            provenance_timestamp: provenance_timestamp.nfc().collect(),
            schema_version: 2,
            is_retraction: false,
            retracted_edge_id: None,
        }
    }

    /// Creates an append-only retraction edge retracting a prior edge relation.
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
            provenance_actor: provenance_actor.nfc().collect(),
            provenance_timestamp: provenance_timestamp.nfc().collect(),
            schema_version: 2,
            is_retraction: true,
            retracted_edge_id: None,
        }
    }

    /// Canonical identifier for deduplicating or targeting an edge relation.
    pub fn relation_key(&self) -> String {
        format!("{}:{}:{}", self.source_key, self.edge_type, self.target_key)
    }

    /// Appends this edge to an archive in Tile 13 (`ScholarTileType::RESEARCH_GRAPH`).
    pub fn add_to_writer(&self, writer: &mut s3a_engine::TileWriter) -> std::io::Result<()> {
        let json_bytes = serde_json::to_vec(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        let source_hash = crc32c::crc32c(self.source_key.as_bytes()) as u64;
        let target_hash = crc32c::crc32c(self.target_key.as_bytes()) as u64;

        let blob_hdr = ScholarEdgeBlobHeader {
            source_hash,
            target_hash,
            total_bytes: json_bytes.len() as u32,
            schema_version: self.schema_version,
            payload_crc32: crc32c::crc32c(&json_bytes),
            is_retraction: if self.is_retraction { 1 } else { 0 },
            _reserved: [0u64; 4],
        };

        let mut tile_payload =
            Vec::with_capacity(std::mem::size_of::<ScholarEdgeBlobHeader>() + json_bytes.len());
        tile_payload.extend_from_slice(bytemuck::bytes_of(&blob_hdr));
        tile_payload.extend_from_slice(&json_bytes);

        let mut hdr = s3a_core::HyperTileHeader::new(0, ScholarTileType::RESEARCH_GRAPH);
        hdr.record_layout_version = 2;
        hdr.payload_bytes = tile_payload.len() as u32;
        hdr.data_crc32 = crc32c::crc32c(&tile_payload);

        writer.write_raw_hyper_tile(&hdr, &tile_payload)?;
        Ok(())
    }

    /// Appends this edge to an in-memory archive byte buffer, returning the updated bytes.
    pub fn add_to_bytes(&self, archive_bytes: &[u8]) -> std::io::Result<Vec<u8>> {
        let json_bytes = serde_json::to_vec(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        let source_hash = crc32c::crc32c(self.source_key.as_bytes()) as u64;
        let target_hash = crc32c::crc32c(self.target_key.as_bytes()) as u64;

        let blob_hdr = ScholarEdgeBlobHeader {
            source_hash,
            target_hash,
            total_bytes: json_bytes.len() as u32,
            schema_version: self.schema_version,
            payload_crc32: crc32c::crc32c(&json_bytes),
            is_retraction: if self.is_retraction { 1 } else { 0 },
            _reserved: [0u64; 4],
        };

        let mut tile_payload =
            Vec::with_capacity(std::mem::size_of::<ScholarEdgeBlobHeader>() + json_bytes.len());
        tile_payload.extend_from_slice(bytemuck::bytes_of(&blob_hdr));
        tile_payload.extend_from_slice(&json_bytes);

        s3a_engine::append_tile_to_buffer(
            archive_bytes,
            ScholarTileType::RESEARCH_GRAPH,
            2,
            &tile_payload,
        )
    }

    /// Queries edges directly from raw archive bytes.
    pub fn list_from_bytes(
        archive_bytes: &[u8],
        node: &str,
        filter_type: Option<&str>,
    ) -> std::io::Result<Vec<ScholarGraphEdge>> {
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
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

        let tile_count = active_hdr.tile_count as usize;
        let mut headers: Vec<s3a_core::HyperTileHeader> = Vec::with_capacity(tile_count);
        let mut payloads: Vec<&[u8]> = Vec::with_capacity(tile_count);
        for i in 0..tile_count {
            let off = s3a_core::FILE_HEADER_SIZE + i * s3a_core::HYPER_TILE_SIZE;
            if off + s3a_core::HYPER_TILE_SIZE <= archive_bytes.len() {
                let hdr: s3a_core::HyperTileHeader = bytemuck::pod_read_unaligned(
                    &archive_bytes[off..off + s3a_core::HYPER_TILE_HEADER_SIZE],
                );
                let payload = &archive_bytes
                    [off + s3a_core::HYPER_TILE_HEADER_SIZE..off + s3a_core::HYPER_TILE_SIZE];
                headers.push(hdr);
                payloads.push(payload);
            }
        }
        let tiles = headers.iter().zip(payloads.into_iter());
        Ok(Self::list_from_tiles(tiles, node, filter_type))
    }

    /// Queries edges in the archive matching `node` (as source or target) and optional `edge_type`.
    ///
    /// Resolves append-only retractions: a retraction edge removes earlier matching edges
    /// without deleting any history on disk.
    pub fn list_from_reader(
        reader: &s3a_engine::MmapReader,
        node: &str,
        filter_type: Option<&str>,
    ) -> Vec<ScholarGraphEdge> {
        let tile_count = reader.tile_count() as usize;
        let mut tiles = Vec::with_capacity(tile_count);
        for i in 0..tile_count {
            if let Some((hdr, payload)) = reader.get_tile(i) {
                tiles.push((hdr, payload));
            }
        }
        Self::list_from_tiles(tiles, node, filter_type)
    }

    /// Resolves edges from a sequence of tiles.
    pub fn list_from_tiles<'a>(
        tiles: impl IntoIterator<Item = (&'a s3a_core::HyperTileHeader, &'a [u8])>,
        node: &str,
        filter_type: Option<&str>,
    ) -> Vec<ScholarGraphEdge> {
        let norm_node: String = node.nfc().collect();
        let norm_type: Option<String> = filter_type.map(|t| t.nfc().collect());

        let mut edges_in_order: Vec<ScholarGraphEdge> = Vec::new();
        let mut retractions: std::collections::HashSet<String> = std::collections::HashSet::new();

        for (hdr, payload) in tiles {
            if hdr.tile_type != ScholarTileType::RESEARCH_GRAPH {
                continue;
            }

            if hdr.record_layout_version == 1 {
                // Legacy 64-byte ResearchGraphEdgeRecord
                if let Ok(records) =
                    bytemuck::try_cast_slice::<u8, ResearchGraphEdgeRecord>(payload)
                {
                    for r in records {
                        if r.is_tombstone() {
                            continue;
                        }
                        let generic = r.to_generic_edge();
                        edges_in_order.push(ScholarGraphEdge {
                            source_key: generic.from,
                            target_key: generic.to,
                            edge_type: generic.edge_type,
                            provenance_actor: "legacy_system".to_string(),
                            provenance_timestamp: "1970-01-01T00:00:00Z".to_string(),
                            schema_version: 1,
                            is_retraction: false,
                            retracted_edge_id: None,
                        });
                    }
                }
            } else if hdr.record_layout_version >= 2 {
                let bsz = std::mem::size_of::<ScholarEdgeBlobHeader>();
                if payload.len() >= bsz {
                    let blob_hdr: ScholarEdgeBlobHeader =
                        bytemuck::pod_read_unaligned(&payload[..bsz]);
                    let edge_bytes = &payload[bsz..bsz + blob_hdr.total_bytes as usize];
                    if crc32c::crc32c(edge_bytes) == blob_hdr.payload_crc32 {
                        if let Ok(edge) = serde_json::from_slice::<ScholarGraphEdge>(edge_bytes) {
                            edges_in_order.push(edge);
                        }
                    }
                }
            }
        }

        // Process retractions in chronological order (retraction retracts prior matching edges)
        for edge in edges_in_order.iter().rev() {
            if edge.is_retraction || edge.edge_type == "retracts" {
                if let Some(target_id) = &edge.retracted_edge_id {
                    retractions.insert(target_id.clone());
                } else {
                    retractions.insert(edge.relation_key());
                }
            }
        }

        let mut results = Vec::new();
        for edge in edges_in_order {
            if edge.is_retraction || edge.edge_type == "retracts" {
                continue;
            }
            if retractions.contains(&edge.relation_key()) {
                continue;
            }
            if let Some(edge_id) = &edge.retracted_edge_id {
                if retractions.contains(edge_id) {
                    continue;
                }
            }

            let matches_node = edge.source_key == norm_node || edge.target_key == norm_node;
            let matches_type = match &norm_type {
                Some(t) => &edge.edge_type == t,
                None => true,
            };

            if matches_node && matches_type {
                results.push(edge);
            }
        }

        results
    }
}
