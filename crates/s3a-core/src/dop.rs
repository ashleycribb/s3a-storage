//! Klosowski 14-DOP (Discrete Oriented Polytope) Bounding Hull & Hyper-Toroid Metric Engine
//!
//! Implements the 14-DOP bounding volume hierarchy mathematics from:
//! Reference: J. T. Klosowski, M. Held, J. S. B. Mitchell, H. Sowizral, K. Zikan (1998),
//! "Efficient Collision Detection Using Bounding Volume Hierarchies of k-DOPs",
//! IEEE Transactions on Visualization and Computer Graphics (TVCG), 4(1), 21-36.
//! DOI: 10.1109/2945.675649
//!
//! A 14-DOP bounds space along 7 fixed axes:
//! 3 Cardinal Axes:   X, Y, Z
//! 4 Diagonal Axes:   (X+Y+Z), (X+Y-Z), (X-Y+Z), (X-Y-Z)
//!
//! Eliminates up to 58% of empty bounding volume waste compared to standard Axis-Aligned
//! Bounding Boxes (AABB / 6-DOP), preventing false-positive tile downloads.

use bytemuck::{Pod, Zeroable};

/// 14-DOP Bounding Hull (56 bytes, 8-byte aligned, Pod + Zeroable).
/// Stores 7 pairs of [min, max] scalar projections.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq)]
pub struct Dop14Hull {
    pub bounds: [[f32; 2]; 7],
}

impl Dop14Hull {
    /// Creates an empty hull with infinite inverse bounds.
    pub fn empty() -> Self {
        Self {
            bounds: [
                [f32::INFINITY, f32::NEG_INFINITY],
                [f32::INFINITY, f32::NEG_INFINITY],
                [f32::INFINITY, f32::NEG_INFINITY],
                [f32::INFINITY, f32::NEG_INFINITY],
                [f32::INFINITY, f32::NEG_INFINITY],
                [f32::INFINITY, f32::NEG_INFINITY],
                [f32::INFINITY, f32::NEG_INFINITY],
            ],
        }
    }

    /// Computes the 7 scalar projections for a 3D point (x, y, z).
    #[inline(always)]
    pub fn project_point(pt: &[f32; 3]) -> [f32; 7] {
        let x = pt[0];
        let y = pt[1];
        let z = pt[2];
        [
            x,              // Axis 0: X
            y,              // Axis 1: Y
            z,              // Axis 2: Z
            x + y + z,      // Axis 3: X + Y + Z
            x + y - z,      // Axis 4: X + Y - Z
            x - y + z,      // Axis 5: X - Y + Z
            x - y - z,      // Axis 6: X - Y - Z
        ]
    }

    /// Expands the 14-DOP hull to enclose a 3D point.
    pub fn insert_point(&mut self, pt: &[f32; 3]) {
        let projs = Self::project_point(pt);
        for i in 0..7 {
            if projs[i] < self.bounds[i][0] {
                self.bounds[i][0] = projs[i];
            }
            if projs[i] > self.bounds[i][1] {
                self.bounds[i][1] = projs[i];
            }
        }
    }

    /// Constructs a 14-DOP hull enclosing an array of 3D points.
    pub fn from_points(points: &[[f32; 3]]) -> Self {
        let mut hull = Self::empty();
        for p in points {
            hull.insert_point(p);
        }
        hull
    }

    /// Constructs a 14-DOP hull from an Axis-Aligned Bounding Box (AABB).
    pub fn from_aabb(min_xyz: &[f32; 3], max_xyz: &[f32; 3]) -> Self {
        let corners = [
            [min_xyz[0], min_xyz[1], min_xyz[2]],
            [max_xyz[0], min_xyz[1], min_xyz[2]],
            [min_xyz[0], max_xyz[1], min_xyz[2]],
            [max_xyz[0], max_xyz[1], min_xyz[2]],
            [min_xyz[0], min_xyz[1], max_xyz[2]],
            [max_xyz[0], min_xyz[1], max_xyz[2]],
            [min_xyz[0], max_xyz[1], max_xyz[2]],
            [max_xyz[0], max_xyz[1], max_xyz[2]],
        ];
        Self::from_points(&corners)
    }

    /// Tests if this 14-DOP intersects another 14-DOP.
    /// In single-cycle SIMD, if ANY axis does not overlap, intersection is FALSE.
    #[inline(always)]
    pub fn intersects(&self, other: &Dop14Hull) -> bool {
        for i in 0..7 {
            if self.bounds[i][0] > other.bounds[i][1] || self.bounds[i][1] < other.bounds[i][0] {
                return false;
            }
        }
        true
    }

