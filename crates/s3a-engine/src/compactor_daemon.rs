use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::ring_buffer::L0RingBuffer;
use crate::{Compactor, MmapReader, QuerySieve, S3ACrudEngine, TelemetryRecord, TileType, TileWriter, HYPER_TILE_PAYLOAD_SIZE};

/// Runtime operational statistics for the Background Compactor Daemon.
#[derive(Debug, Clone, Default)]
pub struct CompactorStats {
    pub flushed_tiles: u64,
    pub flushed_records: u64,
    pub compaction_runs: u64,
    pub last_flush_time: Option<Instant>,
    pub last_compaction_time: Option<Instant>,
}

/// Dedicated background worker thread that drains in-memory L0 Ring Buffers,
/// commits sector-aligned 128 KB Hyper-Tiles to disk using the atomic dual-slot protocol,
/// and executes periodic stratified compaction / tombstone purging.
pub struct BackgroundCompactor {
    archive_path: PathBuf,
    ring_buffer: Arc<L0RingBuffer<TelemetryRecord>>,
    batch_threshold: usize,
    flush_interval: Duration,
    auto_compact_interval: Option<Duration>,
    running: Arc<AtomicBool>,
    stats: Arc<Mutex<CompactorStats>>,
    worker_handle: Option<JoinHandle<()>>,
}

impl BackgroundCompactor {
    /// Configures a new BackgroundCompactor without starting the thread.
    pub fn new<P: AsRef<Path>>(
        archive_path: P,
        ring_buffer: Arc<L0RingBuffer<TelemetryRecord>>,
        batch_threshold: usize,
        flush_interval: Duration,
        auto_compact_interval: Option<Duration>,
    ) -> Self {
        Self {
            archive_path: archive_path.as_ref().to_path_buf(),
            ring_buffer,
            batch_threshold,
            flush_interval,
            auto_compact_interval,
            running: Arc::new(AtomicBool::new(false)),
            stats: Arc::new(Mutex::new(CompactorStats::default())),
            worker_handle: None,
        }
    }

    /// Spawns the background compaction and L0 flush worker thread.
    pub fn start(&mut self) -> io::Result<()> {
        if self.running.load(Ordering::SeqCst) {
            return Ok(());
        }

        self.running.store(true, Ordering::SeqCst);

        let running = Arc::clone(&self.running);
        let ring_buffer = Arc::clone(&self.ring_buffer);
        let archive_path = self.archive_path.clone();
        let stats = Arc::clone(&self.stats);
        let batch_threshold = self.batch_threshold;
        let flush_interval = self.flush_interval;
        let auto_compact_interval = self.auto_compact_interval;

        // Ensure archive file exists
        if !archive_path.exists() {
            let _ = TileWriter::create(&archive_path)?;
        }

        let handle = thread::Builder::new()
            .name("s3a-compactor-daemon".to_string())
            .spawn(move || {
                let mut last_flush = Instant::now();
                let mut last_compaction = Instant::now();
                let tile_rec_capacity = HYPER_TILE_PAYLOAD_SIZE / std::mem::size_of::<TelemetryRecord>();

                while running.load(Ordering::Relaxed) {
                    let buf_len = ring_buffer.len();
                    let elapsed = last_flush.elapsed();

                    let should_flush = buf_len >= batch_threshold
                        || (buf_len > 0 && elapsed >= flush_interval);

                    if should_flush {
                        let records = ring_buffer.drain(tile_rec_capacity);
                        if !records.is_empty() {
                            if let Ok(mut writer) = TileWriter::open_append(&archive_path) {
                                let timestamps: Vec<u64> = records.iter().map(|r| r.timestamp).collect();
                                if writer.write_hyper_tile(TileType::TELEMETRY, &records, Some(&timestamps), None).is_ok() {
                                    let mut s = stats.lock().unwrap();
                                    s.flushed_tiles += 1;
                                    s.flushed_records += records.len() as u64;
                                    s.last_flush_time = Some(Instant::now());
                                }
                            }
                            last_flush = Instant::now();
                        }
                    }

                    // Periodic stratified compaction / tombstone purging
                    if let Some(compact_interval) = auto_compact_interval {
                        if last_compaction.elapsed() >= compact_interval {
                            let temp_path = archive_path.with_extension("compact.tmp.s3a");
                            if Compactor::compact(&[&archive_path], &temp_path).is_ok() {
                                let _ = std::fs::rename(&temp_path, &archive_path);
                                let mut s = stats.lock().unwrap();
                                s.compaction_runs += 1;
                                s.last_compaction_time = Some(Instant::now());
                            }
                            last_compaction = Instant::now();
                        }
                    }

                    thread::sleep(Duration::from_millis(15));
                }

                // Final drain upon termination
                while ring_buffer.len() > 0 {
                    let records = ring_buffer.drain(tile_rec_capacity);
                    if records.is_empty() {
                        break;
                    }
                    if let Ok(mut writer) = TileWriter::open_append(&archive_path) {
                        let timestamps: Vec<u64> = records.iter().map(|r| r.timestamp).collect();
                        let _ = writer.write_hyper_tile(TileType::TELEMETRY, &records, Some(&timestamps), None);
                        let mut s = stats.lock().unwrap();
                        s.flushed_tiles += 1;
                        s.flushed_records += records.len() as u64;
                        s.last_flush_time = Some(Instant::now());
                    }
                }
            })?;

        self.worker_handle = Some(handle);
        Ok(())
    }

