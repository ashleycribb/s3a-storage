use crate::pack::WearableTileType;
use crate::records::CompactWearableRecord;
use s3a_engine::{MmapReader, QuerySieve, S3ACrudEngine};
use std::io;

pub trait WearableCrudExt {
    fn create_wearable_records(&self, records: &[CompactWearableRecord]) -> io::Result<u64>;
}

impl WearableCrudExt for S3ACrudEngine {
    fn create_wearable_records(&self, records: &[CompactWearableRecord]) -> io::Result<u64> {
        let timestamps: Vec<u64> = records.iter().map(|r| r.timestamp_sec as u64).collect();
        self.append_tile_records(
            WearableTileType::WEARABLE_MICRO,
            records,
            Some(&timestamps),
            None,
            None,
        )
    }
}

pub trait WearableQueryExt {
    fn query_wearable_records(
        &self,
        min_ts_sec: u32,
        max_ts_sec: u32,
        sensor_id: Option<u8>,
        metric_id: Option<u8>,
    ) -> Vec<CompactWearableRecord>;
}

impl WearableQueryExt for QuerySieve<'_> {
    fn query_wearable_records(
        &self,
        min_ts_sec: u32,
        max_ts_sec: u32,
        sensor_id: Option<u8>,
        metric_id: Option<u8>,
    ) -> Vec<CompactWearableRecord> {
        self.reader()
            .query_wearable_records(min_ts_sec, max_ts_sec, sensor_id, metric_id)
    }
}

impl WearableQueryExt for MmapReader {
    fn query_wearable_records(
        &self,
        min_ts_sec: u32,
        max_ts_sec: u32,
        sensor_id: Option<u8>,
        metric_id: Option<u8>,
    ) -> Vec<CompactWearableRecord> {
        let mut results = Vec::new();
        let tile_count = self.tile_count() as usize;

        for i in 0..tile_count {
            let (header, payload) = match self.get_tile(i) {
                Some(t) => t,
                None => continue,
            };

            if header.tile_type != WearableTileType::WEARABLE_MICRO {
                continue;
            }

            if header.max_timestamp < min_ts_sec as u64 || header.min_timestamp > max_ts_sec as u64
            {
                continue;
            }

            let valid_payload = &payload[..header.payload_bytes as usize];
            let records: &[CompactWearableRecord] = bytemuck::cast_slice(valid_payload);

            for &rec in records {
                if rec.timestamp_sec >= min_ts_sec && rec.timestamp_sec <= max_ts_sec {
                    if let Some(s) = sensor_id {
                        if rec.sensor_id != s {
                            continue;
                        }
                    }
                    if let Some(m) = metric_id {
                        if rec.metric_id != m {
                            continue;
                        }
                    }
                    results.push(rec);
                }
            }
        }

        results
    }
}
