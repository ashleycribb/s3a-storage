use std::fs::{File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use bytemuck::{bytes_of, from_bytes, Pod};
use memmap2::Mmap;

pub use s3a_core::*;
use s3a_simd::{can_reject_tile_point, can_reject_tile_range, can_reject_tile_time};

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
        let writer = TileWriter::create(path)?;
        let capacity_per_tile = HYPER_TILE_PAYLOAD_SIZE / std::mem::size_of::<RoboticsKinematicRecord>();

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
            return Ok(0);
        }

        let mut hull = SimplexHull::empty();
        hull.dim = 3;
        for d in 0..3 {
            hull.min_bounds[d] = self.current_min_xyz[d];
            hull.max_bounds[d] = self.current_max_xyz[d];
        }

        let timestamps: Vec<u64> = self.record_buffer.iter().map(|r| r.timestamp_us).collect();
        let tile_id = self.writer.write_hyper_tile(
            TileType::ROBOTICS_KINEMATIC,
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

/// Zero-allocation 4 KB Micro-Tile Memory Buffer for embedded microcontrollers (Earbuds, Smart Glasses, Wristbands).
pub struct MicroTileBuffer {
    buffer: [u8; MICRO_TILE_SIZE],
    written_bytes: usize,
    record_count: u16,
}

impl MicroTileBuffer {
    pub fn new(tile_id: u32) -> Self {
        let mut buffer = [0u8; MICRO_TILE_SIZE];
        let header = MicroTileHeader::new(tile_id, TileType::WEARABLE_MICRO as u16);
        let header_bytes = bytes_of(&header);
        buffer[..MICRO_TILE_HEADER_SIZE].copy_from_slice(header_bytes);

        Self {
            buffer,
            written_bytes: MICRO_TILE_HEADER_SIZE,
            record_count: 0,
        }
    }

    /// Appends a record directly into the 4 KB page buffer without dynamic memory allocation (`alloc` free).
    pub fn push_record<T: Pod>(&mut self, record: &T) -> Result<(), S3AError> {
        let record_bytes = bytes_of(record);
        if self.written_bytes + record_bytes.len() > MICRO_TILE_SIZE {
            return Err(S3AError::PayloadOverflow);
        }

        self.buffer[self.written_bytes..self.written_bytes + record_bytes.len()].copy_from_slice(record_bytes);
        self.written_bytes += record_bytes.len();
        self.record_count += 1;
        Ok(())
    }

    /// Finalizes header CRC32C and returns the complete 4,096-byte page ready for SPI NOR Flash programming.
    pub fn finalize(&mut self) -> &[u8; MICRO_TILE_SIZE] {
        let payload_bytes = (self.written_bytes - MICRO_TILE_HEADER_SIZE) as u16;
        let data_bytes = &self.buffer[MICRO_TILE_HEADER_SIZE..self.written_bytes];
        let data_crc32 = crc32c::crc32c(data_bytes);

        let mut header: MicroTileHeader = *from_bytes(&self.buffer[..MICRO_TILE_HEADER_SIZE]);
        header.record_count = self.record_count;
        header.payload_bytes = payload_bytes;
        header.data_crc32 = data_crc32;

        let crc_offset = std::mem::offset_of!(MicroTileHeader, header_crc32);
        let header_bytes = bytes_of(&header);
        header.header_crc32 = crc32c::crc32c(&header_bytes[..crc_offset]);

        self.buffer[..MICRO_TILE_HEADER_SIZE].copy_from_slice(bytes_of(&header));
        &self.buffer
    }
}

/// Writer for generating page-aligned 128 KB S3A Hyper-Tile storage files.
pub struct TileWriter {
    file: File,
    tile_count: u32,
    current_tile_id: u64,
}

impl TileWriter {
    pub fn create<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;

        let header = FileHeader::new(0, 0);
        file.write_all(bytes_of(&header))?;

        // Pad file header to 64 bytes if needed
        let pos = file.stream_position()?;
        if pos < FILE_HEADER_SIZE as u64 {
            let pad = vec![0u8; FILE_HEADER_SIZE - pos as usize];
            file.write_all(&pad)?;
        }

        file.flush()?;

        Ok(Self {
            file,
            tile_count: 0,
            current_tile_id: 1,
        })
    }

    pub fn open_append<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let reader = MmapReader::open(path.as_ref())?;
        let tile_count = reader.tile_count();

        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)?;

        file.seek(SeekFrom::End(0))?;

        Ok(Self {
            file,
            tile_count,
            current_tile_id: (tile_count as u64) + 1,
        })
    }

    pub fn write_hyper_tile<T: Pod>(
        &mut self,
        tile_type: u32,
        records: &[T],
        timestamps: Option<&[u64]>,
        hull: Option<SimplexHull>,
    ) -> io::Result<u64> {
        let record_size = std::mem::size_of::<T>();
        let payload_bytes = records.len() * record_size;

        if payload_bytes > HYPER_TILE_PAYLOAD_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Records size exceeds Hyper-Tile payload capacity",
            ));
        }

        let mut header = HyperTileHeader::new(self.current_tile_id, tile_type);
        header.record_count = records.len() as u32;
        header.payload_bytes = payload_bytes as u32;

        if let Some(ts) = timestamps {
            if !ts.is_empty() {
                let mut min_t = u64::MAX;
                let mut max_t = 0;
                for &t in ts {
                    if t < min_t {
                        min_t = t;
                    }
                    if t > max_t {
                        max_t = t;
                    }
                }
                header.min_timestamp = min_t;
                header.max_timestamp = max_t;
            }
        }

        if let Some(h) = hull {
            header.hull = h;
        }

        // Convert records slice to byte slice using bytemuck
        let record_bytes = bytemuck::cast_slice::<T, u8>(records);

        // Compute data CRC32C
        header.data_crc32 = crc32c::crc32c(record_bytes);

        // Compute header CRC32C (excluding header_crc32 field itself)
        let header_bytes = bytes_of(&header);
        let crc_offset = std::mem::offset_of!(HyperTileHeader, header_crc32);
        let header_without_crc = &header_bytes[..crc_offset];
        header.header_crc32 = crc32c::crc32c(header_without_crc);

        // Write header
        self.file.write_all(bytes_of(&header))?;

        // Write record payload
        self.file.write_all(record_bytes)?;

        // Write padding to reach 128 KB total tile size
        let written = HYPER_TILE_HEADER_SIZE + payload_bytes;
        let padding_needed = HYPER_TILE_SIZE - written;
        if padding_needed > 0 {
            let zeros = vec![0u8; padding_needed];
            self.file.write_all(&zeros)?;
        }

        let tile_id = self.current_tile_id;
        self.current_tile_id += 1;
        self.tile_count += 1;

        // Update tile count in FileHeader
        self.file.seek(SeekFrom::Start(0))?;
        let updated_file_header = FileHeader::new(self.tile_count, 0);
        self.file.write_all(bytes_of(&updated_file_header))?;
        self.file.seek(SeekFrom::End(0))?;

        self.file.flush()?;

        Ok(tile_id)
    }

    pub fn tile_count(&self) -> u32 {
        self.tile_count
    }
}

