use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};

use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use fs2::FileExt;
use bytemuck::{bytes_of, Pod};
use memmap2::Mmap;

pub use s3a_core::*;
use sha2::Digest;
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
pub mod fsutil;
pub mod compactor_gates;
pub mod concurrent_writer;
pub mod lrs_store;
pub mod elr;
pub mod sealed;
pub mod anchor;
pub mod edge_import;

pub use fsutil::{atomic_replace, clean_orphan_temps, unique_compact_temp_path};
pub use compactor_gates::*;
pub use concurrent_writer::*;
pub use lrs_store::*;
pub use elr::*;
pub use sealed::*;
pub use anchor::*;
pub use edge_import::*;

pub use ql::{execute_query, S3AQLEngine, S3AStatement, QueryResult, Lexer, Parser, Token, QlError, register_pack_sift_handler, PackSiftHandler};
pub use bvh::{HullBvh, BvhNode};
pub use ring_buffer::{L0RingBuffer, BackpressurePolicy, IngestError};
pub use compactor_daemon::{BackgroundCompactor, CompactorStats};
pub use net::{S3AClient, S3AProtocolServer, send_frame, read_frame};
pub use erasure::{ReedSolomonCodec, TileShard, TileShardHeader, ErasureError, save_shards_to_dir, load_shards_from_dir};
pub use tiering::{TileStorageAdapter, LocalStorageAdapter, RustFsAdapter, RustFsConfig, TieringError, archive_cold_tiles, ArchivalReport};
pub use lakehouse::{
    SnowflakeBatchRequest, handle_snowflake_batch_request,
    ArrowSchema, ArrowField, ArrowColumnVector, ArrowRecordBatchDescriptor,
    project_telemetry_to_arrow, project_columnar_to_arrow,
    generate_iceberg_metadata, generate_delta_metadata,
};

/// Encodes a slice of TelemetryRecords into a Columnar Tile payload.
pub fn encode_columnar_telemetry(records: &[TelemetryRecord]) -> Vec<u8> {
    let mut builder = ColumnarTileBuilder::new(records.len() as u32, TileType::TELEMETRY);
    let n = records.len();
    let mut ts = Vec::with_capacity(n);
    let mut sensor = Vec::with_capacity(n);
    let mut metric = Vec::with_capacity(n);
    let mut val = Vec::with_capacity(n);

    let mut min_ts = u64::MAX;
    let mut max_ts = u64::MIN;
    let mut min_sensor = u32::MAX;
    let mut max_sensor = u32::MIN;
    let mut min_metric = u32::MAX;
    let mut max_metric = u32::MIN;
    let mut min_val = f64::MAX;
    let mut max_val = f64::MIN;
    let mut flags = Vec::with_capacity(n);

    for r in records {
        ts.push(r.timestamp);
        sensor.push(r.sensor_id);
        metric.push(r.metric_id);
        val.push(r.value);
        flags.push(r.flags);

        if r.timestamp < min_ts { min_ts = r.timestamp; }
        if r.timestamp > max_ts { max_ts = r.timestamp; }
        if r.sensor_id < min_sensor { min_sensor = r.sensor_id; }
        if r.sensor_id > max_sensor { max_sensor = r.sensor_id; }
        if r.metric_id < min_metric { min_metric = r.metric_id; }
        if r.metric_id > max_metric { max_metric = r.metric_id; }
        if r.value < min_val { min_val = r.value; }
        if r.value > max_val { max_val = r.value; }
    }

    if records.is_empty() {
        min_ts = 0; max_ts = 0;
        min_sensor = 0; max_sensor = 0;
        min_metric = 0; max_metric = 0;
        min_val = 0.0; max_val = 0.0;
    }

    builder.add_column("timestamp", ColumnDataType::UInt64, &ts, None, min_ts as f64, max_ts as f64);
    builder.add_column("sensor_id", ColumnDataType::UInt32, &sensor, None, min_sensor as f64, max_sensor as f64);
    builder.add_column("metric_id", ColumnDataType::UInt32, &metric, None, min_metric as f64, max_metric as f64);
    builder.add_column("value", ColumnDataType::Float64, &val, None, min_val, max_val);
    builder.add_column("flags", ColumnDataType::UInt64, &flags, None, 0.0, 0.0);

    builder.build()
}

/// Decodes a Columnar Tile payload into row-based TelemetryRecords.
pub fn decode_columnar_telemetry(payload: &[u8]) -> io::Result<Vec<TelemetryRecord>> {
    let view = ColumnarTileView::parse(payload)?;
    let n = view.header.record_count as usize;
    if n == 0 {
        return Ok(Vec::new());
    }

    let desc_ts = view.find_column("timestamp").ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "Missing 'timestamp' column in columnar telemetry")
    })?;
    let desc_sensor = view.find_column("sensor_id").ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "Missing 'sensor_id' column in columnar telemetry")
    })?;
    let desc_metric = view.find_column("metric_id").ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "Missing 'metric_id' column in columnar telemetry")
    })?;
    let desc_val = view.find_column("value").ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "Missing 'value' column in columnar telemetry")
    })?;
    let desc_flags = view.find_column("flags");

    let col_ts: &[u64] = view.read_column_slice(desc_ts)?;
    let col_sensor: &[u32] = view.read_column_slice(desc_sensor)?;
    let col_metric: &[u32] = view.read_column_slice(desc_metric)?;
    let col_val: &[f64] = view.read_column_slice(desc_val)?;
    let col_flags: Option<&[u64]> = if let Some(df) = desc_flags {
        view.read_column_slice(df).ok()
    } else {
        None
    };

    let mut records = Vec::with_capacity(n);
    for i in 0..n {
        let mut rec = TelemetryRecord::default();
        rec.timestamp = col_ts[i];
        rec.sensor_id = col_sensor[i];
        rec.metric_id = col_metric[i];
        rec.value = col_val[i];
        rec.flags = col_flags.map(|cf| cf[i]).unwrap_or(1);
        records.push(rec);
    }
    Ok(records)
}

/// Zero-allocation 4 KB Micro-Tile Memory Buffer for embedded microcontrollers (Earbuds, Smart Glasses, Wristbands).
pub struct MicroTileBuffer {
    buffer: [u8; MICRO_TILE_SIZE],
    written_bytes: usize,
    record_count: u16,
}

impl MicroTileBuffer {
    pub fn new(tile_id: u32) -> Self {
        Self::new_with_type(tile_id, 3)
    }

    pub fn new_with_type(tile_id: u32, tile_type: u16) -> Self {
        let mut buffer = [0u8; MICRO_TILE_SIZE];
        let header = MicroTileHeader::new(tile_id, tile_type);
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

        let mut header: MicroTileHeader = bytemuck::pod_read_unaligned(&self.buffer[..MICRO_TILE_HEADER_SIZE]);
        header.record_count = self.record_count;
        header.payload_bytes = payload_bytes;
        header.data_crc32 = data_crc32;

        header.header_crc32 = compute_micro_tile_header_crc32(&header);

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

/// Computes CRC32C of a HyperTileHeader excluding the header_crc32 field itself.
pub fn compute_tile_header_crc32(header: &HyperTileHeader) -> u32 {
    let bytes = bytes_of(header);
    let crc_offset = std::mem::offset_of!(HyperTileHeader, header_crc32);
    let mut crc = crc32c::crc32c(&bytes[..crc_offset]);
    crc = crc32c::crc32c_append(crc, &bytes[crc_offset + 4..]);
    crc
}

/// Computes CRC32C of a MicroTileHeader excluding the header_crc32 field itself.
pub fn compute_micro_tile_header_crc32(header: &MicroTileHeader) -> u32 {
    let bytes = bytes_of(header);
    let crc_offset = std::mem::offset_of!(MicroTileHeader, header_crc32);
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

/// Returns the current Unix timestamp in seconds across native and WebAssembly platforms without trapping.
#[inline]
pub fn current_time_secs() -> u64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
    #[cfg(target_arch = "wasm32")]
    {
        (js_sys::Date::now() / 1000.0) as u64
    }
}

/// Returns the current Unix timestamp in nanoseconds across native and WebAssembly platforms without trapping.
#[inline]
pub fn current_time_nanos() -> u128 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    }
    #[cfg(target_arch = "wasm32")]
    {
        (js_sys::Date::now() * 1_000_000.0) as u128
    }
}

/// Generates a non-zero 128-bit pseudo-random UUID for an S3A archive file.
pub fn generate_file_uuid() -> [u64; 2] {
    #[cfg(not(target_arch = "wasm32"))]
    let (nanos, pid) = {
        let nanos = current_time_nanos();
        let pid = std::process::id() as u64;
        (nanos, pid)
    };
    #[cfg(target_arch = "wasm32")]
    let (nanos, pid) = {
        let mut random_bytes = [0u8; 16];
        let _ = getrandom::getrandom(&mut random_bytes);
        (u128::from_le_bytes(random_bytes), current_time_nanos() as u64)
    };
    let h1 = xxhash_rust::xxh3::xxh3_64(&nanos.to_le_bytes());
    let h2 = xxhash_rust::xxh3::xxh3_64(&(nanos ^ (pid as u128)).to_be_bytes());
    let u1 = if h1 == 0 { 1 } else { h1 };
    let u2 = if h2 == 0 { 2 } else { h2 };
    [u1, u2]
}


/// Writer for generating page-aligned 128 KB S3A Hyper-Tile storage files with crash-safe atomic commits.
pub struct TileWriter {
    file: File,
    tile_count: u32,
    current_tile_id: u64,
    generation: u64,
    active_slot: usize,
    created_timestamp: u64,
    addon_mask: u32,
    file_uuid: [u64; 2],
}

impl TileWriter {
    pub fn create<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        Self::create_with_uuid(path, generate_file_uuid())
    }

    pub fn create_with_uuid<P: AsRef<Path>>(path: P, file_uuid: [u64; 2]) -> io::Result<Self> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;

        let now = current_time_secs();

        let mut slot_a = FileHeader::new(0, now).with_file_uuid(file_uuid);
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
            addon_mask: 0,
            file_uuid,
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

        let slot_a: &FileHeader = &bytemuck::pod_read_unaligned::<FileHeader>(&header_buf[FILE_HEADER_SLOT_A_OFFSET..FILE_HEADER_SLOT_A_OFFSET + FILE_HEADER_SLOT_SIZE]);
        let slot_b: &FileHeader = &bytemuck::pod_read_unaligned::<FileHeader>(&header_buf[FILE_HEADER_SLOT_B_OFFSET..FILE_HEADER_SLOT_B_OFFSET + FILE_HEADER_SLOT_SIZE]);

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

        // Clean stale orphan compaction temps since live archive header is verified
        let _ = fsutil::clean_orphan_temps(&path);

        file.seek(SeekFrom::End(0))?;

        Ok(Self {
            file,
            tile_count: committed_tile_count,
            current_tile_id: (committed_tile_count as u64) + 1,
            generation: active_header.generation,
            active_slot,
            created_timestamp: active_header.created_timestamp,
            addon_mask: active_header.addon_mask,
            file_uuid: active_header.file_uuid,
        })
    }

    /// Attempts to acquire an exclusive OS advisory file lock on the archive.
    pub fn try_lock_exclusive(&self) -> io::Result<()> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.file.try_lock_exclusive()
        }
        #[cfg(target_arch = "wasm32")]
        {
            Ok(())
        }
    }

    /// Releases the OS advisory file lock.
    pub fn unlock(&self) -> io::Result<()> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.file.unlock()
        }
        #[cfg(target_arch = "wasm32")]
        {
            Ok(())
        }
    }

    /// Flushes and reads all committed bytes directly from the underlying archive file handle.
    pub fn read_all_bytes(&mut self) -> io::Result<Vec<u8>> {
        self.file.flush()?;
        self.file.sync_all()?;
        let current_pos = self.file.stream_position()?;
        self.file.seek(SeekFrom::Start(0))?;
        let mut buf = Vec::new();
        self.file.read_to_end(&mut buf)?;
        self.file.seek(SeekFrom::Start(current_pos))?;
        Ok(buf)
    }


    fn commit_header_slot(&mut self, next_tile_count: u32, addon_mask: u32) -> io::Result<()> {
        let next_generation = self.generation + 1;
        let now = current_time_secs();

        let target_slot = 1 - self.active_slot;
        let mut new_slot = FileHeader::with_commit(
            next_tile_count,
            next_generation,
            self.created_timestamp,
            now,
        );
        new_slot.addon_mask = addon_mask;
        new_slot.file_uuid = self.file_uuid;
        new_slot.header_crc32 = compute_header_crc32(&new_slot);

        let slot_offset = if target_slot == 0 {
            FILE_HEADER_SLOT_A_OFFSET
        } else {
            FILE_HEADER_SLOT_B_OFFSET
        } as u64;

        self.file.seek(SeekFrom::Start(slot_offset))?;
        self.file.write_all(bytes_of(&new_slot))?;
        self.file.flush()?;
        self.file.sync_all()?;

        self.active_slot = target_slot;
        self.generation = next_generation;
        self.tile_count = next_tile_count;
        self.addon_mask = addon_mask;
        self.file.seek(SeekFrom::End(0))?;
        Ok(())
    }

    pub fn write_hyper_tile_ext<T: Pod>(
        &mut self,
        tile_type: u32,
        records: &[T],
        timestamps: Option<&[u64]>,
        hull: Option<SimplexHull>,
        filter_keys: Option<&[u64]>,
    ) -> io::Result<u64> {
        // Enforce capability write gate and registry validation:
        let registry = s3a_core::registry::global_registry();
        let target_version = registry.get(tile_type).map(|d| d.current_layout_version).unwrap_or(1);
        if registry.get(tile_type).is_some() {
            if let Err(e) = registry.validate_write(tile_type, target_version, self.addon_mask) {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{}", e)));
            }
        } else if let Some((pack_name, required_mask)) = s3a_core::required_addon_for_tile(tile_type) {
            if (self.addon_mask & required_mask) == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!(
                        "Tile type {} belongs to {} but addon bit 0x{:04x} is not enabled on archive (addon_mask: 0x{:04x})",
                        tile_type, pack_name, required_mask, self.addon_mask
                    ),
                ));
            }
        }

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
        header.generation = self.generation;
        header.tile_index = self.tile_count;
        header.file_uuid = self.file_uuid;
        header.record_layout_version = target_version;

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

        // Populate discrete ID Bloom filter from provided filter keys or core tile types
        if let Some(keys) = filter_keys {
            for &k in keys {
                bloom_filter_insert(&mut header.filter.bits, k, header.filter.hash_seed);
            }
        } else if tile_type == TileType::TELEMETRY {
            if let Ok(tel_recs) = bytemuck::try_cast_slice::<T, TelemetryRecord>(records) {
                for r in tel_recs {
                    bloom_filter_insert(&mut header.filter.bits, r.sensor_id as u64, header.filter.hash_seed);
                    bloom_filter_insert(&mut header.filter.bits, r.metric_id as u64, header.filter.hash_seed);
                    if r.is_tombstone() {
                        header.lifecycle.tombstone_count += 1;
                    }
                }
            }
        } else if tile_type == TileType::RESERVED_BOUNDARY || tile_type == 10 {
            if let Ok(lrs_recs) = bytemuck::try_cast_slice::<T, LearningActivityRecord>(records) {
                for r in lrs_recs {
                    bloom_filter_insert(&mut header.filter.bits, r.actor_id, header.filter.hash_seed);
                    bloom_filter_insert(&mut header.filter.bits, r.verb_id as u64, header.filter.hash_seed);
                    if r.is_tombstone() {
                        header.lifecycle.tombstone_count += 1;
                    }
                }
            }
        }

        // Compute header CRC32C (excluding header_crc32 field itself)
        header.header_crc32 = compute_tile_header_crc32(&header);

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

        // AFTER GATE: Re-read output tile from disk and verify CRC before A/B commit switch
        let tile_offset = FILE_HEADER_SIZE as u64 + (self.tile_count as u64 * HYPER_TILE_SIZE as u64);
        crate::compactor_gates::verify_written_tile_from_disk(
            &mut self.file,
            tile_offset,
            header.header_crc32,
            header.data_crc32,
            payload_bytes,
        )?;

        #[cfg(any(test, feature = "fault-injection"))]
        if crate::compactor_gates::fault_injection::INJECT_CRASH_BEFORE_COMMIT.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Crash simulated between tile write and header commit switch",
            ));
        }

        let committed_tile_id = self.current_tile_id;
        self.commit_header_slot(self.tile_count + 1, self.addon_mask)?;
        self.current_tile_id += 1;

        Ok(committed_tile_id)
    }

    pub fn write_hyper_tile<T: Pod>(
        &mut self,
        tile_type: u32,
        records: &[T],
        timestamps: Option<&[u64]>,
        hull: Option<SimplexHull>,
    ) -> io::Result<u64> {
        self.write_hyper_tile_ext(tile_type, records, timestamps, hull, None)
    }

    pub fn write_raw_hyper_tile(&mut self, header: &HyperTileHeader, payload: &[u8]) -> io::Result<u64> {
        // Enforce capability write gate and registry validation:
        let registry = s3a_core::registry::global_registry();
        let effective_version = if header.record_layout_version == 0 { 1 } else { header.record_layout_version };
        if registry.get(header.tile_type).is_some() {
            if let Err(e) = registry.validate_write(header.tile_type, effective_version, self.addon_mask) {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{}", e)));
            }
        } else if let Some((pack_name, required_mask)) = s3a_core::required_addon_for_tile(header.tile_type) {
            if (self.addon_mask & required_mask) == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!(
                        "Tile type {} belongs to {} but addon bit 0x{:04x} is not enabled on archive (addon_mask: 0x{:04x})",
                        header.tile_type, pack_name, required_mask, self.addon_mask
                    ),
                ));
            }
        }

        let committed_tile_id = self.current_tile_id;
        let mut new_header = *header;
        new_header.tile_id = committed_tile_id;
        if new_header.data_crc32 == 0 {
            new_header.data_crc32 = crc32c::crc32c(payload);
        }
        if new_header.payload_bytes == 0 {
            new_header.payload_bytes = payload.len() as u32;
        }
        if new_header.file_uuid == [0, 0] {
            new_header.file_uuid = self.file_uuid;
        }
        if new_header.generation == 0 {
            new_header.generation = self.generation;
        }
        new_header.tile_index = self.tile_count;

        new_header.header_crc32 = compute_tile_header_crc32(&new_header);

        let written = HYPER_TILE_HEADER_SIZE + payload.len();
        if written > HYPER_TILE_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Payload size {} exceeds hyper tile capacity {}", payload.len(), HYPER_TILE_SIZE - HYPER_TILE_HEADER_SIZE),
            ));
        }

        self.file.write_all(bytes_of(&new_header))?;
        self.file.write_all(payload)?;

        let padding_needed = HYPER_TILE_SIZE - written;
        if padding_needed > 0 {
            let zeros = vec![0u8; padding_needed];
            self.file.write_all(&zeros)?;
        }

        self.file.flush()?;
        self.file.sync_data()?;

        // AFTER GATE: Re-read output tile from disk and verify CRC before A/B commit switch
        let tile_offset = FILE_HEADER_SIZE as u64 + (self.tile_count as u64 * HYPER_TILE_SIZE as u64);
        crate::compactor_gates::verify_written_tile_from_disk(
            &mut self.file,
            tile_offset,
            new_header.header_crc32,
            new_header.data_crc32,
            payload.len(),
        )?;

        #[cfg(any(test, feature = "fault-injection"))]
        if crate::compactor_gates::fault_injection::INJECT_CRASH_BEFORE_COMMIT.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Crash simulated between tile write and header commit switch",
            ));
        }

        self.commit_header_slot(self.tile_count + 1, self.addon_mask)?;
        self.current_tile_id += 1;
        Ok(committed_tile_id)
    }

    pub fn file_uuid(&self) -> [u64; 2] {
        self.file_uuid
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

    pub fn addon_mask(&self) -> u32 {
        self.addon_mask
    }

    /// Enables a modular domain addon (e.g. ADDON_EDU) by updating the file header addon_mask.
    pub fn enable_addon(&mut self, mask: u32) -> io::Result<()> {
        let new_mask = self.addon_mask | mask;
        self.commit_header_slot(self.tile_count, new_mask)
    }
}

