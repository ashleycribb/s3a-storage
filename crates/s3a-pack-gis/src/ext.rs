use std::io;
use s3a_core::SimplexHull;
use s3a_engine::lakehouse::{ArrowColumnVector, ArrowField, ArrowRecordBatchDescriptor, ArrowSchema};
use s3a_engine::{MmapReader, QuerySieve, S3ACrudEngine};
use s3a_simd::can_reject_tile_range;
use crate::pack::GisTileType;
use crate::records::GisSurveyPointRecord;

pub trait GisCrudExt {
    fn create_gis_points(&self, points: &[GisSurveyPointRecord]) -> io::Result<u64>;
}

impl GisCrudExt for S3ACrudEngine {
    fn create_gis_points(&self, points: &[GisSurveyPointRecord]) -> io::Result<u64> {
        let mut hull = SimplexHull::empty();
        hull.dim = 3;

        for p in points {
            let lat = p.latitude_microdeg as f32;
            let lon = p.longitude_microdeg as f32;
            let elev = p.elevation_mm as f32;

            if lat < hull.min_bounds[0] { hull.min_bounds[0] = lat; }
            if lat > hull.max_bounds[0] { hull.max_bounds[0] = lat; }
            if lon < hull.min_bounds[1] { hull.min_bounds[1] = lon; }
            if lon > hull.max_bounds[1] { hull.max_bounds[1] = lon; }
            if elev < hull.min_bounds[2] { hull.min_bounds[2] = elev; }
            if elev > hull.max_bounds[2] { hull.max_bounds[2] = elev; }
        }

        let timestamps: Vec<u64> = points.iter().map(|p| p.timestamp_sec as u64).collect();

        self.append_tile_records(
            GisTileType::GIS_SURVEY_MESH,
            points,
            Some(&timestamps),
            Some(hull),
            None,
        )
    }
}

pub trait GisQueryExt {
    fn query_gis_mesh_3d(
        &self,
        min_lat_deg: f64,
        max_lat_deg: f64,
        min_lon_deg: f64,
        max_lon_deg: f64,
        min_elev_m: f64,
        max_elev_m: f64,
    ) -> Vec<GisSurveyPointRecord>;
}

impl GisQueryExt for QuerySieve<'_> {
    fn query_gis_mesh_3d(
        &self,
        min_lat_deg: f64,
        max_lat_deg: f64,
        min_lon_deg: f64,
        max_lon_deg: f64,
        min_elev_m: f64,
        max_elev_m: f64,
    ) -> Vec<GisSurveyPointRecord> {
        self.reader().query_gis_mesh_3d(min_lat_deg, max_lat_deg, min_lon_deg, max_lon_deg, min_elev_m, max_elev_m)
    }
}

impl GisQueryExt for MmapReader {
    fn query_gis_mesh_3d(
        &self,
        min_lat_deg: f64,
        max_lat_deg: f64,
        min_lon_deg: f64,
        max_lon_deg: f64,
        min_elev_m: f64,
        max_elev_m: f64,
    ) -> Vec<GisSurveyPointRecord> {
        let mut results = Vec::new();
        let tile_count = self.tile_count() as usize;

        let min_lat = (min_lat_deg * 1_000_000.0) as f32;
        let max_lat = (max_lat_deg * 1_000_000.0) as f32;
        let min_lon = (min_lon_deg * 1_000_000.0) as f32;
        let max_lon = (max_lon_deg * 1_000_000.0) as f32;

        let query_min = [min_lat, min_lon, (min_elev_m * 1000.0) as f32];
        let query_max = [max_lat, max_lon, (max_elev_m * 1000.0) as f32];

        for i in 0..tile_count {
            let (header, payload) = match self.get_tile(i) {
                Some(t) => t,
                None => continue,
            };

            if header.tile_type != GisTileType::GIS_SURVEY_MESH {
                continue;
            }

            // SIMD 3D Geographic Bounding Box Rejection across Simplex Hulls
            if can_reject_tile_range(&header.hull, &query_min, &query_max) {
                continue;
            }

            let valid_payload = &payload[..header.payload_bytes as usize];
            let records: &[GisSurveyPointRecord] = bytemuck::cast_slice(valid_payload);
            for rec in records {
                let lat = rec.latitude_deg();
                let lon = rec.longitude_deg();
                let elev = rec.elevation_m();

                if lat >= min_lat_deg && lat <= max_lat_deg
                    && lon >= min_lon_deg && lon <= max_lon_deg
                    && elev >= min_elev_m && elev <= max_elev_m
                {
                    results.push(*rec);
                }
            }
        }

        results
    }
}

/// Projects a slice of S3A GisSurveyPointRecords into an Apache Arrow Columnar RecordBatch.
pub fn project_gis_to_arrow(records: &[GisSurveyPointRecord]) -> ArrowRecordBatchDescriptor {
    let schema = ArrowSchema {
        fields: vec![
            ArrowField { name: "latitude_microdeg", data_type: "Int32", nullable: false },
            ArrowField { name: "longitude_microdeg", data_type: "Int32", nullable: false },
            ArrowField { name: "elevation_mm", data_type: "Int32", nullable: false },
            ArrowField { name: "point_class", data_type: "UInt32", nullable: false },
            ArrowField { name: "intensity", data_type: "UInt32", nullable: false },
        ],
    };

    let n = records.len();
    let mut col_lat = Vec::with_capacity(n);
    let mut col_lon = Vec::with_capacity(n);
    let mut col_elev = Vec::with_capacity(n);
    let mut col_class = Vec::with_capacity(n);
    let mut col_intensity = Vec::with_capacity(n);

    for r in records {
        col_lat.push(r.latitude_microdeg);
        col_lon.push(r.longitude_microdeg);
        col_elev.push(r.elevation_mm);
        col_class.push(r.point_class as u32);
        col_intensity.push(r.intensity as u32);
    }

    ArrowRecordBatchDescriptor {
        schema,
        num_rows: n,
        columns: vec![
            ArrowColumnVector::Int32(col_lat),
            ArrowColumnVector::Int32(col_lon),
            ArrowColumnVector::Int32(col_elev),
            ArrowColumnVector::UInt32(col_class),
            ArrowColumnVector::UInt32(col_intensity),
        ],
    }
}
