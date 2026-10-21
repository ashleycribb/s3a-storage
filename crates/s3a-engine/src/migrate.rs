//! M1: offline migration of legacy `.s3a` files to the current format.
//!
//! Supported inputs (reconstructed from git history, see `docs/api_changes/2026-10-21-m1-migrate.md`):
//! - **v1.0** (commit 66f70c4): tile `header_crc32` covers only bytes `0..36` of the tile header,
//!   `record_layout_version` (then `_reserved1`) is 0, `SimplexHull` was 216 bytes so every
//!   field after it sits 8 bytes later, and the file `addon_mask`/`file_uuid` bytes are zero.
//!   Current readers reject these tiles with a checksum error.
//! - **v1.1 unaddressed** (spec §"Legacy Compatibility Path"): full-header CRC, but
//!   `record_layout_version <= 1` and `file_uuid == [0, 0]`.
//!
//! The migration never touches the input. It writes a temp file next to the output, re-opens it
//! with [`MmapReader`], checks every CRC and that each tile's type, record count and payload
//! SHA-256 match the source, and only then moves it into place with [`atomic_replace`].

use crate::{
    atomic_replace, compute_header_crc32, compute_tile_header_crc32, generate_file_uuid,
    resolve_active_header, MmapReader,
};
use bytemuck::{bytes_of, pod_read_unaligned};
use s3a_core::*;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Detected source format version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyVersion {
    /// v1.0: prefix-only (bytes 0..28) tile header CRC.
    V1_0,
    /// v1.1 legacy-unaddressed: full-header CRC, zero file_uuid.
    V1_1Unaddressed,
}

impl LegacyVersion {
    pub fn as_str(&self) -> &'static str {
        match self {
            LegacyVersion::V1_0 => "v1.0",
            LegacyVersion::V1_1Unaddressed => "v1.1-unaddressed",
        }
    }
}

/// Summary of a migration (or dry run).
#[derive(Debug, Clone)]
pub struct MigrationReport {
    pub source_version: LegacyVersion,
    pub tile_count: u32,
    pub record_count: u64,
    pub generation: u64,
    /// SHA-256 of the whole source file, for provenance.
    pub source_sha256: [u8; 32],
    /// SHA-256 of the migrated output bytes.
    pub output_sha256: [u8; 32],
    pub new_file_uuid: [u64; 2],
    pub dry_run: bool,
}

