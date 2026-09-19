use s3a_core::{SimplexHull, MAX_HULL_DIMENSIONS};
use s3a_simd::{can_reject_tile_point, can_reject_tile_range};
use crate::MmapReader;


/// Bounding Volume Hierarchy (BVH) node representing a cluster or hierarchy of Hyper-Tiles.
#[derive(Debug, Clone)]
pub struct BvhNode {
    pub start_tile: usize,
    pub end_tile: usize, // exclusive
    pub min_timestamp: u64,
    pub max_timestamp: u64,
    pub hull: SimplexHull,
    pub children: Vec<BvhNode>,
}

impl BvhNode {
    pub fn is_leaf(&self) -> bool {
        self.children.is_empty()
    }

    pub fn tile_count(&self) -> usize {
        self.end_tile.saturating_sub(self.start_tile)
    }
}

/// Stratified Hierarchical Tree of Hulls (BVH Index) over S3A Hyper-Tiles.
/// Provides logarithmic O(log N) temporal and spatial tile rejection.
#[derive(Debug, Clone)]
pub struct HullBvh {
    pub root: Option<BvhNode>,
    pub total_tiles: usize,
    pub branch_factor: usize,
}

impl HullBvh {
    pub const DEFAULT_BRANCH_FACTOR: usize = 16;

    /// Builds a Hierarchical Tree of Hulls from an open memory-mapped S3A archive.
    pub fn build(reader: &MmapReader) -> Self {
        Self::build_with_branch_factor(reader, Self::DEFAULT_BRANCH_FACTOR)
    }

    pub fn build_with_branch_factor(reader: &MmapReader, branch_factor: usize) -> Self {
        let total_tiles = reader.tile_count() as usize;
        if total_tiles == 0 {
            return Self {
                root: None,
                total_tiles: 0,
                branch_factor,
            };
        }

        // 1. Build leaf nodes for each tile
        let mut leaves = Vec::with_capacity(total_tiles);
        for i in 0..total_tiles {
            if let Some((header, _)) = reader.get_tile(i) {
                leaves.push(BvhNode {
                    start_tile: i,
                    end_tile: i + 1,
                    min_timestamp: header.min_timestamp,
                    max_timestamp: header.max_timestamp,
                    hull: header.hull,
                    children: Vec::new(),
                });
            }
        }

        // 2. Hierarchically build internal summary levels
        let mut current_level = leaves;
        while current_level.len() > 1 {
            let mut next_level = Vec::new();
            for chunk in current_level.chunks(branch_factor) {
                let start_tile = chunk.first().unwrap().start_tile;
                let end_tile = chunk.last().unwrap().end_tile;

                let mut min_ts = u64::MAX;
                let mut max_ts = 0;
                let mut aggregated_hull = SimplexHull::empty();
                let mut max_dim = 0;

                for node in chunk {
                    if node.min_timestamp < min_ts {
                        min_ts = node.min_timestamp;
                    }
                    if node.max_timestamp > max_ts {
                        max_ts = node.max_timestamp;
                    }

                    if node.hull.dim > max_dim {
                        max_dim = node.hull.dim;
                    }

                    for d in 0..MAX_HULL_DIMENSIONS {
                        if node.hull.min_bounds[d] < aggregated_hull.min_bounds[d] {
                            aggregated_hull.min_bounds[d] = node.hull.min_bounds[d];
                        }
                        if node.hull.max_bounds[d] > aggregated_hull.max_bounds[d] {
                            aggregated_hull.max_bounds[d] = node.hull.max_bounds[d];
                        }
                    }
                }

                aggregated_hull.dim = max_dim;

                next_level.push(BvhNode {
                    start_tile,
                    end_tile,
                    min_timestamp: min_ts,
                    max_timestamp: max_ts,
                    hull: aggregated_hull,
                    children: chunk.to_vec(),
                });
            }
            current_level = next_level;
        }

        Self {
            root: current_level.into_iter().next(),
            total_tiles,
            branch_factor,
        }
    }

    /// Prunes the entire tile collection using BVH hierarchy to return only candidate tile indices
    /// intersecting the given timestamp range [query_min_ts, query_max_ts].
    pub fn sift_time(&self, query_min_ts: u64, query_max_ts: u64) -> Vec<usize> {
        let mut candidates = Vec::new();
        if let Some(root) = &self.root {
            Self::traverse_time(root, query_min_ts, query_max_ts, &mut candidates);
        }
        candidates
    }

    fn traverse_time(node: &BvhNode, min_ts: u64, max_ts: u64, out: &mut Vec<usize>) {
        // Check if node time bounds intersect query time bounds
        if node.max_timestamp < min_ts || node.min_timestamp > max_ts {
            // Entire branch rejected!
            return;
        }

        if node.is_leaf() {
            out.push(node.start_tile);
        } else {
            for child in &node.children {
                Self::traverse_time(child, min_ts, max_ts, out);
            }
        }
    }

    /// Prunes the entire tile collection using BVH hierarchy for 3D/n-D bounding box spatial queries.
    pub fn sift_spatial_range(&self, min_bounds: &[f32], max_bounds: &[f32]) -> Vec<usize> {
        let mut candidates = Vec::new();
        if let Some(root) = &self.root {
            Self::traverse_spatial_range(root, min_bounds, max_bounds, &mut candidates);
        }
        candidates
    }

    fn traverse_spatial_range(node: &BvhNode, min_bounds: &[f32], max_bounds: &[f32], out: &mut Vec<usize>) {
        if can_reject_tile_range(&node.hull, min_bounds, max_bounds) {
            // Entire branch rejected!
            return;
        }

        if node.is_leaf() {
            out.push(node.start_tile);
        } else {
            for child in &node.children {
                Self::traverse_spatial_range(child, min_bounds, max_bounds, out);
            }
        }
    }

    /// Prunes the entire tile collection using BVH hierarchy for point containment queries.
    pub fn sift_point(&self, point: &[f32]) -> Vec<usize> {
        let mut candidates = Vec::new();
        if let Some(root) = &self.root {
            Self::traverse_point(root, point, &mut candidates);
        }
        candidates
    }

    fn traverse_point(node: &BvhNode, point: &[f32], out: &mut Vec<usize>) {
        if can_reject_tile_point(&node.hull, point) {
            return;
        }

        if node.is_leaf() {
            out.push(node.start_tile);
        } else {
            for child in &node.children {
                Self::traverse_point(child, point, out);
            }
        }
    }
}
