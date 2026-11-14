//! Graph shared memory engine and adjacency index for S3A.
//!
//! Rebuilds an in-memory graph index upon archive open across `TileType::GRAPH_EDGE` (Tile 87)
//! and registered pack graph tile extractors. Supports `NEIGHBORS`, `TRAVERSE`, and `PATH` queries.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::File;
use std::io;
use std::path::Path;
use std::sync::{Arc, OnceLock, RwLock};

use s3a_core::graph::GenericGraphEdge;
use s3a_core::TileType;

use crate::{MmapReader, TileWriter, HYPER_TILE_PAYLOAD_SIZE};

pub type GraphTileExtractor = Arc<dyn Fn(u32, &[u8]) -> Vec<IndexedEdge> + Send + Sync>;

static GRAPH_EXTRACTORS: OnceLock<RwLock<Vec<GraphTileExtractor>>> = OnceLock::new();

/// Registers a pack-specific graph tile extractor (e.g. for Scholar Tile 13).
pub fn register_graph_tile_extractor(extractor: GraphTileExtractor) {
    let registry = GRAPH_EXTRACTORS.get_or_init(|| RwLock::new(Vec::new()));
    registry.write().unwrap().push(extractor);
}

/// A graph edge indexed in memory.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IndexedEdge {
    pub from: String,
    pub to: String,
    pub edge_type: String,
    pub properties: String,
    pub signer: [u8; 32],
    pub timestamp: u64,
}

impl From<GenericGraphEdge> for IndexedEdge {
    fn from(e: GenericGraphEdge) -> Self {
        Self {
            from: e.from,
            to: e.to,
            edge_type: e.edge_type,
            properties: e.properties,
            signer: e.signer,
            timestamp: e.timestamp,
        }
    }
}

impl From<&GenericGraphEdge> for IndexedEdge {
    fn from(e: &GenericGraphEdge) -> Self {
        Self {
            from: e.from.clone(),
            to: e.to.clone(),
            edge_type: e.edge_type.clone(),
            properties: e.properties.clone(),
            signer: e.signer,
            timestamp: e.timestamp,
        }
    }
}

/// In-memory graph adjacency index, rebuilt on open.
#[derive(Debug, Clone, Default)]
pub struct GraphIndex {
    edges: Vec<IndexedEdge>,
    forward: HashMap<String, Vec<usize>>,
    reverse: HashMap<String, Vec<usize>>,
}