    /// Stops the worker thread cleanly, performing a final flush of all pending L0 records.
    pub fn stop(&mut self) {
        if self.running.swap(false, Ordering::SeqCst) {
            if let Some(handle) = self.worker_handle.take() {
                let _ = handle.join();
            }
        }
    }

    /// Forces an immediate synchronous flush of pending L0 records into a Hyper-Tile.
    pub fn flush_now(&self) -> io::Result<usize> {
        let tile_rec_capacity = HYPER_TILE_PAYLOAD_SIZE / std::mem::size_of::<TelemetryRecord>();
        let records = self.ring_buffer.drain_all();
        if records.is_empty() {
            return Ok(0);
        }

        let mut writer = TileWriter::open_append(&self.archive_path)?;
        let mut total_flushed = 0;

        for chunk in records.chunks(tile_rec_capacity) {
            let timestamps: Vec<u64> = chunk.iter().map(|r| r.timestamp).collect();
            writer.write_hyper_tile(TileType::TELEMETRY, chunk, Some(&timestamps), None)?;
            total_flushed += chunk.len();
        }

        let mut s = self.stats.lock().unwrap();
        s.flushed_tiles += (records.len() + tile_rec_capacity - 1) as u64 / tile_rec_capacity as u64;
        s.flushed_records += total_flushed as u64;
        s.last_flush_time = Some(Instant::now());

        Ok(total_flushed)
    }

    /// Executes an on-demand background compaction run, purging tombstones and re-stratifying tiles.
    pub fn trigger_compaction(&self) -> io::Result<u32> {
        let engine = S3ACrudEngine::open_or_create(&self.archive_path)?;
        let new_tile_count = engine.purge_and_compact()?;

        let mut s = self.stats.lock().unwrap();
        s.compaction_runs += 1;
        s.last_compaction_time = Some(Instant::now());

        Ok(new_tile_count)
    }

    /// Returns a copy of current daemon operational statistics.
    pub fn stats(&self) -> CompactorStats {
        self.stats.lock().unwrap().clone()
    }

    /// Returns true if the daemon background thread is running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// CERTIFIED FRESHNESS QUERY (Delta Overlay):
    /// Queries both disk-committed Hyper-Tiles and uncommitted in-memory L0 Ring Buffer records,
    /// resolving version updates and eliminating tombstones across both layers.
    pub fn query_fresh_telemetry(
        &self,
        min_ts: u64,
        max_ts: u64,
        sensor_id: Option<u32>,
        metric_id: Option<u32>,
    ) -> Vec<TelemetryRecord> {
        let mut all_records = Vec::new();

        // 1. Read committed disk records if file exists
        if self.archive_path.exists() {
            if let Ok(reader) = MmapReader::open(&self.archive_path) {
                let sieve = QuerySieve::new(&reader);
                let disk_records = sieve.query_telemetry(min_ts, max_ts, sensor_id, metric_id);
                all_records.extend(disk_records);
            }
        }

        // 2. Read in-memory L0 Ring Buffer records (Delta Overlay)
        let uncommitted = self.ring_buffer.snapshot();
        for rec in uncommitted {
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
                all_records.push(rec);
            }
        }

        // 3. Resolve versioned updates and tombstones ("alive-before-scoring")
        let mut map: std::collections::BTreeMap<(u32, u32, u64), TelemetryRecord> = std::collections::BTreeMap::new();
        for rec in all_records {
            let key = (rec.sensor_id, rec.metric_id, rec.timestamp);
            if rec.is_tombstone() {
                map.remove(&key);
            } else {
                map.insert(key, rec);
            }
        }

        map.into_values().collect()
    }
}

impl Drop for BackgroundCompactor {
    fn drop(&mut self) {
        self.stop();
    }
}
