pub mod ext;
pub mod pack;
pub mod records;

pub use ext::{DaCrudExt, DaQueryExt};
pub use pack::{register_pack_types, DaPack, DaTileType};
pub use records::{DACommitmentRecord, DaCommitmentRecord};

/// Registers S3A-QL handlers for the Data Availability domain.
pub fn register_ql_handlers() {
    s3a_engine::register_pack_sift_handler(
        s3a_engine::ql::ast::SiftDomain::DaCommitments,
        std::sync::Arc::new(|sift| {
            let reader = s3a_engine::MmapReader::open(&sift.from_path)
                .map_err(s3a_engine::ql::QlError::Io)?;

            if reader.addon_mask() & s3a_core::ADDON_DA == 0 {
                return Err(s3a_engine::ql::QlError::AddonRequired {
                    addon_mask: s3a_core::ADDON_DA,
                    name: "da",
                });
            }

            let mut min_block = 0u64;
            let mut max_block = u64::MAX;

            for cond in &sift.conditions {
                if let s3a_engine::ql::ast::Condition::BlockHeightBetween(min, max) = cond {
                    min_block = *min;
                    max_block = *max;
                }
            }

            let mut results = reader.query_da_commitments(min_block, max_block);
            if let Some(lim) = sift.limit {
                results.truncate(lim);
            }
            let count = results.len();
            Ok(s3a_engine::ql::QueryResult::PackData {
                domain: s3a_engine::ql::ast::SiftDomain::DaCommitments,
                count,
                summary: format!("Found {} DA commitment records in '{}'", count, sift.from_path),
                json_payload: None,
            })
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::size_of;
    use s3a_core::ADDON_DA;
    use s3a_engine::{MmapReader, S3ACrudEngine};

    #[test]
    fn test_da_record_layout() {
        assert_eq!(size_of::<DaCommitmentRecord>(), 64);
        let rec = DaCommitmentRecord::new(100_000, [0xAA; 32], 1600000000, 0xFF, 500);
        assert_eq!(rec.block_height, 100_000);
        assert_eq!(rec.transaction_count, 500);
    }

    #[test]
    fn test_da_pack_metadata() {
        use s3a_core::S3AAddon;
        let pack = DaPack;
        assert_eq!(pack.addon_id(), ADDON_DA);
        assert_eq!(pack.name(), "s3a-pack-da");
        assert_eq!(pack.tile_range(), 6..=6);
    }

    #[test]
    fn test_da_write_and_query_commitments() {
        let temp = std::env::temp_dir().join(format!("s3a_test_da_{}.s3a", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let engine = S3ACrudEngine::open_or_create(&temp).unwrap();
        engine.enable_addon(ADDON_DA).unwrap();

        let commitments = vec![
            DaCommitmentRecord::new(100, [0x01; 32], 1600000000, 0xFF, 120),
            DaCommitmentRecord::new(200, [0x02; 32], 1600000010, 0xFF, 250),
            DaCommitmentRecord::new(500, [0x03; 32], 1600000020, 0xFF, 800),
        ];

        let tile_id = engine.create_da_commitments(&commitments).unwrap();
        assert_eq!(tile_id, 1);

        let reader = MmapReader::open(&temp).unwrap();
        let results = reader.query_da_commitments(150, 300);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].block_height, 200);

        let _ = std::fs::remove_file(&temp);
    }
}
