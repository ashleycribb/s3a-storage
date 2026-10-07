pub mod ext;
pub mod pack;
pub mod records;

pub use ext::{project_robotics_to_arrow, RoboticsCrudExt, RoboticsQueryExt, RoboticsStreamWriter};
pub use pack::{register_pack_types, RoboticsPack, RoboticsTileType};
pub use records::RoboticsKinematicRecord;

/// Registers S3A-QL handlers for the robotics domain.
pub fn register_ql_handlers() {
    s3a_engine::register_pack_sift_handler(
        s3a_engine::ql::ast::SiftDomain::Kinematics,
        std::sync::Arc::new(|sift| {
            let reader = s3a_engine::MmapReader::open(&sift.from_path)
                .map_err(s3a_engine::ql::QlError::Io)?;

            if reader.addon_mask() & s3a_core::ADDON_ROBOTICS == 0 {
                return Err(s3a_engine::ql::QlError::AddonRequired {
                    addon_mask: s3a_core::ADDON_ROBOTICS,
                    name: "robotics",
                });
            }

            let mut min_ts = 0u64;
            let mut max_ts = u64::MAX;
            let mut min_xyz = [-f32::INFINITY; 3];
            let mut max_xyz = [f32::INFINITY; 3];

            for cond in &sift.conditions {
                match cond {
                    s3a_engine::ql::ast::Condition::TimeBetween(min, max) => {
                        min_ts = *min;
                        max_ts = *max;
                    }
                    s3a_engine::ql::ast::Condition::SpatialBoxSimplex { min, max } => {
                        min_xyz = *min;
                        max_xyz = *max;
                    }
                    _ => {}
                }
            }

            let mut results = reader.query_robotics_trajectory_3d(min_xyz, max_xyz, min_ts, max_ts);
            if let Some(lim) = sift.limit {
                results.truncate(lim);
            }
            let count = results.len();
            Ok(s3a_engine::ql::QueryResult::PackData {
                domain: s3a_engine::ql::ast::SiftDomain::Kinematics,
                count,
                summary: format!("Found {} kinematics records in '{}'", count, sift.from_path),
                json_payload: None,
            })
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::size_of;
    use s3a_core::ADDON_ROBOTICS;
    use s3a_engine::MmapReader;

    #[test]
    fn test_robotics_record_layout() {
        assert_eq!(size_of::<RoboticsKinematicRecord>(), 72);
        let rec = RoboticsKinematicRecord::new(
            1_000_000,
            42,
            [1.0, 2.0, 3.0],
            [1.0, 0.0, 0.0, 0.0],
            [0.5, 0.0, 0.0],
            [0.0, 0.1, 0.0],
        );
        assert_eq!(rec.timestamp_us, 1_000_000);
        assert_eq!(rec.robot_id, 42);
        assert_eq!(rec.position_xyz, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn test_robotics_pack_metadata() {
        use s3a_core::S3AAddon;
        let pack = RoboticsPack;
        assert_eq!(pack.addon_id(), ADDON_ROBOTICS);
        assert_eq!(pack.name(), "s3a-pack-robotics");
        assert_eq!(pack.tile_range(), 4..=4);
    }

    #[test]
    fn test_robotics_stream_writer_and_3d_trajectory_query() {
        let temp = std::env::temp_dir().join(format!("s3a_test_robotics_{}.s3a", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let mut stream_writer = RoboticsStreamWriter::create(&temp).unwrap();

        for i in 0..100 {
            let rec = RoboticsKinematicRecord::new(
                i as u64 * 1000,
                1,
                [i as f32 * 0.1, (i % 10) as f32, 0.5],
                [1.0, 0.0, 0.0, 0.0],
                [0.1, 0.0, 0.0],
                [0.0, 0.0, 0.0],
            );
            let _ = stream_writer.push_sample(rec).unwrap();
        }
        stream_writer.flush_tile().unwrap();

        let reader = MmapReader::open(&temp).unwrap();
        let results = reader.query_robotics_trajectory_3d(
            [0.0, 0.0, 0.0],
            [5.0, 5.0, 1.0],
            0,
            1_000_000,
        );

        assert!(!results.is_empty());
        for r in &results {
            assert!(r.position_xyz[0] >= 0.0 && r.position_xyz[0] <= 5.0);
        }

        let _ = std::fs::remove_file(&temp);
    }

    #[test]
    fn test_project_robotics_to_arrow() {
        let rec = RoboticsKinematicRecord::new(
            50000,
            1,
            [1.0, 2.0, 3.0],
            [1.0, 0.0, 0.0, 0.0],
            [0.1, 0.2, 0.3],
            [0.0, 0.0, 0.0],
        );
        let batch = project_robotics_to_arrow(&[rec]);
        assert_eq!(batch.num_rows, 1);
        assert_eq!(batch.schema.fields.len(), 5);
    }
}
