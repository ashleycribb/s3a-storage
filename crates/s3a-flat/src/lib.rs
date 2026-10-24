//! Flattened transport/storage form (`.s3af`) for S3A files.
//!
//! Real `.s3a` files are mostly zero bytes because Hyper-Tiles have a fixed size. `.s3af` keeps
//! only the non-zero runs of each region and restores the original **byte for byte**. It is
//! meant for sealed, export, backup and sync copies. Working files stay `.s3a`. This crate only
//! reads S3A layout constants; it never changes the S3A format or core.
//!
//! The layout is format-agnostic, so any byte string round-trips (even a file that is not valid
//! S3A). The file is cut into regions: a head region of `FILE_HEADER_SIZE` bytes, then
//! `HYPER_TILE_SIZE` tile regions, then any shorter tail kept as its own region (e.g. trailing
//! bytes after the last full tile).
//!
//! ```text
//! .s3af = header (FLAT_HEADER_LEN bytes, little-endian)
//!   magic "S3AF" | version u16 | flags u16 (bit0 = zstd body)
//!   original_len u64 | head_len u32 | stride u32 | record_count u32
//!   body_len u64 (uncompressed body length) | stored_len u64 (body bytes that follow)
//!   original_sha256 [32] | header_crc32c u32 (over all preceding header bytes)
//! body (zstd-compressed if flag bit0) = record*
//!   kind u8 (0 head, 1 tile, 2 tail) | index u32 | region_len u32 | seg_count u32
//!   seg_count * (offset u32, len u32) | segment bytes | region_crc32c u32 (of the original region)
//! ```
//! A zero run is elided only when it is at least [`MIN_ZERO_RUN`] bytes long. Shorter runs stay
//! inside a segment, so segment overhead never outweighs the saving.
use sha2::{Digest, Sha256};
use std::io::Read;

pub const MAGIC: &[u8; 4] = b"S3AF";
pub const VERSION: u16 = 1;
pub const FLAG_ZSTD: u16 = 1;
pub const FLAT_HEADER_LEN: usize = 4 + 2 + 2 + 8 + 4 + 4 + 4 + 8 + 8 + 32 + 4;
/// Zero runs shorter than this are stored inline.
pub const MIN_ZERO_RUN: usize = 32;
/// Hard ceiling on an original file we will rebuild (16 GiB); guards allocation on bad input.
pub const MAX_ORIGINAL_LEN: u64 = 16 << 30;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlatError {
    Truncated(&'static str),
    BadMagic,
    UnsupportedVersion(u16),
    BadField(&'static str),
    HeaderCrc,
    RecordCrc(u32),
    Sha256Mismatch,
    Codec(String),
}

impl std::fmt::Display for FlatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FlatError::Truncated(w) => write!(f, "flat file truncated ({w})"),
            FlatError::BadMagic => write!(f, "not an .s3af file (bad magic)"),
            FlatError::UnsupportedVersion(v) => write!(f, "unsupported .s3af version {v}"),
            FlatError::BadField(w) => write!(f, "corrupt .s3af field: {w}"),
            FlatError::HeaderCrc => write!(f, ".s3af header CRC32C mismatch"),
            FlatError::RecordCrc(i) => write!(f, ".s3af record {i} CRC32C mismatch"),
            FlatError::Sha256Mismatch => write!(f, "rebuilt file does not match original SHA-256"),
            FlatError::Codec(e) => write!(f, "zstd error: {e}"),
        }
    }
}
impl std::error::Error for FlatError {}

/// Summary of a flat file's header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlatInfo {
    pub version: u16,
    pub zstd: bool,
    pub original_len: u64,
    pub record_count: u32,
    pub body_len: u64,
    pub original_sha256: [u8; 32],
}

#[derive(Clone, Copy)]
struct Region {
    kind: u8,
    index: u32,
    start: usize,
    len: usize,
}

fn regions(len: usize, head: usize, stride: usize) -> Vec<Region> {
    let mut out = Vec::new();
    let h = head.min(len);
    if h > 0 {
        out.push(Region {
            kind: 0,
            index: 0,
            start: 0,
            len: h,
        });
    }
    let mut pos = h;
    let mut i = 0u32;
    while pos < len {
        let l = stride.min(len - pos);
        out.push(Region {
            kind: if l == stride { 1 } else { 2 },
            index: i,
            start: pos,
            len: l,
        });
        pos += l;
        i += 1;
    }
    out
}

