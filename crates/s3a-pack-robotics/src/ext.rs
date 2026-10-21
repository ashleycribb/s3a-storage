use crate::pack::RoboticsTileType;
use crate::records::RoboticsKinematicRecord;
use s3a_core::{SimplexHull, ADDON_ROBOTICS, HYPER_TILE_PAYLOAD_SIZE};
use s3a_engine::lakehouse::{
    ArrowColumnVector, ArrowField, ArrowRecordBatchDescriptor, ArrowSchema,
};
use s3a_engine::{MmapReader, QuerySieve, S3ACrudEngine, TileWriter};
use s3a_simd::can_reject_tile_range;
use std::io;
use std::path::Path;

/// High-Frequency Low-Latency Ring-Buffered Writer for Autonomous Robots, Drones, and Humanoid Manipulators.
pub struct RoboticsStreamWriter {
    writer: TileWriter,
    record_buffer: Vec<RoboticsKinematicRecord>,
    capacity_per_tile: usize,
    current_min_xyz: [f32; 3],
    current_max_xyz: [f32; 3],
}

impl RoboticsStreamWriter {
    pub fn create<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let mut writer = TileWriter::create(path)?;
        writer.enable_addon(ADDON_ROBOTICS)?;
        let capacity_per_tile =
            HYPER_TILE_PAYLOAD_SIZE / std::mem::size_of::<RoboticsKinematicRecord>();

        Ok(Self {
            writer,
            record_buffer: Vec::with_capacity(capacity_per_tile),
            capacity_per_tile,
            current_min_xyz: [f32::INFINITY; 3],
            current_max_xyz: [f32::NEG_INFINITY; 3],
        })
    }

    /// Streams a 1 kHz robotics kinematic sample into zero-copy Hyper-Tiles with live 3D bounding hull updates.
    pub fn push_sample(&mut self, record: RoboticsKinematicRecord) -> io::Result<Option<u64>> {
        for d in 0..3 {
            if record.position_xyz[d] < self.current_min_xyz[d] {
                self.current_min_xyz[d] = record.position_xyz[d];
            }
            if record.position_xyz[d] > self.current_max_xyz[d] {
                self.current_max_xyz[d] = record.position_xyz[d];
            }
        }

        self.record_buffer.push(record);

        if self.record_buffer.len() >= self.capacity_per_tile {
            let tile_id = self.flush_tile()?;
            Ok(Some(tile_id))
        } else {
            Ok(None)
        }
    }

    /// Flushes accumulated kinematic records into a page-aligned 128 KB Hyper-Tile with embedded 3D spatial trajectory bounding hulls.
    pub fn flush_tile(&mut self) -> io::Result<u64> {
        if self.record_buffer.is_empty() {
            return Ok(self.writer.tile_count() as u64);
        }

        let mut hull = SimplexHull::empty();
        hull.dim = 3;
        for d in 0..3 {
            hull.min_bounds[d] = self.current_min_xyz[d];
            hull.max_bounds[d] = self.current_max_xyz[d];
        }

        let timestamps: Vec<u64> = self.record_buffer.iter().map(|r| r.timestamp_us).collect();
        let tile_id = self.writer.write_hyper_tile(
            RoboticsTileType::ROBOTICS_KINEMATIC,
            &self.record_buffer,
            Some(&timestamps),
            Some(hull),
        )?;

        self.record_buffer.clear();
        self.current_min_xyz = [f32::INFINITY; 3];
        self.current_max_xyz = [f32::NEG_INFINITY; 3];

        Ok(tile_id)
    }
}

pub trait RoboticsCrudExt {
    fn create_robotics_kinematics(&self, records: &[RoboticsKinematicRecord]) -> io::Result<u64>;
}

impl RoboticsCrudExt for S3ACrudEngine {
    fn create_robotics_kinematics(&self, records: &[RoboticsKinematicRecord]) -> io::Result<u64> {
        let timestamps: Vec<u64> = records.iter().map(|r| r.timestamp_us).collect();
        let mut hull = SimplexHull::empty();
        hull.dim = 3;
        for r in records {
            for d in 0..3 {
                if r.position_xyz[d] < hull.min_bounds[d] {
                    hull.min_bounds[d] = r.position_xyz[d];
                }
                if r.position_xyz[d] > hull.max_bounds[d] {
                    hull.max_bounds[d] = r.position_xyz[d];
                }
            }
        }

        self.append_tile_records(
            RoboticsTileType::ROBOTICS_KINEMATIC,
            records,
            Some(&timestamps),
            Some(hull),
            None,
        )
    }
}

