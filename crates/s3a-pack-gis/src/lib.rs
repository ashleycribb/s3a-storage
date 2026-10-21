pub mod ext;
pub mod pack;
pub mod records;

pub use ext::{project_gis_to_arrow, GisCrudExt, GisQueryExt};
pub use pack::{register_pack_types, GisPack, GisTileType};
pub use records::{GISSurveyPointRecord, GisSurveyPointRecord};

/// Registers S3A-QL handlers for the GIS domain.
pub fn register_ql_handlers() {
    s3a_engine::register_pack_sift_handler(
        s3a_engine::ql::ast::SiftDomain::GisMesh,
        std::sync::Arc::new(|sift| {
            let reader = s3a_engine::MmapReader::open(&sift.from_path)
                .map_err(s3a_engine::ql::QlError::Io)?;

            if reader.addon_mask() & s3a_core::ADDON_GIS == 0 {
                return Err(s3a_engine::ql::QlError::AddonRequired {
                    addon_mask: s3a_core::ADDON_GIS,
                    name: "gis",
                });
            }

            let mut min_lat = -90.0f64;
            let mut max_lat = 90.0f64;
            let mut min_lon = -180.0f64;
            let mut max_lon = 180.0f64;
            let mut min_elev = -10000.0f64;
            let mut max_elev = 50000.0f64;

            for cond in &sift.conditions {
                match cond {
                    s3a_engine::ql::ast::Condition::GisLatitudeBetween(min, max) => {
                        min_lat = *min;
                        max_lat = *max;
                    }
                    s3a_engine::ql::ast::Condition::GisLongitudeBetween(min, max) => {
                        min_lon = *min;
                        max_lon = *max;
                    }
                    s3a_engine::ql::ast::Condition::GisElevationBetween(min, max) => {
                        min_elev = *min;
                        max_elev = *max;
                    }
                    _ => {}
                }
            }

            let mut results =
                reader.query_gis_mesh_3d(min_lat, max_lat, min_lon, max_lon, min_elev, max_elev);
            if let Some(lim) = sift.limit {
                results.truncate(lim);
            }
            let count = results.len();
            Ok(s3a_engine::ql::QueryResult::PackData {
                domain: s3a_engine::ql::ast::SiftDomain::GisMesh,
                count,
                summary: format!("Found {} GIS mesh records in '{}'", count, sift.from_path),
                json_payload: None,
            })
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::size_of;
    use s3a_core::ADDON_GIS;
    use s3a_engine::{MmapReader, S3ACrudEngine};

    #[test]
    fn test_gis_record_layout() {
        assert_eq!(size_of::<GisSurveyPointRecord>(), 32);
        let point = GisSurveyPointRecord::new(37.774929, -122.419416, 125.5, 0, 100, 1600000000);

        assert!((point.latitude_deg() - 37.774929).abs() < 1e-5);
        assert!((point.longitude_deg() - (-122.419416)).abs() < 1e-5);
        assert!((point.elevation_m() - 125.5).abs() < 1e-3);
    }

    #[test]
    fn test_gis_pack_metadata() {
        use s3a_core::S3AAddon;
        let pack = GisPack;
        assert_eq!(pack.addon_id(), ADDON_GIS);
        assert_eq!(pack.name(), "s3a-pack-gis");
        assert_eq!(pack.tile_range(), 5..=5);
    }

    #[test]
    fn test_gis_write_and_query_mesh_3d() {
        let temp = std::env::temp_dir().join(format!(
            "s3a_test_gis_{}.s3a",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let engine = S3ACrudEngine::open_or_create(&temp).unwrap();
        engine.enable_addon(ADDON_GIS).unwrap();

        let points = vec![
            GisSurveyPointRecord::new(37.774929, -122.419416, 100.0, 0, 80, 1600000000), // Ground
            GisSurveyPointRecord::new(37.775000, -122.420000, -50.0, 1, 90, 1600000000), // Subsurface Strata
            GisSurveyPointRecord::new(38.000000, -123.000000, 200.0, 0, 50, 1600000000), // Out of area
        ];

        let tile_id = engine.create_gis_points(&points).unwrap();
        assert_eq!(tile_id, 1);

        let reader = MmapReader::open(&temp).unwrap();
        let res = reader.query_gis_mesh_3d(37.70, 37.80, -122.50, -122.40, -100.0, 150.0);
        assert_eq!(res.len(), 2);

        let _ = std::fs::remove_file(&temp);
    }

    #[test]
    fn test_project_gis_to_arrow() {
        let point = GisSurveyPointRecord::new(37.774929, -122.419416, 125.5, 0, 100, 1600000000);
        let batch = project_gis_to_arrow(&[point]);
        assert_eq!(batch.num_rows, 1);
        assert_eq!(batch.schema.fields.len(), 5);
    }
}
