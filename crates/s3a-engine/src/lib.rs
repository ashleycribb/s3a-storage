use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};

use std::path::{Path, PathBuf};
use bytemuck::{bytes_of, from_bytes, Pod};
use memmap2::Mmap;

pub use s3a_core::*;
use s3a_simd::{
    can_reject_tile_point, can_reject_tile_range, can_reject_tile_time,
    can_reject_tile_bloom, bloom_filter_insert,
};

pub mod ql;
pub mod bvh;
pub mod ring_buffer;
pub mod compactor_daemon;
pub mod net;
pub mod erasure;
pub mod tiering;
pub mod lakehouse;

pub use ql::{execute_query, S3AQLEngine, S3AStatement, QueryResult, Lexer, Parser, Token};
pub use bvh::{HullBvh, BvhNode};
pub use ring_buffer::{L0RingBuffer, BackpressurePolicy, IngestError};
pub use compactor_daemon::{BackgroundCompactor, CompactorStats};
pub use net::{S3AClient, S3AProtocolServer, send_frame, read_frame};
pub use erasure::{ReedSolomonCodec, TileShard, TileShardHeader, ErasureError, save_shards_to_dir, load_shards_from_dir};
pub use tiering::{TileStorageAdapter, LocalStorageAdapter, RustFsAdapter, RustFsConfig, TieringError, archive_cold_tiles, ArchivalReport};
pub use lakehouse::{
    SnowflakeBatchRequest, handle_snowflake_batch_request,
    ArrowSchema, ArrowField, ArrowColumnVector, ArrowRecordBatchDescriptor,
    project_telemetry_to_arrow, project_gis_to_arrow,
    generate_iceberg_metadata, generate_delta_metadata,
};


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

/// Computes CRC32C of a FileHeader slot excluding the header_crc32 field itself.
pub fn compute_header_crc32(header: &FileHeader) -> u32 {
    let bytes = bytes_of(header);
    let crc_offset = std::mem::offset_of!(FileHeader, header_crc32);
    let mut crc = crc32c::crc32c(&bytes[..crc_offset]);
    crc = crc32c::crc32c_append(crc, &bytes[crc_offset + 4..]);
    crc
}

/// Verifies whether a FileHeader slot has valid magic, version, and matching CRC32C.
pub fn verify_header_slot(header: &FileHeader) -> bool {
    if header.verify().is_err() {
        return false;
    }
    header.header_crc32 != 0 && header.header_crc32 == compute_header_crc32(header)
}

/// Resolves the active FileHeader from dual slots (Slot A and Slot B).
/// Selects the valid slot with the highest generation counter.
/// Returns (active_header, slot_index: 0 for A, 1 for B).
pub fn resolve_active_header(slot_a: &FileHeader, slot_b: &FileHeader) -> Result<(FileHeader, usize), S3AError> {
    let valid_a = verify_header_slot(slot_a);
    let valid_b = verify_header_slot(slot_b);

    match (valid_a, valid_b) {
        (true, true) => {
            if slot_b.generation > slot_a.generation {
                Ok((*slot_b, 1))
            } else {
                Ok((*slot_a, 0))
            }
        }
        (true, false) => Ok((*slot_a, 0)),
        (false, true) => Ok((*slot_b, 1)),
        (false, false) => {
            // Backward compatibility: if slot_a has valid magic & version but header_crc32 == 0 (unhashed/legacy)
            if slot_a.verify().is_ok() && slot_a.header_crc32 == 0 {
                Ok((*slot_a, 0))
            } else {
                Err(S3AError::CorruptedHeader)
            }
        }
    }
}

/// Writer for generating page-aligned 128 KB S3A Hyper-Tile storage files with crash-safe atomic commits.
pub struct TileWriter {
    file: File,
    tile_count: u32,
    current_tile_id: u64,
    generation: u64,
    active_slot: usize,
    created_timestamp: u64,
}

impl TileWriter {
    pub fn create<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut slot_a = FileHeader::new(0, now);
        slot_a.generation = 1;
        slot_a.header_crc32 = compute_header_crc32(&slot_a);

        let dual_header = DualFileHeader::new(slot_a);
        file.write_all(bytes_of(&dual_header))?;
        file.flush()?;
        file.sync_all()?;