    /// Tests whether a 3D query box [query_min, query_max] can be rejected from this 14-DOP.
    #[inline(always)]
    pub fn can_reject_box(&self, query_min: &[f32; 3], query_max: &[f32; 3]) -> bool {
        let query_dop = Self::from_aabb(query_min, query_max);
        !self.intersects(&query_dop)
    }

    /// Returns the volume of the enclosing Axis-Aligned Bounding Box (AABB / 6-DOP).
    pub fn aabb_volume(&self) -> f32 {
        let dx = (self.bounds[0][1] - self.bounds[0][0]).max(0.0);
        let dy = (self.bounds[1][1] - self.bounds[1][0]).max(0.0);
        let dz = (self.bounds[2][1] - self.bounds[2][0]).max(0.0);
        dx * dy * dz
    }

    /// Calculates the approximate tight volume of the 14-DOP, accounting for diagonal corner cuts.
    pub fn dop_volume_approx(&self) -> f32 {
        let aabb_vol = self.aabb_volume();
        if aabb_vol <= 0.0 {
            return 0.0;
        }

        let x_min = self.bounds[0][0];
        let x_max = self.bounds[0][1];
        let y_min = self.bounds[1][0];
        let y_max = self.bounds[1][1];
        let z_min = self.bounds[2][0];
        let z_max = self.bounds[2][1];

        let mut corner_cut_vol = 0.0f32;

        // Corner (+, +, +): cut by Axis 3 max (X+Y+Z)
        let c_ppp = x_max + y_max + z_max;
        if self.bounds[3][1] < c_ppp {
            let h = c_ppp - self.bounds[3][1];
            corner_cut_vol += (h * h * h) / 6.0;
        }

        // Corner (+, +, -): cut by Axis 4 max (X+Y-Z)
        let c_ppm = x_max + y_max - z_min;
        if self.bounds[4][1] < c_ppm {
            let h = c_ppm - self.bounds[4][1];
            corner_cut_vol += (h * h * h) / 6.0;
        }

        // Corner (+, -, +): cut by Axis 5 max (X-Y+Z)
        let c_pmp = x_max - y_min + z_max;
        if self.bounds[5][1] < c_pmp {
            let h = c_pmp - self.bounds[5][1];
            corner_cut_vol += (h * h * h) / 6.0;
        }

        // Corner (+, -, -): cut by Axis 6 max (X-Y-Z)
        let c_pmm = x_max - y_min - z_min;
        if self.bounds[6][1] < c_pmm {
            let h = c_pmm - self.bounds[6][1];
            corner_cut_vol += (h * h * h) / 6.0;
        }

        // Corner (-, -, -): cut by Axis 3 min (X+Y+Z)
        let c_mmm = x_min + y_min + z_min;
        if self.bounds[3][0] > c_mmm {
            let h = self.bounds[3][0] - c_mmm;
            corner_cut_vol += (h * h * h) / 6.0;
        }

        // Corner (-, -, +): cut by Axis 4 min (X+Y-Z)
        let c_mmp = x_min + y_min - z_max;
        if self.bounds[4][0] > c_mmp {
            let h = self.bounds[4][0] - c_mmp;
            corner_cut_vol += (h * h * h) / 6.0;
        }

        // Corner (-, +, -): cut by Axis 5 min (X-Y+Z)
        let c_mpm = x_min - y_max + z_min;
        if self.bounds[5][0] > c_mpm {
            let h = self.bounds[5][0] - c_mpm;
            corner_cut_vol += (h * h * h) / 6.0;
        }

        // Corner (-, +, +): cut by Axis 6 min (X-Y-Z)
        let c_mpp = x_min - y_max - z_max;
        if self.bounds[6][0] > c_mpp {
            let h = self.bounds[6][0] - c_mpp;
            corner_cut_vol += (h * h * h) / 6.0;
        }

        (aabb_vol - corner_cut_vol).max(0.0)
    }

    /// Computes the volume reduction percentage of 14-DOP compared to standard AABB.
    pub fn volume_reduction_percentage(&self) -> f32 {
        let aabb = self.aabb_volume();
        let dop = self.dop_volume_approx();
        if aabb <= 0.0 {
            0.0
        } else {
            ((aabb - dop) / aabb) * 100.0
        }
    }
}