fn bad(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

fn legacy_v1_0_tile_crc(raw: &[u8]) -> u32 {
    let off = std::mem::offset_of!(HyperTileHeader, header_crc32);
    crc32c::crc32c(&raw[..off])
}

/// v1.0 used a 216-byte `SimplexHull` (`_reserved: [u32; 4]`), so every field after the hull
/// sat 8 bytes later and bytes 488..512 were `_padding`. Remaps a v1.0 tile header into the
/// current 512-byte layout. Returns None if the dropped reserved bytes are not zero.
fn remap_v1_0_header(raw: &[u8]) -> Option<HyperTileHeader> {
    if raw[256..264].iter().any(|&b| b != 0) || raw[488..512].iter().any(|&b| b != 0) {
        return None;
    }
    let mut buf = [0u8; HYPER_TILE_HEADER_SIZE];
    buf[..256].copy_from_slice(&raw[..256]);
    buf[256..480].copy_from_slice(&raw[264..488]);
    Some(pod_read_unaligned(&buf))
}

/// Parses a legacy tile header into the current struct, detecting its version.
fn parse_legacy_tile(raw: &[u8]) -> Option<(LegacyVersion, HyperTileHeader)> {
    let h: HyperTileHeader = pod_read_unaligned(raw);
    if h.header_crc32 == compute_tile_header_crc32(&h) {
        return Some((LegacyVersion::V1_1Unaddressed, h));
    }
    let off = std::mem::offset_of!(HyperTileHeader, record_layout_version);
    let layout = u32::from_le_bytes(raw[off..off + 4].try_into().ok()?);
    if layout == 0 && h.header_crc32 == legacy_v1_0_tile_crc(raw) {
        return remap_v1_0_header(raw).map(|m| (LegacyVersion::V1_0, m));
    }
    None
}

struct TileFacts {
    tile_type: u32,
    record_count: u32,
    payload_sha: [u8; 32],
}

/// Detects the legacy version and validates every checksum. Rejects current-format, edge-profile,
/// truncated or corrupted inputs.
fn analyze(
    src: &[u8],
) -> io::Result<(
    LegacyVersion,
    FileHeader,
    Vec<TileFacts>,
    Vec<HyperTileHeader>,
)> {
    if src.len() < FILE_HEADER_SIZE {
        return Err(bad("file too small for S3A header"));
    }
    let a: FileHeader = pod_read_unaligned(
        &src[FILE_HEADER_SLOT_A_OFFSET..FILE_HEADER_SLOT_A_OFFSET + FILE_HEADER_SLOT_SIZE],
    );
    let b: FileHeader = pod_read_unaligned(
        &src[FILE_HEADER_SLOT_B_OFFSET..FILE_HEADER_SLOT_B_OFFSET + FILE_HEADER_SLOT_SIZE],
    );
    let (hdr, _) = resolve_active_header(&a, &b).map_err(|e| bad(format!("file header: {e}")))?;
    if hdr.is_edge_profile() || hdr.flags != 0 {
        return Err(bad(
            "edge-profile or flagged files are not legacy hyper-tile files; not migrated",
        ));
    }
    if hdr.file_uuid != [0, 0] {
        return Err(bad(
            "file already has a file_uuid (current format); nothing to migrate",
        ));
    }
    if hdr.addon_mask != 0 || hdr._reserved1 != 0 {
        return Err(bad(
            "unexpected non-zero reserved/addon bytes in a legacy file header",
        ));
    }
    let tc = hdr.tile_count as usize;
    let need = FILE_HEADER_SIZE + tc * HYPER_TILE_SIZE;
    if src.len() < need {
        return Err(bad(format!(
            "truncated: {} tiles need {} bytes, file has {}",
            tc,
            need,
            src.len()
        )));
    }
    let mut version: Option<LegacyVersion> = None;
    let mut facts = Vec::with_capacity(tc);
    let mut headers = Vec::with_capacity(tc);
    for i in 0..tc {
        let off = FILE_HEADER_SIZE + i * HYPER_TILE_SIZE;
        let (v, h) =
            parse_legacy_tile(&src[off..off + HYPER_TILE_HEADER_SIZE]).ok_or_else(|| {
                bad(format!(
                    "tile {i}: header CRC matches neither v1.0 nor v1.1 rules (corrupt)"
                ))
            })?;
        if h.file_uuid != [0, 0] || h.record_layout_version > 1 {
            return Err(bad(format!(
                "tile {i}: not a legacy tile (addressed or layout > 1)"
            )));
        }
        match version {
            None => version = Some(v),
            Some(p) if p != v => {
                return Err(bad(format!("tile {i}: mixed legacy versions in one file")))
            }
            _ => {}
        }
        let len = h.payload_bytes as usize;
        if len > HYPER_TILE_PAYLOAD_SIZE {
            return Err(bad(format!(
                "tile {i}: payload_bytes {len} exceeds capacity"
            )));
        }
        let payload = &src[off + HYPER_TILE_HEADER_SIZE..off + HYPER_TILE_HEADER_SIZE + len];
        if crc32c::crc32c(payload) != h.data_crc32 {
            return Err(bad(format!(
                "tile {i}: payload data CRC mismatch (corrupt)"
            )));
        }
        headers.push(h);
        facts.push(TileFacts {
            tile_type: h.tile_type,
            record_count: h.record_count,
            payload_sha: Sha256::digest(payload).into(),
        });
    }
    // An empty legacy file has no tiles to tell versions apart; treat it as v1.0.
    Ok((version.unwrap_or(LegacyVersion::V1_0), hdr, facts, headers))
}

fn build_output(
    src: &[u8],
    hdr: &FileHeader,
    headers: &[HyperTileHeader],
    uuid: [u64; 2],
) -> Vec<u8> {
    let tc = headers.len();
    let mut out = vec![0u8; FILE_HEADER_SIZE + tc * HYPER_TILE_SIZE];
    let mut slot = *hdr;
    slot.file_uuid = uuid;
    slot.header_crc32 = compute_header_crc32(&slot);
    out[FILE_HEADER_SLOT_A_OFFSET..FILE_HEADER_SLOT_A_OFFSET + FILE_HEADER_SLOT_SIZE]
        .copy_from_slice(bytes_of(&slot));
    for (i, src_h) in headers.iter().enumerate() {
        let off = FILE_HEADER_SIZE + i * HYPER_TILE_SIZE;
        let mut h = *src_h;
        h.record_layout_version = 1;
        h.generation = (i as u64 + 1).min(hdr.generation.max(1));
        h.tile_index = i as u32;
        h._reserved_addr = 0;
        h.file_uuid = uuid;
        h.header_crc32 = compute_tile_header_crc32(&h);
        out[off..off + HYPER_TILE_HEADER_SIZE].copy_from_slice(bytes_of(&h));
        let len = h.payload_bytes as usize;
        let p = off + HYPER_TILE_HEADER_SIZE;
        out[p..p + len].copy_from_slice(&src[p..p + len]);
    }
    out
}

/// Re-reads a migrated file with the current reader and checks it against the source facts.
fn verify_output(path: &Path, facts: &[TileFacts], uuid: [u64; 2]) -> io::Result<()> {
    let r = MmapReader::open(path)?;
    r.verify_checksums()
        .map_err(|e| bad(format!("migrated file failed verify: {e}")))?;
    if r.file_header().file_uuid != uuid || r.tile_count() as usize != facts.len() {
        return Err(bad("migrated file header does not match expectations"));
    }
    for (i, f) in facts.iter().enumerate() {
        let (h, p) = r
            .get_tile(i)
            .ok_or_else(|| bad(format!("tile {i} missing after migration")))?;
        let sha: [u8; 32] = Sha256::digest(p).into();
        if h.tile_type != f.tile_type
            || h.record_count != f.record_count
            || sha != f.payload_sha
            || h.file_uuid != uuid
        {
            return Err(bad(format!("tile {i} differs from source after migration")));
        }
    }
    Ok(())
}

fn temp_path_for(out: &Path) -> PathBuf {
    let mut name = out
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".migrate-tmp");
    out.with_file_name(name)
}