impl GraphIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of edges indexed.
    pub fn len(&self) -> usize {
        self.edges.len()
    }

    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }

    /// All edges indexed.
    pub fn all_edges(&self) -> &[IndexedEdge] {
        &self.edges
    }

    /// Adds an edge to the in-memory index.
    pub fn add_edge(&mut self, edge: IndexedEdge) {
        let idx = self.edges.len();
        self.forward.entry(edge.from.clone()).or_default().push(idx);
        self.reverse.entry(edge.to.clone()).or_default().push(idx);
        self.edges.push(edge);
    }

    /// Builds an adjacency index from an open `MmapReader`.
    pub fn build_from_reader(reader: &MmapReader) -> Self {
        let mut index = Self::new();
        let tile_count = reader.tile_count() as usize;

        for i in 0..tile_count {
            if let Some((hdr, payload)) = reader.get_tile(i) {
                if hdr.tile_type == TileType::GRAPH_EDGE {
                    let decoded = GenericGraphEdge::decode_all(payload);
                    for e in decoded {
                        index.add_edge(e.into());
                    }
                } else if let Some(registry) = GRAPH_EXTRACTORS.get() {
                    let guard = registry.read().unwrap();
                    for extractor in guard.iter() {
                        for e in extractor(hdr.tile_type, payload) {
                            index.add_edge(e);
                        }
                    }
                }
            } else if let Some((hdr, payload)) = reader.get_edge_tile(i) {
                if hdr.type_id as u32 == TileType::GRAPH_EDGE {
                    let decoded = GenericGraphEdge::decode_all(payload);
                    for e in decoded {
                        index.add_edge(e.into());
                    }
                } else if let Some(registry) = GRAPH_EXTRACTORS.get() {
                    let guard = registry.read().unwrap();
                    for extractor in guard.iter() {
                        for e in extractor(hdr.type_id as u32, payload) {
                            index.add_edge(e);
                        }
                    }
                }
            }
        }

        index
    }

    /// Builds an adjacency index from an archive file on disk.
    pub fn build_from_file<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let path_ref = path.as_ref();
        if !path_ref.exists() {
            return Ok(Self::new());
        }
        let reader = MmapReader::open(path_ref)?;
        Ok(Self::build_from_reader(&reader))
    }

    /// Returns all outgoing (and optionally connected) edges for `node`.
    pub fn neighbors(&self, node: &str) -> Vec<IndexedEdge> {
        let mut results = Vec::new();
        if let Some(indices) = self.forward.get(node) {
            for &idx in indices {
                results.push(self.edges[idx].clone());
            }
        }
        results
    }

    /// Traverses the graph from `from_node` via `via_edge_type` up to `max_depth`.
    /// Returns `(visited_nodes, traversed_edges)`.
    pub fn traverse(
        &self,
        from_node: &str,
        via_edge_type: Option<&str>,
        max_depth: usize,
    ) -> (Vec<String>, Vec<IndexedEdge>) {
        let mut visited = HashSet::new();
        let mut visited_order = Vec::new();
        let mut traversed_edges = Vec::new();

        visited.insert(from_node.to_string());
        visited_order.push(from_node.to_string());

        let mut queue = VecDeque::new();
        queue.push_back((from_node.to_string(), 0usize));

        let edge_type_filter = via_edge_type.and_then(|et| {
            let trimmed = et.trim();
            if trimmed.is_empty() || trimmed == "*" {
                None
            } else {
                Some(trimmed.to_ascii_lowercase())
            }
        });

        while let Some((curr_node, depth)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }

            if let Some(edge_indices) = self.forward.get(&curr_node) {
                for &idx in edge_indices {
                    let edge = &self.edges[idx];
                    let matches_type = match &edge_type_filter {
                        Some(filter) => edge.edge_type.to_ascii_lowercase() == *filter,
                        None => true,
                    };

                    if matches_type {
                        traversed_edges.push(edge.clone());
                        if visited.insert(edge.to.clone()) {
                            visited_order.push(edge.to.clone());
                            queue.push_back((edge.to.clone(), depth + 1));
                        }
                    }
                }
            }
        }

        (visited_order, traversed_edges)
    }

    /// Finds the shortest path from `from_node` to `to_node` using BFS.
    /// Returns `Some((nodes_in_path, edges_in_path))` or `None` if unreachable.
    pub fn find_path(
        &self,
        from_node: &str,
        to_node: &str,
        via_edge_type: Option<&str>,
    ) -> Option<(Vec<String>, Vec<IndexedEdge>)> {
        if from_node == to_node {
            return Some((vec![from_node.to_string()], Vec::new()));
        }

        let edge_type_filter = via_edge_type.and_then(|et| {
            let trimmed = et.trim();
            if trimmed.is_empty() || trimmed == "*" {
                None
            } else {
                Some(trimmed.to_ascii_lowercase())
            }
        });

        // BFS queue: (node, path_nodes, path_edges)
        let mut queue = VecDeque::new();
        let mut visited = HashSet::new();

        queue.push_back((
            from_node.to_string(),
            vec![from_node.to_string()],
            Vec::<IndexedEdge>::new(),
        ));
        visited.insert(from_node.to_string());

        while let Some((curr, path_nodes, path_edges)) = queue.pop_front() {
            if let Some(edge_indices) = self.forward.get(&curr) {
                for &idx in edge_indices {
                    let edge = &self.edges[idx];
                    let matches_type = match &edge_type_filter {
                        Some(filter) => edge.edge_type.to_ascii_lowercase() == *filter,
                        None => true,
                    };

                    if matches_type {
                        let mut next_nodes = path_nodes.clone();
                        next_nodes.push(edge.to.clone());
                        let mut next_edges = path_edges.clone();
                        next_edges.push(edge.clone());

                        if edge.to == to_node {
                            return Some((next_nodes, next_edges));
                        }

                        if visited.insert(edge.to.clone()) {
                            queue.push_back((edge.to.clone(), next_nodes, next_edges));
                        }
                    }
                }
            }
        }

        None
    }
}

