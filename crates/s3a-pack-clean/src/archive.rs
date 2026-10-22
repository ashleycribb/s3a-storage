//! Minimal S3A archive I/O for the pack: write payload tiles, seal, read them back.
use s3a_core::{
    FileHeader, HyperTileHeader, FILE_HEADER_SIZE, HYPER_TILE_PAYLOAD_SIZE, HYPER_TILE_SIZE,
};
use s3a_engine::sealed::{seal_buffer_v2, SealOptionsV2};
use s3a_engine::TileWriter;
use std::io;
use std::path::Path;

/// Max bytes of one logical payload chunk per tile (leaves room for an 8-byte chunk header).
pub const CHUNK: usize = HYPER_TILE_PAYLOAD_SIZE - 64;

/// Write `(tile_type, payload)` tiles into a fresh archive at `path` and seal it.
/// Fails if `path` already exists, so a sealed archive is never overwritten.
pub fn write_sealed(path: &Path, tiles: &[(u32, Vec<u8>)], title: &str) -> io::Result<()> {
    if path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "{} exists; sealed archives are never overwritten",
                path.display()
            ),
        ));
    }
    let tmp = path.with_extension("building.s3a");
    let _ = std::fs::remove_file(&tmp);
    {
        let w = TileWriter::create(&tmp)?;
        drop(w);
        let mut w = TileWriter::open_append(&tmp)?;
        for (t, p) in tiles {
            assert!(p.len() <= HYPER_TILE_PAYLOAD_SIZE);
            let mut h = HyperTileHeader::new(0, *t);
            h.record_count = 1;
            h.payload_bytes = p.len() as u32;
            h.data_crc32 = crc32c::crc32c(p);
            w.write_raw_hyper_tile(&h, p)?;
        }
    }
    let buf = std::fs::read(&tmp)?;
    let _ = std::fs::remove_file(&tmp);
    let len = active_len(&buf)?;
    let opts = SealOptionsV2 {
        write_manifest: true,
        title: Some(title.to_string()),
        ..Default::default()
    };
    let sealed = seal_buffer_v2(&buf[..len], &opts).map_err(|e| io::Error::other(e.to_string()))?;
    std::fs::write(path, sealed)
}

fn active_len(buf: &[u8]) -> io::Result<usize> {
    if buf.len() < FILE_HEADER_SIZE {
        return Err(io::Error::other("archive too small"));
    }
    let a: FileHeader = bytemuck::pod_read_unaligned(&buf[0..64]);
    let b: FileHeader = bytemuck::pod_read_unaligned(&buf[64..128]);
    let (h, _) =
        s3a_engine::resolve_active_header(&a, &b).map_err(|e| io::Error::other(e.to_string()))?;
    Ok((FILE_HEADER_SIZE + h.tile_count as usize * HYPER_TILE_SIZE).min(buf.len()))
}

/// Read all tiles of `tile_type` (payloads, CRC-checked) in file order.
pub fn read_tiles(path: &Path, tile_type: u32) -> io::Result<Vec<Vec<u8>>> {
    let buf = std::fs::read(path)?;
    let len = active_len(&buf)?;
    let hsz = std::mem::size_of::<HyperTileHeader>();
    let mut out = Vec::new();
    let mut off = FILE_HEADER_SIZE;
    while off + hsz <= len {
        let h: HyperTileHeader = bytemuck::pod_read_unaligned(&buf[off..off + hsz]);
        if h.tile_type == tile_type {
            let n = h.payload_bytes as usize;
            let p = buf
                .get(off + hsz..off + hsz + n)
                .ok_or_else(|| io::Error::other("truncated tile"))?;
            if crc32c::crc32c(p) != h.data_crc32 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "tile CRC mismatch",
                ));
            }
            out.push(p.to_vec());
        }
        off += HYPER_TILE_SIZE;
    }
    Ok(out)
}

/// Split `data` into tiles of `tile_type`, each prefixed with `key` (32 bytes) + u32 index + u32 total.
pub fn chunk_tiles(tile_type: u32, key: &[u8; 32], data: &[u8]) -> Vec<(u32, Vec<u8>)> {
    let chunks: Vec<&[u8]> = if data.is_empty() {
        vec![&[][..]]
    } else {
        data.chunks(CHUNK - 40).collect()
    };
    let total = chunks.len() as u32;
    chunks
        .into_iter()
        .enumerate()
        .map(|(i, c)| {
            let mut p = Vec::with_capacity(40 + c.len());
            p.extend_from_slice(key);
            p.extend_from_slice(&(i as u32).to_le_bytes());
            p.extend_from_slice(&total.to_le_bytes());
            p.extend_from_slice(c);
            (tile_type, p)
        })
        .collect()
}

/// Reassemble chunked tiles into `key -> bytes`.
pub fn unchunk(tiles: &[Vec<u8>]) -> io::Result<std::collections::BTreeMap<[u8; 32], Vec<u8>>> {
    let mut parts: std::collections::BTreeMap<[u8; 32], Vec<(u32, u32, Vec<u8>)>> =
        Default::default();
    for t in tiles {
        if t.len() < 40 {
            return Err(io::Error::other("short chunk"));
        }
        let key: [u8; 32] = t[0..32].try_into().unwrap();
        let i = u32::from_le_bytes(t[32..36].try_into().unwrap());
        let n = u32::from_le_bytes(t[36..40].try_into().unwrap());
        parts.entry(key).or_default().push((i, n, t[40..].to_vec()));
    }
    let mut out = std::collections::BTreeMap::new();
    for (k, mut v) in parts {
        v.sort_by_key(|x| x.0);
        let n = v[0].1 as usize;
        if v.len() != n || v.iter().enumerate().any(|(j, x)| x.0 as usize != j) {
            return Err(io::Error::other("missing chunk"));
        }
        out.insert(k, v.into_iter().flat_map(|x| x.2).collect());
    }
    Ok(out)
}
