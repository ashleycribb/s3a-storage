//! OGC GeoPose 1.0 Basic-Quaternion record (128 bytes, 8-byte aligned).
//!
//! Layout (REVIEW_0_aq1b.md §2, little-endian wire order — the same convention
//! as the GIS pack's `GisSurveyPointRecord`):
//!
//! | Off | Len | Field |
//! |---:|---:|---|
//! | 0 | 16 | `source_id` (UUID of the source record) |
//! | 16 | 32 | `content_hash` (SHA-256 of the original JSON bytes) |
//! | 48 | 16 | `origin_ptr` (opaque 16-byte pointer to the original file) |
//! | 64 | 8 | `lat` f64 |
//! | 72 | 8 | `lon` f64 |
//! | 80 | 8 | `h` f64 |
//! | 88 | 8 | `qx` f64 |
//! | 96 | 8 | `qy` f64 |
//! | 104 | 8 | `qz` f64 |
//! | 112 | 8 | `qw` f64 |
//! | 120 | 8 | `_pad` (must be zero; no invented `validTime` on Basic) |
//!
//! `origin_ptr` is opaque in step 1: it names the original file, but the
//! encoding of "which file" (file UUID vs tile pointer) is a later-step
//! decision. What step 1 guarantees is that it is present and non-zero.

use bytemuck::{Pod, Zeroable};

/// OGC GeoPose Basic-Quaternion record (tile 33, `geopose_basic_quat`, 128 B).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Default)]
pub struct GeoposeBasicQuatRecord {
    pub source_id: [u8; 16],
    pub content_hash: [u8; 32],
    pub origin_ptr: [u8; 16],
    pub lat: f64,
    pub lon: f64,
    pub h: f64,
    pub qx: f64,
    pub qy: f64,
    pub qz: f64,
    pub qw: f64,
    pub _pad: u64,
}

impl GeoposeBasicQuatRecord {
    /// Wire size of the record in bytes.
    pub const SIZE: usize = 128;

    /// Squared norm of the orientation quaternion (x²+y²+z²+w²).
    pub fn norm_sq(&self) -> f64 {
        self.qx * self.qx + self.qy * self.qy + self.qz * self.qz + self.qw * self.qw
    }

    /// True when the quaternion is unit within `tolerance` on the squared norm.
    pub fn is_unit(&self, tolerance: f64) -> bool {
        (self.norm_sq() - 1.0).abs() <= tolerance
    }
}
