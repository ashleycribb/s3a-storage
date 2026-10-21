use crate::pack::DaTileType;
use crate::records::DaCommitmentRecord;
use s3a_engine::{MmapReader, QuerySieve, S3ACrudEngine};
use s3a_simd::can_reject_tile_time;
use std::io;

pub trait DaCrudExt {
    fn create_da_commitments(&self, commitments: &[DaCommitmentRecord]) -> io::Result<u64>;
}

impl DaCrudExt for S3ACrudEngine {
    fn create_da_commitments(&self, commitments: &[DaCommitmentRecord]) -> io::Result<u64> {
        let block_heights: Vec<u64> = commitments.iter().map(|c| c.block_height).collect();
        self.append_tile_records(
            DaTileType::DATA_AVAILABILITY,
            commitments,
            Some(&block_heights),
            None,
            None,
        )
    }
}

pub trait DaQueryExt {
    fn query_da_commitments(&self, min_block: u64, max_block: u64) -> Vec<DaCommitmentRecord>;
}

impl DaQueryExt for QuerySieve<'_> {
    fn query_da_commitments(&self, min_block: u64, max_block: u64) -> Vec<DaCommitmentRecord> {
        self.reader().query_da_commitments(min_block, max_block)
    }
}

impl DaQueryExt for MmapReader {
    fn query_da_commitments(&self, min_block: u64, max_block: u64) -> Vec<DaCommitmentRecord> {
        let mut results = Vec::new();
        let tile_count = self.tile_count() as usize;

        for i in 0..tile_count {
            let (header, payload) = match self.get_tile(i) {
                Some(t) => t,
                None => continue,
            };

            if header.tile_type != DaTileType::DATA_AVAILABILITY {
                continue;
            }

            if can_reject_tile_time(header, min_block, max_block) {
                continue;
            }

            let valid_payload = &payload[..header.payload_bytes as usize];
            let records: &[DaCommitmentRecord] = bytemuck::cast_slice(valid_payload);
            for rec in records {
                if rec.block_height >= min_block && rec.block_height <= max_block {
                    results.push(*rec);
                }
            }
        }

        results
    }
}
