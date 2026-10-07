pub mod ext;
pub mod pack;
pub mod records;

pub use ext::{WearableCrudExt, WearableQueryExt};
pub use pack::{register_pack_types, WearablePack, WearableTileType};
pub use records::CompactWearableRecord;

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::size_of;
    use s3a_core::{ADDON_WEARABLE, MICRO_TILE_PAYLOAD_SIZE};
    use s3a_engine::{MmapReader, S3ACrudEngine};

    #[test]
    fn test_wearable_record_layout_and_quantization() {
        assert_eq!(size_of::<CompactWearableRecord>(), 16);
        assert_eq!(MICRO_TILE_PAYLOAD_SIZE / size_of::<CompactWearableRecord>(), 248);

        let rec = CompactWearableRecord::new(100, 1, 10, 72.5);
        assert_eq!(rec.sensor_id, 1);
        assert_eq!(rec.metric_id, 10);
        assert!((rec.float_value() - 72.5).abs() < 0.01);
    }

    #[test]
    fn test_wearable_pack_metadata() {
        use s3a_core::S3AAddon;
        let pack = WearablePack;
        assert_eq!(pack.addon_id(), ADDON_WEARABLE);
        assert_eq!(pack.name(), "s3a-pack-wearable");
        assert_eq!(pack.tile_range(), 3..=3);
    }

    #[test]
    fn test_wearable_write_and_query_roundtrip() {
        let temp = std::env::temp_dir().join(format!("s3a_test_wearable_{}.s3a", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let engine = S3ACrudEngine::open_or_create(&temp).unwrap();
        engine.enable_addon(ADDON_WEARABLE).unwrap();

        let rec1 = CompactWearableRecord::new(100, 0, 10, 72.0);
        let rec2 = CompactWearableRecord::new(102, 0, 10, 74.5);
        let rec3 = CompactWearableRecord::new(104, 1, 20, 98.6);

        let tile_id = engine.create_wearable_records(&[rec1, rec2, rec3]).unwrap();
        assert_eq!(tile_id, 1);

        let reader = MmapReader::open(&temp).unwrap();
        let queried = reader.query_wearable_records(100, 103, Some(0), Some(10));
        assert_eq!(queried.len(), 2);
        assert_eq!(queried[0], rec1);
        assert_eq!(queried[1], rec2);

        let _ = std::fs::remove_file(&temp);
    }
}