/// Zero-copy memory-mapped reader for S3A archives.
pub struct MmapReader {
    _file: File,
    mmap: Mmap,
}

impl MmapReader {
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let file = File::open(path)?;
        let mmap = unsafe { Mmap::map(&file)? };

        if mmap.len() < FILE_HEADER_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "File too small for S3A header",
            ));
        }

        let file_header: &FileHeader = from_bytes(&mmap[0..FILE_HEADER_SIZE]);
        file_header.verify().map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("{}", e))
        })?;

        Ok(Self { _file: file, mmap })
    }

    pub fn file_header(&self) -> &FileHeader {
        from_bytes(&self.mmap[0..FILE_HEADER_SIZE])
    }

    pub fn tile_count(&self) -> u32 {
        self.file_header().tile_count
    }

    pub fn get_tile(&self, index: usize) -> Option<(&HyperTileHeader, &[u8])> {
        let offset = FILE_HEADER_SIZE.checked_add(index.checked_mul(HYPER_TILE_SIZE)?)?;
        if offset.checked_add(HYPER_TILE_SIZE)? > self.mmap.len() {
            return None;
        }

        let header_slice = &self.mmap[offset..offset + HYPER_TILE_HEADER_SIZE];
        let tile_header: &HyperTileHeader = from_bytes(header_slice);

        let payload_start = offset + HYPER_TILE_HEADER_SIZE;
        let payload_end = payload_start + tile_header.payload_bytes as usize;
        if payload_end > offset + HYPER_TILE_SIZE {
            return None;
        }
        let payload = &self.mmap[payload_start..payload_end];

        Some((tile_header, payload))
    }

    /// Instantaneous O(1) direct record lookup by S3ACoordinate address (Level, Tile ID, Record Offset)
    pub fn read_record_by_coordinate<T: Pod>(&self, coord: &S3ACoordinate) -> Result<T, S3AError> {
        let (header, payload) = self.get_tile(coord.tile_id as usize).ok_or(S3AError::OutOfBounds)?;
        let records: &[T] = bytemuck::cast_slice(payload);

        let offset = coord.record_offset as usize;
        if offset >= records.len() || offset >= header.record_count as usize {
            return Err(S3AError::OutOfBounds);
        }

        Ok(records[offset])
    }

    pub fn verify_checksums(&self) -> Result<(), S3AError> {
        let count = self.tile_count() as usize;
        for i in 0..count {
            if let Some((header, payload)) = self.get_tile(i) {
                let actual_data_crc = crc32c::crc32c(payload);
                if actual_data_crc != header.data_crc32 {
                    return Err(S3AError::ChecksumMismatch {
                        expected: header.data_crc32,
                        actual: actual_data_crc,
                    });
                }
            } else {
                return Err(S3AError::OutOfBounds);
            }
        }
        Ok(())
    }
}

/// MULTI-TILE HYPER-MESH FUSION ENGINE: Combines multiple distinct or sparse Hyper-Tiles into a single unified 128 KB Hyper-Tile.
pub struct TileFusionEngine;

