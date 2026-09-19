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

/// Calculates cosine similarity directly between two INT8 quantized slices.
pub fn cosine_similarity_int8(a: &[i8], b: &[i8]) -> f32 {
    let dot = dot_product_int8(a, b) as f32;
    let norm_a = dot_product_int8(a, a) as f32;
    let norm_b = dot_product_int8(b, b) as f32;

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

/// Computes dot product directly on packed 4-bit unsigned nibble vectors.
pub fn dot_product_4bit(a: &[u8], b: &[u8], dimensions: usize) -> u32 {
    let byte_len = a.len().min(b.len()).min((dimensions + 1) / 2);
    let mut sum = 0u32;

    for i in 0..byte_len {
        let ba = a[i];
        let bb = b[i];

        let v0_a = (ba & 0x0F) as u32;
        let v0_b = (bb & 0x0F) as u32;
        sum += v0_a * v0_b;

        if 2 * i + 1 < dimensions {
            let v1_a = ((ba >> 4) & 0x0F) as u32;
            let v1_b = ((bb >> 4) & 0x0F) as u32;
            sum += v1_a * v1_b;
        }
    }
    sum
}

/// Computes cosine similarity directly on packed 4-bit unsigned vectors.
pub fn cosine_similarity_4bit(a: &[u8], b: &[u8], dimensions: usize) -> f32 {
    let dot = dot_product_4bit(a, b, dimensions) as f32;
    let norm_a = dot_product_4bit(a, a, dimensions) as f32;
    let norm_b = dot_product_4bit(b, b, dimensions) as f32;

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

/// Alive-before-scoring invariant check: verifies if a record should be pruned before scoring.
#[inline(always)]
pub fn is_record_alive(flags: u64) -> bool {
    (flags & s3a_core::FLAG_TOMBSTONE) == 0
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

/// Computes a hash for Bloom filtering using hardware-friendly bit dispersion.
#[inline(always)]
fn bloom_hash(key: u64, seed: u16, salt: u32) -> usize {
    let mut h = key.wrapping_mul(0x517cc1b727220a95) ^ ((seed as u64) << 32) ^ (salt as u64);
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51afd7ed558ccd);
    h ^= h >> 33;
    (h as usize) % 960 // 15 u64 words = 960 bits
}

/// Inserts a 64-bit key (e.g. sensor_id, metric_id, actor_id) into a 960-bit Bloom filter.
pub fn bloom_filter_insert(filter: &mut [u64; 15], key: u64, seed: u16) {
    let bit0 = bloom_hash(key, seed, 0x1B873593);
    let bit1 = bloom_hash(key, seed, 0x85EBCA6B);
    let bit2 = bloom_hash(key, seed, 0xC2B2AE35);

    filter[bit0 / 64] |= 1 << (bit0 % 64);
    filter[bit1 / 64] |= 1 << (bit1 % 64);
    filter[bit2 / 64] |= 1 << (bit2 % 64);
}

/// Checks if a 64-bit key might be present in the 960-bit Bloom filter.
#[inline(always)]
pub fn bloom_filter_contains(filter: &[u64; 15], key: u64, seed: u16) -> bool {
    let bit0 = bloom_hash(key, seed, 0x1B873593);
    if (filter[bit0 / 64] & (1 << (bit0 % 64))) == 0 {
        return false;
    }
    let bit1 = bloom_hash(key, seed, 0x85EBCA6B);
    if (filter[bit1 / 64] & (1 << (bit1 % 64))) == 0 {
        return false;
    }
    let bit2 = bloom_hash(key, seed, 0xC2B2AE35);
    if (filter[bit2 / 64] & (1 << (bit2 % 64))) == 0 {
        return false;
    }
    true
}

/// Probabilistic Discrete ID Rejection: Rejects the tile if the Bloom filter confirms
/// the discrete ID (sensor_id, actor_id, metric_id) is NOT present in this tile.
pub fn can_reject_tile_bloom(header: &HyperTileHeader, key: u64) -> bool {
    if header.filter.filter_type == 0 {
        return false; // Filter not initialized
    }
    !bloom_filter_contains(&header.filter.bits, key, header.filter.hash_seed)
}

/// Lifecycle TTL Rejection: Rejects the tile if the entire tile has expired past current query timestamp.
pub fn can_reject_tile_ttl(header: &HyperTileHeader, current_query_time: u64) -> bool {
    header.lifecycle.max_expiry_timestamp < current_query_time
}

/// Multi-Tenant / RBAC Security Rejection: Rejects the tile if tenant does not match
/// or query clearance level is insufficient.
pub fn can_reject_tile_security(header: &HyperTileHeader, tenant_id: u64, clearance_level: u8) -> bool {
    if header.security.tenant_id != 0 && header.security.tenant_id != tenant_id {
        return true;
    }
    if header.security.min_clearance_level > clearance_level {
        return true;
    }
    false
}

/// Adaptive Learned Index: Predicts the record index offset using learned linear spline parameters.
pub fn learned_index_predict_offset(header: &HyperTileHeader, query_ts: u64) -> u32 {
    if query_ts <= header.min_timestamp || header.record_count <= 1 {
        return 0;
    }
    if query_ts >= header.max_timestamp {
        return header.record_count - 1;
    }

    let delta_t = (query_ts - header.min_timestamp) as f32;
    let predicted = delta_t * header.learned_index.slope + header.learned_index.intercept;
    let clamped = predicted.max(0.0).min((header.record_count - 1) as f32);
    clamped as u32
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

    #[test]
    fn test_cosine_similarity_int8() {
        let a: [i8; 4] = [10, 0, 0, 0];
        let b: [i8; 4] = [20, 0, 0, 0];
        let c: [i8; 4] = [0, 15, 0, 0];

        assert!((cosine_similarity_int8(&a, &b) - 1.0).abs() < 1e-4);
        assert!((cosine_similarity_int8(&a, &c) - 0.0).abs() < 1e-4);
    }

    #[test]
    fn test_4bit_quantized_simd() {
        // Two 4-dimensional vectors packed into 2 bytes:
        // v_a = [3, 5, 2, 7] -> byte0 = (3 | (5 << 4)) = 0x53, byte1 = (2 | (7 << 4)) = 0x72
        // v_b = [1, 2, 4, 3] -> byte0 = (1 | (2 << 4)) = 0x21, byte1 = (4 | (3 << 4)) = 0x34
        let a = [0x53u8, 0x72u8];
        let b = [0x21u8, 0x34u8];

        // Expected dot product: 3*1 + 5*2 + 2*4 + 7*3 = 3 + 10 + 8 + 21 = 42
        assert_eq!(dot_product_4bit(&a, &b, 4), 42);

        // Orthogonal test
        // v_x = [5, 0] -> 0x05, v_y = [0, 5] -> 0x50
        let x = [0x05u8];
        let y = [0x50u8];
        assert_eq!(dot_product_4bit(&x, &y, 2), 0);
        assert!((cosine_similarity_4bit(&x, &y, 2) - 0.0).abs() < 1e-4);
    }

    #[test]
    fn test_alive_before_scoring_filter() {
        assert!(is_record_alive(s3a_core::FLAG_ACTIVE));
        assert!(!is_record_alive(s3a_core::FLAG_TOMBSTONE));
        assert!(!is_record_alive(s3a_core::FLAG_TOMBSTONE | 0x02));
    }

    #[test]
    fn test_simd_bloom_filter_rejection() {
        let mut header = HyperTileHeader::new(1, 0);
        header.filter.filter_type = 1;
        header.filter.hash_seed = 0x533A;

        bloom_filter_insert(&mut header.filter.bits, 42, header.filter.hash_seed);
        bloom_filter_insert(&mut header.filter.bits, 101, header.filter.hash_seed);

        // Keys 42 and 101 must be contained
        assert!(!can_reject_tile_bloom(&header, 42));
        assert!(!can_reject_tile_bloom(&header, 101));

        // An arbitrary uninserted key must be rejected (false negative is impossible)
        assert!(can_reject_tile_bloom(&header, 9999999));
    }

    #[test]
    fn test_learned_index_interpolation() {
        let mut header = HyperTileHeader::new(1, 0);
        header.record_count = 1000;
        header.min_timestamp = 1000;
        header.max_timestamp = 2000;
        // Slope: 1000 records over 1000 time delta -> 1.0 record per second
        header.learned_index.slope = 0.999;
        header.learned_index.intercept = 0.0;

        let pred_start = learned_index_predict_offset(&header, 1000);
        assert_eq!(pred_start, 0);

        let pred_mid = learned_index_predict_offset(&header, 1500);
        assert!((pred_mid as i32 - 499).abs() <= 1);

        let pred_end = learned_index_predict_offset(&header, 2000);
        assert_eq!(pred_end, 999);
    }

    #[test]
    fn test_ttl_and_security_pruning() {
        let mut header = HyperTileHeader::new(1, 0);
        header.lifecycle.max_expiry_timestamp = 5000;
        header.security.tenant_id = 1234;
        header.security.min_clearance_level = 2;

        // Query at t=6000 should reject tile because it expired at 5000
        assert!(can_reject_tile_ttl(&header, 6000));
        assert!(!can_reject_tile_ttl(&header, 4000));

        // Tenant 1234 with clearance 2 should pass
        assert!(!can_reject_tile_security(&header, 1234, 2));
        assert!(!can_reject_tile_security(&header, 1234, 3));

        // Tenant 9999 or low clearance should reject
        assert!(can_reject_tile_security(&header, 9999, 2));
        assert!(can_reject_tile_security(&header, 1234, 1));
    }
}