/// Test hook: simulate a crash after the temp file is written but before it is renamed.
#[doc(hidden)]
pub static FAIL_BEFORE_RENAME: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Migrates `input` into a NEW file at `output`. Refuses to overwrite an existing output or to
/// write in place. With `dry_run`, everything is computed and checked in memory and nothing is written.
pub fn migrate_file(input: &Path, output: &Path, dry_run: bool) -> io::Result<MigrationReport> {
    if output.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("output exists: {}", output.display()),
        ));
    }
    if let (Ok(a), Some(parent)) = (fs::canonicalize(input), output.parent()) {
        let parent = if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        };
        if let (Ok(p), Some(n)) = (fs::canonicalize(parent), output.file_name()) {
            if p.join(n) == a {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "in-place migration is not allowed",
                ));
            }
        }
    }
    let src = fs::read(input)?;
    let (version, hdr, facts, headers) = analyze(&src)?;
    let uuid = generate_file_uuid();
    let out_bytes = build_output(&src, &hdr, &headers, uuid);
    let report = MigrationReport {
        source_version: version,
        tile_count: facts.len() as u32,
        record_count: facts.iter().map(|f| f.record_count as u64).sum(),
        generation: hdr.generation,
        source_sha256: Sha256::digest(&src).into(),
        output_sha256: Sha256::digest(&out_bytes).into(),
        new_file_uuid: uuid,
        dry_run,
    };
    if dry_run {
        return Ok(report);
    }
    let tmp = temp_path_for(output);
    let result = (|| {
        let mut f = File::create(&tmp)?;
        f.write_all(&out_bytes)?;
        f.sync_all()?;
        drop(f);
        verify_output(&tmp, &facts, uuid)?;
        if FAIL_BEFORE_RENAME.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "simulated crash before rename",
            ));
        }
        if output.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "output appeared during migration",
            ));
        }
        atomic_replace(&tmp, output)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.map(|_| report)
}
