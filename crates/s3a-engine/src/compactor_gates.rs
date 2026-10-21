use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use s3a_core::{HyperTileHeader, HYPER_TILE_HEADER_SIZE};
use xxhash_rust::xxh3::xxh3_64;

use crate::compute_tile_header_crc32;

/// Global in-memory registry of quarantined tiles: (canonical_archive_path, tile_id, generation).
static QUARANTINED_TILES: std::sync::LazyLock<RwLock<HashSet<(PathBuf, u64, u64)>>> =
    std::sync::LazyLock::new(|| RwLock::new(HashSet::new()));

/// Returns sidecar path for quarantine tracking: `<archive_path>.quarantine`.
pub fn quarantine_path<P: AsRef<Path>>(archive_path: P) -> PathBuf {
    let p = archive_path.as_ref();
    let mut s = p.as_os_str().to_os_string();
    s.push(".quarantine");
    PathBuf::from(s)
}

/// Checks whether a specific (tile_id, generation) in an archive is quarantined.
/// Fails closed: an unreadable or corrupt sidecar line returns an error instead of being skipped.
pub fn is_tile_quarantined<P: AsRef<Path>>(
    archive_path: P,
    tile_id: u64,
    generation: u64,
) -> io::Result<bool> {
    let p = archive_path.as_ref();
    let canonical = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());

    // 1. Check in-memory set
    if let Ok(set) = QUARANTINED_TILES.read() {
        if set.contains(&(canonical.clone(), tile_id, generation))
            || set.contains(&(p.to_path_buf(), tile_id, generation))
        {
            return Ok(true);
        }
    }

    // 2. Check persistent sidecar file
    let q_path = quarantine_path(p);
    if q_path.exists() {
        let file = fs::File::open(&q_path)?;
        let reader = BufReader::new(file);
        for (idx, line_res) in reader.lines().enumerate() {
            let line = line_res?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            // Must match format "tile_id:generation" or "tile_id,generation"
            let mut parts = trimmed.split([':', ',']);
            let id_opt = parts.next();
            let gen_opt = parts.next();
            let extra_opt = parts.next();

            match (id_opt, gen_opt, extra_opt) {
                (Some(id_s), Some(gen_s), None) => {
                    let parsed_id = id_s.trim().parse::<u64>().map_err(|e| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!(
                                "Corrupted quarantine sidecar in '{}' at line {}: invalid tile_id '{}' ({})",
                                q_path.display(),
                                idx + 1,
                                id_s,
                                e
                            ),
                        )
                    })?;
                    let parsed_gen = gen_s.trim().parse::<u64>().map_err(|e| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!(
                                "Corrupted quarantine sidecar in '{}' at line {}: invalid generation '{}' ({})",
                                q_path.display(),
                                idx + 1,
                                gen_s,
                                e
                            ),
                        )
                    })?;
                    if parsed_id == tile_id && parsed_gen == generation {
                        return Ok(true);
                    }
                }
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "Corrupted quarantine sidecar in '{}' at line {}: unreadable line '{}' (expected 'tile_id:generation')",
                            q_path.display(),
                            idx + 1,
                            trimmed
                        ),
                    ));
                }
            }
        }
    }

    Ok(false)
}

/// Marks a (tile_id, generation) quarantined both in memory and in the persistent sidecar file.
pub fn quarantine_tile<P: AsRef<Path>>(
    archive_path: P,
    tile_id: u64,
    generation: u64,
) -> io::Result<()> {
    let p = archive_path.as_ref();
    let canonical = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());

    if let Ok(mut set) = QUARANTINED_TILES.write() {
        set.insert((canonical, tile_id, generation));
        set.insert((p.to_path_buf(), tile_id, generation));
    }

    let q_path = quarantine_path(p);
    let mut file = OpenOptions::new().create(true).append(true).open(&q_path)?;
    writeln!(file, "{}:{}", tile_id, generation)?;
    file.flush()?;
    file.sync_all()?;

    Ok(())
}

/// Clears quarantine records for an archive (used in test teardown / resets).
pub fn clear_quarantine<P: AsRef<Path>>(archive_path: P) {
    let p = archive_path.as_ref();
    let canonical = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());

    if let Ok(mut set) = QUARANTINED_TILES.write() {
        set.retain(|(path, _, _)| path != &canonical && path != p);
    }
    let q_path = quarantine_path(p);
    let _ = fs::remove_file(q_path);
}

/// Order-independent accumulator for live records to verify count and integrity during compaction
/// powered by high-avalanche 64-bit XXH3 hashing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LiveRecordAccumulator {
    pub count: u64,
    pub sum_hash: u64,
    pub xor_hash: u64,
}

impl LiveRecordAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_record_bytes(&mut self, bytes: &[u8]) {
        self.count += 1;
        let h = xxh3_64(bytes);
        self.sum_hash = self.sum_hash.wrapping_add(h);
        self.xor_hash ^= h;
    }

    pub fn push_record<T: bytemuck::Pod>(&mut self, record: &T) {
        self.push_record_bytes(bytemuck::bytes_of(record));
    }

    pub fn verify_equal(&self, out: &Self) -> io::Result<()> {
        if self.count != out.count {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Compaction During-Gate Failed: live record count mismatch (in: {}, out: {})",
                    self.count, out.count
                ),
            ));
        }
        if self.sum_hash != out.sum_hash || self.xor_hash != out.xor_hash {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Compaction During-Gate Failed: live record order-independent xxh3_64 hash mismatch (in: sum=0x{:016X}, xor=0x{:016X}; out: sum=0x{:016X}, xor=0x{:016X})",
                    self.sum_hash, self.xor_hash, out.sum_hash, out.xor_hash
                ),
            ));
        }
        Ok(())
    }
}