/// Appends a single edge to an archive at `path`.
pub fn append_edge<P: AsRef<Path>>(path: P, edge: &GenericGraphEdge) -> io::Result<()> {
    append_edges(path, std::slice::from_ref(edge))
}

/// Appends a batch of edges to an archive at `path`, packaging them into 128 KB `TileType::GRAPH_EDGE` hyper-tiles.
pub fn append_edges<P: AsRef<Path>>(path: P, edges: &[GenericGraphEdge]) -> io::Result<()> {
    let path_ref = path.as_ref();
    let mut writer = if path_ref.exists() && File::open(path_ref)?.metadata()?.len() >= 512 {
        TileWriter::open_append(path_ref)?
    } else {
        TileWriter::create(path_ref)?
    };

    let mut current_tile_payload = Vec::new();

    for edge in edges {
        let encoded = edge.encode();
        if !current_tile_payload.is_empty()
            && current_tile_payload.len() + encoded.len() > HYPER_TILE_PAYLOAD_SIZE
        {
            writer.write_hyper_tile::<u8>(
                TileType::GRAPH_EDGE,
                &current_tile_payload,
                None,
                None,
            )?;
            current_tile_payload.clear();
        }
        current_tile_payload.extend_from_slice(&encoded);
    }

    if !current_tile_payload.is_empty() {
        writer.write_hyper_tile::<u8>(TileType::GRAPH_EDGE, &current_tile_payload, None, None)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_graph_index_traverse_and_path() {
        let mut index = GraphIndex::new();
        // A -> B -> C -> D
        // A -> E
        index.add_edge(IndexedEdge {
            from: "A".into(),
            to: "B".into(),
            edge_type: "child".into(),
            properties: "".into(),
            signer: [0u8; 32],
            timestamp: 100,
        });
        index.add_edge(IndexedEdge {
            from: "B".into(),
            to: "C".into(),
            edge_type: "child".into(),
            properties: "".into(),
            signer: [0u8; 32],
            timestamp: 101,
        });
        index.add_edge(IndexedEdge {
            from: "C".into(),
            to: "D".into(),
            edge_type: "child".into(),
            properties: "".into(),
            signer: [0u8; 32],
            timestamp: 102,
        });
        index.add_edge(IndexedEdge {
            from: "A".into(),
            to: "E".into(),
            edge_type: "cites".into(),
            properties: "".into(),
            signer: [0u8; 32],
            timestamp: 103,
        });

        // Neighbors of A
        let a_neighbors = index.neighbors("A");
        assert_eq!(a_neighbors.len(), 2);

        // Traverse from A via "child" depth 2
        let (visited, edges) = index.traverse("A", Some("child"), 2);
        assert_eq!(visited, vec!["A", "B", "C"]);
        assert_eq!(edges.len(), 2);

        // Path from A to D
        let (path_nodes, path_edges) = index.find_path("A", "D", None).expect("path found");
        assert_eq!(path_nodes, vec!["A", "B", "C", "D"]);
        assert_eq!(path_edges.len(), 3);

        // Path from E to D (should be None)
        assert!(index.find_path("E", "D", None).is_none());
    }
}