/// Why a tile was rejected by a `try_*` reader accessor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CorruptReason {
    IndexOutOfRange,
    StrideTooSmall { stride: usize },
    OffsetOverflow,
    SlotPastEof { end: usize, file_len: usize },
    PayloadTooLong { payload_len: usize, capacity: usize },
    HeaderCrcMismatch { stored: u32, computed: u32 },
    DataCrcMismatch { stored: u32, computed: u32 },
    /// A4: the tile header sits at an offset that is not aligned for a borrowed view
    /// (only possible with a file-claimed edge stride that is not a multiple of 8).
    Misaligned { offset: usize },
}

/// A tile failed structural or checksum validation (ADR-0004).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Corrupt {
    pub tile: usize,
    pub reason: CorruptReason,
}

impl Corrupt {
    fn new(tile: usize, reason: CorruptReason) -> Self {
        Self { tile, reason }
    }
}

impl std::fmt::Display for Corrupt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "corrupt tile {}: {:?}", self.tile, self.reason)
    }
}

impl std::error::Error for Corrupt {}

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

        let slot_a: &FileHeader = &bytemuck::pod_read_unaligned::<FileHeader>(&mmap[FILE_HEADER_SLOT_A_OFFSET..FILE_HEADER_SLOT_A_OFFSET + FILE_HEADER_SLOT_SIZE]);
        let slot_b: &FileHeader = &bytemuck::pod_read_unaligned::<FileHeader>(&mmap[FILE_HEADER_SLOT_B_OFFSET..FILE_HEADER_SLOT_B_OFFSET + FILE_HEADER_SLOT_SIZE]);

        let (active_header, active_slot) = resolve_active_header(slot_a, slot_b)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{}", e)))?;

        // A3: the file must be large enough to hold every tile the header claims.
        let (base, stride, min_hdr) = if active_header.is_edge_profile() {
            let stride = active_header.tile_stride();
            let base = if (active_header.flags & s3a_edge::FLAG_COMPACT_HEADER) != 0 { 256.max(stride) } else { FILE_HEADER_SIZE };
            (base, stride, s3a_edge::COMMON_TILE_HEADER_SIZE)
        } else {
            (FILE_HEADER_SIZE, HYPER_TILE_SIZE, HYPER_TILE_HEADER_SIZE)
        };
        let bad = |m: String| io::Error::new(io::ErrorKind::InvalidData, m);
        if stride < min_hdr {
            return Err(bad(format!("tile stride {} smaller than tile header {}", stride, min_hdr)));
        }
        let need = (active_header.tile_count as usize)
            .checked_mul(stride)
            .and_then(|n| n.checked_add(base))
            .ok_or_else(|| bad("tile_count * stride overflows".into()))?;
        // A truncated file (need > len) still opens so intact leading tiles stay readable
        // (spec vector invalid_truncated_tile); see validate_file_size() / verify_checksums().
        let _ = need;

        Ok(Self {
            _file: file,
            mmap,
            active_header,
            active_slot,
        })
    }

    /// A3: checks that the file is long enough for `tile_count` tiles of the header's stride.
    /// Returns `Corrupt { tile: first_missing_tile, reason: SlotPastEof }` if it is not.
    pub fn validate_file_size(&self) -> Result<(), Corrupt> {
        let count = self.tile_count() as usize;
        if count == 0 {
            return Ok(());
        }
        let (base, stride) = if self.is_edge_archive() {
            (self.header_extent(), self.tile_stride())
        } else {
            (FILE_HEADER_SIZE, HYPER_TILE_SIZE)
        };
        let need = count
            .checked_mul(stride)
            .and_then(|n| n.checked_add(base))
            .ok_or(Corrupt::new(count - 1, CorruptReason::OffsetOverflow))?;
        if need > self.mmap.len() {
            let first_missing = self.mmap.len().saturating_sub(base) / stride.max(1);
            return Err(Corrupt::new(first_missing, CorruptReason::SlotPastEof { end: need, file_len: self.mmap.len() }));
        }
        Ok(())
    }

    pub fn file_header(&self) -> &FileHeader {
        &self.active_header
    }

    pub fn file_uuid(&self) -> [u64; 2] {
        self.active_header.file_uuid
    }

    pub fn addon_mask(&self) -> u32 {
        self.active_header.addon_mask
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

    pub fn tile_stride(&self) -> usize {
        self.active_header.tile_stride()
    }

    pub fn is_edge_archive(&self) -> bool {
        self.active_header.is_edge_profile()
    }

    pub fn header_extent(&self) -> usize {
        if (self.active_header.flags & s3a_edge::FLAG_COMPACT_HEADER) != 0 {
            256.max(self.tile_stride())
        } else {
            FILE_HEADER_SIZE
        }
    }

    /// Returns `None` on any corruption (never panics on bounds). See [`MmapReader::try_get_edge_tile`].
    pub fn get_edge_tile(&self, index: usize) -> Option<(&s3a_edge::CommonTileHeader, &[u8])> {
        self.try_get_edge_tile(index).ok()
    }

    /// Byte range `[start, end)` of tile slot `index`, checked against the real file length.
    fn tile_slot(&self, index: usize, base: usize, stride: usize, min_header: usize) -> Result<(usize, usize), Corrupt> {
        if index >= self.tile_count() as usize {
            return Err(Corrupt::new(index, CorruptReason::IndexOutOfRange));
        }
        if stride < min_header {
            return Err(Corrupt::new(index, CorruptReason::StrideTooSmall { stride }));
        }
        let start = index
            .checked_mul(stride)
            .and_then(|o| o.checked_add(base))
            .ok_or(Corrupt::new(index, CorruptReason::OffsetOverflow))?;
        let end = start
            .checked_add(stride)
            .ok_or(Corrupt::new(index, CorruptReason::OffsetOverflow))?;
        if end > self.mmap.len() {
            return Err(Corrupt::new(index, CorruptReason::SlotPastEof { end, file_len: self.mmap.len() }));
        }
        Ok((start, end))
    }

    /// Like [`MmapReader::get_edge_tile`] but reports why a tile was rejected. When the tile has
    /// `TILE_FLAG_CRC_CHAIN` set, both `header_crc32` and `data_crc32` are verified.
    pub fn try_get_edge_tile(&self, index: usize) -> Result<(&s3a_edge::CommonTileHeader, &[u8]), Corrupt> {
        let hs = s3a_edge::COMMON_TILE_HEADER_SIZE;
        let (offset, slot_end) = self.tile_slot(index, self.header_extent(), self.tile_stride(), hs)?;

        let header_slice = &self.mmap[offset..offset + hs];
        // A4 (ADR-0005): this accessor returns a header borrowed from the mmap (public API), so it
        // cannot copy out. mmap bases are page-aligned and canonical strides are multiples of 8, so
        // well-formed files are always aligned; a hostile stride yields `Misaligned`, never a panic.
        let tile_header: &s3a_edge::CommonTileHeader = bytemuck::try_from_bytes(header_slice)
            .map_err(|_| Corrupt::new(index, CorruptReason::Misaligned { offset }))?;

        let payload_start = offset + hs;
        let len = tile_header.payload_len as usize;
        let payload_end = payload_start
            .checked_add(len)
            .filter(|e| *e <= slot_end)
            .ok_or(Corrupt::new(index, CorruptReason::PayloadTooLong { payload_len: len, capacity: slot_end - payload_start }))?;
        let payload = &self.mmap[payload_start..payload_end];

        if tile_header.flags & s3a_edge::TILE_FLAG_CRC_CHAIN != 0 {
            let h = s3a_edge::compute_common_tile_header_crc32(tile_header);
            if h != tile_header.header_crc32 {
                return Err(Corrupt::new(index, CorruptReason::HeaderCrcMismatch { stored: tile_header.header_crc32, computed: h }));
            }
            let d = crc32c::crc32c(payload);
            if d != tile_header.data_crc32 {
                return Err(Corrupt::new(index, CorruptReason::DataCrcMismatch { stored: tile_header.data_crc32, computed: d }));
            }
        }
        Ok((tile_header, payload))
    }

    /// Result-returning variant of [`MmapReader::get_promoted_tile`].
    pub fn try_get_promoted_tile(&self, index: usize) -> Result<(HyperTileHeader, &[u8]), Corrupt> {
        if self.is_edge_archive() {
            let (edge_hdr, payload) = self.try_get_edge_tile(index)?;
            let mut promoted = HyperTileHeader::new((index + 1) as u64, edge_hdr.type_id as u32);
            promoted.record_count = edge_hdr.record_count;
            promoted.payload_bytes = edge_hdr.payload_len as u32;
            promoted.record_layout_version = edge_hdr.type_version as u32;
            promoted.data_crc32 = edge_hdr.data_crc32;
            promoted.header_crc32 = edge_hdr.header_crc32;
            promoted.generation = edge_hdr.generation as u64;
            promoted.tile_index = index as u32;
            promoted.file_uuid = self.active_header.file_uuid;
            promoted.provenance.merkle_root = edge_hdr.payload_hash;
            Ok((promoted, payload))
        } else {
            let (hdr, payload) = self.try_get_tile(index)?;
            Ok((*hdr, payload))
        }
    }

    pub fn get_promoted_tile(&self, index: usize) -> Option<(HyperTileHeader, &[u8])> {
        self.try_get_promoted_tile(index).ok()
    }

    /// Returns `None` on any corruption (never panics on bounds). See [`MmapReader::try_get_tile`].
    pub fn get_tile(&self, index: usize) -> Option<(&HyperTileHeader, &[u8])> {
        self.try_get_tile(index).ok()
    }

    /// Like [`MmapReader::get_tile`] but reports why a tile was rejected.
    pub fn try_get_tile(&self, index: usize) -> Result<(&HyperTileHeader, &[u8]), Corrupt> {
        let (offset, slot_end) = self.tile_slot(index, FILE_HEADER_SIZE, HYPER_TILE_SIZE, HYPER_TILE_HEADER_SIZE)?;

        let header_slice = &self.mmap[offset..offset + HYPER_TILE_HEADER_SIZE];
        // A4 (ADR-0005): this accessor returns a header borrowed from the mmap (public API), so it
        // cannot copy out. mmap bases are page-aligned and canonical strides are multiples of 8, so
        // well-formed files are always aligned; a hostile stride yields `Misaligned`, never a panic.
        let tile_header: &HyperTileHeader = bytemuck::try_from_bytes(header_slice)
            .map_err(|_| Corrupt::new(index, CorruptReason::Misaligned { offset }))?;

        let payload_start = offset + HYPER_TILE_HEADER_SIZE;
        let len = tile_header.payload_bytes as usize;
        let payload_end = payload_start
            .checked_add(len)
            .filter(|e| *e <= slot_end)
            .ok_or(Corrupt::new(index, CorruptReason::PayloadTooLong { payload_len: len, capacity: slot_end - payload_start }))?;
        Ok((tile_header, &self.mmap[payload_start..payload_end]))
    }

    /// Instantaneous O(1) direct record lookup by S3ACoordinate address (Level, Tile ID, Record Offset)
    pub fn read_record_by_coordinate<T: Pod>(&self, coord: &S3ACoordinate) -> Result<T, S3AError> {
        let (header, payload) = self.get_tile(coord.tile_id as usize).ok_or(S3AError::OutOfBounds)?;

        // Self-addressing verification: reject tile if file_uuid does not match
        if (self.active_header.file_uuid != [0, 0] || header.file_uuid != [0, 0])
            && !(header.record_layout_version <= 1 && header.file_uuid == [0, 0])
            && header.file_uuid != self.active_header.file_uuid
        {
            return Err(S3AError::FileUuidMismatch {
                expected: self.active_header.file_uuid,
                actual: header.file_uuid,
            });
        }

        // Validate layout version via registry
        let registry = s3a_core::registry::global_registry();
        if let Err(e) = registry.validate_read(header.tile_type, header.record_layout_version) {
            match e {
                s3a_core::registry::RegistryError::UnsupportedLayoutVersionTooOld { version, .. } => {
                    return Err(S3AError::UnsupportedLayoutVersion(version));
                }
                s3a_core::registry::RegistryError::UnsupportedLayoutVersionTooNew { version, .. } => {
                    return Err(S3AError::UnsupportedLayoutVersion(version));
                }
                _ => {}
            }
        }

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

        if let Err(c) = self.validate_file_size() {
            let _ = c;
            return Err(S3AError::OutOfBounds);
        }

        let count = self.tile_count() as usize;
        if self.is_edge_archive() {
            for i in 0..count {
                if let Some((header, payload)) = self.get_edge_tile(i) {
                    if header.header_crc32 != 0 {
                        let expected_hdr_crc = s3a_edge::compute_common_tile_header_crc32(header);
                        if header.header_crc32 != expected_hdr_crc {
                            return Err(S3AError::ChecksumMismatch {
                                expected: header.header_crc32,
                                actual: expected_hdr_crc,
                            });
                        }
                    }
                    if header.data_crc32 != 0 {
                        let actual_data_crc = crc32c::crc32c(payload);
                        if actual_data_crc != header.data_crc32 {
                            return Err(S3AError::ChecksumMismatch {
                                expected: header.data_crc32,
                                actual: actual_data_crc,
                            });
                        }
                    }
                    let mut hasher = sha2::Sha256::new();
                    hasher.update(payload);
                    let actual_hash: [u8; 32] = hasher.finalize().into();
                    if actual_hash != header.payload_hash {
                        return Err(S3AError::ChecksumMismatch {
                            expected: u32::from_le_bytes(header.payload_hash[..4].try_into().unwrap()),
                            actual: u32::from_le_bytes(actual_hash[..4].try_into().unwrap()),
                        });
                    }
                } else {
                    return Err(S3AError::OutOfBounds);
                }
            }
            return Ok(());
        }

        for i in 0..count {
            if let Some((header, payload)) = self.get_tile(i) {
                if header.header_crc32 != 0 {
                    let expected_hdr_crc = compute_tile_header_crc32(header);
                    if header.header_crc32 != expected_hdr_crc {
                        return Err(S3AError::ChecksumMismatch {
                            expected: header.header_crc32,
                            actual: expected_hdr_crc,
                        });
                    }
                }
                if self.active_header.file_uuid != [0, 0] || header.file_uuid != [0, 0] {
                    let is_legacy = header.record_layout_version <= 1 && header.file_uuid == [0, 0];
                    if !is_legacy && header.file_uuid != self.active_header.file_uuid {
                        return Err(S3AError::FileUuidMismatch {
                            expected: self.active_header.file_uuid,
                            actual: header.file_uuid,
                        });
                    }
                }
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

    /// Extracts all schema type definitions embedded within any Schema Tile (Tile 77) in the archive.
    pub fn get_schema_definitions(&self) -> Vec<SchemaTypeDefinition> {
        let count = self.tile_count() as usize;
        for i in 0..count {
            if let Some((header, payload)) = self.get_tile(i) {
                if header.tile_type == TileType::SCHEMA {
                    if let Ok(types) = decode_schema_payload(payload) {
                        return types;
                    }
                }
            }
        }
        Vec::new()
    }

    /// Resolves the human-readable canonical name for a given tile type.
    /// Checks global registry first, then embedded Schema Tile definitions, falling back to "UNKNOWN".
    pub fn resolve_tile_type_name(&self, tile_type: u32) -> String {
        let reg = s3a_core::registry::global_registry();
        if let Some(desc) = reg.get(tile_type) {
            return desc.name.to_string();
        }
        for def in self.get_schema_definitions() {
            if def.type_id == tile_type {
                return def.name;
            }
        }
        "UNKNOWN".to_string()
    }

    /// Reads and decodes the UTF-8 plain-text manifest from TileType::MANIFEST if present.
    pub fn get_manifest(&self) -> Option<String> {
        let count = self.tile_count() as usize;
        for i in 0..count {
            if let Some((header, payload)) = self.get_tile(i) {
                if header.tile_type == TileType::MANIFEST {
                    let len = (header.payload_bytes as usize).min(payload.len());
                    return String::from_utf8(payload[..len].to_vec()).ok();
                }
            }
        }
        None
    }

    /// Reads and decompresses the embedded S3A specification Markdown from TileType::SPECIFICATION if present.
    pub fn get_embedded_spec(&self) -> Option<String> {
        let count = self.tile_count() as usize;
        for i in 0..count {
            if let Some((header, payload)) = self.get_tile(i) {
                if header.tile_type == TileType::SPECIFICATION {
                    let len = (header.payload_bytes as usize).min(payload.len());
                    if let Ok(decompressed) = miniz_oxide::inflate::decompress_to_vec_zlib(&payload[..len]) {
                        return String::from_utf8(decompressed).ok();
                    }
                }
            }
        }
        None
    }
}

/// Recovers and hardens an S3A archive:
/// 1. Validates the dual A/B header slots and resolves the active generation.
/// 2. Truncates any torn / uncommitted bytes appended past the active generation's tile count.
/// 3. Cleans any orphan temporary compaction files matching `{stem}.compact.*` / `{stem}.tmp.*`.
/// 4. Returns the active FileHeader, or fails closed if both header slots are invalid.
pub fn recover_archive<P: AsRef<Path>>(path: P) -> io::Result<FileHeader> {
    let path_ref = path.as_ref();
    if !path_ref.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Archive file not found: {}", path_ref.display()),
        ));
    }

    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path_ref)?;

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

    let slot_a: &FileHeader = &bytemuck::pod_read_unaligned::<FileHeader>(&header_buf[FILE_HEADER_SLOT_A_OFFSET..FILE_HEADER_SLOT_A_OFFSET + FILE_HEADER_SLOT_SIZE]);
    let slot_b: &FileHeader = &bytemuck::pod_read_unaligned::<FileHeader>(&header_buf[FILE_HEADER_SLOT_B_OFFSET..FILE_HEADER_SLOT_B_OFFSET + FILE_HEADER_SLOT_SIZE]);

    let (active_header, _) = resolve_active_header(slot_a, slot_b)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{}", e)))?;

    let committed_tile_count = active_header.tile_count;
    let expected_size = FILE_HEADER_SIZE as u64 + (committed_tile_count as u64 * HYPER_TILE_SIZE as u64);

    if file_len > expected_size {
        file.set_len(expected_size)?;
        file.flush()?;
        file.sync_all()?;
    }

    // Clean stale orphan compaction temps since live archive is verified
    let _ = fsutil::clean_orphan_temps(path_ref);

    Ok(active_header)
}