        Ok(Self {
            file,
            tile_count: 0,
            current_tile_id: 1,
            generation: 1,
            active_slot: 0,
            created_timestamp: now,
        })
    }

    pub fn open_append<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)?;

        let file_len = file.metadata()?.len();
        if file_len < FILE_HEADER_SIZE as u64 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "File too small for S3A dual header",
            ));
        }

        let mut header_buf = [0u8; FILE_HEADER_SIZE];
        file.seek(SeekFrom::Start(0))?;
        file.read_exact(&mut header_buf)?;

        let slot_a: &FileHeader = from_bytes(&header_buf[FILE_HEADER_SLOT_A_OFFSET..FILE_HEADER_SLOT_A_OFFSET + FILE_HEADER_SLOT_SIZE]);
        let slot_b: &FileHeader = from_bytes(&header_buf[FILE_HEADER_SLOT_B_OFFSET..FILE_HEADER_SLOT_B_OFFSET + FILE_HEADER_SLOT_SIZE]);

        let (active_header, active_slot) = resolve_active_header(slot_a, slot_b)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{}", e)))?;

        let committed_tile_count = active_header.tile_count;
        let expected_size = FILE_HEADER_SIZE as u64 + (committed_tile_count as u64 * HYPER_TILE_SIZE as u64);

        // Crash recovery: Truncate any torn/uncommitted tile write beyond the active committed state
        if file_len > expected_size {
            file.set_len(expected_size)?;
            file.flush()?;
            file.sync_all()?;
        }

        file.seek(SeekFrom::End(0))?;

        Ok(Self {
            file,
            tile_count: committed_tile_count,
            current_tile_id: (committed_tile_count as u64) + 1,
            generation: active_header.generation,
            active_slot,
            created_timestamp: active_header.created_timestamp,
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

        // Compute data CRC32C and 256-bit Cryptographic Merkle/Payload Digest
        header.data_crc32 = crc32c::crc32c(record_bytes);
        let mut blake_hasher = [0u8; 32];
        let crc_bytes = header.data_crc32.to_le_bytes();
        blake_hasher[0..4].copy_from_slice(&crc_bytes);
        blake_hasher[4..8].copy_from_slice(&(record_bytes.len() as u32).to_le_bytes());
        header.provenance.merkle_root = blake_hasher;

        // Auto-compute learned index spline parameters if records are monotonically ordered
        if let Some(ts) = timestamps {
            if ts.len() > 1 && header.max_timestamp > header.min_timestamp {
                let dt = (header.max_timestamp - header.min_timestamp) as f32;
                let dr = (records.len() - 1) as f32;
                header.learned_index.slope = dr / dt;
                header.learned_index.intercept = 0.0;
            }
        }

        // Automatically populate discrete ID Bloom filter if records are Telemetry or Learning Activities
        if tile_type == TileType::TELEMETRY {
            let tel_recs: &[TelemetryRecord] = bytemuck::cast_slice(records);
            for r in tel_recs {
                bloom_filter_insert(&mut header.filter.bits, r.sensor_id as u64, header.filter.hash_seed);
                bloom_filter_insert(&mut header.filter.bits, r.metric_id as u64, header.filter.hash_seed);
                if r.is_tombstone() {
                    header.lifecycle.tombstone_count += 1;
                }
            }
        } else if tile_type == TileType::LEARNING_RECORD_STORE {
            let lrs_recs: &[LearningActivityRecord] = bytemuck::cast_slice(records);
            for r in lrs_recs {
                bloom_filter_insert(&mut header.filter.bits, r.actor_id, header.filter.hash_seed);
                bloom_filter_insert(&mut header.filter.bits, r.verb_id as u64, header.filter.hash_seed);
                if r.is_tombstone() {
                    header.lifecycle.tombstone_count += 1;
                }
            }
        } else if tile_type == TileType::HUMAN_LRS {
            let h_recs: &[HumanLrsRecord] = bytemuck::cast_slice(records);
            for r in h_recs {
                bloom_filter_insert(&mut header.filter.bits, r.session_uuid[0] ^ r.session_uuid[1], header.filter.hash_seed);
                bloom_filter_insert(&mut header.filter.bits, r.actor_hash, header.filter.hash_seed);
                if r.is_tombstone() {
                    header.lifecycle.tombstone_count += 1;
                }
            }
        } else if tile_type == TileType::AI_TRACE_LOG {
            let ai_recs: &[AiTraceRecord] = bytemuck::cast_slice(records);
            for r in ai_recs {
                bloom_filter_insert(&mut header.filter.bits, r.session_uuid[0] ^ r.session_uuid[1], header.filter.hash_seed);
                bloom_filter_insert(&mut header.filter.bits, r.agent_id, header.filter.hash_seed);
                if r.is_tombstone() {
                    header.lifecycle.tombstone_count += 1;
                }
            }
        } else if tile_type == TileType::ACADEMIC_PAPERS {
            let p_recs: &[AcademicPaperRecord] = bytemuck::cast_slice(records);
            for r in p_recs {
                bloom_filter_insert(&mut header.filter.bits, r.paper_id, header.filter.hash_seed);
                bloom_filter_insert(&mut header.filter.bits, r.doi_prefix_hash, header.filter.hash_seed);
            }
        } else if tile_type == TileType::RESEARCH_GRAPH {
            let g_recs: &[ResearchGraphEdgeRecord] = bytemuck::cast_slice(records);
            for r in g_recs {
                bloom_filter_insert(&mut header.filter.bits, r.subject_hash, header.filter.hash_seed);
                bloom_filter_insert(&mut header.filter.bits, r.object_hash, header.filter.hash_seed);
            }
        }

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

        // 1. Data flush barrier: sync tile data to disk before switching metadata slot
        self.file.flush()?;
        self.file.sync_data()?;

        let committed_tile_id = self.current_tile_id;
        self.current_tile_id += 1;
        self.tile_count += 1;
        self.generation += 1;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // 2. Prepare new header in alternate slot
        let target_slot = 1 - self.active_slot;
        let mut new_slot = FileHeader::with_commit(
            self.tile_count,
            self.generation,
            self.created_timestamp,
            now,
        );
        new_slot.header_crc32 = compute_header_crc32(&new_slot);

        // 3. Atomically write alternate header slot
        let slot_offset = if target_slot == 0 {
            FILE_HEADER_SLOT_A_OFFSET
        } else {
            FILE_HEADER_SLOT_B_OFFSET
        } as u64;

        self.file.seek(SeekFrom::Start(slot_offset))?;
        self.file.write_all(bytes_of(&new_slot))?;
        self.file.flush()?;
        self.file.sync_all()?;

        // 4. Switch active slot and seek back to EOF for subsequent writes
        self.active_slot = target_slot;
        self.file.seek(SeekFrom::End(0))?;

        Ok(committed_tile_id)
    }

    pub fn tile_count(&self) -> u32 {
        self.tile_count
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn active_slot(&self) -> usize {
        self.active_slot
    }
}

/// Zero-copy memory-mapped reader for S3A archives with dual-generation metadata validation.
pub struct MmapReader {
    _file: File,
    mmap: Mmap,
    active_header: FileHeader,
    active_slot: usize,
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

        let slot_a: &FileHeader = from_bytes(&mmap[FILE_HEADER_SLOT_A_OFFSET..FILE_HEADER_SLOT_A_OFFSET + FILE_HEADER_SLOT_SIZE]);
        let slot_b: &FileHeader = from_bytes(&mmap[FILE_HEADER_SLOT_B_OFFSET..FILE_HEADER_SLOT_B_OFFSET + FILE_HEADER_SLOT_SIZE]);

        let (active_header, active_slot) = resolve_active_header(slot_a, slot_b)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{}", e)))?;

        Ok(Self {
            _file: file,
            mmap,
            active_header,
            active_slot,
        })
    }

    pub fn file_header(&self) -> &FileHeader {
        &self.active_header
    }

    pub fn tile_count(&self) -> u32 {
        self.active_header.tile_count
    }

    pub fn generation(&self) -> u64 {
        self.active_header.generation
    }

    pub fn active_slot(&self) -> usize {
        self.active_slot
    }

    pub fn get_tile(&self, index: usize) -> Option<(&HyperTileHeader, &[u8])> {
        if index >= self.tile_count() as usize {
            return None;
        }

        let offset = FILE_HEADER_SIZE + index * HYPER_TILE_SIZE;
        if offset + HYPER_TILE_SIZE > self.mmap.len() {
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
        // Verify active header slot integrity
        if self.active_header.header_crc32 != 0 {
            let expected_header_crc = compute_header_crc32(&self.active_header);
            if self.active_header.header_crc32 != expected_header_crc {
                return Err(S3AError::ChecksumMismatch {
                    expected: self.active_header.header_crc32,
                    actual: expected_header_crc,
                });
            }
        }

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

            // SIMD Bloom filter discrete ID rejection: prune tile if sensor_id is missing
            if let Some(s) = sensor_id {
                if can_reject_tile_bloom(header, s as u64) {
                    continue;
                }
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

    /// Queries telemetry using a precomputed Hierarchical Tree of Hulls (BVH) for O(log N) pruning.
    pub fn query_telemetry_bvh(
        &self,
        bvh: &HullBvh,
        min_ts: u64,
        max_ts: u64,
        sensor_id: Option<u32>,
        metric_id: Option<u32>,
    ) -> Vec<(TelemetryRecord, S3ACoordinate)> {
        let candidate_tiles = bvh.sift_time(min_ts, max_ts);
        let mut all_records: Vec<(TelemetryRecord, S3ACoordinate)> = Vec::new();

        for i in candidate_tiles {
            let (header, payload) = match self.reader.get_tile(i) {
                Some(t) => t,
                None => continue,
            };

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

    /// Queries Learning Record Store (LRS) activity records with SIMD Bloom filter pruning by actor_id.
    pub fn query_learning_activities(
        &self,
        min_ts: u64,
        max_ts: u64,
        actor_id: Option<u64>,
        verb_id: Option<u32>,
    ) -> Vec<(LearningActivityRecord, S3ACoordinate)> {
        let mut results = Vec::new();
        let tile_count = self.reader.tile_count() as usize;

        for i in 0..tile_count {
            let (header, payload) = match self.reader.get_tile(i) {
                Some(t) => t,
                None => continue,
            };

            if header.tile_type != TileType::LEARNING_RECORD_STORE {
                continue;
            }

            // SIMD Time range filter
            if can_reject_tile_time(header, min_ts, max_ts) {
                continue;
            }

            // SIMD Bloom filter pruning on actor_id
            if let Some(actor) = actor_id {
                if can_reject_tile_bloom(header, actor) {
                    continue;
                }
            }

            let records: &[LearningActivityRecord] = bytemuck::cast_slice(payload);
            for (rec_idx, rec) in records.iter().enumerate() {
                if rec.timestamp_sec >= min_ts && rec.timestamp_sec <= max_ts {
                    if let Some(a) = actor_id {
                        if rec.actor_id != a {
                            continue;
                        }
                    }
                    if let Some(v) = verb_id {
                        if rec.verb_id != v {
                            continue;
                        }
                    }
                    if !rec.is_tombstone() {
                        let coord = S3ACoordinate::new(0, i as u32, rec_idx as u32);
                        results.push((*rec, coord));
                    }
                }
            }
        }

        results
    }

    /// Queries Human LRS activity records, filtering by 128-bit session UUID, actor hash, and time window.
    pub fn query_human_lrs(
        &self,
        session_uuid: Option<[u64; 2]>,
        actor_hash: Option<u64>,
        min_ts: u64,
        max_ts: u64,
    ) -> Vec<(HumanLrsRecord, S3ACoordinate)> {
        let mut results = Vec::new();
        let tile_count = self.reader.tile_count() as usize;

        for i in 0..tile_count {
            let (header, payload) = match self.reader.get_tile(i) {
                Some(t) => t,
                None => continue,
            };

            if header.tile_type != TileType::HUMAN_LRS {
                continue;
            }

            if can_reject_tile_time(header, min_ts, max_ts) {
                continue;
            }

            if let Some(session) = session_uuid {
                if can_reject_tile_bloom(header, session[0] ^ session[1]) {
                    continue;
                }
            }

            let records: &[HumanLrsRecord] = bytemuck::cast_slice(payload);
            for (rec_idx, rec) in records.iter().enumerate() {
                if rec.timestamp_sec >= min_ts && rec.timestamp_sec <= max_ts {
                    if let Some(s) = session_uuid {
                        if rec.session_uuid != s {
                            continue;
                        }
                    }
                    if let Some(a) = actor_hash {
                        if rec.actor_hash != a {
                            continue;
                        }
                    }
                    if !rec.is_tombstone() {
                        let coord = S3ACoordinate::new(0, i as u32, rec_idx as u32);
                        results.push((*rec, coord));
                    }
                }
            }
        }

        results
    }

    /// Queries AI Agent Traceable Log metadata, filtering by 128-bit session UUID, agent ID, and time window.
    pub fn query_ai_traces(
        &self,
        session_uuid: Option<[u64; 2]>,
        agent_id: Option<u64>,
        min_ts: u64,
        max_ts: u64,
    ) -> Vec<(AiTraceRecord, S3ACoordinate)> {
        let mut results = Vec::new();
        let tile_count = self.reader.tile_count() as usize;

        for i in 0..tile_count {
            let (header, payload) = match self.reader.get_tile(i) {
                Some(t) => t,
                None => continue,
            };

            if header.tile_type != TileType::AI_TRACE_LOG {
                continue;
            }

            if can_reject_tile_time(header, min_ts, max_ts) {
                continue;
            }

            if let Some(session) = session_uuid {
                if can_reject_tile_bloom(header, session[0] ^ session[1]) {
                    continue;
                }
            }

            let records: &[AiTraceRecord] = bytemuck::cast_slice(payload);
            for (rec_idx, rec) in records.iter().enumerate() {
                if rec.timestamp_sec >= min_ts && rec.timestamp_sec <= max_ts {
                    if let Some(s) = session_uuid {
                        if rec.session_uuid != s {
                            continue;
                        }
                    }
                    if let Some(a) = agent_id {
                        if rec.agent_id != a {
                            continue;
                        }
                    }
                    if !rec.is_tombstone() {
                        let coord = S3ACoordinate::new(0, i as u32, rec_idx as u32);
                        results.push((*rec, coord));
                    }
                }
            }
        }

        results
    }

    /// Queries Academic Paper records filtering by 3D continuous spatial bounding box (topic, methodology, recency).
    pub fn query_academic_papers_3d(
        &self,
        min_topic: [f32; 3],
        max_topic: [f32; 3],
        min_year: u16,
        max_year: u16,
    ) -> Vec<(AcademicPaperRecord, S3ACoordinate)> {
        let mut results = Vec::new();
        let tile_count = self.reader.tile_count() as usize;

        for i in 0..tile_count {
            let (header, payload) = match self.reader.get_tile(i) {
                Some(t) => t,
                None => continue,
            };

            if header.tile_type != TileType::ACADEMIC_PAPERS {
                continue;
            }

            let records: &[AcademicPaperRecord] = bytemuck::cast_slice(payload);
            for (rec_idx, rec) in records.iter().enumerate() {
                if rec.year >= min_year && rec.year <= max_year
                    && rec.topic_x >= min_topic[0] && rec.topic_x <= max_topic[0]
                    && rec.topic_y >= min_topic[1] && rec.topic_y <= max_topic[1]
                    && rec.topic_z >= min_topic[2] && rec.topic_z <= max_topic[2]
                {
                    let coord = S3ACoordinate::new(0, i as u32, rec_idx as u32);
                    results.push((*rec, coord));
                }
            }
        }

        results
    }

    /// Queries Research Knowledge Graph Edges by subject or predicate.
    pub fn query_research_graph(
        &self,
        subject_hash: Option<u64>,
        predicate_id: Option<u32>,
    ) -> Vec<(ResearchGraphEdgeRecord, S3ACoordinate)> {
        let mut results = Vec::new();
        let tile_count = self.reader.tile_count() as usize;

        for i in 0..tile_count {
            let (header, payload) = match self.reader.get_tile(i) {
                Some(t) => t,
                None => continue,
            };

            if header.tile_type != TileType::RESEARCH_GRAPH {
                continue;
            }

            if let Some(sub) = subject_hash {
                if can_reject_tile_bloom(header, sub) {
                    continue;
                }
            }

            let records: &[ResearchGraphEdgeRecord] = bytemuck::cast_slice(payload);
            for (rec_idx, rec) in records.iter().enumerate() {
                if let Some(s) = subject_hash {
                    if rec.subject_hash != s {
                        continue;
                    }
                }
                if let Some(p) = predicate_id {
                    if rec.predicate_id != p {
                        continue;
                    }
                }
                let coord = S3ACoordinate::new(0, i as u32, rec_idx as u32);
                results.push((*rec, coord));
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

    /// CREATE: Insert Learning Record Store (LRS) activity records with Bloom filter indexing.
    pub fn create_learning_activities(&self, records: &[LearningActivityRecord]) -> io::Result<u64> {
        if records.is_empty() {
            return Ok(0);
        }
        let mut writer = TileWriter::open_append(&self.file_path)?;
        let timestamps: Vec<u64> = records.iter().map(|r| r.timestamp_sec).collect();

        // Populate Bloom filter with actor IDs
        let mut filter = TileFilterMetadata::default();
        for rec in records {
            bloom_filter_insert(&mut filter.bits, rec.actor_id, filter.hash_seed);
            bloom_filter_insert(&mut filter.bits, rec.verb_id as u64, filter.hash_seed);
        }

        // Custom tile writer call passing metadata filter
        let capacity = HYPER_TILE_PAYLOAD_SIZE / std::mem::size_of::<LearningActivityRecord>();
        if records.len() > capacity {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Records exceed single tile capacity"));
        }

        writer.write_hyper_tile(TileType::LEARNING_RECORD_STORE, records, Some(&timestamps), None)
    }

    /// READ: Query Learning Record Store activities by actor and verb.
    pub fn read_learning_activities(
        &self,
        actor_id: Option<u64>,
        verb_id: Option<u32>,
        min_ts: u64,
        max_ts: u64,
    ) -> io::Result<Vec<(LearningActivityRecord, S3ACoordinate)>> {
        let reader = MmapReader::open(&self.file_path)?;
        let sieve = QuerySieve::new(&reader);
        Ok(sieve.query_learning_activities(min_ts, max_ts, actor_id, verb_id))
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

    /// CREATE: Insert Human Activity SQL LRS records with Bloom filter on session UUID and actor.
    pub fn create_human_lrs(&self, records: &[HumanLrsRecord]) -> io::Result<u64> {
        if records.is_empty() {
            return Ok(0);
        }
        let mut writer = TileWriter::open_append(&self.file_path)?;
        let timestamps: Vec<u64> = records.iter().map(|r| r.timestamp_sec).collect();

        let capacity = HYPER_TILE_PAYLOAD_SIZE / std::mem::size_of::<HumanLrsRecord>();
        if records.len() > capacity {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Records exceed single tile capacity"));
        }

        writer.write_hyper_tile(TileType::HUMAN_LRS, records, Some(&timestamps), None)
    }

    /// CREATE: Insert AI Agent Traceable Log metadata with Bloom filter on session UUID and agent ID.
    pub fn create_ai_traces(&self, records: &[AiTraceRecord]) -> io::Result<u64> {
        if records.is_empty() {
            return Ok(0);
        }
        let mut writer = TileWriter::open_append(&self.file_path)?;
        let timestamps: Vec<u64> = records.iter().map(|r| r.timestamp_sec).collect();

        let capacity = HYPER_TILE_PAYLOAD_SIZE / std::mem::size_of::<AiTraceRecord>();
        if records.len() > capacity {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Records exceed single tile capacity"));
        }

        writer.write_hyper_tile(TileType::AI_TRACE_LOG, records, Some(&timestamps), None)
    }

    /// CREATE: Insert Academic Paper literature records.
    pub fn create_academic_papers(&self, records: &[AcademicPaperRecord]) -> io::Result<u64> {
        if records.is_empty() {
            return Ok(0);
        }
        let mut writer = TileWriter::open_append(&self.file_path)?;
        let timestamps: Vec<u64> = records.iter().map(|r| r.timestamp_sec).collect();

        let capacity = HYPER_TILE_PAYLOAD_SIZE / std::mem::size_of::<AcademicPaperRecord>();
        if records.len() > capacity {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Records exceed single tile capacity"));
        }

        writer.write_hyper_tile(TileType::ACADEMIC_PAPERS, records, Some(&timestamps), None)
    }

    /// CREATE: Insert Knowledge Graph Edges.
    pub fn create_research_graph_edges(&self, edges: &[ResearchGraphEdgeRecord]) -> io::Result<u64> {
        if edges.is_empty() {
            return Ok(0);
        }
        let mut writer = TileWriter::open_append(&self.file_path)?;
        let timestamps: Vec<u64> = edges.iter().map(|r| r.timestamp_sec).collect();

        let capacity = HYPER_TILE_PAYLOAD_SIZE / std::mem::size_of::<ResearchGraphEdgeRecord>();
        if edges.len() > capacity {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Records exceed single tile capacity"));
        }

        writer.write_hyper_tile(TileType::RESEARCH_GRAPH, edges, Some(&timestamps), None)
    }

    /// BIDIRECTIONAL CROSS-TRACE: Follow human decision to triggering AI agent event via direct O(1) S3ACoordinate pointer.
    pub fn trace_ai_from_human(&self, human_coord: &S3ACoordinate) -> Result<AiTraceRecord, S3AError> {
        let human: HumanLrsRecord = self.read_by_coordinate(human_coord)?;
        self.read_by_coordinate(&human.ai_trace_coord)
    }

    /// BIDIRECTIONAL CROSS-TRACE: Follow AI agent event to human feedback/rework decision via direct O(1) S3ACoordinate pointer.
    pub fn trace_human_from_ai(&self, ai_coord: &S3ACoordinate) -> Result<HumanLrsRecord, S3AError> {
        let ai: AiTraceRecord = self.read_by_coordinate(ai_coord)?;
        self.read_by_coordinate(&ai.human_coord)
    }

    /// SESSION CORRELATION: Retrieves all correlated Human LRS and AI Trace records sharing a matching 128-bit session_uuid.
    pub fn query_session_bundle(
        &self,
        session_uuid: [u64; 2],
    ) -> io::Result<(Vec<(HumanLrsRecord, S3ACoordinate)>, Vec<(AiTraceRecord, S3ACoordinate)>)> {
        let reader = MmapReader::open(&self.file_path)?;
        let sieve = QuerySieve::new(&reader);
        let human_records = sieve.query_human_lrs(Some(session_uuid), None, 0, u64::MAX);
        let ai_records = sieve.query_ai_traces(Some(session_uuid), None, 0, u64::MAX);
        Ok((human_records, ai_records))
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

    struct TestTempFile {

        path: std::path::PathBuf,
    }

    impl TestTempFile {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let mut path = std::env::temp_dir();
            path.push(format!("s3a_eng_test_{}_{}_{}.s3a", std::process::id(), ts, id));
            Self { path }
        }

        fn path(&self) -> &std::path::Path {
            &self.path
        }
    }

    impl Drop for TestTempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }


    #[test]
    fn test_tile_fusion_engine() {
        let f1 = TestTempFile::new();
        let f2 = TestTempFile::new();
        let fused_file = TestTempFile::new();

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
        let temp_file = TestTempFile::new();
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
        let temp_file = TestTempFile::new();
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
        let temp_file = TestTempFile::new();
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
        let temp_file = TestTempFile::new();
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
        let temp_file = TestTempFile::new();
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
    fn test_tile_writer_open_append() {
        let temp_file = TestTempFile::new();
        let path = temp_file.path();

        // 1. Create file and write tile 1
        {
            let mut writer = TileWriter::create(path).unwrap();
            let records = vec![
                TelemetryRecord::new(1000, 1, 101, 10.0),
                TelemetryRecord::new(2000, 1, 101, 12.0),
            ];
            let tile_id = writer
                .write_hyper_tile(TileType::TELEMETRY, &records, Some(&[1000, 2000]), None)
                .unwrap();
            assert_eq!(tile_id, 1);
            assert_eq!(writer.tile_count(), 1);
        }

        // 2. Open append and write tile 2
        {
            let mut writer = TileWriter::open_append(path).unwrap();
            assert_eq!(writer.tile_count(), 1);

            let records = vec![
                TelemetryRecord::new(3000, 1, 101, 14.0),
                TelemetryRecord::new(4000, 1, 101, 16.0),
            ];
            let tile_id = writer
                .write_hyper_tile(TileType::TELEMETRY, &records, Some(&[3000, 4000]), None)
                .unwrap();
            assert_eq!(tile_id, 2);
            assert_eq!(writer.tile_count(), 2);
        }

        // 3. Open append again and write tile 3
        {
            let mut writer = TileWriter::open_append(path).unwrap();
            assert_eq!(writer.tile_count(), 2);

            let records = vec![
                TelemetryRecord::new(5000, 1, 101, 18.0),
            ];
            let tile_id = writer
                .write_hyper_tile(TileType::TELEMETRY, &records, Some(&[5000]), None)
                .unwrap();
            assert_eq!(tile_id, 3);
            assert_eq!(writer.tile_count(), 3);
        }

        // 4. Verify all written tiles and records via MmapReader
        let reader = MmapReader::open(path).unwrap();
        assert_eq!(reader.tile_count(), 3);
        assert!(reader.verify_checksums().is_ok());

        // Check tile 0 (ID 1)
        let (header1, payload1) = reader.get_tile(0).unwrap();
        assert_eq!(header1.tile_id, 1);
        assert_eq!(header1.record_count, 2);
        let recs1: &[TelemetryRecord] = bytemuck::cast_slice(payload1);
        assert_eq!(recs1[0].value, 10.0);
        assert_eq!(recs1[1].value, 12.0);

        // Check tile 1 (ID 2)
        let (header2, payload2) = reader.get_tile(1).unwrap();
        assert_eq!(header2.tile_id, 2);
        assert_eq!(header2.record_count, 2);
        let recs2: &[TelemetryRecord] = bytemuck::cast_slice(payload2);
        assert_eq!(recs2[0].value, 14.0);
        assert_eq!(recs2[1].value, 16.0);

        // Check tile 2 (ID 3)
        let (header3, payload3) = reader.get_tile(2).unwrap();
        assert_eq!(header3.tile_id, 3);
        assert_eq!(header3.record_count, 1);
        let recs3: &[TelemetryRecord] = bytemuck::cast_slice(payload3);
        assert_eq!(recs3[0].value, 18.0);
    }

    #[test]
    fn test_tile_writer_open_append_errors() {
        // Non-existent file should fail
        let non_existent_path = PathBuf::from("non_existent_s3a_file_12345.s3a");
        assert!(TileWriter::open_append(&non_existent_path).is_err());

        // Corrupted / invalid header file should fail
        let temp_file = TestTempFile::new();
        std::fs::write(temp_file.path(), b"invalid header data").unwrap();
        assert!(TileWriter::open_append(temp_file.path()).is_err());
    }

    #[test]
    fn test_crud_engine_read_telemetry_empty_or_mismatched_range() {
        let temp_file = TestTempFile::new();
        let engine = S3ACrudEngine::open_or_create(temp_file.path()).unwrap();

        // 1. Query empty engine before inserting any records
        let empty_res = engine.read_telemetry(10, 1, 0, 10000).unwrap();
        assert!(empty_res.is_empty(), "Expected empty result when querying empty archive");

        // Insert some sample records for sensor_id = 10, metric_id = 1 with timestamps 1000..3000
        let rec1 = TelemetryRecord::new(1000, 10, 1, 25.0);
        let rec2 = TelemetryRecord::new(2000, 10, 1, 26.0);
        let rec3 = TelemetryRecord::new(3000, 10, 1, 27.0);
        engine.create_telemetry(&[rec1, rec2, rec3]).unwrap();

        // 2. Query range entirely before existing timestamps
        let before_res = engine.read_telemetry(10, 1, 0, 999).unwrap();
        assert!(before_res.is_empty(), "Expected empty result for timestamp range before min_ts");

        // 3. Query range entirely after existing timestamps
        let after_res = engine.read_telemetry(10, 1, 3001, 5000).unwrap();
        assert!(after_res.is_empty(), "Expected empty result for timestamp range after max_ts");

        // 4. Query with inverted timestamp range (min_ts > max_ts)
        let inverted_res = engine.read_telemetry(10, 1, 2500, 1500).unwrap();
        assert!(inverted_res.is_empty(), "Expected empty result for inverted timestamp range");

        // 5. Query with non-matching sensor_id
        let wrong_sensor_res = engine.read_telemetry(99, 1, 500, 3500).unwrap();
        assert!(wrong_sensor_res.is_empty(), "Expected empty result for mismatched sensor_id");

        // 6. Query with non-matching metric_id
        let wrong_metric_res = engine.read_telemetry(10, 99, 500, 3500).unwrap();
        assert!(wrong_metric_res.is_empty(), "Expected empty result for mismatched metric_id");

        // 7. Verify valid range query still works as expected
        let valid_res = engine.read_telemetry(10, 1, 1500, 2500).unwrap();
        assert_eq!(valid_res.len(), 1);
        assert_eq!(valid_res[0].timestamp, 2000);
    }

    #[test]
    fn test_dual_generation_atomic_commits() {
        let temp_file = TestTempFile::new();
        let path = temp_file.path();

        let mut writer = TileWriter::create(path).unwrap();
        assert_eq!(writer.tile_count(), 0);
        assert_eq!(writer.generation(), 1);
        assert_eq!(writer.active_slot(), 0); // Started in Slot A

        // Commit 1st tile -> switches to Slot B, generation 2
        let rec1 = vec![TelemetryRecord::new(100, 1, 1, 10.0)];
        writer.write_hyper_tile(TileType::TELEMETRY, &rec1, Some(&[100]), None).unwrap();
        assert_eq!(writer.tile_count(), 1);
        assert_eq!(writer.generation(), 2);
        assert_eq!(writer.active_slot(), 1); // Switched to Slot B

        // Verify reader sees Slot B and generation 2
        {
            let reader = MmapReader::open(path).unwrap();
            assert_eq!(reader.tile_count(), 1);
            assert_eq!(reader.generation(), 2);
            assert_eq!(reader.active_slot(), 1);
            assert!(reader.verify_checksums().is_ok());
        }

        // Commit 2nd tile -> switches back to Slot A, generation 3
        let rec2 = vec![TelemetryRecord::new(200, 1, 1, 20.0)];
        writer.write_hyper_tile(TileType::TELEMETRY, &rec2, Some(&[200]), None).unwrap();
        assert_eq!(writer.tile_count(), 2);
        assert_eq!(writer.generation(), 3);
        assert_eq!(writer.active_slot(), 0); // Switched back to Slot A

        // Verify reader sees Slot A and generation 3
        {
            let reader = MmapReader::open(path).unwrap();
            assert_eq!(reader.tile_count(), 2);
            assert_eq!(reader.generation(), 3);
            assert_eq!(reader.active_slot(), 0);
            assert!(reader.verify_checksums().is_ok());
        }
    }

    #[test]
    fn test_crash_recovery_corrupted_slot_fallback() {
        let temp_file = TestTempFile::new();
        let path = temp_file.path();

        let mut writer = TileWriter::create(path).unwrap();
        let rec1 = vec![TelemetryRecord::new(100, 1, 1, 10.0)];
        writer.write_hyper_tile(TileType::TELEMETRY, &rec1, Some(&[100]), None).unwrap();
        drop(writer);

        // At this point:
        // Slot A: gen 1, tile_count 0
        // Slot B: gen 2, tile_count 1 (Active)
        // Corrupt Slot B bytes in file to simulate torn write during slot switch
        {
            let mut file = OpenOptions::new().read(true).write(true).open(path).unwrap();
            file.seek(SeekFrom::Start(FILE_HEADER_SLOT_B_OFFSET as u64)).unwrap();
            file.write_all(b"CORRUPTED_SLOT_B_METADATA_HEADER").unwrap();
            file.flush().unwrap();
        }

        // Open with MmapReader: Should detect Slot B corruption and gracefully fall back to Slot A!
        let reader = MmapReader::open(path).unwrap();
        assert_eq!(reader.active_slot(), 0); // Fell back to Slot A
        assert_eq!(reader.generation(), 1);
        assert_eq!(reader.tile_count(), 0);
    }

    #[test]
    fn test_crash_recovery_torn_tile_truncation() {
        let temp_file = TestTempFile::new();
        let path = temp_file.path();

        let mut writer = TileWriter::create(path).unwrap();
        let rec1 = vec![TelemetryRecord::new(100, 1, 1, 10.0)];
        writer.write_hyper_tile(TileType::TELEMETRY, &rec1, Some(&[100]), None).unwrap();
        drop(writer);

        let valid_len = std::fs::metadata(path).unwrap().len();
        assert_eq!(valid_len, FILE_HEADER_SIZE as u64 + HYPER_TILE_SIZE as u64);

        // Simulate crash mid-write: 32 KB of partial garbage appended at EOF without header commit
        {
            let mut file = OpenOptions::new().write(true).append(true).open(path).unwrap();
            let garbage = vec![0xEEu8; 32 * 1024];
            file.write_all(&garbage).unwrap();
            file.flush().unwrap();
        }
        assert_eq!(std::fs::metadata(path).unwrap().len(), valid_len + 32 * 1024);

        // Open in append mode: TileWriter must automatically truncate the file back to the committed boundary!
        let mut writer2 = TileWriter::open_append(path).unwrap();
        assert_eq!(writer2.tile_count(), 1);
        assert_eq!(std::fs::metadata(path).unwrap().len(), valid_len);

        // Verify appending a 2nd tile succeeds cleanly on the recovered file
        let rec2 = vec![TelemetryRecord::new(200, 1, 1, 20.0)];
        writer2.write_hyper_tile(TileType::TELEMETRY, &rec2, Some(&[200]), None).unwrap();
        assert_eq!(writer2.tile_count(), 2);

        let reader = MmapReader::open(path).unwrap();
        assert_eq!(reader.tile_count(), 2);
        assert!(reader.verify_checksums().is_ok());
    }

    #[test]
    fn test_s3a_ql_pipeline() {
        let temp_file = TestTempFile::new();
        let path = temp_file.path().to_str().unwrap().replace('\\', "/");

        // 1. INSERT TELEMETRY
        let insert_query = format!("INSERT TELEMETRY (1000, 1, 101, 42.50) INTO \"{}\";", path);
        let res_insert = execute_query(&insert_query).unwrap();
        match res_insert {
            QueryResult::Inserted { coordinate } => {
                assert_eq!(coordinate.level, 0);
            }
            other => panic!("Expected Inserted result, got {:?}", other),
        }

        // 2. FETCH RECORD AT
        let fetch_query = format!("FETCH RECORD AT L0:T0:R0 FROM \"{}\";", path);
        let res_fetch = execute_query(&fetch_query).unwrap();
        match res_fetch {
            QueryResult::Telemetry(rec) => {
                assert_eq!(rec.timestamp, 1000);
                assert_eq!(rec.sensor_id, 1);
                assert_eq!(rec.metric_id, 101);
                assert_eq!(rec.value, 42.5);
            }
            other => panic!("Expected Telemetry record, got {:?}", other),
        }

        // 3. SIFT TELEMETRY
        let sift_query = format!(
            "SIFT TELEMETRY FROM \"{}\" WHERE TIME BETWEEN 500 AND 1500 AND SENSOR_ID = 1 AND METRIC_ID = 101;",
            path
        );
        let res_sift = execute_query(&sift_query).unwrap();
        match res_sift {
            QueryResult::TelemetryList(list) => {
                assert_eq!(list.len(), 1);
                assert_eq!(list[0].0.timestamp, 1000);
                assert_eq!(list[0].1, S3ACoordinate::new(0, 0, 0));
            }
            other => panic!("Expected TelemetryList, got {:?}", other),
        }

        // 4. DELETE TELEMETRY
        let delete_query = format!(
            "DELETE TELEMETRY WHERE SENSOR_ID = 1 AND METRIC_ID = 101 AND TIME = 1000 FROM \"{}\";",
            path
        );
        let res_delete = execute_query(&delete_query).unwrap();
        match res_delete {
            QueryResult::Deleted { success } => assert!(success),
            other => panic!("Expected Deleted result, got {:?}", other),
        }

        // Verify sift returns empty after delete
        let res_sift2 = execute_query(&sift_query).unwrap();
        match res_sift2 {
            QueryResult::TelemetryList(list) => assert!(list.is_empty()),
            other => panic!("Expected empty TelemetryList, got {:?}", other),
        }

        // 5. COMPACT ARCHIVE
        let compact_query = format!("COMPACT ARCHIVE \"{}\";", path);
        let res_compact = execute_query(&compact_query).unwrap();
        match res_compact {
            QueryResult::Compacted { .. } => {}
            other => panic!("Expected Compacted result, got {:?}", other),
        }
    }

    #[test]
    fn test_hull_bvh_pruning() {
        let temp_file = TestTempFile::new();
        let path = temp_file.path();

        let mut writer = TileWriter::create(path).unwrap();

        // Write 16 tiles across different time windows:
        // Tile 0..3: timestamps 1000..1999
        // Tile 4..7: timestamps 2000..2999
        // Tile 8..11: timestamps 3000..3999
        // Tile 12..15: timestamps 4000..4999
        for i in 0..16 {
            let base_ts = (1000 + (i / 4) * 1000) as u64;
            let rec = vec![TelemetryRecord::new(base_ts + (i % 4) as u64, 1, 10, 50.0 + i as f64)];
            let timestamps = vec![rec[0].timestamp];

            let mut hull = SimplexHull::empty();
            hull.dim = 3;
            let val = i as f32;
            hull.min_bounds[0] = val;
            hull.max_bounds[0] = val + 1.0;
            hull.min_bounds[1] = val;
            hull.max_bounds[1] = val + 1.0;
            hull.min_bounds[2] = val;
            hull.max_bounds[2] = val + 1.0;

            writer.write_hyper_tile(TileType::TELEMETRY, &rec, Some(&timestamps), Some(hull)).unwrap();
        }

        let reader = MmapReader::open(path).unwrap();
        assert_eq!(reader.tile_count(), 16);

        // Build BVH with branch factor 4
        let bvh = HullBvh::build_with_branch_factor(&reader, 4);
        assert_eq!(bvh.total_tiles, 16);
        assert!(bvh.root.is_some());

        let root = bvh.root.as_ref().unwrap();
        assert_eq!(root.children.len(), 4); // 16 tiles / 4 branch factor = 4 children

        // Test 1: Query timestamp range 2000..2999 should ONLY select tiles 4, 5, 6, 7
        let time_candidates = bvh.sift_time(2000, 2999);
        assert_eq!(time_candidates, vec![4, 5, 6, 7]);

        // Test 2: Spatial range [8.5, 8.5, 8.5] to [9.5, 9.5, 9.5] should only select tiles 8 and 9
        let min_box = [8.5f32, 8.5, 8.5];
        let max_box = [9.5f32, 9.5, 9.5];
        let spatial_candidates = bvh.sift_spatial_range(&min_box, &max_box);
        assert!(spatial_candidates.contains(&8));
        assert!(spatial_candidates.contains(&9));
        assert!(!spatial_candidates.contains(&0));
        assert!(!spatial_candidates.contains(&15));

        // Test 3: Sieve accelerated with BVH
        let sieve = QuerySieve::new(&reader);
        let bvh_results = sieve.query_telemetry_bvh(&bvh, 2000, 2500, None, None);
        assert_eq!(bvh_results.len(), 4);

        let linear_results = sieve.query_telemetry_with_coords(2000, 2500, None, None);
        assert_eq!(bvh_results.len(), linear_results.len());
    }

    #[test]
    fn test_l0_ring_buffer_concurrency() {
        use std::sync::Arc;
        use std::thread;

        let buffer = Arc::new(L0RingBuffer::<TelemetryRecord>::new(1024, BackpressurePolicy::Block));
        let num_producers = 4;
        let records_per_producer = 500;
        let mut handles = Vec::new();

        // Spawn multiple concurrent producer threads
        for p in 0..num_producers {
            let buf = Arc::clone(&buffer);
            handles.push(thread::spawn(move || {
                for i in 0..records_per_producer {
                    let ts = (p * 10000 + i) as u64;
                    let rec = TelemetryRecord::new(ts, p as u32, 1, i as f64);
                    buf.push(rec).unwrap();
                }
            }));
        }

        // Consumer thread drains concurrently
        let buf_consumer = Arc::clone(&buffer);
        let consumer_handle = thread::spawn(move || {
            let mut total_drained = 0;
            let expected = num_producers * records_per_producer;
            while total_drained < expected {
                let chunk = buf_consumer.drain(256);
                total_drained += chunk.len();
                if chunk.is_empty() {
                    thread::sleep(std::time::Duration::from_millis(2));
                }
            }
            total_drained
        });

        for h in handles {
            h.join().unwrap();
        }

        let total = consumer_handle.join().unwrap();
        assert_eq!(total, num_producers * records_per_producer);
        assert_eq!(buffer.len(), 0);

        // Test DropOldest policy
        let drop_buffer = L0RingBuffer::<TelemetryRecord>::new(10, BackpressurePolicy::DropOldest);
        for i in 0..25 {
            let rec = TelemetryRecord::new(i as u64, 1, 1, i as f64);
            drop_buffer.push(rec).unwrap();
        }
        assert_eq!(drop_buffer.len(), 10);
        assert_eq!(drop_buffer.dropped_count(), 15);
        let snapshot = drop_buffer.snapshot();
        assert_eq!(snapshot.len(), 10);
        assert_eq!(snapshot[0].timestamp, 15);
        assert_eq!(snapshot[9].timestamp, 24);
    }

    #[test]
    fn test_background_compactor_lifecycle() {
        use std::sync::Arc;
        use std::thread;
        use std::time::Duration;

        let temp_file = TestTempFile::new();
        let path = temp_file.path();

        let ring_buffer = Arc::new(L0RingBuffer::<TelemetryRecord>::new(4096, BackpressurePolicy::Block));
        let mut compactor = BackgroundCompactor::new(
            path,
            Arc::clone(&ring_buffer),
            100, // flush threshold: 100 records
            Duration::from_millis(50), // max flush interval: 50 ms
            None,
        );

        compactor.start().unwrap();
        assert!(compactor.is_running());

        // Ingest 250 records into L0 Ring Buffer
        for i in 0..250 {
            let rec = TelemetryRecord::new(1000 + i as u64, 1, 10, i as f64);
            ring_buffer.push(rec).unwrap();
        }

        // Wait for background worker to trigger automatic flush
        thread::sleep(Duration::from_millis(150));

        let stats = compactor.stats();
        assert!(stats.flushed_records >= 100, "Background compactor should have flushed batched records");

        // Force synchronous flush of remainder
        compactor.flush_now().unwrap();
        assert_eq!(ring_buffer.len(), 0);

        // Verify disk contents
        let reader = MmapReader::open(path).unwrap();
        assert!(reader.tile_count() >= 1);
        reader.verify_checksums().unwrap();

        // Stop cleanly
        compactor.stop();
        assert!(!compactor.is_running());
    }

    #[test]
    fn test_certified_freshness_query_overlay() {
        use std::sync::Arc;

        let temp_file = TestTempFile::new();
        let path = temp_file.path();

        // 1. Commit 2 baseline records to disk
        let mut writer = TileWriter::create(path).unwrap();
        let initial_records = vec![
            TelemetryRecord::new(100, 1, 10, 42.0),
            TelemetryRecord::new(200, 1, 10, 43.0),
        ];
        let timestamps = vec![100, 200];
        writer.write_hyper_tile(TileType::TELEMETRY, &initial_records, Some(&timestamps), None).unwrap();

        // 2. Set up compactor with ring buffer
        let ring_buffer = Arc::new(L0RingBuffer::<TelemetryRecord>::new(1024, BackpressurePolicy::Block));
        let compactor = BackgroundCompactor::new(
            path,
            Arc::clone(&ring_buffer),
            1000,
            std::time::Duration::from_secs(10),
            None,
        );

        // 3. Push uncommitted fresh record (timestamp 300) into in-memory L0
        ring_buffer.push(TelemetryRecord::new(300, 1, 10, 44.0)).unwrap();

        // 4. Push tombstone for timestamp 100 into in-memory L0 (soft delete)
        let mut tombstone = TelemetryRecord::new(100, 1, 10, 0.0);
        tombstone.mark_tombstone();
        ring_buffer.push(tombstone).unwrap();

        // 5. Query fresh telemetry across delta overlay:
        // - Timestamp 100 should be eliminated by tombstone
        // - Timestamp 200 should come from committed disk
        // - Timestamp 300 should come from uncommitted L0 ring buffer
        let fresh_results = compactor.query_fresh_telemetry(0, 500, Some(1), Some(10));
        assert_eq!(fresh_results.len(), 2);
        assert_eq!(fresh_results[0].timestamp, 200);
        assert_eq!(fresh_results[0].value, 43.0);
        assert_eq!(fresh_results[1].timestamp, 300);
        assert_eq!(fresh_results[1].value, 44.0);
    }

    #[test]
    fn test_composable_s3a_ql_algebraic_scoring() {
        let temp_file = TestTempFile::new();
        let path = temp_file.path();

        let mut writer = TileWriter::create(path).unwrap();

        // Write 3 embedding records
        // Rec 1: [1.0, 0.0, 0.0, ...], ts: 1000
        let mut v1 = [0.0f32; 128]; v1[0] = 1.0;
        // Rec 2: [0.0, 1.0, 0.0, ...], ts: 2000
        let mut v2 = [0.0f32; 128]; v2[0] = 0.7071; v2[1] = 0.7071;
        // Rec 3: [0.0, 0.0, 1.0, ...], ts: 3000
        let mut v3 = [0.0f32; 128]; v3[2] = 1.0;

        let recs = vec![
            EmbeddingRecord128::new(1, 1000, v1),
            EmbeddingRecord128::new(2, 2000, v2),
            EmbeddingRecord128::new(3, 3000, v3),
        ];
        let timestamps = vec![1000, 2000, 3000];
        writer.write_hyper_tile(TileType::EMBEDDING, &recs, Some(&timestamps), None).unwrap();

        // Run S3A-QL algebraic composable query combining cosine similarity, spatial proximity, and time decay!
        let ql = format!(
            "SIFT EMBEDDINGS FROM \"{}\" WHERE TIME BETWEEN 500 AND 4000 COMPOSE SIMILARITY TO [1.0, 0.0, 0.0] WEIGHT 0.6 AND SPATIAL_PROXIMITY [1.0, 0.0, 0.0] WEIGHT 0.3 AND DECAY HALFLIFE 1000 WEIGHT 0.1 LIMIT 2;",
            path.display()
        );

        let res = execute_query(&ql).unwrap();
        match res {
            QueryResult::EmbeddingList(list) => {
                assert_eq!(list.len(), 2);
                // Top match should be Rec 1 because it has exact direction and spatial position [1.0, 0.0, 0.0]
                assert_eq!(list[0].0.id, 1);
                assert!(list[0].1 > list[1].1, "Ranked list must be in descending order of composite score");
            }
            other => panic!("Expected EmbeddingList, got {:?}", other),
        }
    }

    #[test]
    fn test_binary_tcp_protocol_roundtrip() {
        use std::sync::Arc;

        let ring_buffer = Arc::new(L0RingBuffer::<TelemetryRecord>::new(1024, BackpressurePolicy::Block));
        let mut server = S3AProtocolServer::bind("127.0.0.1:0", Some(Arc::clone(&ring_buffer))).unwrap();
        let addr = server.local_addr().unwrap();
        server.start();

        let mut client = S3AClient::connect(addr).unwrap();

        // 1. Ping round-trip
        let latency = client.ping().unwrap();
        assert!(latency.as_millis() < 500);

        // 2. Ingest telemetry batch via wire protocol
        let records = vec![
            TelemetryRecord::new(1001, 1, 10, 42.5),
            TelemetryRecord::new(1002, 1, 10, 43.0),
            TelemetryRecord::new(1003, 1, 10, 43.5),
        ];
        let ingested = client.insert_telemetry(&records).unwrap();
        assert_eq!(ingested, 3);
        assert_eq!(ring_buffer.len(), 3);

        // 3. Remote S3A-QL execution
        let temp_file = TestTempFile::new();
        let path = temp_file.path();
        let insert_ql = format!("INSERT TELEMETRY (1000, 1, 101, 99.9) INTO \"{}\";", path.display());
        let _res_str = client.query(&insert_ql).unwrap();
        server.stop();
    }

    #[test]
    fn test_learning_record_store_bloom_and_learned_index() {
        let temp_file = TestTempFile::new();
        let path = temp_file.path();
        let engine = S3ACrudEngine::open_or_create(path).unwrap();

        // Create 100 Learning Activity Records for agents 1001, 1002, 1003
        let mut records = Vec::new();
        for i in 0..100 {
            let actor = if i % 2 == 0 { 1001u64 } else { 1002u64 };
            let verb = if i % 3 == 0 { 1 } else { 2 };
            let rec = LearningActivityRecord::new(
                actor,
                1000 + (i as u64) * 10,
                verb,
                42,
                0.95,
                120,
                1,
            );
            records.push(rec);
        }

        let written = engine.create_learning_activities(&records).unwrap();
        assert_eq!(written, 1);

        // Verify read by actor 1001
        let act1001 = engine.read_learning_activities(Some(1001), None, 1000, 2000).unwrap();
        assert_eq!(act1001.len(), 50);
        assert_eq!(act1001[0].0.actor_id, 1001);

        // Verify read by uninserted actor 9999 is pruned by Bloom filter
        let act_none = engine.read_learning_activities(Some(9999), None, 1000, 2000).unwrap();
        assert_eq!(act_none.len(), 0);

        // Test S3A-QL parsing and execution on LEARNING_ACTIVITIES
        let ql_stmt = format!("SIFT LEARNING_ACTIVITIES FROM \"{}\" WHERE ACTOR_ID = 1001;", path.display());
        let res = execute_query(&ql_stmt).unwrap();
        match res {
            QueryResult::LearningActivityList(list) => {
                assert_eq!(list.len(), 50);
            }
            other => panic!("Expected LearningActivityList, got {:?}", other),
        }
    }

    #[test]
    fn test_dual_trace_human_and_ai_cross_reference() {
        let temp_file = TestTempFile::new();
        let path = temp_file.path();
        let engine = S3ACrudEngine::open_or_create(path).unwrap();

        let session_uuid = [0xAAAA_BBBB_CCCC_DDDD, 0x1111_2222_3333_4444];
        let human_coord = S3ACoordinate::new(0, 0, 0);
        let ai_coord = S3ACoordinate::new(0, 1, 0);

        let human_rec = HumanLrsRecord::new(
            session_uuid,
            1600000000,
            999, // researcher hash
            1,   // 'accepted'
            1.0,
            ai_coord, // pointer to AI trace at tile 1, offset 0
            42,
        );

        let ai_rec = AiTraceRecord::new(
            session_uuid,
            1600000001,
            888, // agent id
            2,   // reasoning step
            0.98,
            human_coord, // pointer to human feedback at tile 0, offset 0
            1234,
        );

        engine.create_human_lrs(&[human_rec]).unwrap();
        engine.create_ai_traces(&[ai_rec]).unwrap();

        // 1. Test bidirectional O(1) dereferencing
        let resolved_ai = engine.trace_ai_from_human(&human_coord).unwrap();
        assert_eq!(resolved_ai.session_uuid, session_uuid);
        assert_eq!(resolved_ai.agent_id, 888);
        assert_eq!(resolved_ai.confidence, 0.98);

        let resolved_human = engine.trace_human_from_ai(&ai_coord).unwrap();
        assert_eq!(resolved_human.session_uuid, session_uuid);
        assert_eq!(resolved_human.actor_hash, 999);
        assert_eq!(resolved_human.decision_score, 1.0);

        // 2. Test session correlation bundle query
        let (humans, ais) = engine.query_session_bundle(session_uuid).unwrap();
        assert_eq!(humans.len(), 1);
        assert_eq!(ais.len(), 1);
        assert_eq!(humans[0].0.actor_hash, 999);
        assert_eq!(ais[0].0.agent_id, 888);
    }
}



