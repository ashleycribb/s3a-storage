use std::fs::{File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::Path;
use bytemuck::{bytes_of, from_bytes, Pod};
use memmap2::Mmap;

pub use s3a_core::*;
use s3a_simd::{can_reject_tile_point, can_reject_tile_range, can_reject_tile_time};

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
        let offset = FILE_HEADER_SIZE + index * HYPER_TILE_SIZE;
        if offset + HYPER_TILE_SIZE > self.mmap.len() {
            return None;
        }

        let header_slice = &self.mmap[offset..offset + HYPER_TILE_HEADER_SIZE];
        let tile_header: &HyperTileHeader = from_bytes(header_slice);

        let payload_start = offset + HYPER_TILE_HEADER_SIZE;
        let payload_end = payload_start + tile_header.payload_bytes as usize;
        let payload = &self.mmap[payload_start..payload_end];

        Some((tile_header, payload))
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

/// Query Sieve for high-performance single-cycle SIMD tile rejection and fast scanning.
pub struct QuerySieve<'a> {
    reader: &'a MmapReader,
}

impl<'a> QuerySieve<'a> {
    pub fn new(reader: &'a MmapReader) -> Self {
        Self { reader }
    }

    /// Queries telemetry records filtering by timestamp interval and optional sensor/metric IDs.
    pub fn query_telemetry(
        &self,
        min_ts: u64,
        max_ts: u64,
        sensor_id: Option<u32>,
        metric_id: Option<u32>,
    ) -> Vec<TelemetryRecord> {
        let mut results = Vec::new();
        let tile_count = self.reader.tile_count() as usize;

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
            for rec in records {
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
                    results.push(*rec);
                }
            }
        }

        results
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

        // Sort telemetry by timestamp
        telemetry_buffer.sort_by_key(|r| r.timestamp);

        // Compact telemetry into full 128 KB tiles
        let tel_rec_capacity = HYPER_TILE_PAYLOAD_SIZE / std::mem::size_of::<TelemetryRecord>();
        for chunk in telemetry_buffer.chunks(tel_rec_capacity) {
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
    fn test_query_sieve() {
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path();

        let mut writer = TileWriter::create(path).unwrap();

        let records = vec![
            TelemetryRecord::new(1000, 1, 101, 42.5),
            TelemetryRecord::new(2000, 1, 101, 43.0),
            TelemetryRecord::new(3000, 2, 102, 12.0),
        ];
        let timestamps = vec![1000, 2000, 3000];
        writer
            .write_hyper_tile(TileType::TELEMETRY, &records, Some(&timestamps), None)
            .unwrap();

        let reader = MmapReader::open(path).unwrap();
        let sieve = QuerySieve::new(&reader);

        let results = sieve.query_telemetry(1500, 2500, None, None);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].timestamp, 2000);

        let sensor_results = sieve.query_telemetry(0, 4000, Some(2), None);
        assert_eq!(sensor_results.len(), 1);
        assert_eq!(sensor_results[0].sensor_id, 2);
    }

    #[test]
    fn test_compactor() {
        let in1 = NamedTempFile::new().unwrap();
        let in2 = NamedTempFile::new().unwrap();
        let out = NamedTempFile::new().unwrap();

        let mut w1 = TileWriter::create(in1.path()).unwrap();
        let recs1 = vec![TelemetryRecord::new(100, 1, 1, 1.0)];
        w1.write_hyper_tile(TileType::TELEMETRY, &recs1, Some(&[100]), None).unwrap();

        let mut w2 = TileWriter::create(in2.path()).unwrap();
        let recs2 = vec![TelemetryRecord::new(200, 1, 1, 2.0)];
        w2.write_hyper_tile(TileType::TELEMETRY, &recs2, Some(&[200]), None).unwrap();

        let compacted_tiles = Compactor::compact(&[in1.path(), in2.path()], out.path()).unwrap();
        assert_eq!(compacted_tiles, 1);

        let reader = MmapReader::open(out.path()).unwrap();
        let sieve = QuerySieve::new(&reader);
        let res = sieve.query_telemetry(0, 500, None, None);
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].timestamp, 100);
        assert_eq!(res[1].timestamp, 200);
    }
}