/// Non-zero segments (offset, len) of `r`, with zero runs shorter than MIN_ZERO_RUN kept inline.
fn segments(r: &[u8]) -> Vec<(usize, usize)> {
    let mut segs: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < r.len() {
        if r[i] == 0 {
            i += 1;
            continue;
        }
        let s = i;
        let mut zeros = 0;
        let mut j = i;
        while j < r.len() {
            if r[j] == 0 {
                zeros += 1;
                if zeros >= MIN_ZERO_RUN {
                    break;
                }
            } else {
                zeros = 0;
            }
            j += 1;
        }
        let end = if j < r.len() {
            j + 1 - zeros
        } else {
            j - zeros
        };
        segs.push((s, end - s));
        i = end;
    }
    segs
}

/// Flatten `original` into `.s3af` bytes. `zstd_level` = Some(level) compresses the body.
pub fn flatten(original: &[u8], zstd_level: Option<i32>) -> Result<Vec<u8>, FlatError> {
    let head = s3a_core::FILE_HEADER_SIZE;
    let stride = s3a_core::HYPER_TILE_SIZE;
    let regs = regions(original.len(), head, stride);
    let mut body = Vec::new();
    for r in &regs {
        let bytes = &original[r.start..r.start + r.len];
        let segs = segments(bytes);
        body.push(r.kind);
        body.extend_from_slice(&r.index.to_le_bytes());
        body.extend_from_slice(&(r.len as u32).to_le_bytes());
        body.extend_from_slice(&(segs.len() as u32).to_le_bytes());
        for (o, l) in &segs {
            body.extend_from_slice(&(*o as u32).to_le_bytes());
            body.extend_from_slice(&(*l as u32).to_le_bytes());
        }
        for (o, l) in &segs {
            body.extend_from_slice(&bytes[*o..*o + *l]);
        }
        body.extend_from_slice(&crc32c::crc32c(bytes).to_le_bytes());
    }
    let body_len = body.len() as u64;
    let (flags, stored) = match zstd_level {
        Some(l) => (
            FLAG_ZSTD,
            zstd::bulk::compress(&body, l).map_err(|e| FlatError::Codec(e.to_string()))?,
        ),
        None => (0, body),
    };
    let mut out = Vec::with_capacity(FLAT_HEADER_LEN + stored.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&(original.len() as u64).to_le_bytes());
    out.extend_from_slice(&(head as u32).to_le_bytes());
    out.extend_from_slice(&(stride as u32).to_le_bytes());
    out.extend_from_slice(&(regs.len() as u32).to_le_bytes());
    out.extend_from_slice(&body_len.to_le_bytes());
    out.extend_from_slice(&(stored.len() as u64).to_le_bytes());
    out.extend_from_slice(&Sha256::digest(original));
    let crc = crc32c::crc32c(&out);
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(&stored);
    Ok(out)
}