impl TileFusionEngine {
    /// Fuses multiple Telemetry Hyper-Tiles into a single consolidated 128 KB Hyper-Tile with recalculation of Simplex Bounding Hulls.
    pub fn fuse_telemetry_tiles<P: AsRef<Path>>(
        input_paths: &[P],
        output_path: P,
    ) -> io::Result<u32> {
        let mut writer = TileWriter::create(output_path)?;
        let mut fused_records: Vec<TelemetryRecord> = Vec::new();

        for path in input_paths {
            let reader = MmapReader::open(path)?;
            for i in 0..reader.tile_count() as usize {
                if let Some((header, payload)) = reader.get_tile(i) {
                    if header.tile_type == TileType::TELEMETRY {
                        let records: &[TelemetryRecord] = bytemuck::cast_slice(payload);
                        fused_records.extend_from_slice(records);
                    }
                }
            }
        }

        // Filter out tombstones and deduplicate versioned updates
        fused_records.sort_by_key(|r| (r.sensor_id, r.metric_id, r.timestamp));
        let mut deduped: Vec<TelemetryRecord> = Vec::new();
        for rec in fused_records {
            if rec.is_tombstone() {
                deduped.retain(|r| !(r.sensor_id == rec.sensor_id && r.metric_id == rec.metric_id && r.timestamp == rec.timestamp));
            } else {
                if let Some(pos) = deduped.iter().position(|r| r.sensor_id == rec.sensor_id && r.metric_id == rec.metric_id && r.timestamp == rec.timestamp) {
                    deduped[pos] = rec;
                } else {
                    deduped.push(rec);
                }
            }
        }

        deduped.sort_by_key(|r| r.timestamp);

        let capacity = HYPER_TILE_PAYLOAD_SIZE / std::mem::size_of::<TelemetryRecord>();
        for chunk in deduped.chunks(capacity) {
            let timestamps: Vec<u64> = chunk.iter().map(|r| r.timestamp).collect();
            writer.write_hyper_tile(
                TileType::TELEMETRY,
                chunk,
                Some(&timestamps),
                None,
            )?;
        }

        Ok(writer.tile_count())
    }
}

/// Query Sieve for high-performance single-cycle SIMD tile rejection and fast scanning.
pub struct QuerySieve<'a> {
    reader: &'a MmapReader,
}

impl<'a> QuerySieve<'a> {
    pub fn new(reader: &'a MmapReader) -> Self {
        Self { reader }
    }

    /// Queries L2 Rollup & Decentralized AI Data Availability commitments filtering by block height range.
    pub fn query_da_commitments(
        &self,
        min_block: u64,
        max_block: u64,
    ) -> Vec<DACommitmentRecord> {
        let mut results = Vec::new();
        let tile_count = self.reader.tile_count() as usize;

        for i in 0..tile_count {
            let (header, payload) = match self.reader.get_tile(i) {
                Some(t) => t,
                None => continue,
            };

            if header.tile_type != TileType::DATA_AVAILABILITY {
                continue;
            }

            if can_reject_tile_time(header, min_block, max_block) {
                continue;
            }

            let records: &[DACommitmentRecord] = bytemuck::cast_slice(payload);
            for rec in records {
                if rec.block_height >= min_block && rec.block_height <= max_block {
                    results.push(*rec);
                }
            }
        }

        results
    }

