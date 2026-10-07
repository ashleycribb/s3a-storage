use bytemuck::{Pod, Zeroable};

/// GIS SURVEYING & SUBSURFACE POINT CLOUD / MESH RECORD (32 bytes, 8-byte aligned).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Default)]
pub struct GisSurveyPointRecord {
    pub latitude_microdeg: i32,  // Latitude in micro-degrees
    pub longitude_microdeg: i32, // Longitude in micro-degrees
    pub elevation_mm: i32,       // Elevation/Depth above/below sea level in millimeters
    pub point_class: u16,        // Classification (0=Ground, 1=Subsurface Strata, 2=Vegetation, 3=Structure)
    pub intensity: u16,          // LiDAR/Radar return intensity
    pub color_rgb: u32,          // Packed 24-bit RGB point color
    pub timestamp_sec: u32,      // Survey timestamp
    pub _reserved: u32,
    pub _padding: u32,
}

pub type GISSurveyPointRecord = GisSurveyPointRecord;

impl GisSurveyPointRecord {
    pub fn new(
        latitude_deg: f64,
        longitude_deg: f64,
        elevation_m: f64,
        point_class: u16,
        intensity: u16,
        timestamp_sec: u32,
    ) -> Self {
        Self {
            latitude_microdeg: (latitude_deg * 1_000_000.0) as i32,
            longitude_microdeg: (longitude_deg * 1_000_000.0) as i32,
            elevation_mm: (elevation_m * 1000.0) as i32,
            point_class,
            intensity,
            color_rgb: 0x00FFFFFF,
            timestamp_sec,
            _reserved: 0,
            _padding: 0,
        }
    }

    pub fn latitude_deg(&self) -> f64 {
        (self.latitude_microdeg as f64) / 1_000_000.0
    }

    pub fn longitude_deg(&self) -> f64 {
        (self.longitude_microdeg as f64) / 1_000_000.0
    }

    pub fn elevation_m(&self) -> f64 {
        (self.elevation_mm as f64) / 1000.0
    }
}
