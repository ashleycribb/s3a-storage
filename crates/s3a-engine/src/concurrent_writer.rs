use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};
use bytemuck::Pod;
use s3a_core::{HyperTileHeader, SimplexHull};

use crate::ring_buffer::{BackpressurePolicy, IngestError, L0RingBuffer};
use crate::TileWriter;

/// Thread-safe concurrent writer for S3A archives.
///
/// Wraps `TileWriter` with a synchronization layer, enabling multiple producer threads
/// or parallel AI agents to append hyper-tiles and records concurrently without file corruption,
/// torn writes, or header generation collisions.
#[derive(Clone)]
pub struct ConcurrentTileWriter {
    writer: Arc<Mutex<TileWriter>>,
}

impl ConcurrentTileWriter {
    /// Helper to acquire an exclusive OS advisory lock on the underlying archive file.
    fn acquire_lock(writer: &TileWriter) -> io::Result<()> {
        writer.try_lock_exclusive().map_err(|e| {
            // Windows ERROR_LOCK_VIOLATION = 33; Unix EWOULDBLOCK / EAGAIN
            if e.raw_os_error() == Some(33) || e.kind() == io::ErrorKind::WouldBlock {
                io::Error::new(io::ErrorKind::WouldBlock, "Archive file is already locked by another process")
            } else {
                e
            }
        })
    }

    /// Creates a new S3A storage archive at `path` managed by a concurrent writer,
    /// acquiring an exclusive OS file lock to prevent multi-process corruption.
    pub fn create<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let writer = TileWriter::create(path)?;
        Self::acquire_lock(&writer)?;
        Ok(Self {
            writer: Arc::new(Mutex::new(writer)),
        })
    }

    /// Opens an existing S3A archive for appending, resolving the active dual-header slot,
    /// acquiring an exclusive OS file lock to prevent multi-process corruption.
    pub fn open_append<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let writer = TileWriter::open_append(path).map_err(|e| {
            if e.raw_os_error() == Some(33) || e.kind() == io::ErrorKind::WouldBlock {
                io::Error::new(io::ErrorKind::WouldBlock, "Archive file is already locked by another process")
            } else {
                e
            }
        })?;
        Self::acquire_lock(&writer)?;
        Ok(Self {
            writer: Arc::new(Mutex::new(writer)),
        })
    }

    /// Wraps an existing `TileWriter` into a `ConcurrentTileWriter`, acquiring an exclusive OS file lock.
    pub fn from_writer(writer: TileWriter) -> io::Result<Self> {
        Self::acquire_lock(&writer)?;
        Ok(Self {
            writer: Arc::new(Mutex::new(writer)),
        })
    }

    /// Enables an addon capability bitmask (e.g. `ADDON_EDU`) across all concurrent writers.
    pub fn enable_addon(&self, mask: u32) -> io::Result<()> {
        let mut writer = self.writer.lock().map_err(|_| {
            io::Error::new(io::ErrorKind::Other, "Failed to acquire TileWriter lock")
        })?;
        writer.enable_addon(mask)
    }

    /// Returns the currently committed tile count, reporting an error if lock acquisition fails.
    pub fn tile_count(&self) -> io::Result<u32> {
        let writer = self.writer.lock().map_err(|_| {
            io::Error::new(io::ErrorKind::Other, "Failed to acquire TileWriter lock")
        })?;
        Ok(writer.tile_count())
    }

    /// Reads all committed archive bytes safely under the write lock.
    pub fn read_all_bytes(&self) -> io::Result<Vec<u8>> {
        let mut writer = self.writer.lock().map_err(|_| {
            io::Error::new(io::ErrorKind::Other, "Failed to acquire TileWriter lock")
        })?;
        writer.read_all_bytes()
    }

    /// Returns the current header generation counter, reporting an error if lock acquisition fails.
    pub fn generation(&self) -> io::Result<u64> {
        let writer = self.writer.lock().map_err(|_| {
            io::Error::new(io::ErrorKind::Other, "Failed to acquire TileWriter lock")
        })?;
        Ok(writer.generation())
    }

    /// Returns the active addon mask, reporting an error if lock acquisition fails.
    pub fn addon_mask(&self) -> io::Result<u32> {
        let writer = self.writer.lock().map_err(|_| {
            io::Error::new(io::ErrorKind::Other, "Failed to acquire TileWriter lock")
        })?;
        Ok(writer.addon_mask())
    }

    /// Atomically appends a complete 128 KB Hyper-Tile to disk, advancing the header generation.
    ///
    /// Thread-safe: serializes multiple concurrent callers and guarantees monotonic generation slots.
    pub fn write_hyper_tile<T: Pod>(
        &self,
        tile_type: u32,
        records: &[T],
        timestamps: Option<&[u64]>,
        hull: Option<SimplexHull>,
    ) -> io::Result<u64> {
        let mut writer = self.writer.lock().map_err(|_| {
            io::Error::new(io::ErrorKind::Other, "Failed to acquire TileWriter lock")
        })?;
        writer.write_hyper_tile(tile_type, records, timestamps, hull)
    }

    /// Atomically appends a complete 128 KB Hyper-Tile with explicit Simplex convex hull bounds.
    pub fn write_hyper_tile_ext<T: Pod>(
        &self,
        tile_type: u32,
        records: &[T],
        timestamps: Option<&[u64]>,
        hull: Option<SimplexHull>,
        filter_keys: Option<&[u64]>,
    ) -> io::Result<u64> {
        let mut writer = self.writer.lock().map_err(|_| {
            io::Error::new(io::ErrorKind::Other, "Failed to acquire TileWriter lock")
        })?;
        writer.write_hyper_tile_ext(tile_type, records, timestamps, hull, filter_keys)
    }

    /// Atomically appends a raw variable-length payload Hyper-Tile to disk, advancing the header generation.
    pub fn write_raw_hyper_tile(&self, header: &HyperTileHeader, payload: &[u8]) -> io::Result<u64> {
        let mut writer = self.writer.lock().map_err(|_| {
            io::Error::new(io::ErrorKind::Other, "Failed to acquire TileWriter lock")
        })?;
        writer.write_raw_hyper_tile(header, payload)
    }

    /// Creates an asynchronous batching ingester for high-frequency record streams.
    ///
    /// Producers push individual records lock-free into an `L0RingBuffer`, and the
    /// ingester flushes full 128 KB hyper-tiles to disk when thresholds are reached.
    pub fn create_ingester<T: Pod + Copy + Default + Send + 'static>(
        &self,
        tile_type: u32,
        buffer_capacity: usize,
        policy: BackpressurePolicy,
    ) -> ConcurrentRecordIngester<T> {
        ConcurrentRecordIngester {
            writer: self.clone(),
            tile_type,
            ring_buffer: Arc::new(L0RingBuffer::new(buffer_capacity, policy)),
        }
    }
}