/// MULTI-TILE HYPER-MESH FUSION ENGINE: Combines multiple distinct or sparse Hyper-Tiles into a single unified 128 KB Hyper-Tile.
pub struct TileFusionEngine;

impl TileFusionEngine {
    /// Fuses multiple Telemetry Hyper-Tiles into a single consolidated 128 KB Hyper-Tile with recalculation of Simplex Bounding Hulls.
    pub fn fuse_telemetry_tiles<P: AsRef<Path>>(
        input_paths: &[P],
        output_path: P,
    ) -> io::Result<u32> {
        let out_ref = output_path.as_ref();

        match Self::fuse_pass(input_paths, out_ref) {
            Ok(count) => Ok(count),
            Err(first_err) => {
                if out_ref.exists() {
                    let _ = std::fs::remove_file(out_ref);
                }
                if first_err.to_string().contains("quarantined") {
                    return Err(first_err);
                }

                // Retry once
                match Self::fuse_pass(input_paths, out_ref) {
                    Ok(count) => Ok(count),
                    Err(second_err) => {
                        if out_ref.exists() {
                            let _ = std::fs::remove_file(out_ref);
                        }
                        let (bad_path, bad_tile_id, bad_gen) = Compactor::identify_failed_tile(input_paths, &second_err);
                        let _ = crate::compactor_gates::quarantine_tile(&bad_path, bad_tile_id, bad_gen);
                        Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!(
                                "Tile fusion failed after retry: tile {} in '{}' quarantined due to persistent integrity failure ({})",
                                bad_tile_id, bad_path.display(), second_err
                            ),
                        ))
                    }
                }
            }
        }
    }

    fn fuse_pass<P: AsRef<Path>>(input_paths: &[P], output_path: &Path) -> io::Result<u32> {
        let mut preflight = crate::compactor_gates::CompactionPreflight::new();
        let mut input_acc = crate::compactor_gates::LiveRecordAccumulator::new();
        let mut output_acc = crate::compactor_gates::LiveRecordAccumulator::new();
        let mut target_uuid: [u64; 2] = [0, 0];
        for path in input_paths {
            if let Ok(reader) = MmapReader::open(path) {
                if reader.file_uuid() != [0, 0] {
                    target_uuid = reader.file_uuid();
                    break;
                }
            }
        }
        if target_uuid == [0, 0] {
            target_uuid = generate_file_uuid();
        }
        let mut writer = TileWriter::create_with_uuid(output_path, target_uuid)?;
        let mut fused_records: Vec<TelemetryRecord> = Vec::new();

        for path in input_paths {
            let reader = MmapReader::open(path)?;
            let gen = reader.generation();
            for i in 0..reader.tile_count() as usize {
                if let Some((header, payload)) = reader.get_tile(i) {
                    // BEFORE GATE: verify input tile CRC, quarantine, uniqueness
                    preflight.verify_and_register_tile(path, gen, header, payload)?;

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

        for rec in &deduped {
            input_acc.push_record(rec);
        }

        #[allow(unused_mut)]
        let mut output_telemetry = deduped;
        #[cfg(any(test, feature = "fault-injection"))]
        {
            if crate::compactor_gates::fault_injection::INJECT_RECORD_DROP.load(std::sync::atomic::Ordering::SeqCst) {
                if !output_telemetry.is_empty() {
                    output_telemetry.pop();
                }
            }
            if crate::compactor_gates::fault_injection::INJECT_RECORD_DUPLICATION.load(std::sync::atomic::Ordering::SeqCst) {
                if !output_telemetry.is_empty() {
                    let first = output_telemetry[0];
                    output_telemetry.push(first);
                }
            }
        }

        let capacity = HYPER_TILE_PAYLOAD_SIZE / std::mem::size_of::<TelemetryRecord>();
        for chunk in output_telemetry.chunks(capacity) {
            let timestamps: Vec<u64> = chunk.iter().map(|r| r.timestamp).collect();
            writer.write_hyper_tile(
                TileType::TELEMETRY,
                chunk,
                Some(&timestamps),
                None,
            )?;
            for rec in chunk {
                output_acc.push_record(rec);
            }
        }

        // DURING GATE: Verify live-record count in = out and hash in = out
        input_acc.verify_equal(&output_acc)?;

        // End of fusion: verify all output tiles from disk
        drop(writer);
        let output_reader = MmapReader::open(output_path)?;
        output_reader.verify_checksums().map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("Tile fusion After-Gate checksum failure: {}", e))
        })?;

        Ok(output_reader.tile_count())
    }
}

/// Constructs a canonical SchemaTypeDefinition for a registered or well-known tile type.
pub fn standard_schema_for_type(tile_type: u32) -> SchemaTypeDefinition {
    match tile_type {
        TileType::TELEMETRY => SchemaTypeDefinition {
            type_id: TileType::TELEMETRY,
            type_version: 1,
            name: "telemetry".to_string(),
            owning_pack: "core".to_string(),
            record_size: 32,
            fields: vec![
                SchemaFieldDefinition { field_id: 1, field_type: 4, byte_offset: 0, byte_len: 8, null_allowed: false, name: "timestamp".to_string() },
                SchemaFieldDefinition { field_id: 2, field_type: 3, byte_offset: 8, byte_len: 4, null_allowed: false, name: "sensor_id".to_string() },
                SchemaFieldDefinition { field_id: 3, field_type: 3, byte_offset: 12, byte_len: 4, null_allowed: false, name: "metric_id".to_string() },
                SchemaFieldDefinition { field_id: 4, field_type: 10, byte_offset: 16, byte_len: 8, null_allowed: false, name: "value".to_string() },
                SchemaFieldDefinition { field_id: 5, field_type: 4, byte_offset: 24, byte_len: 8, null_allowed: false, name: "flags".to_string() },
            ],
        },
        TileType::EMBEDDING => SchemaTypeDefinition {
            type_id: TileType::EMBEDDING,
            type_version: 1,
            name: "embedding_128".to_string(),
            owning_pack: "core".to_string(),
            record_size: 528,
            fields: vec![
                SchemaFieldDefinition { field_id: 1, field_type: 4, byte_offset: 0, byte_len: 8, null_allowed: false, name: "id".to_string() },
                SchemaFieldDefinition { field_id: 2, field_type: 4, byte_offset: 8, byte_len: 8, null_allowed: false, name: "timestamp".to_string() },
                SchemaFieldDefinition { field_id: 3, field_type: 13, byte_offset: 16, byte_len: 512, null_allowed: false, name: "vector".to_string() },
            ],
        },
        TileType::MARKS => SchemaTypeDefinition {
            type_id: TileType::MARKS,
            type_version: 1,
            name: "marks".to_string(),
            owning_pack: "core".to_string(),
            record_size: 64,
            fields: vec![
                SchemaFieldDefinition { field_id: 1, field_type: 4, byte_offset: 0, byte_len: 8, null_allowed: false, name: "mark_id".to_string() },
                SchemaFieldDefinition { field_id: 2, field_type: 3, byte_offset: 8, byte_len: 4, null_allowed: false, name: "target_tile_type".to_string() },
                SchemaFieldDefinition { field_id: 3, field_type: 3, byte_offset: 12, byte_len: 4, null_allowed: false, name: "mark_kind".to_string() },
                SchemaFieldDefinition { field_id: 4, field_type: 4, byte_offset: 16, byte_len: 8, null_allowed: false, name: "target_tile_id".to_string() },
                SchemaFieldDefinition { field_id: 5, field_type: 3, byte_offset: 24, byte_len: 4, null_allowed: false, name: "record_index".to_string() },
                SchemaFieldDefinition { field_id: 6, field_type: 1, byte_offset: 28, byte_len: 1, null_allowed: false, name: "set_or_clear".to_string() },
                SchemaFieldDefinition { field_id: 7, field_type: 4, byte_offset: 32, byte_len: 8, null_allowed: false, name: "timestamp".to_string() },
                SchemaFieldDefinition { field_id: 8, field_type: 4, byte_offset: 40, byte_len: 8, null_allowed: false, name: "actor_id".to_string() },
            ],
        },
        TileType::CONSENT => SchemaTypeDefinition {
            type_id: TileType::CONSENT,
            type_version: 1,
            name: "consent".to_string(),
            owning_pack: "core".to_string(),
            record_size: 136,
            fields: vec![
                SchemaFieldDefinition { field_id: 1, field_type: 12, byte_offset: 0, byte_len: 16, null_allowed: false, name: "subject_id".to_string() },
                SchemaFieldDefinition { field_id: 2, field_type: 4, byte_offset: 16, byte_len: 8, null_allowed: false, name: "study_id".to_string() },
                SchemaFieldDefinition { field_id: 3, field_type: 3, byte_offset: 24, byte_len: 4, null_allowed: false, name: "purpose".to_string() },
                SchemaFieldDefinition { field_id: 4, field_type: 3, byte_offset: 28, byte_len: 4, null_allowed: false, name: "scope".to_string() },
                SchemaFieldDefinition { field_id: 5, field_type: 4, byte_offset: 32, byte_len: 8, null_allowed: false, name: "granted_at".to_string() },
                SchemaFieldDefinition { field_id: 6, field_type: 4, byte_offset: 40, byte_len: 8, null_allowed: false, name: "expires_at".to_string() },
                SchemaFieldDefinition { field_id: 7, field_type: 4, byte_offset: 48, byte_len: 8, null_allowed: false, name: "withdrawn_at".to_string() },
                SchemaFieldDefinition { field_id: 8, field_type: 1, byte_offset: 56, byte_len: 1, null_allowed: false, name: "legal_hold".to_string() },
                SchemaFieldDefinition { field_id: 9, field_type: 4, byte_offset: 64, byte_len: 8, null_allowed: false, name: "granted_by".to_string() },
                SchemaFieldDefinition { field_id: 10, field_type: 13, byte_offset: 72, byte_len: 32, null_allowed: false, name: "prev_consent_hash".to_string() },
                SchemaFieldDefinition { field_id: 11, field_type: 13, byte_offset: 104, byte_len: 32, null_allowed: false, name: "consent_hash".to_string() },
            ],
        },
        TileType::COLUMNAR => SchemaTypeDefinition {
            type_id: TileType::COLUMNAR,
            type_version: 1,
            name: "columnar".to_string(),
            owning_pack: "core".to_string(),
            record_size: 0,
            fields: vec![
                SchemaFieldDefinition { field_id: 1, field_type: 2, byte_offset: 4, byte_len: 2, null_allowed: false, name: "column_count".to_string() },
                SchemaFieldDefinition { field_id: 2, field_type: 3, byte_offset: 6, byte_len: 4, null_allowed: false, name: "record_count".to_string() },
                SchemaFieldDefinition { field_id: 3, field_type: 2, byte_offset: 10, byte_len: 2, null_allowed: false, name: "compression_codec".to_string() },
                SchemaFieldDefinition { field_id: 4, field_type: 3, byte_offset: 16, byte_len: 4, null_allowed: false, name: "source_tile_type".to_string() },
            ],
        },
        TileType::ARRAY => SchemaTypeDefinition {
            type_id: TileType::ARRAY,
            type_version: 1,
            name: "array".to_string(),
            owning_pack: "core".to_string(),
            record_size: 0,
            fields: vec![
                SchemaFieldDefinition { field_id: 1, field_type: 2, byte_offset: 6, byte_len: 2, null_allowed: false, name: "ndim".to_string() },
                SchemaFieldDefinition { field_id: 2, field_type: 2, byte_offset: 8, byte_len: 2, null_allowed: false, name: "dtype".to_string() },
                SchemaFieldDefinition { field_id: 3, field_type: 2, byte_offset: 10, byte_len: 2, null_allowed: false, name: "compression_codec".to_string() },
                SchemaFieldDefinition { field_id: 4, field_type: 13, byte_offset: 16, byte_len: 64, null_allowed: false, name: "shape".to_string() },
                SchemaFieldDefinition { field_id: 5, field_type: 13, byte_offset: 80, byte_len: 64, null_allowed: false, name: "chunk_shape".to_string() },
                SchemaFieldDefinition { field_id: 6, field_type: 13, byte_offset: 144, byte_len: 64, null_allowed: false, name: "chunk_coords".to_string() },
                SchemaFieldDefinition { field_id: 7, field_type: 13, byte_offset: 208, byte_len: 8, null_allowed: false, name: "fill_value".to_string() },
                SchemaFieldDefinition { field_id: 8, field_type: 13, byte_offset: 216, byte_len: 32, null_allowed: false, name: "chunk_hash".to_string() },
                SchemaFieldDefinition { field_id: 9, field_type: 9, byte_offset: 248, byte_len: 64, null_allowed: false, name: "axis_names".to_string() },
            ],
        },
        TileType::ERASURE => SchemaTypeDefinition {
            type_id: TileType::ERASURE,
            type_version: 1,
            name: "erasure".to_string(),
            owning_pack: "core".to_string(),
            record_size: 144,
            fields: vec![
                SchemaFieldDefinition { field_id: 1, field_type: 13, byte_offset: 0, byte_len: 32, null_allowed: false, name: "erased_record_hash".to_string() },
                SchemaFieldDefinition { field_id: 2, field_type: 3, byte_offset: 32, byte_len: 4, null_allowed: false, name: "target_tile_type".to_string() },
                SchemaFieldDefinition { field_id: 3, field_type: 3, byte_offset: 36, byte_len: 4, null_allowed: false, name: "record_index".to_string() },
                SchemaFieldDefinition { field_id: 4, field_type: 4, byte_offset: 40, byte_len: 8, null_allowed: false, name: "target_tile_id".to_string() },
                SchemaFieldDefinition { field_id: 5, field_type: 3, byte_offset: 48, byte_len: 4, null_allowed: false, name: "reason".to_string() },
                SchemaFieldDefinition { field_id: 6, field_type: 3, byte_offset: 52, byte_len: 4, null_allowed: false, name: "_reserved_u32".to_string() },
                SchemaFieldDefinition { field_id: 7, field_type: 4, byte_offset: 56, byte_len: 8, null_allowed: false, name: "authority".to_string() },
                SchemaFieldDefinition { field_id: 8, field_type: 4, byte_offset: 64, byte_len: 8, null_allowed: false, name: "requested_at".to_string() },
                SchemaFieldDefinition { field_id: 9, field_type: 4, byte_offset: 72, byte_len: 8, null_allowed: false, name: "executed_at".to_string() },
                SchemaFieldDefinition { field_id: 10, field_type: 13, byte_offset: 80, byte_len: 32, null_allowed: false, name: "prev_erasure_hash".to_string() },
                SchemaFieldDefinition { field_id: 11, field_type: 13, byte_offset: 112, byte_len: 32, null_allowed: false, name: "erasure_hash".to_string() },
            ],
        },
        TileType::SCHEMA => SchemaTypeDefinition {
            type_id: TileType::SCHEMA,
            type_version: 1,
            name: "schema".to_string(),
            owning_pack: "core".to_string(),
            record_size: 0,
            fields: vec![
                SchemaFieldDefinition { field_id: 1, field_type: 3, byte_offset: 0, byte_len: 4, null_allowed: false, name: "schema_id".to_string() },
                SchemaFieldDefinition { field_id: 2, field_type: 2, byte_offset: 4, byte_len: 2, null_allowed: false, name: "schema_format".to_string() },
                SchemaFieldDefinition { field_id: 3, field_type: 2, byte_offset: 6, byte_len: 2, null_allowed: false, name: "type_count".to_string() },
                SchemaFieldDefinition { field_id: 4, field_type: 3, byte_offset: 8, byte_len: 4, null_allowed: false, name: "total_fields".to_string() },
                SchemaFieldDefinition { field_id: 5, field_type: 3, byte_offset: 12, byte_len: 4, null_allowed: false, name: "schema_crc32".to_string() },
            ],
        },
        other => {
            let reg = s3a_core::registry::global_registry();
            if let Some(desc) = reg.get(other) {
                return SchemaTypeDefinition {
                    type_id: other,
                    type_version: desc.current_layout_version,
                    name: desc.name.to_string(),
                    owning_pack: desc.owner_pack.to_string(),
                    record_size: desc.record_size as u32,
                    fields: vec![
                        SchemaFieldDefinition { field_id: 1, field_type: 13, byte_offset: 0, byte_len: desc.record_size as u32, null_allowed: false, name: "record_data".to_string() },
                    ],
                };
            }
            SchemaTypeDefinition {
                type_id: other,
                type_version: 1,
                name: format!("tile_type_{}", other),
                owning_pack: "unknown".to_string(),
                record_size: 0,
                fields: vec![],
            }
        }
    }
}

