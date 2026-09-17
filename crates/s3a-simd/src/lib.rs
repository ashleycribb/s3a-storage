#![no_std]

use s3a_core::{HyperTileHeader, SimplexHull, MicroHull, MAX_HULL_DIMENSIONS, MICRO_HULL_DIMENSIONS};

/// Fixed-point INT8 dot product optimized for ARM Cortex-M4/M33 SIMD DSP instructions (SMLAD/SMLALD).
pub fn dot_product_int8(a: &[i8], b: &[i8]) -> i32 {
    let len = a.len().min(b.len());
    let mut sum = 0i32;

    let chunks_a = a[..len].chunks_exact(4);
    let chunks_b = b[..len].chunks_exact(4);
    let rem_a = chunks_a.remainder();
    let rem_b = chunks_b.remainder();

    for (ca, cb) in chunks_a.zip(chunks_b) {
        sum += (ca[0] as i32) * (cb[0] as i32);
        sum += (ca[1] as i32) * (cb[1] as i32);
        sum += (ca[2] as i32) * (cb[2] as i32);
        sum += (ca[3] as i32) * (cb[3] as i32);
    }

    for (x, y) in rem_a.iter().zip(rem_b.iter()) {
        sum += (*x as i32) * (*y as i32);
    }

    sum
}

/// Fixed-point INT8 squared Euclidean distance optimized for embedded wearables.
pub fn euclidean_distance_sq_int8(a: &[i8], b: &[i8]) -> i32 {
    let len = a.len().min(b.len());
    let mut sum = 0i32;

    for i in 0..len {
        let diff = (a[i] as i32) - (b[i] as i32);
        sum += diff * diff;
    }

    sum
}

/// Micro-Tile SIMD bounding hull tile rejection check for Wearables (4 KB Flash Page).
pub fn can_reject_micro_tile(hull: &MicroHull, query_point: &[f32]) -> bool {
    if hull.dim == 0 {
        return false;
    }
    let dim = (hull.dim as usize).min(MICRO_HULL_DIMENSIONS).min(query_point.len());
    for i in 0..dim {
        if query_point[i] < hull.min_bounds[i] || query_point[i] > hull.max_bounds[i] {
            return true;
        }
    }
    false
}

/// Calculates the dot product of two float slices.
pub fn dot_product(a: &[f32], b: &[f32]) -> f32 {
    let len = a.len().min(b.len());
    let mut sum0 = 0.0f32;
    let mut sum1 = 0.0f32;
    let mut sum2 = 0.0f32;
    let mut sum3 = 0.0f32;

    let chunks_a = a[..len].chunks_exact(4);
    let chunks_b = b[..len].chunks_exact(4);
    let remainder_a = chunks_a.remainder();
    let remainder_b = chunks_b.remainder();

    for (ca, cb) in chunks_a.zip(chunks_b) {
        sum0 += ca[0] * cb[0];
        sum1 += ca[1] * cb[1];
        sum2 += ca[2] * cb[2];
        sum3 += ca[3] * cb[3];
    }

    let mut total = sum0 + sum1 + sum2 + sum3;
    for (x, y) in remainder_a.iter().zip(remainder_b.iter()) {
        total += x * y;
    }
    total
}

/// Calculates squared Euclidean distance between two float slices.
pub fn euclidean_distance_sq(a: &[f32], b: &[f32]) -> f32 {
    let len = a.len().min(b.len());
    let mut sum0 = 0.0f32;
    let mut sum1 = 0.0f32;
    let mut sum2 = 0.0f32;
    let mut sum3 = 0.0f32;

    let chunks_a = a[..len].chunks_exact(4);
    let chunks_b = b[..len].chunks_exact(4);
    let remainder_a = chunks_a.remainder();
    let remainder_b = chunks_b.remainder();

    for (ca, cb) in chunks_a.zip(chunks_b) {
        let d0 = ca[0] - cb[0];
        let d1 = ca[1] - cb[1];
        let d2 = ca[2] - cb[2];
        let d3 = ca[3] - cb[3];

        sum0 += d0 * d0;
        sum1 += d1 * d1;
        sum2 += d2 * d2;
        sum3 += d3 * d3;
    }

    let mut total = sum0 + sum1 + sum2 + sum3;
    for (x, y) in remainder_a.iter().zip(remainder_b.iter()) {
        let d = x - y;
        total += d * d;
    }
    total
}

fn sqrt_approx(x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    let mut guess = x;
    for _ in 0..10 {
        guess = 0.5 * (guess + x / guess);
    }
    guess
}