/// High-throughput, thread-safe record batching ingester.
///
/// Enables many producer threads to push records concurrently with configurable backpressure,
/// batching flushes into 128 KB Hyper-Tiles on the underlying `ConcurrentTileWriter`.
pub struct ConcurrentRecordIngester<T: Pod + Copy + Default + Send + 'static> {
    writer: ConcurrentTileWriter,
    tile_type: u32,
    ring_buffer: Arc<L0RingBuffer<T>>,
}

impl<T: Pod + Copy + Default + Send + 'static> Clone for ConcurrentRecordIngester<T> {
    fn clone(&self) -> Self {
        Self {
            writer: self.writer.clone(),
            tile_type: self.tile_type,
            ring_buffer: Arc::clone(&self.ring_buffer),
        }
    }
}

impl<T: Pod + Copy + Default + Send + 'static> ConcurrentRecordIngester<T> {
    /// Pushes a record into the ingestion buffer according to backpressure policy.
    pub fn push(&self, record: T) -> Result<(), IngestError> {
        self.ring_buffer.push(record)
    }

    /// Returns the number of currently buffered unwritten records.
    pub fn buffered_count(&self) -> usize {
        self.ring_buffer.len()
    }

    /// Flushes buffered records if the count reaches or exceeds `threshold`.
    ///
    /// Returns `Some(tile_id)` if a tile was written, or `None` if the threshold was not reached.
    pub fn flush_if_threshold(&self, threshold: usize) -> io::Result<Option<u64>> {
        if self.ring_buffer.len() >= threshold {
            self.flush_tile()
        } else {
            Ok(None)
        }
    }

    /// Drains all buffered records (up to max HyperTile capacity) and writes a Hyper-Tile to disk.
    pub fn flush_tile(&self) -> io::Result<Option<u64>> {
        let max_records = (s3a_core::HYPER_TILE_PAYLOAD_SIZE / std::mem::size_of::<T>()).max(1);
        let drained = self.ring_buffer.drain(max_records);
        if drained.is_empty() {
            return Ok(None);
        }

        let tile_id = self.writer.write_hyper_tile(self.tile_type, &drained, None, None)?;
        Ok(Some(tile_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;
    use std::thread;
    use s3a_core::TelemetryRecord;
    use crate::MmapReader;

    #[test]
    fn test_concurrent_tile_writer_parallel_threads() {
        let temp_dir = std::env::temp_dir();
        let path = temp_dir.join(format!("s3a_concurrent_test_{}.s3a", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));

        let writer = ConcurrentTileWriter::create(&path).expect("create concurrent writer");

        let thread_count = 8;
        let tiles_per_thread = 4;
        let barrier = Arc::new(Barrier::new(thread_count));

        let mut handles = Vec::new();
        for t_idx in 0..thread_count {
            let w = writer.clone();
            let b = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                b.wait();
                for i in 0..tiles_per_thread {
                    let records = vec![
                        TelemetryRecord::new((t_idx * 1000 + i) as u64, t_idx as u32, 1, (t_idx * 10 + i) as f64);
                        16
                    ];
                    w.write_hyper_tile(0, &records, None, None).expect("write hyper tile");
                }
            }));
        }

        for h in handles {
            h.join().expect("thread failed");
        }

        assert_eq!(writer.tile_count().unwrap(), (thread_count * tiles_per_thread) as u32);
        assert_eq!(writer.generation().unwrap(), (thread_count * tiles_per_thread + 1) as u64);

        // Verify with MmapReader
        let reader = MmapReader::open(&path).expect("open reader");
        assert_eq!(reader.tile_count(), (thread_count * tiles_per_thread) as u32);
        for i in 0..reader.tile_count() as usize {
            let (hdr, payload) = reader.get_tile(i).expect("get tile");
            assert_eq!(hdr.tile_type, 0);
            assert_eq!(hdr.record_count, 16);
            assert_eq!(hdr.data_crc32, crc32c::crc32c(payload));
        }

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_concurrent_tile_writer_second_open_fails_with_would_block() {
        let temp_dir = std::env::temp_dir();
        let path = temp_dir.join(format!("s3a_lock_test_{}.s3a", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));

        let writer1 = ConcurrentTileWriter::create(&path).expect("create first writer");
        assert_eq!(writer1.tile_count().unwrap(), 0);

        // A second concurrent writer attempting to open the same file must fail fast with WouldBlock
        let res = ConcurrentTileWriter::open_append(&path);
        assert!(res.is_err(), "Second open must fail when lock is held");
        let err = res.err().unwrap();
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock);

        // Dropping first writer releases lock
        drop(writer1);

        let writer2 = ConcurrentTileWriter::open_append(&path).expect("open after release");
        assert_eq!(writer2.tile_count().unwrap(), 0);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_concurrent_ingester_batching() {
        let temp_dir = std::env::temp_dir();
        let path = temp_dir.join(format!("s3a_ingester_test_{}.s3a", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));

        let writer = ConcurrentTileWriter::create(&path).expect("create concurrent writer");
        let ingester = writer.create_ingester::<TelemetryRecord>(0, 1024, BackpressurePolicy::Block);

        let thread_count = 4;
        let records_per_thread = 50;

        let mut handles = Vec::new();
        for t_idx in 0..thread_count {
            let ing = ingester.clone();
            handles.push(thread::spawn(move || {
                for i in 0..records_per_thread {
                    let rec = TelemetryRecord::new(i as u64, t_idx as u32, 1, 42.0);
                    ing.push(rec).expect("push record");
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(ingester.buffered_count(), thread_count * records_per_thread);

        let written_tile = ingester.flush_tile().expect("flush tile");
        assert!(written_tile.is_some());
        assert_eq!(ingester.buffered_count(), 0);
        assert_eq!(writer.tile_count().unwrap(), 1);

        let _ = std::fs::remove_file(path);
    }
}