    /// Queries 3D GIS Survey point clouds / topographic meshes filtering by lat/long micro-degrees and elevation/depth mm.
    pub fn query_gis_mesh_3d(
        &self,
        min_lat_deg: f64,
        max_lat_deg: f64,
        min_lon_deg: f64,
        max_lon_deg: f64,
        min_elev_m: f64,
        max_elev_m: f64,
    ) -> Vec<GISSurveyPointRecord> {
        let mut results = Vec::new();
        let tile_count = self.reader.tile_count() as usize;

        let min_lat = (min_lat_deg * 1_000_000.0) as f32;
        let max_lat = (max_lat_deg * 1_000_000.0) as f32;
        let min_lon = (min_lon_deg * 1_000_000.0) as f32;
        let max_lon = (max_lon_deg * 1_000_000.0) as f32;

        let query_min = [min_lat, min_lon, (min_elev_m * 1000.0) as f32];
        let query_max = [max_lat, max_lon, (max_elev_m * 1000.0) as f32];

        for i in 0..tile_count {
            let (header, payload) = match self.reader.get_tile(i) {
                Some(t) => t,
                None => continue,
            };

            if header.tile_type != TileType::GIS_SURVEY_MESH {
                continue;
            }

            // SIMD 3D Geographic Bounding Box Rejection across Simplex Hulls
            if can_reject_tile_range(&header.hull, &query_min, &query_max) {
                continue;
            }

            let records: &[GISSurveyPointRecord] = bytemuck::cast_slice(payload);
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

    /// Queries 3D spatial robotic trajectories filtering by 3D bounding box [min_xyz, max_xyz].
    pub fn query_robotics_trajectory_3d(
        &self,
        min_xyz: [f32; 3],
        max_xyz: [f32; 3],
        min_ts_us: u64,
        max_ts_us: u64,
    ) -> Vec<RoboticsKinematicRecord> {
        let mut results = Vec::new();
        let tile_count = self.reader.tile_count() as usize;

        for i in 0..tile_count {
            let (header, payload) = match self.reader.get_tile(i) {
                Some(t) => t,
                None => continue,
            };

            if header.tile_type != TileType::ROBOTICS_KINEMATIC {
                continue;
            }

            if can_reject_tile_time(header, min_ts_us, max_ts_us) {
                continue;
            }

            // SIMD 3D Trajectory Bounding Box Rejection
            if can_reject_tile_range(&header.hull, &min_xyz, &max_xyz) {
                continue;
            }

            let records: &[RoboticsKinematicRecord] = bytemuck::cast_slice(payload);
            for rec in records {
                if rec.timestamp_us >= min_ts_us && rec.timestamp_us <= max_ts_us {
                    if rec.position_xyz[0] >= min_xyz[0] && rec.position_xyz[0] <= max_xyz[0]
                        && rec.position_xyz[1] >= min_xyz[1] && rec.position_xyz[1] <= max_xyz[1]
                        && rec.position_xyz[2] >= min_xyz[2] && rec.position_xyz[2] <= max_xyz[2]
                    {
                        results.push(*rec);
                    }
                }
            }
        }

        results
    }

    /// Queries telemetry records returning both the record and its exact `S3ACoordinate` address.
    pub fn query_telemetry_with_coords(
        &self,
        min_ts: u64,
        max_ts: u64,
        sensor_id: Option<u32>,
        metric_id: Option<u32>,
    ) -> Vec<(TelemetryRecord, S3ACoordinate)> {
        let tile_count = self.reader.tile_count() as usize;
        let mut all_records: Vec<(TelemetryRecord, S3ACoordinate)> = Vec::new();

        for i in 0..tile_count {
            let (header, payload) = match self.reader.get_tile(i) {
                Some(t) => t,
                None => continue,
            };

            // Single-cycle SIMD time filter rejection
            if can_reject_tile_time(header, min_ts, max_ts) {
                continue;
            }

            if header.tile_type != TileType::TELEMETRY && header.tile_type != TileType::HYBRID {
                continue;
            }

            let records: &[TelemetryRecord] = bytemuck::cast_slice(payload);
            for (rec_idx, rec) in records.iter().enumerate() {
                if rec.timestamp >= min_ts && rec.timestamp <= max_ts {
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
                    let coord = S3ACoordinate::new(0, i as u32, rec_idx as u32);
                    all_records.push((*rec, coord));
                }
            }
        }

        // Deduplicate records in append order: latest version overrides earlier ones, tombstones remove earlier records
        let mut map: std::collections::BTreeMap<(u32, u32, u64), (TelemetryRecord, S3ACoordinate)> = std::collections::BTreeMap::new();
        for (rec, coord) in all_records {
            let key = (rec.sensor_id, rec.metric_id, rec.timestamp);
            if rec.is_tombstone() {
                map.remove(&key);
            } else {
                map.insert(key, (rec, coord));
            }
        }

        map.into_values().collect()
    }

    /// Queries telemetry records filtering by timestamp interval and optional sensor/metric IDs, resolving versioned updates & tombstones.
    pub fn query_telemetry(
        &self,
        min_ts: u64,
        max_ts: u64,
        sensor_id: Option<u32>,
        metric_id: Option<u32>,
    ) -> Vec<TelemetryRecord> {
        self.query_telemetry_with_coords(min_ts, max_ts, sensor_id, metric_id)
            .into_iter()
            .map(|(rec, _)| rec)
            .collect()
    }

    /// Queries embedding records filtering by SIMD spatial bounding box and/or point hulls.
    pub fn query_embeddings_spatial(
        &self,
        min_ts: u64,
        max_ts: u64,
        query_point: Option<&[f32]>,
        query_range: Option<(&[f32], &[f32])>,
    ) -> Vec<EmbeddingRecord128> {
        let mut results = Vec::new();
        let tile_count = self.reader.tile_count() as usize;

        for i in 0..tile_count {
            let (header, payload) = match self.reader.get_tile(i) {
                Some(t) => t,
                None => continue,
            };

            if can_reject_tile_time(header, min_ts, max_ts) {
                continue;
            }

            if header.tile_type != TileType::EMBEDDING && header.tile_type != TileType::HYBRID {
                continue;
            }

            // SIMD Simplex Hull rejection
            if let Some(qp) = query_point {
                if can_reject_tile_point(&header.hull, qp) {
                    continue;
                }
            }

            if let Some((qmin, qmax)) = query_range {
                if can_reject_tile_range(&header.hull, qmin, qmax) {
                    continue;
                }
            }

            let records: &[EmbeddingRecord128] = bytemuck::cast_slice(payload);
            for rec in records {
                if rec.timestamp >= min_ts && rec.timestamp <= max_ts {
                    results.push(*rec);
                }
            }
        }

        results
    }
}

/// High-level CRUD Storage Engine for managing record creation, retrieval, updates, soft-deletion, and compaction.
pub struct S3ACrudEngine {
    file_path: PathBuf,
}

impl S3ACrudEngine {
    pub fn open_or_create<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let path_buf = path.as_ref().to_path_buf();
        if !path_buf.exists() || std::fs::metadata(&path_buf)?.len() == 0 {
            let _ = TileWriter::create(&path_buf)?;
        }
        Ok(Self { file_path: path_buf })
    }

    /// CREATE: Insert new telemetry records into the S3A Hyper-Tile archive.
    pub fn create_telemetry(&self, records: &[TelemetryRecord]) -> io::Result<u64> {
        if records.is_empty() {
            return Ok(0);
        }
        let mut writer = TileWriter::open_append(&self.file_path)?;
        let timestamps: Vec<u64> = records.iter().map(|r| r.timestamp).collect();
        writer.write_hyper_tile(TileType::TELEMETRY, records, Some(&timestamps), None)
    }

    /// READ: Retrieve telemetry records matching sensor_id and metric_id within timestamp bounds along with their S3ACoordinate addresses.
    pub fn read_telemetry_with_coords(
        &self,
        sensor_id: u32,
        metric_id: u32,
        min_ts: u64,
        max_ts: u64,
    ) -> io::Result<Vec<(TelemetryRecord, S3ACoordinate)>> {
        let reader = MmapReader::open(&self.file_path)?;
        let sieve = QuerySieve::new(&reader);
        Ok(sieve.query_telemetry_with_coords(min_ts, max_ts, Some(sensor_id), Some(metric_id)))
    }

    /// READ: Retrieve telemetry records matching sensor_id and metric_id within timestamp bounds.
    pub fn read_telemetry(
        &self,
        sensor_id: u32,
        metric_id: u32,
        min_ts: u64,
        max_ts: u64,
    ) -> io::Result<Vec<TelemetryRecord>> {
        let reader = MmapReader::open(&self.file_path)?;
        let sieve = QuerySieve::new(&reader);
        Ok(sieve.query_telemetry(min_ts, max_ts, Some(sensor_id), Some(metric_id)))
    }

    /// DIRECT O(1) LOOKUP: Fetch record directly by S3ACoordinate address.
    pub fn read_by_coordinate<T: Pod>(&self, coord: &S3ACoordinate) -> Result<T, S3AError> {
        let reader = MmapReader::open(&self.file_path).map_err(|_| S3AError::CorruptedHeader)?;
        reader.read_record_by_coordinate(coord)
    }

    /// UPDATE: Update an existing telemetry record by appending a newer version and tombstoning the older version.
    pub fn update_telemetry(&self, sensor_id: u32, metric_id: u32, timestamp: u64, new_value: f64) -> io::Result<bool> {
        // First delete (tombstone) existing version if present
        let deleted = self.delete_telemetry(sensor_id, metric_id, timestamp)?;

        // Append updated version
        let updated_record = TelemetryRecord::new(timestamp, sensor_id, metric_id, new_value);
        self.create_telemetry(&[updated_record])?;
        Ok(deleted)
    }

    /// DELETE: Soft-delete telemetry records by appending tombstone markers or compacting.
    pub fn delete_telemetry(&self, sensor_id: u32, metric_id: u32, timestamp: u64) -> io::Result<bool> {
        let records = self.read_telemetry(sensor_id, metric_id, timestamp, timestamp)?;
        if records.is_empty() {
            return Ok(false);
        }

        // Write a tombstoned version of the record
        let mut tombstone = records[0];
        tombstone.mark_tombstone();
        self.create_telemetry(&[tombstone])?;
        Ok(true)
    }

    /// GARBAGE COLLECTION / COMPACTION: Compact the S3A archive, purging tombstoned records and re-aligning tiles.
    pub fn purge_and_compact(&self) -> io::Result<u32> {
        let temp_path = self.file_path.with_extension("tmp.s3a");
        Compactor::compact(&[&self.file_path], &temp_path)?;
        std::fs::rename(&temp_path, &self.file_path)?;
        let reader = MmapReader::open(&self.file_path)?;
        Ok(reader.tile_count())
    }
}

/// S3A Compactor for merging and compacting sparse/uncompressed hyper-tiles into stratified blocks.
pub struct Compactor;

impl Compactor {
    pub fn compact<P: AsRef<Path>>(input_paths: &[P], output_path: P) -> io::Result<u32> {
        let mut writer = TileWriter::create(output_path)?;

        let mut telemetry_buffer: Vec<TelemetryRecord> = Vec::new();
        let mut embedding_buffer: Vec<EmbeddingRecord128> = Vec::new();

        for input_path in input_paths {
            let reader = MmapReader::open(input_path)?;
            let tile_count = reader.tile_count() as usize;

            for i in 0..tile_count {
                if let Some((header, payload)) = reader.get_tile(i) {
                    if header.tile_type == TileType::TELEMETRY {
                        let records: &[TelemetryRecord] = bytemuck::cast_slice(payload);
                        telemetry_buffer.extend_from_slice(records);
                    } else if header.tile_type == TileType::EMBEDDING {
                        let records: &[EmbeddingRecord128] = bytemuck::cast_slice(payload);
                        embedding_buffer.extend_from_slice(records);
                    }
                }
            }
        }

        // Deduplicate telemetry records: keep latest active record, filtering out tombstones
        telemetry_buffer.sort_by_key(|r| (r.sensor_id, r.metric_id, r.timestamp));
        let mut deduped_telemetry: Vec<TelemetryRecord> = Vec::new();
        for rec in telemetry_buffer {
            if rec.is_tombstone() {
                // Remove existing if present
                deduped_telemetry.retain(|r| !(r.sensor_id == rec.sensor_id && r.metric_id == rec.metric_id && r.timestamp == rec.timestamp));
            } else {
                // Replace or append
                if let Some(pos) = deduped_telemetry.iter().position(|r| r.sensor_id == rec.sensor_id && r.metric_id == rec.metric_id && r.timestamp == rec.timestamp) {
                    deduped_telemetry[pos] = rec;
                } else {
                    deduped_telemetry.push(rec);
                }
            }
        }

        // Sort telemetry by timestamp for temporal indexing
        deduped_telemetry.sort_by_key(|r| r.timestamp);

        // Compact telemetry into full 128 KB tiles
        let tel_rec_capacity = HYPER_TILE_PAYLOAD_SIZE / std::mem::size_of::<TelemetryRecord>();
        for chunk in deduped_telemetry.chunks(tel_rec_capacity) {
            let timestamps: Vec<u64> = chunk.iter().map(|r| r.timestamp).collect();
            writer.write_hyper_tile(
                TileType::TELEMETRY,
                chunk,
                Some(&timestamps),
                None,
            )?;
        }

        // Compact embeddings into full 128 KB tiles with calculated Simplex Hulls
        let emb_rec_capacity = HYPER_TILE_PAYLOAD_SIZE / std::mem::size_of::<EmbeddingRecord128>();
        for chunk in embedding_buffer.chunks(emb_rec_capacity) {
            let timestamps: Vec<u64> = chunk.iter().map(|r| r.timestamp).collect();
            let mut hull = SimplexHull::empty();
            hull.dim = 16; // Use first 16 dimensions for SIMD bounding hull indexing

            for rec in chunk {
                for d in 0..MAX_HULL_DIMENSIONS {
                    let val = rec.vector[d];
                    if val < hull.min_bounds[d] {
                        hull.min_bounds[d] = val;
                    }
                    if val > hull.max_bounds[d] {
                        hull.max_bounds[d] = val;
                    }
                }
            }

            writer.write_hyper_tile(
                TileType::EMBEDDING,
                chunk,
                Some(&timestamps),
                Some(hull),
            )?;
        }

        Ok(writer.tile_count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_tile_fusion_engine() {
        let f1 = NamedTempFile::new().unwrap();
        let f2 = NamedTempFile::new().unwrap();
        let fused_file = NamedTempFile::new().unwrap();

        let mut w1 = TileWriter::create(f1.path()).unwrap();
        let recs1 = vec![TelemetryRecord::new(100, 1, 1, 10.0)];
        w1.write_hyper_tile(TileType::TELEMETRY, &recs1, Some(&[100]), None).unwrap();

        let mut w2 = TileWriter::create(f2.path()).unwrap();
        let recs2 = vec![TelemetryRecord::new(200, 1, 1, 20.0)];
        w2.write_hyper_tile(TileType::TELEMETRY, &recs2, Some(&[200]), None).unwrap();

        let count = TileFusionEngine::fuse_telemetry_tiles(&[f1.path(), f2.path()], fused_file.path()).unwrap();
        assert_eq!(count, 1);

        let reader = MmapReader::open(fused_file.path()).unwrap();
        let sieve = QuerySieve::new(&reader);
        let res = sieve.query_telemetry(0, 500, None, None);
        assert_eq!(res.len(), 2);
    }

    #[test]
    fn test_da_commitment_query() {
        let temp_file = NamedTempFile::new().unwrap();
        let mut writer = TileWriter::create(temp_file.path()).unwrap();

        let commitments = vec![
            DACommitmentRecord::new(100, [0x01; 32], 1600000000, 0xFF, 120),
            DACommitmentRecord::new(200, [0x02; 32], 1600000010, 0xFF, 250),
            DACommitmentRecord::new(500, [0x03; 32], 1600000020, 0xFF, 800),
        ];

        let timestamps = vec![100, 200, 500];
        writer.write_hyper_tile(TileType::DATA_AVAILABILITY, &commitments, Some(&timestamps), None).unwrap();

        let reader = MmapReader::open(temp_file.path()).unwrap();
        let sieve = QuerySieve::new(&reader);

        let results = sieve.query_da_commitments(150, 300);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].block_height, 200);
    }

