//! S3A Core Generic Graph Edge Storage Structures.
//!
//! Provides the primary Hyper-Tile structures for storing directed graph edges
//! in `TileType::GRAPH_EDGE` (Tile 87).
//!
//! Adheres strictly to the S3A schema evolution contract (Pod/Zeroable, 8-byte aligned, FLAG_ACTIVE = 0).

use bytemuck::{Pod, Zeroable};
use xxhash_rust::xxh3::xxh3_64;

pub const EDGE_FLAG_ACTIVE: u32 = 0;
pub const EDGE_FLAG_DELETED: u32 = 1 << 0;

/// Computes the canonical 64-bit XXH3 hash for node and edge type strings.
#[inline]
pub fn compute_string_hash(s: &str) -> u64 {
    xxh3_64(s.as_bytes())
}

/// EXACT 96-BYTE GRAPH EDGE RECORD HEADER (8-byte aligned, Pod/Zeroable).
///
/// Precedes variable-length strings and properties in `TileType::GRAPH_EDGE` (Tile 87).
/// Layout:
/// - 96-byte fixed header (`GraphEdgeRecordHeader`)
/// - `from_len` bytes of UTF-8 source node identifier
/// - `to_len` bytes of UTF-8 target node identifier
/// - `edge_type_len` bytes of UTF-8 edge type string
/// - `properties_len` bytes of JSON / arbitrary properties payload
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct GraphEdgeRecordHeader {
    pub from_hash: u64,        // XXH3 64-bit hash of source node (offset 0..8)
    pub to_hash: u64,          // XXH3 64-bit hash of target node (offset 8..16)
    pub edge_type_hash: u64,   // XXH3 64-bit hash of edge type string (offset 16..24)
    pub timestamp: u64,        // Milliseconds epoch timestamp (offset 24..32)
    pub signer: [u8; 32],      // 32-byte cryptographic signer / pubkey (offset 32..64)
    pub flags: u32,            // EDGE_FLAG_ACTIVE, EDGE_FLAG_DELETED (offset 64..68)
    pub from_len: u16,         // Byte length of UTF-8 `from` identifier (offset 68..70)
    pub to_len: u16,           // Byte length of UTF-8 `to` identifier (offset 70..72)
    pub edge_type_len: u16,    // Byte length of UTF-8 `edge_type` string (offset 72..74)
    pub _reserved1: u16,       // Alignment padding (offset 74..76)
    pub properties_len: u32,   // Byte length of properties payload (offset 76..80)
    pub properties_crc32: u32, // CRC32C of properties payload (offset 80..84)
    pub _reserved2: [u8; 12],  // Alignment padding to 96 bytes (offset 84..96)
}

impl Default for GraphEdgeRecordHeader {
    fn default() -> Self {
        Self {
            from_hash: 0,
            to_hash: 0,
            edge_type_hash: 0,
            timestamp: 0,
            signer: [0u8; 32],
            flags: EDGE_FLAG_ACTIVE,
            from_len: 0,
            to_len: 0,
            edge_type_len: 0,
            _reserved1: 0,
            properties_len: 0,
            properties_crc32: 0,
            _reserved2: [0u8; 12],
        }
    }
}

impl GraphEdgeRecordHeader {
    #[inline]
    pub fn is_deleted(&self) -> bool {
        (self.flags & EDGE_FLAG_DELETED) != 0
    }
}

/// High-level generic graph edge representation in memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenericGraphEdge {
    pub from: String,
    pub to: String,
    pub edge_type: String,
    pub properties: String,
    pub signer: [u8; 32],
    pub timestamp: u64,
}