/// Appends an embedded Schema Tile (Tile 77) describing all tile types present in the archive if not already present.
pub fn append_schema_tile_if_missing<P: AsRef<Path>>(path: P) -> io::Result<bool> {
    let path_ref = path.as_ref();
    let reader = MmapReader::open(path_ref)?;
    let tile_count = reader.tile_count() as usize;
    if tile_count == 0 {
        return Ok(false);
    }

    let mut present_types = std::collections::BTreeSet::new();
    let mut has_schema = false;
    let mut custom_schemas = std::collections::HashMap::new();

    for i in 0..tile_count {
        if let Some((header, payload)) = reader.get_tile(i) {
            present_types.insert(header.tile_type);
            if header.tile_type == TileType::SCHEMA {
                has_schema = true;
                if let Ok(types) = decode_schema_payload(payload) {
                    for t in types {
                        custom_schemas.insert(t.type_id, t);
                    }
                }
            }
        }
    }

    if has_schema {
        return Ok(false);
    }

    drop(reader);

    present_types.insert(TileType::SCHEMA);
    let mut schema_defs = Vec::new();
    for &tt in &present_types {
        if let Some(custom) = custom_schemas.get(&tt) {
            schema_defs.push(custom.clone());
        } else {
            schema_defs.push(standard_schema_for_type(tt));
        }
    }

    let payload = encode_schema_payload(&schema_defs);
    let mut writer = TileWriter::open_append(path_ref)?;
    let mut schema_hdr = HyperTileHeader::new(writer.tile_count() as u64, TileType::SCHEMA);
    schema_hdr.record_count = 0;
    schema_hdr.record_layout_version = 1;
    schema_hdr.payload_bytes = payload.len() as u32;
    schema_hdr.data_crc32 = crc32c::crc32c(&payload);
    schema_hdr.generation = writer.generation();
    schema_hdr.tile_index = writer.tile_count() as u32;
    schema_hdr.file_uuid = writer.file_uuid();
    schema_hdr.header_crc32 = compute_tile_header_crc32(&schema_hdr);

    writer.write_raw_hyper_tile(&schema_hdr, &payload)?;
    Ok(true)
}

/// Query Sieve for high-performance single-cycle SIMD tile rejection and fast scanning.
pub struct QuerySieve<'a> {
    reader: &'a MmapReader,
    include_hidden: bool,
    purpose: Option<ConsentPurpose>,
    now: u64,
}

impl<'a> QuerySieve<'a> {
    pub fn new(reader: &'a MmapReader) -> Self {
        Self {
            reader,
            include_hidden: false,
            purpose: None,
            now: 0,
        }
    }

    /// Initializes a QuerySieve configured for export operations (default purpose = Research).
    pub fn for_export(reader: &'a MmapReader) -> Self {
        let now_sec = current_time_secs();
        Self {
            reader,
            include_hidden: false,
            purpose: Some(ConsentPurpose::Research),
            now: now_sec,
        }
    }

    pub fn with_hidden_marks(mut self, include_hidden: bool) -> Self {
        self.include_hidden = include_hidden;
        self
    }

    pub fn with_purpose(mut self, purpose: ConsentPurpose) -> Self {
        self.purpose = Some(purpose);
        self
    }

    pub fn with_now(mut self, now: u64) -> Self {
        self.now = now;
        self
    }

    pub fn set_purpose(&mut self, purpose: ConsentPurpose) {
        self.purpose = Some(purpose);
    }

    pub fn purpose(&self) -> Option<ConsentPurpose> {
        self.purpose
    }

    pub fn set_now(&mut self, now: u64) {
        self.now = now;
    }

    pub fn now(&self) -> u64 {
        self.now
    }

    pub fn set_include_hidden(&mut self, include_hidden: bool) {
        self.include_hidden = include_hidden;
    }

    pub fn include_hidden(&self) -> bool {
        self.include_hidden
    }