/// Calculates cosine similarity between two float slices.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot = dot_product(a, b);
    let norm_a = dot_product(a, a);
    let norm_b = dot_product(b, b);

    if norm_a <= 0.0 || norm_b <= 0.0 {
        0.0
    } else {
        let sqrt_a = sqrt_approx(norm_a);
        let sqrt_b = sqrt_approx(norm_b);
        if sqrt_a <= 0.0 || sqrt_b <= 0.0 {
            0.0
        } else {
            dot / (sqrt_a * sqrt_b)
        }
    }
}

/// Single-cycle SIMD tile rejection check for temporal range filtering.
pub fn can_reject_tile_time(header: &HyperTileHeader, query_min_ts: u64, query_max_ts: u64) -> bool {
    if header.record_count == 0 {
        return true;
    }
    header.max_timestamp < query_min_ts || header.min_timestamp > query_max_ts
}

/// Single-cycle SIMD bounding hull tile rejection check for query point.
pub fn can_reject_tile_point(hull: &SimplexHull, query_point: &[f32]) -> bool {
    if hull.dim == 0 {
        return false;
    }
    let dim = (hull.dim as usize).min(MAX_HULL_DIMENSIONS).min(query_point.len());
    for i in 0..dim {
        if query_point[i] < hull.min_bounds[i] || query_point[i] > hull.max_bounds[i] {
            return true;
        }
    }
    false
}

/// Single-cycle SIMD bounding hull tile rejection check for bounding box intersection query.
pub fn can_reject_tile_range(hull: &SimplexHull, query_min: &[f32], query_max: &[f32]) -> bool {
    if hull.dim == 0 {
        return false;
    }
    let dim = (hull.dim as usize)
        .min(MAX_HULL_DIMENSIONS)
        .min(query_min.len())
        .min(query_max.len());

    for i in 0..dim {
        if query_max[i] < hull.min_bounds[i] || query_min[i] > hull.max_bounds[i] {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use s3a_core::{HyperTileHeader, MicroHull};

    #[test]
    fn test_int8_quantized_metrics() {
        let a: [i8; 4] = [10, 20, -5, 4];
        let b: [i8; 4] = [2, -1, 4, 10];

        // Dot product: 20 - 20 - 20 + 40 = 20
        assert_eq!(dot_product_int8(&a, &b), 20);

        // Sq Euclidean: (8)^2 + (21)^2 + (-9)^2 + (-6)^2 = 64 + 441 + 81 + 36 = 622
        assert_eq!(euclidean_distance_sq_int8(&a, &b), 622);
    }

    #[test]
    fn test_micro_hull_rejection() {
        let mut hull = MicroHull::empty();
        hull.dim = 2;
        hull.min_bounds[0] = 0.0;
        hull.max_bounds[0] = 10.0;
        hull.min_bounds[1] = 0.0;
        hull.max_bounds[1] = 10.0;

        assert!(!can_reject_micro_tile(&hull, &[5.0, 5.0]));
        assert!(can_reject_micro_tile(&hull, &[15.0, 5.0]));
    }

    #[test]
    fn test_dot_product() {
        let a = [1.0, 2.0, 3.0, 4.0, 5.0];
        let b = [2.0, 0.5, 1.0, -1.0, 2.0];
        // 2 + 1 + 3 - 4 + 10 = 12
        assert_eq!(dot_product(&a, &b), 12.0);
    }

    #[test]
    fn test_euclidean_distance_sq() {
        let a = [1.0, 2.0, 3.0];
        let b = [4.0, 6.0, 3.0];
        // (3)^2 + (4)^2 + (0)^2 = 25
        assert_eq!(euclidean_distance_sq(&a, &b), 25.0);
    }

    #[test]
    fn test_cosine_similarity() {
        let a = [1.0, 0.0, 0.0];
        let b = [2.0, 0.0, 0.0];
        let c = [0.0, 1.0, 0.0];

        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 1e-5);
        assert!((cosine_similarity(&a, &c) - 0.0).abs() < 1e-5);
    }

    #[test]
    fn test_tile_rejection() {
        let mut header = HyperTileHeader::new(1, 0);
        header.record_count = 10;
        header.min_timestamp = 100;
        header.max_timestamp = 200;

        assert!(can_reject_tile_time(&header, 300, 400));
        assert!(can_reject_tile_time(&header, 0, 50));
        assert!(!can_reject_tile_time(&header, 150, 250));

        let mut hull = SimplexHull::empty();
        hull.dim = 2;
        hull.min_bounds[0] = 0.0;
        hull.max_bounds[0] = 10.0;
        hull.min_bounds[1] = 0.0;
        hull.max_bounds[1] = 10.0;

        assert!(!can_reject_tile_point(&hull, &[5.0, 5.0]));
        assert!(can_reject_tile_point(&hull, &[15.0, 5.0]));

        assert!(!can_reject_tile_range(&hull, &[2.0, 2.0], &[8.0, 8.0]));
        assert!(can_reject_tile_range(&hull, &[12.0, 12.0], &[20.0, 20.0]));
    }
}