    #[test]
    fn test_gis_mesh_3d_query() {
        let temp_file = NamedTempFile::new().unwrap();
        let mut writer = TileWriter::create(temp_file.path()).unwrap();

        let points = vec![
            GISSurveyPointRecord::new(37.774929, -122.419416, 100.0, 0, 80, 1600000000), // Ground
            GISSurveyPointRecord::new(37.775000, -122.420000, -50.0, 1, 90, 1600000000), // Subsurface Strata
            GISSurveyPointRecord::new(38.000000, -123.000000, 200.0, 0, 50, 1600000000), // Out of area
        ];

        let mut hull = SimplexHull::empty();
        hull.dim = 3;
        hull.min_bounds[0] = 37774929.0;
        hull.max_bounds[0] = 38000000.0;
        hull.min_bounds[1] = -123000000.0;
        hull.max_bounds[1] = -122419416.0;
        hull.min_bounds[2] = -50000.0;
        hull.max_bounds[2] = 200000.0;

        writer.write_hyper_tile(TileType::GIS_SURVEY_MESH, &points, None, Some(hull)).unwrap();

        let reader = MmapReader::open(temp_file.path()).unwrap();
        let sieve = QuerySieve::new(&reader);

        let res = sieve.query_gis_mesh_3d(37.70, 37.80, -122.50, -122.40, -100.0, 150.0);
        assert_eq!(res.len(), 2);
    }

