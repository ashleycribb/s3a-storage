//! EXPERIMENT (inspired by CliffordNet's differential context `C = C_loc - H`):
//! store each Hyper-Tile payload as a residual against a neighbourhood
//! predictor, then compress with zstd. Lossless and bit-exact.
//!
//! This crate only *reads* S3A constants from `s3a-core`; it never changes the
//! on-disk format. Encoded streams are an experimental container of their own.
//!
//! Residual arithmetic:
//! * `U32`: wrapping integer subtraction (exact for every bit pattern).
//! * `F32`: `bits(x) XOR bits(pred)` where `pred` is computed in f32 from
//!   already-decoded values, so NaN / inf / -0.0 round-trip bit-exactly.
//! * `U8`: wrapping byte subtraction.

use s3a_core::{HYPER_TILE_HEADER_SIZE, HYPER_TILE_PAYLOAD_SIZE, HYPER_TILE_SIZE};

pub const MAGIC: [u8; 4] = *b"NDX1";
const HDR: usize = 4 + 1 + 1 + 1 + 4 + 4 + 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Predictor {
    /// Baseline: residual = value.
    None,
    /// residual = tile[t] - tile[t-1] (element-wise).
    PrevTile,
    /// Laplacian-style: residual = tile[t] - mean(tile[t-1], tile[t-2]).
    NeighborTileMean,
    /// In-tile neighbour values: residual = x[i] - mean(x[i-1], x[i-2]).
    NeighborValueMean,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Elem {
    U8,
    U32,
    F32,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    BadMagic,
    Truncated,
    BadField(&'static str),
    Codec(String),
}

impl Predictor {
    pub const ALL: [Predictor; 4] = [
        Predictor::None,
        Predictor::PrevTile,
        Predictor::NeighborTileMean,
        Predictor::NeighborValueMean,
    ];
    fn id(self) -> u8 {
        self as u8
    }
    fn from_id(v: u8) -> Result<Self, Error> {
        Self::ALL.get(v as usize).copied().ok_or(Error::BadField("predictor"))
    }
}
impl Elem {
    fn id(self) -> u8 {
        self as u8
    }
    fn from_id(v: u8) -> Result<Self, Error> {
        [Elem::U8, Elem::U32, Elem::F32]
            .get(v as usize)
            .copied()
            .ok_or(Error::BadField("elem"))
    }
    fn width(self) -> usize {
        match self {
            Elem::U8 => 1,
            _ => 4,
        }
    }
}

/// Hyper-Tile payloads extracted read-only from an S3A file image
/// (file header, then fixed-size Hyper-Tiles). Trailing partial tiles are ignored.
pub fn hyper_tile_payloads(file: &[u8]) -> Vec<&[u8]> {
    let body = file.get(s3a_core::FILE_HEADER_SIZE..).unwrap_or(&[]);
    body.chunks_exact(HYPER_TILE_SIZE)
        .map(|t| &t[HYPER_TILE_HEADER_SIZE..HYPER_TILE_HEADER_SIZE + HYPER_TILE_PAYLOAD_SIZE])
        .collect()
}

// ---- element views (little-endian) ----
fn get(buf: &[u8], e: Elem, i: usize) -> u32 {
    match e {
        Elem::U8 => buf[i] as u32,
        _ => u32::from_le_bytes(buf[i * 4..i * 4 + 4].try_into().unwrap()),
    }
}
fn put(buf: &mut [u8], e: Elem, i: usize, v: u32) {
    match e {
        Elem::U8 => buf[i] = v as u8,
        _ => buf[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes()),
    }
}
fn mean(e: Elem, a: u32, b: u32) -> u32 {
    match e {
        // overflow-free integer mean
        Elem::U8 | Elem::U32 => (a & b) + ((a ^ b) >> 1),
        Elem::F32 => {
            let m = (f32::from_bits(a) + f32::from_bits(b)) * 0.5;
            m.to_bits()
        }
    }
}
fn sub(e: Elem, x: u32, p: u32) -> u32 {
    match e {
        Elem::U8 => (x as u8).wrapping_sub(p as u8) as u32,
        Elem::U32 => x.wrapping_sub(p),
        Elem::F32 => x ^ p,
    }
}
fn add(e: Elem, r: u32, p: u32) -> u32 {
    match e {
        Elem::U8 => (r as u8).wrapping_add(p as u8) as u32,
        Elem::U32 => r.wrapping_add(p),
        Elem::F32 => r ^ p,
    }
}

/// Prediction for element `i` of tile `t`, given fully-decoded tiles `prev`
/// (t-1, t-2) and the already-decoded prefix of the current tile.
fn predict(p: Predictor, e: Elem, prev: &[&[u8]], cur: &[u8], i: usize) -> u32 {
    match p {
        Predictor::None => 0,
        Predictor::PrevTile => prev.first().map_or(0, |a| get(a, e, i)),
        Predictor::NeighborTileMean => match prev {
            [a, b, ..] => mean(e, get(a, e, i), get(b, e, i)),
            [a] => get(a, e, i),
            [] => 0,
        },
        Predictor::NeighborValueMean => match i {
            0 => 0,
            1 => get(cur, e, 0),
            _ => mean(e, get(cur, e, i - 1), get(cur, e, i - 2)),
        },
    }
}

/// Encode equal-length tiles. Bytes beyond a whole number of elements in each
/// tile are carried through unpredicted.
pub fn encode(tiles: &[&[u8]], p: Predictor, e: Elem, level: i32) -> Result<Vec<u8>, Error> {
    let tile_len = tiles.first().map_or(0, |t| t.len());
    if tiles.iter().any(|t| t.len() != tile_len) {
        return Err(Error::BadField("tiles must be equal length"));
    }
    let n = tile_len / e.width();
    let mut resid = Vec::with_capacity(tile_len * tiles.len());
    for (t, tile) in tiles.iter().enumerate() {
        let prev: Vec<&[u8]> = (1..=2).filter(|k| t >= *k).map(|k| tiles[t - k]).collect();
        let mut out = tile.to_vec();
        for i in 0..n {
            let r = sub(e, get(tile, e, i), predict(p, e, &prev, tile, i));
            put(&mut out, e, i, r);
        }
        resid.extend_from_slice(&out);
    }
    let z = zstd::bulk::compress(&resid, level).map_err(|x| Error::Codec(x.to_string()))?;
    let mut v = Vec::with_capacity(HDR + z.len());
    v.extend_from_slice(&MAGIC);
    v.push(1); // version
    v.push(p.id());
    v.push(e.id());
    v.extend_from_slice(&(tile_len as u32).to_le_bytes());
    v.extend_from_slice(&(tiles.len() as u32).to_le_bytes());
    v.extend_from_slice(&(resid.len() as u64).to_le_bytes());
    v.extend_from_slice(&z);
    Ok(v)
}

pub fn decode(buf: &[u8]) -> Result<Vec<Vec<u8>>, Error> {
    if buf.len() < HDR {
        return Err(Error::Truncated);
    }
    if buf[..4] != MAGIC {
        return Err(Error::BadMagic);
    }
    if buf[4] != 1 {
        return Err(Error::BadField("version"));
    }
    let p = Predictor::from_id(buf[5])?;
    let e = Elem::from_id(buf[6])?;
    let tile_len = u32::from_le_bytes(buf[7..11].try_into().unwrap()) as usize;
    let count = u32::from_le_bytes(buf[11..15].try_into().unwrap()) as usize;
    let raw_len = u64::from_le_bytes(buf[15..23].try_into().unwrap()) as usize;
    if tile_len.checked_mul(count) != Some(raw_len) {
        return Err(Error::BadField("length"));
    }
    let resid = zstd::bulk::decompress(&buf[HDR..], raw_len).map_err(|x| Error::Codec(x.to_string()))?;
    if resid.len() != raw_len {
        return Err(Error::Truncated);
    }
    let n = if tile_len == 0 { 0 } else { tile_len / e.width() };
    let mut tiles: Vec<Vec<u8>> = Vec::with_capacity(count);
    for t in 0..count {
        let mut cur = resid[t * tile_len..(t + 1) * tile_len].to_vec();
        for i in 0..n {
            let pr = {
                let prev: Vec<&[u8]> = (1..=2).filter(|k| t >= *k).map(|k| tiles[t - k].as_slice()).collect();
                predict(p, e, &prev, &cur, i)
            };
            let v = add(e, get(&cur, e, i), pr);
            put(&mut cur, e, i, v);
        }
        tiles.push(cur);
    }
    Ok(tiles)
}