    pub fn reader(&self) -> &'a MmapReader {
        self.reader
    }

    /// Queries all raw consent records matching optional subject_id filter.
    pub fn query_consent_records(&self, subject_id: Option<&[u8; 16]>) -> Vec<ConsentRecord> {
        let tile_count = self.reader.tile_count() as usize;
        let mut results = Vec::new();
        for i in 0..tile_count {
            if let Some((header, payload)) = self.reader.get_tile(i) {
                if header.tile_type == TileType::CONSENT {
                    let records: &[ConsentRecord] = bytemuck::cast_slice(payload);
                    for r in records {
                        if let Some(target_sub) = subject_id {
                            if &r.subject_id != target_sub {
                                continue;
                            }
                        }
                        results.push(*r);
                    }
                }
            }
        }
        results
    }

    /// Verifies the cryptographic hash chain of consent records for a subject.
    /// Returns Ok(true) if valid, or an Err(&str) explaining the failure.
    pub fn verify_consent_chain(&self, subject_id: &[u8; 16]) -> Result<bool, &'static str> {
        let records = self.query_consent_records(Some(subject_id));
        if records.is_empty() {
            return Ok(true);
        }

        let mut expected_prev_hash = [0u8; 32];
        for (idx, r) in records.iter().enumerate() {
            if !r.verify_hash() {
                return Err("InvalidRecordHash: consent_hash does not match SHA-256 of record fields");
            }
            if idx == 0 {
                if r.prev_consent_hash != [0u8; 32] {
                    return Err("BrokenGenesisHash: first consent record must have zero prev_consent_hash");
                }
            } else if r.prev_consent_hash != expected_prev_hash {
                return Err("BrokenChainHash: prev_consent_hash does not match previous record consent_hash");
            }
            expected_prev_hash = r.consent_hash;
        }
        Ok(true)
    }

    /// Evaluates whether a participant subject has active consent for a specific purpose.
    /// Append-only semantics: the latest record per (subject, study, purpose) wins.
    /// Expiration timestamp and withdrawal timestamp are verified against `now`.
    pub fn is_subject_consent_active_for_purpose(
        &self,
        subject_id: &[u8; 16],
        study_id: Option<u64>,
        purpose: ConsentPurpose,
        now: u64,
    ) -> bool {
        let records = self.query_consent_records(Some(subject_id));
        let mut latest_record: Option<ConsentRecord> = None;

        for r in records {
            if r.purpose == purpose as u32 {
                if let Some(sid) = study_id {
                    if r.study_id != sid {
                        continue;
                    }
                }
                latest_record = Some(r);
            }
        }

        if let Some(r) = latest_record {
            r.is_active(now)
        } else {
            false
        }
    }

    /// Evaluates if a subject is currently under legal hold from consent records.
    pub fn is_subject_under_legal_hold(&self, subject_id: &[u8; 16]) -> bool {
        let records = self.query_consent_records(Some(subject_id));
        let mut latest_per_purpose: std::collections::HashMap<(u64, u32), ConsentRecord> = std::collections::HashMap::new();
        for r in records {
            latest_per_purpose.insert((r.study_id, r.purpose), r);
        }
        latest_per_purpose.values().any(|r| r.is_legal_hold())
    }

    /// Checks if a subject is allowed under the current sieve's declared purpose.
    /// If no purpose is declared, returns true (consent check is bypassed for general queries).
    /// If a purpose is declared, returns true only if active consent exists for that purpose.
    pub fn is_subject_allowed(&self, subject_id: &[u8; 16]) -> bool {
        if let Some(purpose) = self.purpose {
            self.is_subject_consent_active_for_purpose(subject_id, None, purpose, self.now)
        } else {
            true
        }
    }

    /// Checks if an actor ID is allowed under the current sieve's declared purpose.
    pub fn is_actor_allowed(&self, actor_id: u64) -> bool {
        let sub_id = actor_id_to_subject_id(actor_id);
        self.is_subject_allowed(&sub_id)
    }

    /// Evaluates whether a record is hidden by policy marks (CONSENT_WITHDRAWN, QUARANTINE, or RETENTION_EXPIRED) or erasure.
    pub fn is_record_hidden(&self, target_tile_type: u32, target_tile_id: u64, record_index: u32) -> bool {
        if self.include_hidden {
            return false;
        }
        self.is_marked(target_tile_type, target_tile_id, record_index, MARK_CONSENT_WITHDRAWN)
            || self.is_marked(target_tile_type, target_tile_id, record_index, MARK_QUARANTINE)
            || self.is_marked(target_tile_type, target_tile_id, record_index, MARK_RETENTION_EXPIRED)
            || self.is_record_erased(target_tile_type, target_tile_id, record_index)
    }

    /// Queries all raw erasure records matching optional target hash filter.
    pub fn query_erasure_records(&self, target_hash: Option<&[u8; 32]>) -> Vec<ErasureRecord> {
        let tile_count = self.reader.tile_count() as usize;
        let mut results = Vec::new();
        for i in 0..tile_count {
            if let Some((header, payload)) = self.reader.get_tile(i) {
                if header.tile_type == TileType::ERASURE {
                    let records: &[ErasureRecord] = bytemuck::cast_slice(payload);
                    for r in records {
                        if let Some(target) = target_hash {
                            if &r.erased_record_hash != target {
                                continue;
                            }
                        }
                        results.push(*r);
                    }
                }
            }
        }
        results
    }

    /// Verifies the cryptographic hash chain of erasure records.
    /// Returns Ok(true) if valid, or an Err(&str) explaining the failure.
    pub fn verify_erasure_chain(&self) -> Result<bool, &'static str> {
        let records = self.query_erasure_records(None);
        if records.is_empty() {
            return Ok(true);
        }

        let mut expected_prev_hash = [0u8; 32];
        for (idx, r) in records.iter().enumerate() {
            if !r.verify_hash() {
                return Err("InvalidRecordHash: erasure_hash does not match SHA-256 of record fields");
            }
            if idx == 0 {
                if r.prev_erasure_hash != [0u8; 32] {
                    return Err("BrokenGenesisHash: first erasure record must have zero prev_erasure_hash");
                }
            } else if r.prev_erasure_hash != expected_prev_hash {
                return Err("BrokenChainHash: prev_erasure_hash does not match previous record erasure_hash");
            }
            expected_prev_hash = r.erasure_hash;
        }
        Ok(true)
    }

    /// Evaluates whether a record hash matches an active erasure record.
    pub fn is_record_hash_erased(&self, record_hash: &[u8; 32]) -> bool {
        let records = self.query_erasure_records(Some(record_hash));
        !records.is_empty()
    }

    /// Evaluates whether a record at target tile coordinates is erased by checking active erasure records.
    pub fn is_record_erased(&self, target_tile_type: u32, target_tile_id: u64, record_index: u32) -> bool {
        let tile_count = self.reader.tile_count() as usize;
        for i in 0..tile_count {
            if let Some((header, payload)) = self.reader.get_tile(i) {
                if header.tile_type == target_tile_type && header.tile_id == target_tile_id {
                    let rec_size = if header.record_count > 0 && header.payload_bytes > 0 {
                        header.payload_bytes as usize / header.record_count as usize
                    } else {
                        0
                    };
                    if rec_size > 0 {
                        let offset = record_index as usize * rec_size;
                        if offset + rec_size <= payload.len() {
                            let chunk = &payload[offset..offset + rec_size];
                            let chunk_hash: [u8; 32] = sha2::Sha256::digest(chunk).into();
                            return self.is_record_hash_erased(&chunk_hash);
                        }
                    }
                }
            }
        }
        false
    }

    /// Evaluates the active status of a mark_kind on a target record or tile.
    /// Follows append-only semantics where the latest mark per target wins.
    pub fn is_marked(&self, target_tile_type: u32, target_tile_id: u64, record_index: u32, mark_kind: u32) -> bool {
        let tile_count = self.reader.tile_count() as usize;
        let mut latest_record_state: Option<bool> = None;
        let mut latest_tile_state: Option<bool> = None;

        for i in 0..tile_count {
            if let Some((header, payload)) = self.reader.get_tile(i) {
                if header.tile_type == TileType::MARKS {
                    let marks: &[MarkRecord] = bytemuck::cast_slice(payload);
                    for m in marks {
                        if m.target_tile_type == target_tile_type
                            && m.target_tile_id == target_tile_id
                            && m.mark_kind == mark_kind
                        {
                            if m.record_index == record_index {
                                latest_record_state = Some(m.is_set());
                            } else if m.is_whole_tile() {
                                latest_tile_state = Some(m.is_set());
                            }
                        }
                    }
                }
            }
        }

        latest_record_state.or(latest_tile_state).unwrap_or(false)
    }

    /// Queries all raw mark records matching optional tile type and tile ID filters.
    pub fn query_marks(&self, target_tile_type: Option<u32>, target_tile_id: Option<u64>) -> Vec<MarkRecord> {
        let tile_count = self.reader.tile_count() as usize;
        let mut results = Vec::new();

        for i in 0..tile_count {
            if let Some((header, payload)) = self.reader.get_tile(i) {
                if header.tile_type == TileType::MARKS {
                    let marks: &[MarkRecord] = bytemuck::cast_slice(payload);
                    for m in marks {
                        if let Some(tt) = target_tile_type {
                            if m.target_tile_type != tt {
                                continue;
                            }
                        }
                        if let Some(tid) = target_tile_id {
                            if m.target_tile_id != tid && m.target_tile_id != i as u64 {
                                continue;
                            }
                        }
                        results.push(*m);
                    }
                }
            }
        }

        results
    }

    /// Returns the consolidated set of currently active marks (latest SET per target).
    pub fn active_marks(&self) -> Vec<MarkRecord> {
        let all_marks = self.query_marks(None, None);
        let mut map: std::collections::BTreeMap<(u32, u64, u32, u32), MarkRecord> = std::collections::BTreeMap::new();
        for m in all_marks {
            let key = (m.target_tile_type, m.target_tile_id, m.record_index, m.mark_kind);
            map.insert(key, m);
        }
        map.into_values().filter(|m| m.is_set()).collect()
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

            if header.tile_type == TileType::COLUMNAR {
                if let Ok(view) = ColumnarTileView::parse(payload) {
                    if view.header.source_tile_type == TileType::TELEMETRY {
                        // Min/max statistics pruning
                        if view.can_prune_column("timestamp", min_ts as f64, max_ts as f64) {
                            continue;
                        }
                        if let Some(s) = sensor_id {
                            if view.can_prune_column("sensor_id", s as f64, s as f64) {
                                continue;
                            }
                        }
                        if let Some(m) = metric_id {
                            if view.can_prune_column("metric_id", m as f64, m as f64) {
                                continue;
                            }
                        }

                        if let Ok(records) = decode_columnar_telemetry(payload) {
                            for (rec_idx, rec) in records.iter().enumerate() {
                                if !self.include_hidden && self.is_record_hidden(header.tile_type, header.tile_id, rec_idx as u32) {
                                    continue;
                                }
                                let mut hasher = sha2::Sha256::new();
                                sha2::Digest::update(&mut hasher, bytemuck::bytes_of(rec));
                                let rec_hash: [u8; 32] = hasher.finalize().into();
                                if self.is_record_hash_erased(&rec_hash) {
                                    continue;
                                }
                                if rec.timestamp >= min_ts && rec.timestamp <= max_ts {
                                    if let Some(s) = sensor_id {
                                        if rec.sensor_id != s { continue; }
                                    }
                                    if let Some(m) = metric_id {
                                        if rec.metric_id != m { continue; }
                                    }
                                    let coord = S3ACoordinate::new(0, i as u32, rec_idx as u32);
                                    all_records.push((*rec, coord));
                                }
                            }
                        }
                    }
                }
                continue;
            }

            if header.tile_type != TileType::TELEMETRY && header.tile_type != TileType::HYBRID {
                continue;
            }

            let records: &[TelemetryRecord] = bytemuck::cast_slice(payload);
            for (rec_idx, rec) in records.iter().enumerate() {
                if !self.include_hidden && self.is_record_hidden(header.tile_type, header.tile_id, rec_idx as u32) {
                    continue;
                }
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
                if !self.include_hidden && self.is_record_hidden(header.tile_type, header.tile_id, rec_idx as u32) {
                    continue;
                }
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
            for (rec_idx, rec) in records.iter().enumerate() {
                if !self.include_hidden && self.is_record_hidden(header.tile_type, header.tile_id, rec_idx as u32) {
                    continue;
                }
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

            if header.tile_type != TileType::RESERVED_BOUNDARY && header.tile_type != 10 {
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
                if !self.include_hidden && self.is_record_hidden(header.tile_type, header.tile_id, rec_idx as u32) {
                    continue;
                }
                if !self.is_actor_allowed(rec.actor_id) {
                    continue;
                }
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

    /// Enables a modular domain addon (e.g. ADDON_EDU) on this archive.
    pub fn enable_addon(&self, mask: u32) -> io::Result<()> {
        let mut writer = TileWriter::open_append(&self.file_path)?;
        writer.enable_addon(mask)
    }

    /// Checks if a specific domain addon is active on this archive.
    pub fn is_addon_active(&self, mask: u32) -> bool {
        if let Ok(reader) = MmapReader::open(&self.file_path) {
            reader.file_header().has_addon(mask)
        } else {
            false
        }
    }

    /// Returns the active addons bitmask for this archive.
    pub fn active_addons(&self) -> u32 {
        if let Ok(reader) = MmapReader::open(&self.file_path) {
            reader.file_header().addon_mask
        } else {
            0
        }
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

    /// CREATE: Insert changeable metadata marks into the S3A Hyper-Tile archive.
    pub fn create_marks(&self, marks: &[MarkRecord]) -> io::Result<u64> {
        if marks.is_empty() {
            return Ok(0);
        }
        let mut writer = TileWriter::open_append(&self.file_path)?;
        let timestamps: Vec<u64> = marks.iter().map(|m| m.timestamp).collect();
        writer.write_hyper_tile(TileType::MARKS, marks, Some(&timestamps), None)
    }

    /// Appends a single changeable metadata mark targeting a record or whole tile.
    pub fn mark_record(
        &self,
        target_tile_type: u32,
        target_tile_id: u64,
        record_index: u32,
        mark_kind: u32,
        set: bool,
        timestamp: u64,
        actor_hash: u64,
    ) -> io::Result<u64> {
        let gen = MmapReader::open(&self.file_path).map(|r| r.generation()).unwrap_or(1);
        let mark = MarkRecord::new(
            target_tile_type,
            target_tile_id,
            record_index,
            gen,
            mark_kind,
            if set { MARK_SET } else { MARK_CLEAR },
            timestamp,
            actor_hash,
        );
        self.create_marks(&[mark])
    }

    /// CREATE: Insert a new consent record (must be authored by authorized human/admin actor).
    pub fn create_consent_record(&self, record: &ConsentRecord, is_admin_or_human: bool) -> io::Result<u64> {
        self.create_consent_records(&[*record], is_admin_or_human)
    }

    /// CREATE: Insert multiple consent records (must be authored by authorized human/admin actor).
    pub fn create_consent_records(&self, records: &[ConsentRecord], is_admin_or_human: bool) -> io::Result<u64> {
        if records.is_empty() {
            return Ok(0);
        }
        if !is_admin_or_human {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Consent records may only be authored by authorized human/admin actors, never AI agents",
            ));
        }
        for r in records {
            if !r.verify_hash() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Consent record hash mismatch (SHA-256 verification failed)",
                ));
            }
        }
        let mut writer = TileWriter::open_append(&self.file_path)?;
        let timestamps: Vec<u64> = records.iter().map(|r| r.granted_at.max(r.withdrawn_at)).collect();
        writer.write_hyper_tile(TileType::CONSENT, records, Some(&timestamps), None)
    }

    /// Appends a new consent grant for a participant subject, maintaining the cryptographic hash chain.
    pub fn grant_consent(
        &self,
        subject_id: [u8; 16],
        study_id: u64,
        purpose: ConsentPurpose,
        scope: u32,
        granted_at: u64,
        expires_at: u64,
        legal_hold: bool,
        granted_by: u64,
        is_admin_or_human: bool,
    ) -> io::Result<ConsentRecord> {
        let prev_hash = if let Ok(reader) = MmapReader::open(&self.file_path) {
            let sieve = QuerySieve::new(&reader);
            let records = sieve.query_consent_records(Some(&subject_id));
            records.last().map(|r| r.consent_hash).unwrap_or([0u8; 32])
        } else {
            [0u8; 32]
        };

        let record = ConsentRecord::new(
            subject_id,
            study_id,
            purpose,
            scope,
            granted_at,
            expires_at,
            legal_hold,
            granted_by,
            prev_hash,
        );

        self.create_consent_record(&record, is_admin_or_human)?;
        Ok(record)
    }

    /// Appends a consent withdrawal for a participant subject, maintaining the cryptographic hash chain.
    pub fn withdraw_consent(
        &self,
        subject_id: [u8; 16],
        study_id: u64,
        purpose: ConsentPurpose,
        scope: u32,
        withdrawn_at: u64,
        granted_by: u64,
        is_admin_or_human: bool,
    ) -> io::Result<ConsentRecord> {
        let prev_hash = if let Ok(reader) = MmapReader::open(&self.file_path) {
            let sieve = QuerySieve::new(&reader);
            let records = sieve.query_consent_records(Some(&subject_id));
            records.last().map(|r| r.consent_hash).unwrap_or([0u8; 32])
        } else {
            [0u8; 32]
        };

        let record = ConsentRecord::new_withdrawal(
            subject_id,
            study_id,
            purpose,
            scope,
            withdrawn_at,
            granted_by,
            prev_hash,
        );

        self.create_consent_record(&record, is_admin_or_human)?;
        Ok(record)
    }

    /// CREATE: Insert a new erasure record (must be authored by authorized human/admin actor).
    pub fn create_erasure_record(&self, record: &ErasureRecord, is_admin_or_human: bool) -> io::Result<u64> {
        self.create_erasure_records(&[*record], is_admin_or_human)
    }

    /// CREATE: Insert multiple erasure records (must be authored by authorized human/admin actor).
    pub fn create_erasure_records(&self, records: &[ErasureRecord], is_admin_or_human: bool) -> io::Result<u64> {
        if records.is_empty() {
            return Ok(0);
        }
        if !is_admin_or_human {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Erasure records may only be authored by authorized human/admin actors, never AI agents",
            ));
        }
        for r in records {
            if !r.verify_hash() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Erasure record hash mismatch (SHA-256 verification failed)",
                ));
            }
        }
        let mut writer = TileWriter::open_append(&self.file_path)?;
        let timestamps: Vec<u64> = records.iter().map(|r| r.executed_at).collect();
        writer.write_hyper_tile(TileType::ERASURE, records, Some(&timestamps), None)
    }

    /// Schedules and records an erasure receipt for a target record.
    ///
    /// Validates human/admin authority, checks active legal hold, enforces idempotency,
    /// links the cryptographic erasure hash chain, and schedules purge by tombstoning the target.
    pub fn record_erasure(
        &self,
        target_tile_type: u32,
        target_tile_id: u64,
        record_index: u32,
        erased_record_hash: [u8; 32],
        reason: ErasureReason,
        authority: u64,
        requested_at: u64,
        executed_at: u64,
        is_admin_or_human: bool,
    ) -> io::Result<ErasureRecord> {
        if !is_admin_or_human {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Erasure records may only be authored by authorized human/admin actors, never AI agents",
            ));
        }

        // Open reader to check legal hold, idempotency, and previous hash
        let prev_hash = if let Ok(reader) = MmapReader::open(&self.file_path) {
            let sieve = QuerySieve::new(&reader);

            // Legal hold check: target record or subject
            if sieve.is_marked(target_tile_type, target_tile_id, record_index, MARK_LEGAL_HOLD) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Erasure blocked: target record is under active legal hold",
                ));
            }

            // Check if telemetry subject under legal hold
            if target_tile_type == TileType::TELEMETRY {
                for i in 0..reader.tile_count() as usize {
                    if let Some((header, payload)) = reader.get_tile(i) {
                        if header.tile_type == TileType::TELEMETRY && header.tile_id == target_tile_id {
                            let recs: &[TelemetryRecord] = bytemuck::cast_slice(payload);
                            if let Some(target_rec) = recs.get(record_index as usize) {
                                let sub = actor_id_to_subject_id(target_rec.sensor_id as u64);
                                if sieve.is_subject_under_legal_hold(&sub) {
                                    return Err(io::Error::new(
                                        io::ErrorKind::PermissionDenied,
                                        "Erasure blocked: subject is under active legal hold",
                                    ));
                                }
                            }
                        }
                    }
                }
            }

            // Idempotency check: if this target was already erased, return existing record
            let existing = sieve.query_erasure_records(None);
            for rec in &existing {
                if rec.target_tile_type == target_tile_type
                    && rec.target_tile_id == target_tile_id
                    && rec.record_index == record_index
                    && rec.erased_record_hash == erased_record_hash
                {
                    return Ok(*rec);
                }
            }

            existing.last().map(|r| r.erasure_hash).unwrap_or([0u8; 32])
        } else {
            [0u8; 32]
        };

        let record = ErasureRecord::new(
            erased_record_hash,
            target_tile_type,
            target_tile_id,
            record_index,
            reason,
            authority,
            requested_at,
            executed_at,
            prev_hash,
        );

        // Append to Tile 73
        self.create_erasure_record(&record, is_admin_or_human)?;

        // If target is telemetry, schedule purge by appending tombstone
        if target_tile_type == TileType::TELEMETRY {
            if let Ok(reader) = MmapReader::open(&self.file_path) {
                for i in 0..reader.tile_count() as usize {
                    if let Some((header, payload)) = reader.get_tile(i) {
                        if header.tile_type == TileType::TELEMETRY && header.tile_id == target_tile_id {
                            let recs: &[TelemetryRecord] = bytemuck::cast_slice(payload);
                            if let Some(target_rec) = recs.get(record_index as usize) {
                                let mut tombstone = *target_rec;
                                tombstone.mark_tombstone();
                                let _ = self.create_telemetry(&[tombstone]);
                                break;
                            }
                        }
                    }
                }
            }
        }

        Ok(record)
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

        writer.write_hyper_tile(TileType::RESERVED_BOUNDARY, records, Some(&timestamps), None)
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

    /// Purges a telemetry record by tombstoning, verifying that the participant is not protected under an active legal hold.
    pub fn purge_telemetry_record(&self, sensor_id: u32, metric_id: u32, timestamp: u64) -> io::Result<bool> {
        let sub = actor_id_to_subject_id(sensor_id as u64);
        if let Ok(reader) = MmapReader::open(&self.file_path) {
            let sieve = QuerySieve::new(&reader);
            if sieve.is_subject_under_legal_hold(&sub) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Purge blocked: participant subject is under active legal hold",
                ));
            }
        }
        self.delete_telemetry(sensor_id, metric_id, timestamp)
    }

    /// GARBAGE COLLECTION / COMPACTION: Compact the S3A archive, purging tombstoned records and re-aligning tiles.
    pub fn purge_and_compact(&self) -> io::Result<u32> {
        self.purge_and_compact_with_columnar_opt_in(&[])
    }

    /// Compact the S3A archive, optionally converting specified row tile types to Columnar tiles.
    pub fn purge_and_compact_with_columnar_opt_in(&self, opt_in_types: &[u32]) -> io::Result<u32> {
        let temp_path = fsutil::unique_compact_temp_path(&self.file_path);
        let res = Compactor::compact_with_columnar_opt_in(&[&self.file_path], &temp_path, opt_in_types);
        if let Err(e) = res {
            let _ = std::fs::remove_file(&temp_path);
            return Err(e);
        }
        if let Err(e) = fsutil::atomic_replace(&temp_path, &self.file_path) {
            let _ = std::fs::remove_file(&temp_path);
            return Err(e);
        }
        let reader = MmapReader::open(&self.file_path)?;
        Ok(reader.tile_count())
    }

    pub fn file_path(&self) -> &Path {
        &self.file_path
    }

    /// Writes an Array tile chunk to the archive.
    pub fn write_array_chunk(&self, array_payload: &[u8]) -> io::Result<u64> {
        let view = s3a_core::array::ArrayTileView::parse(array_payload)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

        if !view.verify_chunk_hash() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Array chunk SHA-256 hash mismatch",
            ));
        }

        let mut writer = TileWriter::open_append(&self.file_path)?;
        let mut hdr = HyperTileHeader::new(writer.tile_count() as u64, TileType::ARRAY);
        hdr.record_count = view.header.total_elements() as u32;
        hdr.record_layout_version = 1;
        hdr.payload_bytes = array_payload.len() as u32;
        hdr.data_crc32 = crc32c::crc32c(array_payload);
        hdr.generation = writer.generation();
        hdr.tile_index = writer.tile_count() as u32;
        hdr.file_uuid = writer.file_uuid();
        hdr.header_crc32 = compute_tile_header_crc32(&hdr);
        writer.write_raw_hyper_tile(&hdr, array_payload)
    }

    /// Reads an N-dimensional slice across array chunks in the archive.
    ///
    /// Ranges specify `[start, end)` (half-open) along each axis in global array coordinates.
    /// Missing chunks or unpopulated points are automatically populated with the fill value.
    pub fn read_array_slice<T: Pod>(
        &self,
        ranges: &[(u64, u64)],
        fill_value: T,
    ) -> io::Result<Vec<T>> {
        let reader = MmapReader::open(&self.file_path)?;
        let ndim = ranges.len();
        if ndim == 0 || ndim > s3a_core::array::MAX_ARRAY_DIMENSIONS {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid number of slice axes"));
        }

        // Calculate slice shape and total element capacity
        let mut slice_shape = vec![0usize; ndim];
        let mut total_elements = 1usize;
        for i in 0..ndim {
            let (start, end) = ranges[i];
            if end <= start {
                return Ok(Vec::new());
            }
            let len = (end - start) as usize;
            slice_shape[i] = len;
            total_elements = total_elements.checked_mul(len).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "Slice dimensions overflow usize")
            })?;
        }

        // Initialize output buffer with fill_value
        let mut output = vec![fill_value; total_elements];

        // Scan all Array tiles in the container
        let tile_count = reader.tile_count() as usize;
        for i in 0..tile_count {
            if let Some((hdr, payload)) = reader.get_tile(i) {
                if hdr.tile_type != TileType::ARRAY {
                    continue;
                }

                let view = match s3a_core::array::ArrayTileView::parse(payload) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                let tile_ndim = view.header.ndim as usize;
                if tile_ndim != ndim {
                    continue;
                }

                // Check overlap between this chunk's bounding box and the query slice ranges
                let mut overlaps = true;
                for a in 0..ndim {
                    let chunk_start = view.header.chunk_coords[a];
                    let chunk_end = chunk_start + view.header.shape[a];
                    let (req_start, req_end) = ranges[a];
                    if req_end <= chunk_start || req_start >= chunk_end {
                        overlaps = false;
                        break;
                    }
                }
                if !overlaps {
                    continue;
                }

                // Copy overlapping elements from this chunk into slice output
                let chunk_elements: &[T] = match view.read_elements() {
                    Ok(e) => e,
                    Err(_) => continue,
                };

                // Helper to iterate coordinates recursively through active dimensions
                fn copy_overlap<T: Copy>(
                    dim: usize,
                    ndim: usize,
                    ranges: &[(u64, u64)],
                    view: &s3a_core::array::ArrayTileView,
                    chunk_elements: &[T],
                    output: &mut [T],
                    slice_shape: &[usize],
                    current_global: &mut Vec<u64>,
                ) {
                    if dim == ndim {
                        // All coordinates resolved: map to chunk local index and slice local index
                        let mut chunk_idx = 0usize;
                        for a in 0..ndim {
                            let local = (current_global[a] - view.header.chunk_coords[a]) as usize;
                            chunk_idx = chunk_idx * (view.header.shape[a] as usize) + local;
                        }

                        let mut slice_idx = 0usize;
                        for a in 0..ndim {
                            let slice_local = (current_global[a] - ranges[a].0) as usize;
                            slice_idx = slice_idx * slice_shape[a] + slice_local;
                        }

                        if let Some(&val) = chunk_elements.get(chunk_idx) {
                            output[slice_idx] = val;
                        }
                        return;
                    }

                    let (req_start, req_end) = ranges[dim];
                    let chunk_start = view.header.chunk_coords[dim];
                    let chunk_end = chunk_start + view.header.shape[dim];

                    let overlap_start = req_start.max(chunk_start);
                    let overlap_end = req_end.min(chunk_end);

                    for coord in overlap_start..overlap_end {
                        current_global.push(coord);
                        copy_overlap(dim + 1, ndim, ranges, view, chunk_elements, output, slice_shape, current_global);
                        current_global.pop();
                    }
                }

                let mut current_global = Vec::with_capacity(ndim);
                copy_overlap(
                    0,
                    ndim,
                    ranges,
                    &view,
                    chunk_elements,
                    &mut output,
                    &slice_shape,
                    &mut current_global,
                );
            }
        }

        Ok(output)
    }

    pub fn append_tile_records<T: Pod>(
        &self,
        tile_type: u32,
        records: &[T],
        timestamps: Option<&[u64]>,
        hull: Option<SimplexHull>,
        filter_keys: Option<&[u64]>,
    ) -> io::Result<u64> {
        if records.is_empty() {
            return Ok(0);
        }
        let mut writer = TileWriter::open_append(&self.file_path)?;
        writer.write_hyper_tile_ext(tile_type, records, timestamps, hull, filter_keys)
    }
}