impl GenericGraphEdge {
    pub fn new(
        from: impl Into<String>,
        to: impl Into<String>,
        edge_type: impl Into<String>,
        properties: impl Into<String>,
        signer: [u8; 32],
        timestamp: u64,
    ) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            edge_type: edge_type.into(),
            properties: properties.into(),
            signer,
            timestamp,
        }
    }

    /// Serializes this edge into raw binary payload bytes conforming to `TileType::GRAPH_EDGE`.
    pub fn encode(&self) -> Vec<u8> {
        let from_bytes = self.from.as_bytes();
        let to_bytes = self.to.as_bytes();
        let edge_type_bytes = self.edge_type.as_bytes();
        let prop_bytes = self.properties.as_bytes();

        let header = GraphEdgeRecordHeader {
            from_hash: compute_string_hash(&self.from),
            to_hash: compute_string_hash(&self.to),
            edge_type_hash: compute_string_hash(&self.edge_type),
            timestamp: self.timestamp,
            signer: self.signer,
            flags: EDGE_FLAG_ACTIVE,
            from_len: from_bytes.len().min(u16::MAX as usize) as u16,
            to_len: to_bytes.len().min(u16::MAX as usize) as u16,
            edge_type_len: edge_type_bytes.len().min(u16::MAX as usize) as u16,
            _reserved1: 0,
            properties_len: prop_bytes.len() as u32,
            properties_crc32: crc32c::crc32c(prop_bytes),
            _reserved2: [0u8; 12],
        };

        let hdr_bytes = bytemuck::bytes_of(&header);
        let content_len =
            from_bytes.len() + to_bytes.len() + edge_type_bytes.len() + prop_bytes.len();
        let unpadded_len = hdr_bytes.len() + content_len;
        let padding = (8 - (unpadded_len % 8)) % 8;
        let mut out = Vec::with_capacity(unpadded_len + padding);
        out.extend_from_slice(hdr_bytes);
        out.extend_from_slice(from_bytes);
        out.extend_from_slice(to_bytes);
        out.extend_from_slice(edge_type_bytes);
        out.extend_from_slice(prop_bytes);
        out.resize(unpadded_len + padding, 0);
        out
    }

    /// Decodes a single edge starting at the beginning of `bytes`, returning `(edge, total_consumed_bytes)`.
    pub fn decode(bytes: &[u8]) -> Option<(Self, usize)> {
        let hdr_size = core::mem::size_of::<GraphEdgeRecordHeader>();
        if bytes.len() < hdr_size {
            return None;
        }

        let hdr: GraphEdgeRecordHeader = bytemuck::pod_read_unaligned(&bytes[..hdr_size]);
        let from_len = hdr.from_len as usize;
        let to_len = hdr.to_len as usize;
        let edge_type_len = hdr.edge_type_len as usize;
        let prop_len = hdr.properties_len as usize;

        let unpadded_len = hdr_size + from_len + to_len + edge_type_len + prop_len;
        let padding = (8 - (unpadded_len % 8)) % 8;
        let total_len = unpadded_len + padding;
        if bytes.len() < unpadded_len {
            return None;
        }

        let mut offset = hdr_size;
        let from_str = core::str::from_utf8(&bytes[offset..offset + from_len]).ok()?;
        offset += from_len;

        let to_str = core::str::from_utf8(&bytes[offset..offset + to_len]).ok()?;
        offset += to_len;

        let edge_type_str = core::str::from_utf8(&bytes[offset..offset + edge_type_len]).ok()?;
        offset += edge_type_len;

        let prop_bytes = &bytes[offset..offset + prop_len];
        let prop_str = core::str::from_utf8(prop_bytes)
            .unwrap_or_default()
            .to_string();

        let edge = Self {
            from: from_str.to_string(),
            to: to_str.to_string(),
            edge_type: edge_type_str.to_string(),
            properties: prop_str,
            signer: hdr.signer,
            timestamp: hdr.timestamp,
        };

        Some((edge, total_len.min(bytes.len())))
    }

    /// Decodes all non-deleted edges from a tile payload slice.
    pub fn decode_all(payload: &[u8]) -> Vec<Self> {
        let mut edges = Vec::new();
        let mut offset = 0;
        let hdr_size = core::mem::size_of::<GraphEdgeRecordHeader>();

        while offset + hdr_size <= payload.len() {
            let hdr: GraphEdgeRecordHeader =
                bytemuck::pod_read_unaligned(&payload[offset..offset + hdr_size]);
            // Stop if reading uninitialized zero memory
            if hdr.from_len == 0
                && hdr.to_len == 0
                && hdr.edge_type_len == 0
                && hdr.properties_len == 0
                && hdr.from_hash == 0
            {
                break;
            }

            let from_len = hdr.from_len as usize;
            let to_len = hdr.to_len as usize;
            let edge_type_len = hdr.edge_type_len as usize;
            let prop_len = hdr.properties_len as usize;
            let unpadded_len = hdr_size + from_len + to_len + edge_type_len + prop_len;
            let padding = (8 - (unpadded_len % 8)) % 8;
            let total_len = unpadded_len + padding;

            if offset + unpadded_len > payload.len() {
                break;
            }

            if !hdr.is_deleted() {
                let slice = &payload[offset..];
                if let Some((edge, _)) = Self::decode(slice) {
                    edges.push(edge);
                }
            }

            offset += total_len;
        }

        edges
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{align_of, size_of};

    #[test]
    fn test_graph_edge_record_header_contract() {
        assert_eq!(size_of::<GraphEdgeRecordHeader>(), 96);
        assert_eq!(align_of::<GraphEdgeRecordHeader>(), 8);
        assert_eq!(EDGE_FLAG_ACTIVE, 0);

        let edge = GenericGraphEdge::new(
            "GEARBOX-001",
            "SHAFT-002",
            "has_part",
            "{\"qty\":1}",
            [7u8; 32],
            1700000000,
        );
        let encoded = edge.encode();
        let (decoded, consumed) = GenericGraphEdge::decode(&encoded).expect("failed decode");
        assert_eq!(consumed, encoded.len());
        assert_eq!(decoded, edge);
    }

    #[test]
    fn test_decode_all_multiple_edges() {
        let e1 = GenericGraphEdge::new("A", "B", "cites", "", [0u8; 32], 100);
        let e2 = GenericGraphEdge::new("B", "C", "cites", "{\"score\":0.9}", [1u8; 32], 200);

        let mut buf = e1.encode();
        buf.extend_from_slice(&e2.encode());

        let decoded = GenericGraphEdge::decode_all(&buf);
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0], e1);
        assert_eq!(decoded[1], e2);
    }
}