struct Cur<'a> {
    b: &'a [u8],
    p: usize,
}
impl<'a> Cur<'a> {
    fn take(&mut self, n: usize, w: &'static str) -> Result<&'a [u8], FlatError> {
        let end = self.p.checked_add(n).ok_or(FlatError::BadField(w))?;
        let s = self.b.get(self.p..end).ok_or(FlatError::Truncated(w))?;
        self.p = end;
        Ok(s)
    }
    fn u8(&mut self, w: &'static str) -> Result<u8, FlatError> {
        Ok(self.take(1, w)?[0])
    }
    fn u16(&mut self, w: &'static str) -> Result<u16, FlatError> {
        Ok(u16::from_le_bytes(
            self.take(2, w)?
                .try_into()
                .map_err(|_| FlatError::Truncated(w))?,
        ))
    }
    fn u32(&mut self, w: &'static str) -> Result<u32, FlatError> {
        Ok(u32::from_le_bytes(
            self.take(4, w)?
                .try_into()
                .map_err(|_| FlatError::Truncated(w))?,
        ))
    }
    fn u64(&mut self, w: &'static str) -> Result<u64, FlatError> {
        Ok(u64::from_le_bytes(
            self.take(8, w)?
                .try_into()
                .map_err(|_| FlatError::Truncated(w))?,
        ))
    }
}

struct Header {
    info: FlatInfo,
    head: usize,
    stride: usize,
    stored_len: u64,
}

fn parse_header(flat: &[u8]) -> Result<Header, FlatError> {
    let mut c = Cur { b: flat, p: 0 };
    if c.take(4, "magic")? != MAGIC {
        return Err(FlatError::BadMagic);
    }
    let version = c.u16("version")?;
    if version != VERSION {
        return Err(FlatError::UnsupportedVersion(version));
    }
    let flags = c.u16("flags")?;
    if flags & !FLAG_ZSTD != 0 {
        return Err(FlatError::BadField("flags"));
    }
    let original_len = c.u64("original_len")?;
    let head = c.u32("head_len")? as usize;
    let stride = c.u32("stride")? as usize;
    let record_count = c.u32("record_count")?;
    let body_len = c.u64("body_len")?;
    let stored_len = c.u64("stored_len")?;
    let mut sha = [0u8; 32];
    sha.copy_from_slice(c.take(32, "sha256")?);
    let crc_at = c.p;
    let crc = c.u32("header_crc")?;
    if crc32c::crc32c(&flat[..crc_at]) != crc {
        return Err(FlatError::HeaderCrc);
    }
    if original_len > MAX_ORIGINAL_LEN || stride == 0 {
        return Err(FlatError::BadField("original_len/stride"));
    }
    Ok(Header {
        info: FlatInfo {
            version,
            zstd: flags & FLAG_ZSTD != 0,
            original_len,
            record_count,
            body_len,
            original_sha256: sha,
        },
        head,
        stride,
        stored_len,
    })
}

/// Read only the header (cheap; validates the header CRC).
pub fn info(flat: &[u8]) -> Result<FlatInfo, FlatError> {
    parse_header(flat).map(|h| h.info)
}

/// Rebuild the original bytes. Verifies header CRC, every record CRC32C, lengths and the
/// whole-file SHA-256; any mismatch or truncation is an error (never a panic).
pub fn unflatten(flat: &[u8]) -> Result<Vec<u8>, FlatError> {
    let h = parse_header(flat)?;
    let stored = flat
        .get(FLAT_HEADER_LEN..)
        .ok_or(FlatError::Truncated("body"))?;
    if stored.len() as u64 != h.stored_len {
        return Err(if (stored.len() as u64) < h.stored_len {
            FlatError::Truncated("body")
        } else {
            FlatError::BadField("stored_len")
        });
    }
    // Body must be no bigger than the original plus per-record overhead.
    let max_body = h
        .info
        .original_len
        .saturating_mul(9)
        .saturating_add(64 * (h.info.record_count as u64 + 1));
    if h.info.body_len > max_body {
        return Err(FlatError::BadField("body_len"));
    }
    let body_owned;
    let body: &[u8] = if h.info.zstd {
        let mut d = Vec::new();
        zstd::stream::read::Decoder::new(stored)
            .map_err(|e| FlatError::Codec(e.to_string()))?
            .take(h.info.body_len + 1)
            .read_to_end(&mut d)
            .map_err(|e| FlatError::Codec(e.to_string()))?;
        body_owned = d;
        &body_owned
    } else {
        stored
    };
    if body.len() as u64 != h.info.body_len {
        return Err(FlatError::BadField("body_len"));
    }
    let expect = regions(h.info.original_len as usize, h.head, h.stride);
    if expect.len() as u64 != h.info.record_count as u64 {
        return Err(FlatError::BadField("record_count"));
    }
    let mut out = Vec::new();
    let mut c = Cur { b: body, p: 0 };
    for (n, r) in expect.iter().enumerate() {
        let kind = c.u8("record kind")?;
        let index = c.u32("record index")?;
        let len = c.u32("region_len")? as usize;
        if kind != r.kind || index != r.index || len != r.len {
            return Err(FlatError::BadField("record header"));
        }
        let segc = c.u32("seg_count")? as usize;
        if segc > len / 2 + 1 {
            return Err(FlatError::BadField("seg_count"));
        }
        let table = c.take(
            segc.checked_mul(8)
                .ok_or(FlatError::BadField("seg_count"))?,
            "segment table",
        )?;
        let start = out.len();
        out.resize(start + len, 0);
        let mut last_end = 0usize;
        for s in table.as_chunks::<8>().0 {
            let o = u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize;
            let l = u32::from_le_bytes([s[4], s[5], s[6], s[7]]) as usize;
            let end = o.checked_add(l).ok_or(FlatError::BadField("segment"))?;
            if o < last_end || end > len {
                return Err(FlatError::BadField("segment bounds"));
            }
            out[start + o..start + end].copy_from_slice(c.take(l, "segment bytes")?);
            last_end = end;
        }
        let crc = c.u32("record crc")?;
        if crc32c::crc32c(&out[start..]) != crc {
            return Err(FlatError::RecordCrc(n as u32));
        }
    }
    if c.p != body.len() {
        return Err(FlatError::BadField("trailing body bytes"));
    }
    if out.len() as u64 != h.info.original_len {
        return Err(FlatError::BadField("original_len"));
    }
    if Sha256::digest(&out).as_slice() != h.info.original_sha256 {
        return Err(FlatError::Sha256Mismatch);
    }
    Ok(out)
}