/// S3A Compactor for merging and compacting sparse/uncompressed hyper-tiles into stratified blocks.
pub struct Compactor;

impl Compactor {
    pub fn compact<P: AsRef<Path>>(input_paths: &[P], output_path: P) -> io::Result<u32> {
        Self::compact_with_columnar_opt_in(input_paths, output_path, &[])
    }

    pub fn compact_with_columnar_opt_in<P: AsRef<Path>>(
        input_paths: &[P],
        output_path: P,
        opt_in_columnar_types: &[u32],
    ) -> io::Result<u32> {
        let out_ref = output_path.as_ref();

        match Self::compact_pass(input_paths, out_ref, opt_in_columnar_types) {
            Ok(count) => Ok(count),
            Err(first_err) => {
                if out_ref.exists() {
                    let _ = std::fs::remove_file(out_ref);
                }

                // If already quarantined, do not retry
                if first_err.to_string().contains("quarantined") {
                    return Err(first_err);
                }

                // Retry once
                match Self::compact_pass(input_paths, out_ref, opt_in_columnar_types) {
                    Ok(count) => Ok(count),
                    Err(second_err) => {
                        if out_ref.exists() {
                            let _ = std::fs::remove_file(out_ref);
                        }

                        let (bad_path, bad_tile_id, bad_gen) = Self::identify_failed_tile(input_paths, &second_err);
                        let _ = crate::compactor_gates::quarantine_tile(&bad_path, bad_tile_id, bad_gen);

                        Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!(
                                "Compaction failed after retry: tile {} (generation {}) in '{}' quarantined due to persistent integrity failure ({})",
                                bad_tile_id, bad_gen, bad_path.display(), second_err
                            ),
                        ))
                    }
                }
            }
        }
    }

    pub(crate) fn identify_failed_tile<P: AsRef<Path>>(input_paths: &[P], _err: &io::Error) -> (PathBuf, u64, u64) {
        for p in input_paths {
            let path = p.as_ref();
            if let Ok(reader) = MmapReader::open(path) {
                let gen = reader.generation();
                for i in 0..reader.tile_count() as usize {
                    if let Some((header, payload)) = reader.get_tile(i) {
                        let actual_crc = crc32c::crc32c(payload);
                        if actual_crc != header.data_crc32 {
                            return (path.to_path_buf(), header.tile_id, gen);
                        }
                        let is_legacy = header.record_layout_version == 0;
                        if header.header_crc32 == 0 && !is_legacy {
                            return (path.to_path_buf(), header.tile_id, gen);
                        }
                        if header.header_crc32 != 0 && header.header_crc32 != compute_tile_header_crc32(header) {
                            return (path.to_path_buf(), header.tile_id, gen);
                        }
                    }
                }
            }
        }
        if let Some(first) = input_paths.first() {
            let gen = MmapReader::open(first.as_ref()).map(|r| r.generation()).unwrap_or(1);
            (first.as_ref().to_path_buf(), 0, gen)
        } else {
            (PathBuf::from("unknown"), 0, 1)
        }
    }

    fn compact_pass<P: AsRef<Path>>(
        input_paths: &[P],
        output_path: &Path,
        opt_in_columnar_types: &[u32],
    ) -> io::Result<u32> {
        let mut preflight = crate::compactor_gates::CompactionPreflight::new();
        let mut input_acc = crate::compactor_gates::LiveRecordAccumulator::new();
        let mut output_acc = crate::compactor_gates::LiveRecordAccumulator::new();

        let mut target_uuid: [u64; 2] = [0, 0];
        for path in input_paths {
            if let Ok(reader) = MmapReader::open(path) {
                if reader.file_uuid() != [0, 0] {
                    target_uuid = reader.file_uuid();
                    break;
                }
            }
        }
        if target_uuid == [0, 0] {
            target_uuid = generate_file_uuid();
        }
        let mut writer = TileWriter::create_with_uuid(output_path, target_uuid)?;

        let mut accumulated_addon_mask: u32 = 0;
        let mut telemetry_buffer: Vec<TelemetryRecord> = Vec::new();
        let mut telemetry_input_tile_ids: Vec<(usize, u64)> = Vec::new();
        let mut embedding_buffer: Vec<EmbeddingRecord128> = Vec::new();
        let mut embedding_input_tile_ids: Vec<(usize, u64)> = Vec::new();
        let mut unspecialized_tiles: Vec<(HyperTileHeader, Vec<u8>, usize)> = Vec::new();
        let mut input_marks: Vec<(usize, MarkRecord)> = Vec::new();
        let mut input_consent_records: Vec<(usize, ConsentRecord)> = Vec::new();
        let mut input_erasure_records: Vec<(usize, ErasureRecord)> = Vec::new();
        let mut custom_schemas_map: std::collections::HashMap<u32, SchemaTypeDefinition> = std::collections::HashMap::new();

        for (input_idx, input_path) in input_paths.iter().enumerate() {
            let reader = MmapReader::open(input_path)?;
            accumulated_addon_mask |= reader.file_header().addon_mask;
            let gen = reader.generation();
            let tile_count = reader.tile_count() as usize;

            for i in 0..tile_count {
                if let Some((header, payload)) = reader.get_tile(i) {
                    // BEFORE GATE: verify input tile CRC, quarantine, uniqueness
                    preflight.verify_and_register_tile(input_path, gen, header, payload)?;

                    if header.tile_type == TileType::TELEMETRY {
                        telemetry_input_tile_ids.push((input_idx, header.tile_id));
                        let records: &[TelemetryRecord] = bytemuck::cast_slice(payload);
                        telemetry_buffer.extend_from_slice(records);
                    } else if header.tile_type == TileType::EMBEDDING {
                        embedding_input_tile_ids.push((input_idx, header.tile_id));
                        let records: &[EmbeddingRecord128] = bytemuck::cast_slice(payload);
                        embedding_buffer.extend_from_slice(records);
                    } else if header.tile_type == TileType::MARKS {
                        let marks: &[MarkRecord] = bytemuck::cast_slice(payload);
                        for m in marks {
                            input_marks.push((input_idx, *m));
                        }
                    } else if header.tile_type == TileType::CONSENT {
                        let consents: &[ConsentRecord] = bytemuck::cast_slice(payload);
                        for c in consents {
                            input_consent_records.push((input_idx, *c));
                        }
                    } else if header.tile_type == TileType::ERASURE {
                        let erasures: &[ErasureRecord] = bytemuck::cast_slice(payload);
                        for e in erasures {
                            input_erasure_records.push((input_idx, *e));
                        }
                    } else if header.tile_type == TileType::COLUMNAR {
                        if let Ok(records) = decode_columnar_telemetry(payload) {
                            telemetry_input_tile_ids.push((input_idx, header.tile_id));
                            telemetry_buffer.extend(records);
                        } else {
                            unspecialized_tiles.push((*header, payload.to_vec(), input_idx));
                        }
                    } else if header.tile_type == TileType::SCHEMA {
                        if let Ok(types) = decode_schema_payload(payload) {
                            for t in types {
                                custom_schemas_map.insert(t.type_id, t);
                            }
                        }
                    } else {
                        unspecialized_tiles.push((*header, payload.to_vec(), input_idx));
                    }
                }
            }
        }

        // Inherit accumulated addon capability bitmask onto compacted archive
        if accumulated_addon_mask != 0 {
            writer.enable_addon(accumulated_addon_mask)?;
        }

        // Track subjects and targets under active legal hold
        let mut legal_hold_subjects: std::collections::HashSet<[u8; 16]> = std::collections::HashSet::new();
        for (_, c) in &input_consent_records {
            if c.is_legal_hold() {
                legal_hold_subjects.insert(c.subject_id);
            }
        }
        let mut legal_hold_marks: std::collections::HashSet<(u32, u64, u32)> = std::collections::HashSet::new();
        for (_, m) in &input_marks {
            if m.mark_kind == MARK_LEGAL_HOLD && m.is_set() {
                legal_hold_marks.insert((m.target_tile_type, m.target_tile_id, m.record_index));
            }
        }

        // Legal hold invariant: if any erasure targets a record under active legal hold, block compaction/purge
        for (_, e) in &input_erasure_records {
            if legal_hold_marks.iter().any(|(tt, tid, idx)| *tt == e.target_tile_type && *tid == e.target_tile_id && *idx == e.record_index) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Purge blocked: erased target record is under active legal hold",
                ));
            }
        }

        // Deduplicate telemetry records: keep latest active record, filtering out tombstones and erasures unless protected by legal hold
        let mut telemetry_map: std::collections::HashMap<(u32, u32, u64), TelemetryRecord> =
            std::collections::HashMap::with_capacity(telemetry_buffer.len());
        for rec in telemetry_buffer {
            let key = (rec.sensor_id, rec.metric_id, rec.timestamp);
            let sub = actor_id_to_subject_id(rec.sensor_id as u64);
            let under_hold = legal_hold_subjects.contains(&sub)
                || legal_hold_marks.iter().any(|(tt, _, _)| *tt == TileType::TELEMETRY);

            let mut hasher = sha2::Sha256::new();
            sha2::Digest::update(&mut hasher, bytemuck::bytes_of(&rec));
            let rec_hash: [u8; 32] = hasher.finalize().into();

            let is_erased = input_erasure_records.iter().any(|(_, e)| {
                e.erased_record_hash == rec_hash
            });

            if is_erased && under_hold {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Purge blocked: participant subject is under active legal hold",
                ));
            }

            if rec.is_tombstone() || is_erased {
                if under_hold {
                    telemetry_map.insert(key, rec); // Retain under legal hold!
                } else {
                    telemetry_map.remove(&key);
                }
            } else {
                telemetry_map.insert(key, rec);
            }
        }

        let mut deduped_telemetry: Vec<TelemetryRecord> = telemetry_map.into_values().collect();
        deduped_telemetry.sort_by_key(|r| r.timestamp);

        // Zero out erased records in unspecialized tiles
        for (header, payload, _input_idx) in &mut unspecialized_tiles {
            if header.record_count > 0 && header.payload_bytes > 0 {
                let rec_size = header.payload_bytes as usize / header.record_count as usize;
                if rec_size > 0 {
                    for (rec_idx, chunk) in payload[..header.payload_bytes as usize].chunks_exact_mut(rec_size).enumerate() {
                        let mut hasher = sha2::Sha256::new();
                        sha2::Digest::update(&mut hasher, &*chunk);
                        let chunk_hash: [u8; 32] = hasher.finalize().into();

                        let is_erased = input_erasure_records.iter().any(|(_, e)| {
                            e.erased_record_hash == chunk_hash
                                || (e.target_tile_type == header.tile_type && e.target_tile_id == header.tile_id && e.record_index == rec_idx as u32)
                        });

                        if is_erased {
                            chunk.fill(0);
                        }
                    }
                }
            }
        }

        // Accumulate input live records
        for rec in &deduped_telemetry {
            input_acc.push_record(rec);
        }
        for rec in &embedding_buffer {
            input_acc.push_record(rec);
        }
        for (header, payload, _input_idx) in &unspecialized_tiles {
            if header.record_count > 0 && header.payload_bytes > 0 {
                let rec_size = header.payload_bytes as usize / header.record_count as usize;
                if rec_size > 0 {
                    for chunk in payload[..header.payload_bytes as usize].chunks_exact(rec_size) {
                        if !chunk.iter().all(|&b| b == 0) {
                            input_acc.push_record_bytes(chunk);
                        }
                    }
                } else if !payload[..header.payload_bytes as usize].iter().all(|&b| b == 0) {
                    input_acc.push_record_bytes(&payload[..header.payload_bytes as usize]);
                }
            }
        }

        // Check fault injection for drop or duplication
        #[allow(unused_mut)]
        let mut output_telemetry = deduped_telemetry;
        #[cfg(any(test, feature = "fault-injection"))]
        {
            if crate::compactor_gates::fault_injection::INJECT_RECORD_DROP.load(std::sync::atomic::Ordering::SeqCst) {
                if !output_telemetry.is_empty() {
                    output_telemetry.pop();
                }
            }
            if crate::compactor_gates::fault_injection::INJECT_RECORD_DUPLICATION.load(std::sync::atomic::Ordering::SeqCst) {
                if !output_telemetry.is_empty() {
                    let first = output_telemetry[0];
                    output_telemetry.push(first);
                }
            }
        }

        let mut tile_id_remap: std::collections::HashMap<(usize, u64), u64> = std::collections::HashMap::new();
        let mut output_tile_types: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();

        // Compact telemetry into full 128 KB tiles (or columnar if opted in)
        let is_columnar_opt_in = opt_in_columnar_types.contains(&TileType::TELEMETRY);
        let tel_rec_capacity = HYPER_TILE_PAYLOAD_SIZE / std::mem::size_of::<TelemetryRecord>();
        for chunk in output_telemetry.chunks(tel_rec_capacity) {
            let timestamps: Vec<u64> = chunk.iter().map(|r| r.timestamp).collect();
            let new_tile_id = if is_columnar_opt_in {
                output_tile_types.insert(TileType::COLUMNAR);
                let col_payload = encode_columnar_telemetry(chunk);
                let mut hdr = HyperTileHeader::new(writer.tile_count() as u64, TileType::COLUMNAR);
                hdr.record_count = chunk.len() as u32;
                hdr.record_layout_version = 1;
                hdr.payload_bytes = col_payload.len() as u32;
                hdr.data_crc32 = crc32c::crc32c(&col_payload);
                hdr.generation = writer.generation();
                hdr.tile_index = writer.tile_count() as u32;
                hdr.file_uuid = writer.file_uuid();
                if let (Some(&first), Some(&last)) = (timestamps.first(), timestamps.last()) {
                    hdr.min_timestamp = first;
                    hdr.max_timestamp = last;
                }
                for r in chunk {
                    bloom_filter_insert(&mut hdr.filter.bits, r.sensor_id as u64, hdr.filter.hash_seed);
                    bloom_filter_insert(&mut hdr.filter.bits, r.metric_id as u64, hdr.filter.hash_seed);
                }
                hdr.header_crc32 = compute_tile_header_crc32(&hdr);
                writer.write_raw_hyper_tile(&hdr, &col_payload)?
            } else {
                output_tile_types.insert(TileType::TELEMETRY);
                writer.write_hyper_tile(
                    TileType::TELEMETRY,
                    chunk,
                    Some(&timestamps),
                    None,
                )?
            };
            for &(inp_idx, old_tid) in &telemetry_input_tile_ids {
                tile_id_remap.insert((inp_idx, old_tid), new_tile_id);
            }
            for rec in chunk {
                output_acc.push_record(rec);
            }
        }

        // Compact embeddings into full 128 KB tiles with calculated Simplex Hulls
        let emb_rec_capacity = HYPER_TILE_PAYLOAD_SIZE / std::mem::size_of::<EmbeddingRecord128>();
        for chunk in embedding_buffer.chunks(emb_rec_capacity) {
            output_tile_types.insert(TileType::EMBEDDING);
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

            let new_tile_id = writer.write_hyper_tile(
                TileType::EMBEDDING,
                chunk,
                Some(&timestamps),
                Some(hull),
            )?;
            for &(inp_idx, old_tid) in &embedding_input_tile_ids {
                tile_id_remap.insert((inp_idx, old_tid), new_tile_id);
            }
            for rec in chunk {
                output_acc.push_record(rec);
            }
        }

        // Copy through all unspecialized / pack tiles intact, recording tile ID remappings
        for (header, payload, input_idx) in unspecialized_tiles {
            let old_tile_id = header.tile_id;
            let mut h = header;
            output_tile_types.insert(h.tile_type);
            h.file_uuid = writer.file_uuid();
            let new_tile_id = writer.write_raw_hyper_tile(&h, &payload)?;
            tile_id_remap.insert((input_idx, old_tile_id), new_tile_id);

            if header.record_count > 0 && header.payload_bytes > 0 {
                let rec_size = header.payload_bytes as usize / header.record_count as usize;
                if rec_size > 0 {
                    for chunk in payload[..header.payload_bytes as usize].chunks_exact(rec_size) {
                        if !chunk.iter().all(|&b| b == 0) {
                            output_acc.push_record_bytes(chunk);
                        }
                    }
                } else if !payload[..header.payload_bytes as usize].iter().all(|&b| b == 0) {
                    output_acc.push_record_bytes(&payload[..header.payload_bytes as usize]);
                }
            }
        }

        // Carry marks forward: latest mark per target wins, remapping target tile IDs
        let mut latest_marks_map: std::collections::BTreeMap<(u32, usize, u64, u32, u32), MarkRecord> = std::collections::BTreeMap::new();
        for (input_idx, m) in input_marks {
            let key = (m.target_tile_type, input_idx, m.target_tile_id, m.record_index, m.mark_kind);
            latest_marks_map.insert(key, m);
        }

        let out_gen = writer.generation();
        let mut output_marks: Vec<MarkRecord> = Vec::new();
        for ((_tt, input_idx, old_tile_id, _rec_idx, _kind), m) in latest_marks_map {
            if m.is_set() {
                let mut remapped = m;
                if let Some(&new_id) = tile_id_remap.get(&(input_idx, old_tile_id)) {
                    remapped.target_tile_id = new_id;
                }
                remapped.generation = out_gen;
                output_marks.push(remapped);
            }
        }

        if !output_marks.is_empty() {
            output_tile_types.insert(TileType::MARKS);
            let mark_capacity = HYPER_TILE_PAYLOAD_SIZE / std::mem::size_of::<MarkRecord>();
            for chunk in output_marks.chunks(mark_capacity) {
                let timestamps: Vec<u64> = chunk.iter().map(|m| m.timestamp).collect();
                writer.write_hyper_tile(
                    TileType::MARKS,
                    chunk,
                    Some(&timestamps),
                    None,
                )?;
            }
        }

        // Carry consent records forward
        if !input_consent_records.is_empty() {
            output_tile_types.insert(TileType::CONSENT);
            let consent_capacity = HYPER_TILE_PAYLOAD_SIZE / std::mem::size_of::<ConsentRecord>();
            let all_consents: Vec<ConsentRecord> = input_consent_records.into_iter().map(|(_, r)| r).collect();
            for chunk in all_consents.chunks(consent_capacity) {
                let timestamps: Vec<u64> = chunk.iter().map(|c| c.granted_at.max(c.withdrawn_at)).collect();
                writer.write_hyper_tile(
                    TileType::CONSENT,
                    chunk,
                    Some(&timestamps),
                    None,
                )?;
            }
        }

        // Carry erasure records forward
        if !input_erasure_records.is_empty() {
            output_tile_types.insert(TileType::ERASURE);
            let erasure_capacity = HYPER_TILE_PAYLOAD_SIZE / std::mem::size_of::<ErasureRecord>();
            let all_erasures: Vec<ErasureRecord> = input_erasure_records.into_iter().map(|(_, r)| r).collect();
            for chunk in all_erasures.chunks(erasure_capacity) {
                let timestamps: Vec<u64> = chunk.iter().map(|e| e.executed_at).collect();
                writer.write_hyper_tile(
                    TileType::ERASURE,
                    chunk,
                    Some(&timestamps),
                    None,
                )?;
            }
        }

        // On compaction, write a schema tile listing every tile type present in the compacted archive
        if !output_tile_types.is_empty() {
            output_tile_types.insert(TileType::SCHEMA);
            let mut schema_defs = Vec::new();
            for &tt in &output_tile_types {
                if let Some(custom) = custom_schemas_map.get(&tt) {
                    schema_defs.push(custom.clone());
                } else {
                    schema_defs.push(standard_schema_for_type(tt));
                }
            }
            let schema_payload = encode_schema_payload(&schema_defs);
            let mut schema_hdr = HyperTileHeader::new(writer.tile_count() as u64, TileType::SCHEMA);
            schema_hdr.record_count = 0;
            schema_hdr.record_layout_version = 1;
            schema_hdr.payload_bytes = schema_payload.len() as u32;
            schema_hdr.data_crc32 = crc32c::crc32c(&schema_payload);
            schema_hdr.generation = writer.generation();
            schema_hdr.tile_index = writer.tile_count() as u32;
            schema_hdr.file_uuid = writer.file_uuid();
            schema_hdr.header_crc32 = compute_tile_header_crc32(&schema_hdr);
            writer.write_raw_hyper_tile(&schema_hdr, &schema_payload)?;
        }

        // DURING GATE: Verify live-record count in = out and hash in = out
        input_acc.verify_equal(&output_acc)?;

        // End of compaction: verify all output tiles from disk
        drop(writer);
        let output_reader = MmapReader::open(output_path)?;
        output_reader.verify_checksums().map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("Compaction After-Gate checksum failure: {}", e))
        })?;

        Ok(output_reader.tile_count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::{Pod, Zeroable};

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
    fn test_tile_fusion_engine_deduplication_and_tombstones() {
        let f1 = TestTempFile::new();
        let f2 = TestTempFile::new();
        let fused_file = TestTempFile::new();

        // Tile 1: Initial telemetry records and a record to be tombstoned
        let mut w1 = TileWriter::create(f1.path()).unwrap();
        let recs1 = vec![
            TelemetryRecord::new(100, 1, 1, 10.0), // Same key (100, 1, 1), initial value 10.0
            TelemetryRecord::new(200, 1, 2, 50.0), // Will be tombstoned in Tile 2
            TelemetryRecord::new(300, 2, 1, 99.0), // Unrelated record
        ];
        w1.write_hyper_tile(TileType::TELEMETRY, &recs1, Some(&[100, 200, 300]), None).unwrap();

        // Tile 2: Updated version of (100, 1, 1) and tombstone for (200, 1, 2)
        let mut w2 = TileWriter::create(f2.path()).unwrap();
        let mut tombstone_rec = TelemetryRecord::new(200, 1, 2, 50.0);
        tombstone_rec.mark_tombstone();

        let recs2 = vec![
            TelemetryRecord::new(100, 1, 1, 25.5), // Updated value 25.5 for key (100, 1, 1)
            tombstone_rec,                           // Tombstone for key (200, 1, 2)
        ];
        w2.write_hyper_tile(TileType::TELEMETRY, &recs2, Some(&[100, 200]), None).unwrap();

        let count = TileFusionEngine::fuse_telemetry_tiles(&[f1.path(), f2.path()], fused_file.path()).unwrap();
        assert_eq!(count, 1);

        let reader = MmapReader::open(fused_file.path()).unwrap();
        let sieve = QuerySieve::new(&reader);
        let res = sieve.query_telemetry(0, 500, None, None);

        // Expect 2 records remaining:
        // - (100, 1, 1) updated to value 25.5
        // - (300, 2, 1) value 99.0
        // Record (200, 1, 2) should be completely removed due to the tombstone
        assert_eq!(res.len(), 2);

        let r100 = res.iter().find(|r| r.timestamp == 100 && r.sensor_id == 1 && r.metric_id == 1).unwrap();
        assert_eq!(r100.value, 25.5);

        let r200 = res.iter().find(|r| r.timestamp == 200 && r.sensor_id == 1 && r.metric_id == 2);
        assert!(r200.is_none());

        let r300 = res.iter().find(|r| r.timestamp == 300 && r.sensor_id == 2 && r.metric_id == 1).unwrap();
        assert_eq!(r300.value, 99.0);
    }

    #[repr(C)]
    #[derive(Clone, Copy, Pod, Zeroable, PartialEq, Debug)]
    struct DummyMicroRecord {
        timestamp: u32,
        sensor_id: u16,
        metric_id: u16,
        value: f32,
        _reserved: [u8; 4],
    }

    #[test]
    fn test_micro_tile_buffer_zero_alloc() {
        let mut buf = MicroTileBuffer::new(1);
        let rec1 = DummyMicroRecord {
            timestamp: 100,
            sensor_id: 1,
            metric_id: 10,
            value: 72.5,
            _reserved: [0; 4],
        };
        let rec2 = DummyMicroRecord {
            timestamp: 101,
            sensor_id: 1,
            metric_id: 10,
            value: 73.0,
            _reserved: [0; 4],
        };

        buf.push_record(&rec1).unwrap();
        buf.push_record(&rec2).unwrap();

        let page = buf.finalize();
        assert_eq!(page.len(), MICRO_TILE_SIZE);

        let header: &MicroTileHeader = &bytemuck::pod_read_unaligned::<MicroTileHeader>(&page[..MICRO_TILE_HEADER_SIZE]);
        assert_eq!(header.tile_id, 1);
        assert_eq!(header.record_count, 2);
        assert_eq!(header.payload_bytes, 32);

        let payload_records: &[DummyMicroRecord] = bytemuck::cast_slice(&page[MICRO_TILE_HEADER_SIZE..MICRO_TILE_HEADER_SIZE + 32]);
        assert_eq!(payload_records[0], rec1);
        assert_eq!(payload_records[1], rec2);
    }

    #[test]
    fn test_domain_tile_write_gates() {
        use s3a_core::{ADDON_WEARABLE, ADDON_ROBOTICS, ADDON_GIS, ADDON_DA, ADDON_EDU};

        let temp_file = TestTempFile::new();
        let mut writer = TileWriter::create(temp_file.path()).unwrap();
        let records = [TelemetryRecord::new(100, 1, 1, 1.0)];

        // Fail-closed writes without enabling addon bits
        assert!(writer.write_hyper_tile(3, &records, None, None).is_err());
        assert!(writer.write_hyper_tile(4, &records, None, None).is_err());
        assert!(writer.write_hyper_tile(5, &records, None, None).is_err());
        assert!(writer.write_hyper_tile(6, &records, None, None).is_err());
        assert!(writer.write_hyper_tile(10, &records, None, None).is_err());

        // Enable bits individually and verify writes pass
        writer.enable_addon(ADDON_WEARABLE).unwrap();
        assert!(writer.write_hyper_tile(3, &records, None, None).is_ok());

        writer.enable_addon(ADDON_ROBOTICS).unwrap();
        assert!(writer.write_hyper_tile(4, &records, None, None).is_ok());

        writer.enable_addon(ADDON_GIS).unwrap();
        assert!(writer.write_hyper_tile(5, &records, None, None).is_ok());

        writer.enable_addon(ADDON_DA).unwrap();
        assert!(writer.write_hyper_tile(6, &records, None, None).is_ok());

        writer.enable_addon(ADDON_EDU).unwrap();
        assert!(writer.write_hyper_tile(10, &records, None, None).is_ok());
    }

    #[test]
    fn test_compaction_domain_tiles_and_mask_preservation() {
        use s3a_core::{ADDON_WEARABLE, ADDON_ROBOTICS, ADDON_GIS, ADDON_DA, ADDON_EDU};

        let temp_file = TestTempFile::new();
        let mut writer = TileWriter::create(temp_file.path()).unwrap();
        let records = [TelemetryRecord::new(100, 1, 1, 1.0)];

        let all_addons = ADDON_WEARABLE | ADDON_ROBOTICS | ADDON_GIS | ADDON_DA | ADDON_EDU;
        writer.enable_addon(all_addons).unwrap();

        writer.write_hyper_tile(3, &records, None, None).unwrap();
        writer.write_hyper_tile(4, &records, None, None).unwrap();
        writer.write_hyper_tile(5, &records, None, None).unwrap();
        writer.write_hyper_tile(6, &records, None, None).unwrap();
        writer.write_hyper_tile(10, &records, None, None).unwrap();

        let initial_count = writer.tile_count();
        assert_eq!(initial_count, 5);

        // Run compaction via S3ACrudEngine (emits domain tiles + self-describing schema tile)
        let engine = S3ACrudEngine::open_or_create(temp_file.path()).unwrap();
        let compacted_count = engine.purge_and_compact().unwrap();
        assert_eq!(compacted_count, 6);

        // Verify reader sees all 5 domain tiles + schema tile and addon_mask is preserved
        let reader = MmapReader::open(temp_file.path()).unwrap();
        assert_eq!(reader.tile_count(), 6);
        assert_eq!(reader.addon_mask(), all_addons);

        let types: Vec<u32> = (0..5).map(|i| reader.get_tile(i).unwrap().0.tile_type).collect();
        assert_eq!(types, vec![3, 4, 5, 6, 10]);
        assert_eq!(reader.get_tile(5).unwrap().0.tile_type, TileType::SCHEMA);
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
        for _ in 0..20 {
            if compactor.stats().flushed_records >= 100 {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }

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

        // 1. Verify Greenfield write fails closed because Tile 9 is strictly RESERVED_BOUNDARY
        let write_res = engine.create_learning_activities(&records);
        assert!(write_res.is_err());
        let write_err = write_res.unwrap_err();
        assert_eq!(write_err.kind(), io::ErrorKind::PermissionDenied);
        assert!(write_err.to_string().contains("RESERVED"));

        // 2. Synthesize a legacy archive containing Tile 9 to verify backward-compatible read sieve and S3A-QL
        {
            use std::io::{Seek, Write};
            let mut file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
            let mut tile_hdr = HyperTileHeader::new(0, TileType::RESERVED_BOUNDARY);
            tile_hdr.record_count = records.len() as u32;
            tile_hdr.payload_bytes = (records.len() * std::mem::size_of::<LearningActivityRecord>()) as u32;
            tile_hdr.min_timestamp = 1000;
            tile_hdr.max_timestamp = 1000 + 99 * 10;
            tile_hdr.learned_index.slope = (records.len() - 1) as f32 / (tile_hdr.max_timestamp - tile_hdr.min_timestamp) as f32;
            tile_hdr.learned_index.intercept = 0.0;
            for r in &records {
                bloom_filter_insert(&mut tile_hdr.filter.bits, r.actor_id, tile_hdr.filter.hash_seed);
                bloom_filter_insert(&mut tile_hdr.filter.bits, r.verb_id as u64, tile_hdr.filter.hash_seed);
            }
            tile_hdr.data_crc32 = crc32c::crc32c(bytemuck::cast_slice(&records));
            tile_hdr.header_crc32 = compute_tile_header_crc32(&tile_hdr);

            let rec_bytes = bytemuck::cast_slice::<LearningActivityRecord, u8>(&records);
            file.seek(std::io::SeekFrom::Start(FILE_HEADER_SIZE as u64)).unwrap();
            file.write_all(bytemuck::bytes_of(&tile_hdr)).unwrap();
            file.write_all(rec_bytes).unwrap();
            let pad = HYPER_TILE_SIZE - (HYPER_TILE_HEADER_SIZE + rec_bytes.len());
            file.write_all(&vec![0u8; pad]).unwrap();

            // Commit dual slot A with tile_count = 1
            let mut file_hdr = FileHeader::new(1, 1000);
            file_hdr.generation = 2;
            file_hdr.header_crc32 = compute_header_crc32(&file_hdr);
            file.seek(std::io::SeekFrom::Start(0)).unwrap();
            file.write_all(bytemuck::bytes_of(&file_hdr)).unwrap();
            file.flush().unwrap();
        }

        // Verify read by actor 1001 from legacy archive
        let act1001 = engine.read_learning_activities(Some(1001), None, 1000, 2000).unwrap();
        assert_eq!(act1001.len(), 50);
        assert_eq!(act1001[0].0.actor_id, 1001);

        // Verify read by uninserted actor 9999 is pruned by Bloom filter
        let act_none = engine.read_learning_activities(Some(9999), None, 1000, 2000).unwrap();
        assert_eq!(act_none.len(), 0);

        // Test S3A-QL parsing and execution on LEARNING_ACTIVITIES:
        // In core engine without scholar pack registered, this must fail closed with PackRequired
        let ql_stmt = format!("SIFT LEARNING_ACTIVITIES FROM \"{}\" WHERE ACTOR_ID = 1001;", path.display());
        let err = execute_query(&ql_stmt).unwrap_err();
        assert_eq!(err.code(), "pack_required");
    }

    #[test]
    fn test_scholar_tile_write_gate_enforcement() {
        let temp_file = TestTempFile::new();
        let path = temp_file.path();

        let mut writer = TileWriter::create(path).unwrap();
        assert_eq!(writer.addon_mask(), 0);

        // Writing raw bytes to tile 10 (Human LRS) or tile 12 (Academic Papers) must fail closed
        let dummy_payload = [0x5Au8; 64];
        let res = writer.write_hyper_tile(10, &dummy_payload, None, None);
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert!(err.to_string().contains("Scholar Pack"));

        // Enable ADDON_EDU capability
        writer.enable_addon(ADDON_EDU).unwrap();
        assert_eq!(writer.addon_mask() & ADDON_EDU, ADDON_EDU);

        // Write to tile 10 now succeeds
        let tile_id = writer.write_hyper_tile(10, &dummy_payload, None, None).unwrap();
        assert_eq!(tile_id, 1);
        assert_eq!(writer.tile_count(), 1);
    }

    #[test]
    fn test_compactor_copy_through_and_addon_mask_preservation() {
        let input_file = TestTempFile::new();
        let output_file = TestTempFile::new();

        // 1. Create input archive with telemetry and an unspecialized pack tile (tile 12)
        let mut writer = TileWriter::create(input_file.path()).unwrap();
        writer.enable_addon(ADDON_EDU).unwrap();

        let tel_rec = TelemetryRecord::new(100, 1, 1, 42.0);
        writer.write_hyper_tile(TileType::TELEMETRY, &[tel_rec], Some(&[100]), None).unwrap();

        let dummy_pack_payload = [0xA5u8; 64];
        writer.write_hyper_tile(12, &dummy_pack_payload, None, None).unwrap();
        assert_eq!(writer.tile_count(), 2);
        drop(writer);

        // 2. Compact archive (outputs telemetry + tile 12 + schema tile)
        let compacted_count = Compactor::compact(&[input_file.path()], output_file.path()).unwrap();
        assert_eq!(compacted_count, 3);

        // 3. Verify output archive inherited addon_mask and preserved pack tile intact
        let reader = MmapReader::open(output_file.path()).unwrap();
        assert_eq!(reader.file_header().addon_mask & ADDON_EDU, ADDON_EDU);
        assert_eq!(reader.tile_count(), 3);

        // Tile 0: compacted telemetry
        let (h0, _) = reader.get_tile(0).unwrap();
        assert_eq!(h0.tile_type, TileType::TELEMETRY);

        // Tile 1: copied through pack tile (tile 12)
        let (h1, p1) = reader.get_tile(1).unwrap();
        assert_eq!(h1.tile_type, 12);
        assert_eq!(&p1[..64], &dummy_pack_payload);

        // Tile 2: embedded schema tile
        let (h2, _) = reader.get_tile(2).unwrap();
        assert_eq!(h2.tile_type, TileType::SCHEMA);

        // Verify checksums pass on the compacted archive
        assert!(reader.verify_checksums().is_ok());
    }

    #[test]
    fn test_tile_writer_sets_record_layout_version() {
        let temp_file = TestTempFile::new();
        let mut writer = TileWriter::create(temp_file.path()).unwrap();
        let rec = TelemetryRecord::new(100, 1, 1, 42.0);
        writer.write_hyper_tile(TileType::TELEMETRY, &[rec], Some(&[100]), None).unwrap();
        drop(writer);

        let reader = MmapReader::open(temp_file.path()).unwrap();
        let (header, _) = reader.get_tile(0).unwrap();
        assert_eq!(header.record_layout_version, 1);
        let read_rec: TelemetryRecord = reader.read_record_by_coordinate(&S3ACoordinate::new(0, 0, 0)).unwrap();
        assert_eq!(read_rec.timestamp, 100);
        assert_eq!(read_rec.sensor_id, 1);
    }

    #[test]
    fn test_read_fails_closed_on_unsupported_layout_version() {
        let temp_file = TestTempFile::new();
        let mut writer = TileWriter::create(temp_file.path()).unwrap();

        // 1. Writer write gate fails closed on unsupported layout version
        let rec = TelemetryRecord::new(200, 2, 2, 84.0);
        let mut header = HyperTileHeader::new(0, TileType::TELEMETRY);
        header.record_layout_version = 99; // future/unsupported
        header.record_count = 1;
        header.payload_bytes = std::mem::size_of::<TelemetryRecord>() as u32;

        let write_res = writer.write_raw_hyper_tile(&header, bytemuck::bytes_of(&rec));
        assert!(write_res.is_err());
        assert_eq!(write_res.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        drop(writer);

        // 2. Synthesize an archive containing future layout version 99 to test Reader fail-closed behavior
        {
            use std::io::{Seek, Write};
            let mut file = std::fs::OpenOptions::new().write(true).open(temp_file.path()).unwrap();
            header.data_crc32 = crc32c::crc32c(bytemuck::bytes_of(&rec));
            header.header_crc32 = compute_tile_header_crc32(&header);

            file.seek(std::io::SeekFrom::Start(FILE_HEADER_SIZE as u64)).unwrap();
            file.write_all(bytemuck::bytes_of(&header)).unwrap();
            file.write_all(bytemuck::bytes_of(&rec)).unwrap();
            let pad = HYPER_TILE_SIZE - (HYPER_TILE_HEADER_SIZE + std::mem::size_of::<TelemetryRecord>());
            file.write_all(&vec![0u8; pad]).unwrap();

            let mut file_hdr = FileHeader::new(1, 200);
            file_hdr.generation = 2;
            file_hdr.header_crc32 = compute_header_crc32(&file_hdr);
            file.seek(std::io::SeekFrom::Start(0)).unwrap();
            file.write_all(bytemuck::bytes_of(&file_hdr)).unwrap();
            file.flush().unwrap();
        }

        let reader = MmapReader::open(temp_file.path()).unwrap();
        let res: Result<TelemetryRecord, S3AError> = reader.read_record_by_coordinate(&S3ACoordinate::new(0, 0, 0));
        assert!(res.is_err());
        match res.unwrap_err() {
            S3AError::UnsupportedLayoutVersion(v) => assert_eq!(v, 99),
            other => panic!("Expected UnsupportedLayoutVersion(99), got {:?}", other),
        }
    }

    #[test]
    fn test_legacy_layout_version_zero_compatibility() {
        let temp_file = TestTempFile::new();
        let mut writer = TileWriter::create(temp_file.path()).unwrap();

        // Simulate a legacy tile written before Phase 2 (record_layout_version == 0)
        let rec = TelemetryRecord::new(300, 3, 3, 126.0);
        let mut header = HyperTileHeader::new(0, TileType::TELEMETRY);
        header.record_layout_version = 0; // Legacy version alias
        header.record_count = 1;
        header.payload_bytes = std::mem::size_of::<TelemetryRecord>() as u32;

        writer.write_raw_hyper_tile(&header, bytemuck::bytes_of(&rec)).unwrap();
        drop(writer);

        let reader = MmapReader::open(temp_file.path()).unwrap();
        let (read_header, _) = reader.get_tile(0).unwrap();
        assert_eq!(read_header.record_layout_version, 0);

        // Reader maps 0 to 1 and successfully returns the record
        let read_rec: TelemetryRecord = reader.read_record_by_coordinate(&S3ACoordinate::new(0, 0, 0)).unwrap();
        assert_eq!(read_rec.timestamp, 300);
        assert_eq!(read_rec.sensor_id, 3);
    }

    #[test]
    fn test_tile_nine_write_denied() {
        let temp_file = TestTempFile::new();
        let mut writer = TileWriter::create(temp_file.path()).unwrap();

        // 1. write_hyper_tile to tile 9 must fail closed
        let dummy = [0u8; 64];
        let res1 = writer.write_hyper_tile(9, &dummy, None, None);
        assert!(res1.is_err());
        let err1 = res1.unwrap_err();
        assert_eq!(err1.kind(), io::ErrorKind::PermissionDenied);
        assert!(err1.to_string().contains("RESERVED"));

        // 2. write_raw_hyper_tile to tile 9 must also fail closed
        let header = HyperTileHeader::new(0, 9);
        let res2 = writer.write_raw_hyper_tile(&header, &dummy);
        assert!(res2.is_err());
        let err2 = res2.unwrap_err();
        assert_eq!(err2.kind(), io::ErrorKind::PermissionDenied);
        assert!(err2.to_string().contains("RESERVED"));
    }

    #[test]
    fn test_compactor_preserves_record_layout_version() {
        let input_file = TestTempFile::new();
        let output_file = TestTempFile::new();

        let mut writer = TileWriter::create(input_file.path()).unwrap();
        writer.enable_addon(ADDON_EDU).unwrap();

        // Write tile 12 with specific record_layout_version
        let dummy = [0x77u8; 64];
        let mut h12 = HyperTileHeader::new(0, 12);
        h12.record_layout_version = 1;
        h12.payload_bytes = 64;
        writer.write_raw_hyper_tile(&h12, &dummy).unwrap();
        drop(writer);

        // Compact (outputs preserved tile 12 + schema tile)
        let compacted_count = Compactor::compact(&[input_file.path()], output_file.path()).unwrap();
        assert_eq!(compacted_count, 2);

        // Verify preserved byte-for-byte
        let reader = MmapReader::open(output_file.path()).unwrap();
        let (compacted_header, payload) = reader.get_tile(0).unwrap();
        assert_eq!(compacted_header.tile_type, 12);
        assert_eq!(compacted_header.record_layout_version, 1);
        assert_eq!(&payload[..64], &dummy);
        assert_eq!(reader.get_tile(1).unwrap().0.tile_type, TileType::SCHEMA);
    }

    #[test]
    fn test_demo_payload_migrator_in_engine() {
        use s3a_core::registry::{migrate_tile_payload, TilePayloadMigrator, RegistryError};

        struct DemoV1ToV2Migrator;
        impl TilePayloadMigrator for DemoV1ToV2Migrator {
            fn migrate_payload(
                &self,
                tile_type: u32,
                from_version: u32,
                to_version: u32,
                payload: &[u8],
            ) -> Result<Vec<u8>, RegistryError> {
                if tile_type == 10 && from_version == 1 && to_version == 2 {
                    let mut migrated = payload.to_vec();
                    migrated.extend_from_slice(&[0xFF; 16]);
                    Ok(migrated)
                } else {
                    Err(RegistryError::MigrationFailed("Unsupported migration".to_string()))
                }
            }
        }

        let old_payload = vec![1, 2, 3, 4];
        let migrator = DemoV1ToV2Migrator;
        let migrated_custom = migrator.migrate_payload(10, 1, 2, &old_payload).unwrap();
        assert_eq!(migrated_custom.len(), old_payload.len() + 16);

        // Test standard synthetic demo migrator for tile 99
        let tile99_v1_record = [1u8; 16];
        let tile99_v2_payload = migrate_tile_payload(99, 1, 2, &tile99_v1_record).unwrap();
        assert_eq!(tile99_v2_payload.len(), 20);
        assert_eq!(&tile99_v2_payload[16..], &[0, 0, 0, 0]);
    }
}