pub trait RoboticsQueryExt {
    fn query_robotics_trajectory_3d(
        &self,
        min_xyz: [f32; 3],
        max_xyz: [f32; 3],
        min_ts_us: u64,
        max_ts_us: u64,
    ) -> Vec<RoboticsKinematicRecord>;
}

impl RoboticsQueryExt for QuerySieve<'_> {
    fn query_robotics_trajectory_3d(
        &self,
        min_xyz: [f32; 3],
        max_xyz: [f32; 3],
        min_ts_us: u64,
        max_ts_us: u64,
    ) -> Vec<RoboticsKinematicRecord> {
        self.reader()
            .query_robotics_trajectory_3d(min_xyz, max_xyz, min_ts_us, max_ts_us)
    }
}

impl RoboticsQueryExt for MmapReader {
    fn query_robotics_trajectory_3d(
        &self,
        min_xyz: [f32; 3],
        max_xyz: [f32; 3],
        min_ts_us: u64,
        max_ts_us: u64,
    ) -> Vec<RoboticsKinematicRecord> {
        let mut results = Vec::new();
        let tile_count = self.tile_count() as usize;

        for i in 0..tile_count {
            let (header, payload) = match self.get_tile(i) {
                Some(t) => t,
                None => continue,
            };

            if header.tile_type != RoboticsTileType::ROBOTICS_KINEMATIC {
                continue;
            }

            if header.max_timestamp < min_ts_us || header.min_timestamp > max_ts_us {
                continue;
            }

            // SIMD 3D Trajectory Bounding Box Rejection
            if can_reject_tile_range(&header.hull, &min_xyz, &max_xyz) {
                continue;
            }

            let valid_payload = &payload[..header.payload_bytes as usize];
            let records: &[RoboticsKinematicRecord] = bytemuck::cast_slice(valid_payload);
            for rec in records {
                if rec.timestamp_us >= min_ts_us && rec.timestamp_us <= max_ts_us {
                    if rec.position_xyz[0] >= min_xyz[0]
                        && rec.position_xyz[0] <= max_xyz[0]
                        && rec.position_xyz[1] >= min_xyz[1]
                        && rec.position_xyz[1] <= max_xyz[1]
                        && rec.position_xyz[2] >= min_xyz[2]
                        && rec.position_xyz[2] <= max_xyz[2]
                    {
                        results.push(*rec);
                    }
                }
            }
        }

        results
    }
}

/// Projects a slice of S3A RoboticsKinematicRecords into an Apache Arrow Columnar RecordBatch.
pub fn project_robotics_to_arrow(
    records: &[RoboticsKinematicRecord],
) -> ArrowRecordBatchDescriptor {
    let schema = ArrowSchema {
        fields: vec![
            ArrowField {
                name: "timestamp_us",
                data_type: "UInt64",
                nullable: false,
            },
            ArrowField {
                name: "robot_id",
                data_type: "UInt32",
                nullable: false,
            },
            ArrowField {
                name: "joint_mask",
                data_type: "UInt32",
                nullable: false,
            },
            ArrowField {
                name: "position_xyz",
                data_type: "FixedSizeBinary[12]",
                nullable: false,
            },
            ArrowField {
                name: "orientation_quat",
                data_type: "FixedSizeBinary[16]",
                nullable: false,
            },
        ],
    };

    let n = records.len();
    let mut col_ts = Vec::with_capacity(n);
    let mut col_robot = Vec::with_capacity(n);
    let mut col_joint = Vec::with_capacity(n);
    let mut col_pos = Vec::with_capacity(n * 12);
    let mut col_quat = Vec::with_capacity(n * 16);

    for r in records {
        col_ts.push(r.timestamp_us);
        col_robot.push(r.robot_id);
        col_joint.push(r.joint_mask);
        for i in 0..3 {
            col_pos.extend_from_slice(&r.position_xyz[i].to_le_bytes());
        }
        for i in 0..4 {
            col_quat.extend_from_slice(&r.orientation_quat[i].to_le_bytes());
        }
    }

    ArrowRecordBatchDescriptor {
        schema,
        num_rows: n,
        columns: vec![
            ArrowColumnVector::UInt64(col_ts),
            ArrowColumnVector::UInt32(col_robot),
            ArrowColumnVector::UInt32(col_joint),
            ArrowColumnVector::FixedSizeBinary {
                data: col_pos,
                byte_width: 12,
            },
            ArrowColumnVector::FixedSizeBinary {
                data: col_quat,
                byte_width: 16,
            },
        ],
    }
}