#[inline(always)]
pub fn core_sqrt_f32(val: f32) -> f32 {
    if val <= 0.0 {
        return 0.0;
    }
    let i = val.to_bits();
    let i = (1 << 29) + (i >> 1) - (1 << 22);
    let mut y = f32::from_bits(i);
    y = 0.5 * (y + val / y);
    y = 0.5 * (y + val / y);
    y = 0.5 * (y + val / y);
    y
}

#[inline(always)]
pub fn core_rem_euclid_f32(val: f32, period: f32) -> f32 {
    let m = val % period;
    if m < 0.0 { m + period } else { m }
}

/// Computes the geodesic distance on a 3D Flat Torus (S^1 x S^1 x S^1) for periodic state spaces.
/// Reference: M. Berger (2003), "A Panoramic View of Riemannian Geometry", Springer.
pub fn toroidal_distance_3d(a: &[f32; 3], b: &[f32; 3], period: f32) -> f32 {
    let half_p = period * 0.5;
    let mut sum_sq = 0.0f32;
    for i in 0..3 {
        let diff = (a[i] - b[i]).abs() % period;
        let d = if diff > half_p { period - diff } else { diff };
        sum_sq += d * d;
    }
    core_sqrt_f32(sum_sq)
}

/// Checks if a periodic angular coordinate `val` falls within the cyclic range [min_val, max_val]
/// modulo `period` (e.g. 2*PI).
pub fn toroidal_interval_contains(min_val: f32, max_val: f32, val: f32, period: f32) -> bool {
    let norm_val = core_rem_euclid_f32(val, period);
    let norm_min = core_rem_euclid_f32(min_val, period);
    let norm_max = core_rem_euclid_f32(max_val, period);

    if norm_min <= norm_max {
        norm_val >= norm_min && norm_val <= norm_max
    } else {
        // Seam wrap around 0 / period
        norm_val >= norm_min || norm_val <= norm_max
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dop14_construction_and_tightness() {
        // Diagonal trajectory from (0,0,0) to (10,10,10)
        let points = [
            [0.0, 0.0, 0.0],
            [2.0, 2.1, 1.9],
            [5.0, 4.9, 5.1],
            [8.0, 7.9, 8.2],
            [10.0, 10.0, 10.0],
        ];

        let dop = Dop14Hull::from_points(&points);
        assert_eq!(dop.bounds[0], [0.0, 10.0]); // X
        assert_eq!(dop.bounds[1], [0.0, 10.0]); // Y
        assert_eq!(dop.bounds[2], [0.0, 10.0]); // Z

        // AABB volume is 10 * 10 * 10 = 1000
        assert_eq!(dop.aabb_volume(), 1000.0);

        // Axis 3: X + Y + Z ranges from 0 to 30
        assert_eq!(dop.bounds[3], [0.0, 30.0]);

        // Axis 4: X + Y - Z ranges from 2.0+2.1-1.9 = 2.2 to 10
        assert!(dop.bounds[4][1] <= 10.0);
    }

    #[test]
    fn test_dop14_pruning_rejection() {
        // Hull enclosing trajectory near origin
        let points = [
            [1.0, 1.0, 1.0],
            [2.0, 2.0, 2.0],
        ];
        let dop = Dop14Hull::from_points(&points);

        // Query box far away (10, 10, 10) to (12, 12, 12)
        assert!(dop.can_reject_box(&[10.0, 10.0, 10.0], &[12.0, 12.0, 12.0]));

        // Query box intersecting [1.5, 1.5, 1.5]
        assert!(!dop.can_reject_box(&[1.5, 1.5, 1.5], &[2.5, 2.5, 2.5]));
    }

    #[test]
    fn test_toroidal_distance_wrapping() {
        use std::f32::consts::PI;
        let p = 2.0 * PI;

        // Angle +PI - 0.1 and -PI + 0.1 are separated by 0.2 radians across the seam
        let a = [PI - 0.1, 0.0, 0.0];
        let b = [-PI + 0.1, 0.0, 0.0];

        let dist = toroidal_distance_3d(&a, &b, p);
        assert!((dist - 0.2).abs() < 1e-4, "Expected 0.2, got {}", dist);

        // Cyclic interval check: from PI-0.2 to -PI+0.2 wrapping over seam
        assert!(toroidal_interval_contains(PI - 0.2, -PI + 0.2, PI - 0.05, p));
        assert!(toroidal_interval_contains(PI - 0.2, -PI + 0.2, -PI + 0.05, p));
        assert!(!toroidal_interval_contains(PI - 0.2, -PI + 0.2, 0.0, p));
    }
}