/// Enforces the Before-Gate: verifies CRC of every input tile, checks quarantine,
/// and ensures each input tile is consumed only once per compaction generation.
pub struct CompactionPreflight {
    seen_tiles: HashSet<(PathBuf, u64, u64)>, // (canonical_path, tile_id, generation)
}

impl CompactionPreflight {
    pub fn new() -> Self {
        Self {
            seen_tiles: HashSet::new(),
        }
    }

    pub fn verify_and_register_tile<P: AsRef<Path>>(
        &mut self,
        archive_path: P,
        generation: u64,
        header: &HyperTileHeader,
        payload: &[u8],
    ) -> io::Result<()> {
        let p = archive_path.as_ref();
        let canonical = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());

        // 1. Check quarantine by (tile_id, generation)
        if is_tile_quarantined(p, header.tile_id, generation)? {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "Compaction refused: tile {} (generation {}) in archive '{}' is quarantined",
                    header.tile_id,
                    generation,
                    p.display()
                ),
            ));
        }

        // 2. Allow each input tile to be used only once per compaction generation
        let key = (canonical, header.tile_id, generation);
        if !self.seen_tiles.insert(key) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "Compaction Before-Gate Failed: duplicate input tile detected: tile {} (gen {}) in '{}' consumed more than once",
                    header.tile_id,
                    generation,
                    p.display()
                ),
            ));
        }

        // 3. Verify input tile header CRC (fail-closed: zero CRC rejected unless explicitly legacy)
        let is_legacy = header.record_layout_version == 0;
        if header.header_crc32 == 0 {
            if !is_legacy {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "Compaction Before-Gate Failed: input tile {} has zero header CRC but is not marked as legacy",
                        header.tile_id
                    ),
                ));
            }
        } else {
            let expected_hdr_crc = compute_tile_header_crc32(header);
            if header.header_crc32 != expected_hdr_crc {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "Compaction Before-Gate Failed: input tile {} header CRC mismatch (expected 0x{:08X}, calculated 0x{:08X})",
                        header.tile_id, header.header_crc32, expected_hdr_crc
                    ),
                ));
            }
        }

        // 4. Verify input tile data CRC
        let actual_data_crc = crc32c::crc32c(payload);
        if actual_data_crc != header.data_crc32 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Compaction Before-Gate Failed: input tile {} data CRC mismatch (expected 0x{:08X}, calculated 0x{:08X})",
                    header.tile_id, header.data_crc32, actual_data_crc
                ),
            ));
        }

        Ok(())
    }
}

/// Test-only fault-injection hooks for simulating corrupted pipelines, crashes, and drops.
#[cfg(any(test, feature = "fault-injection"))]
pub mod fault_injection {
    use std::sync::atomic::{AtomicBool, Ordering};

    pub static INJECT_RECORD_DROP: AtomicBool = AtomicBool::new(false);
    pub static INJECT_RECORD_DUPLICATION: AtomicBool = AtomicBool::new(false);
    pub static INJECT_CRASH_BEFORE_COMMIT: AtomicBool = AtomicBool::new(false);

    pub fn reset() {
        INJECT_RECORD_DROP.store(false, Ordering::SeqCst);
        INJECT_RECORD_DUPLICATION.store(false, Ordering::SeqCst);
        INJECT_CRASH_BEFORE_COMMIT.store(false, Ordering::SeqCst);
    }
}

/// Re-reads a written tile from disk and verifies both header CRC and payload CRC before the A/B commit switch.
pub fn verify_written_tile_from_disk(
    file: &mut fs::File,
    tile_offset: u64,
    expected_header_crc: u32,
    expected_data_crc: u32,
    payload_bytes: usize,
) -> io::Result<()> {
    file.seek(SeekFrom::Start(tile_offset))?;
    let mut header_buf = [0u8; HYPER_TILE_HEADER_SIZE];
    file.read_exact(&mut header_buf)?;

    let disk_header: &HyperTileHeader =
        &bytemuck::pod_read_unaligned::<HyperTileHeader>(&header_buf);
    let disk_header_crc = compute_tile_header_crc32(disk_header);
    if disk_header.header_crc32 != expected_header_crc
        || disk_header.header_crc32 != disk_header_crc
    {
        file.seek(SeekFrom::End(0))?;
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "After-gate output tile header CRC mismatch on disk: expected 0x{:08X}, read 0x{:08X}",
                expected_header_crc, disk_header.header_crc32
            ),
        ));
    }

    let mut payload_buf = vec![0u8; payload_bytes];
    file.read_exact(&mut payload_buf)?;
    let disk_data_crc = crc32c::crc32c(&payload_buf);
    if disk_data_crc != expected_data_crc {
        file.seek(SeekFrom::End(0))?;
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "After-gate output tile data CRC mismatch on disk: expected 0x{:08X}, read 0x{:08X}",
                expected_data_crc, disk_data_crc
            ),
        ));
    }

    file.seek(SeekFrom::End(0))?;
    Ok(())
}