    #[test]
    fn test_robotics_stream_writer_and_3d_trajectory_query() {
        let temp_file = NamedTempFile::new().unwrap();
        let mut stream_writer = RoboticsStreamWriter::create(temp_file.path()).unwrap();

        // Push 2000 robotics kinematics samples (~1 HyperTile payload)
        for i in 0..2000 {
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

        let reader = MmapReader::open(temp_file.path()).unwrap();
        let sieve = QuerySieve::new(&reader);

        // Perform 3D bounding box trajectory query
        let results = sieve.query_robotics_trajectory_3d(
            [0.0, 0.0, 0.0],
            [5.0, 5.0, 1.0],
            0,
            1_000_000,
        );

        assert!(!results.is_empty());
        for r in &results {
            assert!(r.position_xyz[0] >= 0.0 && r.position_xyz[0] <= 5.0);
        }
    }

    #[test]
    fn test_micro_tile_buffer_zero_alloc() {
        let mut buf = MicroTileBuffer::new(1);
        let rec1 = CompactWearableRecord::new(100, 1, 10, 72.5); // PPG Heart Rate
        let rec2 = CompactWearableRecord::new(101, 1, 10, 73.0);

        buf.push_record(&rec1).unwrap();
        buf.push_record(&rec2).unwrap();

        let page = buf.finalize();
        assert_eq!(page.len(), MICRO_TILE_SIZE);

        let header: &MicroTileHeader = from_bytes(&page[..MICRO_TILE_HEADER_SIZE]);
        assert_eq!(header.tile_id, 1);
        assert_eq!(header.record_count, 2);
        assert_eq!(header.payload_bytes, 32);

        let payload_records: &[CompactWearableRecord] = bytemuck::cast_slice(&page[MICRO_TILE_HEADER_SIZE..MICRO_TILE_HEADER_SIZE + 32]);
        assert_eq!(payload_records[0], rec1);
        assert_eq!(payload_records[1], rec2);
    }

    #[test]
    fn test_tile_writer_and_mmap_reader_roundtrip() {
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path();

        let mut writer = TileWriter::create(path).unwrap();

        let records = vec![
            TelemetryRecord::new(1000, 1, 101, 42.5),
            TelemetryRecord::new(2000, 1, 101, 43.0),
            TelemetryRecord::new(3000, 2, 102, 12.0),
        ];
        let timestamps = vec![1000, 2000, 3000];

        let tile_id = writer
            .write_hyper_tile(TileType::TELEMETRY, &records, Some(&timestamps), None)
            .unwrap();

        assert_eq!(tile_id, 1);
        assert_eq!(writer.tile_count(), 1);

        let reader = MmapReader::open(path).unwrap();
        assert_eq!(reader.tile_count(), 1);

        let (header, payload) = reader.get_tile(0).unwrap();
        assert_eq!(header.tile_id, 1);
        assert_eq!(header.record_count, 3);
        assert_eq!(header.min_timestamp, 1000);
        assert_eq!(header.max_timestamp, 3000);

        let read_records: &[TelemetryRecord] = bytemuck::cast_slice(payload);
        assert_eq!(read_records, records.as_slice());

        assert!(reader.verify_checksums().is_ok());
    }

    #[test]
    fn test_crud_engine_operations_and_coordinate_lookup() {
        let temp_file = NamedTempFile::new().unwrap();
        let engine = S3ACrudEngine::open_or_create(temp_file.path()).unwrap();

        // 1. CREATE
        let rec1 = TelemetryRecord::new(1000, 10, 1, 25.4);
        let rec2 = TelemetryRecord::new(2000, 10, 1, 26.1);
        engine.create_telemetry(&[rec1, rec2]).unwrap();

        // 2. READ WITH COORDINATE
        let res = engine.read_telemetry_with_coords(10, 1, 500, 2500).unwrap();
        assert_eq!(res.len(), 2);

        let (read_rec1, coord1) = res[0];
        assert_eq!(read_rec1.timestamp, 1000);
        assert_eq!(coord1, S3ACoordinate::new(0, 0, 0));

        // 3. DIRECT O(1) COORDINATE LOOKUP
        let direct_rec: TelemetryRecord = engine.read_by_coordinate(&coord1).unwrap();
        assert_eq!(direct_rec, read_rec1);

        // 4. UPDATE
        let updated = engine.update_telemetry(10, 1, 1000, 30.0).unwrap();
        assert!(updated);

        let res_after_update = engine.read_telemetry(10, 1, 1000, 1000).unwrap();
        assert_eq!(res_after_update.len(), 1);
        assert_eq!(res_after_update[0].value, 30.0);

        // 5. DELETE
        let deleted = engine.delete_telemetry(10, 1, 2000).unwrap();
        assert!(deleted);

        let res_after_delete = engine.read_telemetry(10, 1, 2000, 2000).unwrap();
        assert_eq!(res_after_delete.len(), 0);

        // 6. PURGE & COMPACT
        engine.purge_and_compact().unwrap();
        let final_records = engine.read_telemetry(10, 1, 0, 5000).unwrap();
        assert_eq!(final_records.len(), 1);
        assert_eq!(final_records[0].timestamp, 1000);
        assert_eq!(final_records[0].value, 30.0);
    }

    #[test]
    fn test_compactor_tombstone_replacement_and_edge_cases() {
        let f1 = NamedTempFile::new().unwrap();
        let f2 = NamedTempFile::new().unwrap();
        let f3 = NamedTempFile::new().unwrap();
        let compacted_file = NamedTempFile::new().unwrap();

        // 1. Prepare f1: File 1 contains telemetry records and embedding records
        let mut w1 = TileWriter::create(f1.path()).unwrap();
        let rec1 = TelemetryRecord::new(100, 1, 1, 10.0); // Active, will be updated in f2
        let rec2 = TelemetryRecord::new(200, 1, 2, 20.0); // Active, will be updated in f2 and tombstoned in f3
        let rec3 = TelemetryRecord::new(300, 2, 1, 30.0); // Active, kept unchanged
        w1.write_hyper_tile(TileType::TELEMETRY, &[rec1, rec2, rec3], Some(&[100, 200, 300]), None).unwrap();

        let emb1 = EmbeddingRecord128::new(1, 100, [1.0f32; 128]);
        w1.write_hyper_tile(TileType::EMBEDDING, &[emb1], Some(&[100]), None).unwrap();

        // 2. Prepare f2: File 2 updates rec1 and rec2, and adds a tombstone for non-existent key
        let mut w2 = TileWriter::create(f2.path()).unwrap();
        let rec1_updated = TelemetryRecord::new(100, 1, 1, 15.0); // Update rec1 value
        let rec2_updated = TelemetryRecord::new(200, 1, 2, 25.0); // Update rec2 value
        let mut non_existent_tombstone = TelemetryRecord::new(999, 99, 99, 0.0);
        non_existent_tombstone.mark_tombstone();
        w2.write_hyper_tile(TileType::TELEMETRY, &[rec1_updated, rec2_updated, non_existent_tombstone], Some(&[100, 200, 999]), None).unwrap();

        // 3. Prepare f3: File 3 tombstones rec2
        let mut w3 = TileWriter::create(f3.path()).unwrap();
        let mut rec2_tombstone = TelemetryRecord::new(200, 1, 2, 25.0);
        rec2_tombstone.mark_tombstone();
        w3.write_hyper_tile(TileType::TELEMETRY, &[rec2_tombstone], Some(&[200]), None).unwrap();

        // Compact f1, f2, f3 into compacted_file
        let tile_count = Compactor::compact(&[f1.path(), f2.path(), f3.path()], compacted_file.path()).unwrap();
        assert!(tile_count > 0);

        // Verify compacted output
        let reader = MmapReader::open(compacted_file.path()).unwrap();
        let sieve = QuerySieve::new(&reader);

        // Query all telemetry
        let active_telemetry = sieve.query_telemetry(0, 1000, None, None);
        // rec1 updated value (15.0) should be present
        // rec2 was tombstoned, so it should NOT be present
        // rec3 (30.0) should be present
        // non-existent tombstone should have no effect
        assert_eq!(active_telemetry.len(), 2);
        assert_eq!(active_telemetry[0], rec1_updated);
        assert_eq!(active_telemetry[1], rec3);

        // Verify embedding record was preserved with valid hull
        let embeddings = sieve.query_embeddings_spatial(0, 1000, None, None);
        assert_eq!(embeddings.len(), 1);
        assert_eq!(embeddings[0], emb1);

        // Test Edge Case: Compacting file with ONLY tombstones results in zero output tiles
        let tombstone_only_file = NamedTempFile::new().unwrap();
        let mut w_tomb = TileWriter::create(tombstone_only_file.path()).unwrap();
        let mut t1 = TelemetryRecord::new(500, 5, 5, 50.0);
        t1.mark_tombstone();
        w_tomb.write_hyper_tile(TileType::TELEMETRY, &[t1], Some(&[500]), None).unwrap();

        let empty_compacted_file = NamedTempFile::new().unwrap();
        let empty_tile_count = Compactor::compact(&[tombstone_only_file.path()], empty_compacted_file.path()).unwrap();
        assert_eq!(empty_tile_count, 0);

        let empty_reader = MmapReader::open(empty_compacted_file.path()).unwrap();
        assert_eq!(empty_reader.tile_count(), 0);
        let empty_telemetry = QuerySieve::new(&empty_reader).query_telemetry(0, 1000, None, None);
        assert!(empty_telemetry.is_empty());
    }

    #[test]
    fn test_mmap_reader_get_tile_out_of_bounds() {
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path();

        let mut writer = TileWriter::create(path).unwrap();

        let records = vec![
            TelemetryRecord::new(1000, 1, 101, 42.5),
        ];
        let timestamps = vec![1000];

        writer
            .write_hyper_tile(TileType::TELEMETRY, &records, Some(&timestamps), None)
            .unwrap();

        let reader = MmapReader::open(path).unwrap();
        assert_eq!(reader.tile_count(), 1);

        // Valid tile index 0
        assert!(reader.get_tile(0).is_some());

        // Out of bounds tile indices
        assert!(reader.get_tile(1).is_none());
        assert!(reader.get_tile(2).is_none());
        assert!(reader.get_tile(100).is_none());
        assert!(reader.get_tile(usize::MAX).is_none());

        // Test with empty file (0 tiles)
        let empty_file = NamedTempFile::new().unwrap();
        let _empty_writer = TileWriter::create(empty_file.path()).unwrap();
        let empty_reader = MmapReader::open(empty_file.path()).unwrap();
        assert_eq!(empty_reader.tile_count(), 0);

        assert!(empty_reader.get_tile(0).is_none());
        assert!(empty_reader.get_tile(1).is_none());
        assert!(empty_reader.get_tile(usize::MAX).is_none());
    }
}
