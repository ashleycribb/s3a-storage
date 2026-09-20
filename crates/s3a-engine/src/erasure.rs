//! Pure Rust Reed-Solomon Erasure Coding Engine ($GF(2^8)$ Galois Field)
//!
//! Provides enterprise cluster-grade durability for S3A Hyper-Tiles without legacy $3\times$ storage costs.
//! Using systematic Cauchy distribution matrices, any 128 KB Hyper-Tile is striped across $K$ data shards
//! and $M$ parity shards (e.g. $4+2$ or $8+2$). If up to $M$ drives or servers fail, the original Hyper-Tile
//! is reconstructed byte-for-byte with 100% verified CRC32C integrity.
//!
//! Storage overhead:
//! - Legacy distributed 3-way replication: $3.0\times$ (200% bloat)
//! - S3A with 8+2 Erasure Coding: $1.25\times$ (only 25% overhead, surviving 2 drive failures)
//! - S3A with 4+2 Erasure Coding: $1.50\times$ (50% overhead, surviving 2 drive failures)

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use bytemuck::{bytes_of, from_bytes, Pod, Zeroable};

/// Error types for Erasure Coding operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErasureError {
    InvalidParameters { k: usize, m: usize },
    DataEmpty,
    InsufficientShards { required: usize, available: usize },
    CorruptedShard { shard_idx: u16, expected: u32, actual: u32 },
    OriginalChecksumMismatch { expected: u32, actual: u32 },
    SingularMatrix,
    Io(String),
}

impl fmt::Display for ErasureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErasureError::InvalidParameters { k, m } => {
                write!(f, "Invalid Reed-Solomon parameters: K={}, M={} (require K >= 1, M >= 1, K+M <= 256)", k, m)
            }
            ErasureError::DataEmpty => write!(f, "Cannot encode empty data buffer"),
            ErasureError::InsufficientShards { required, available } => {
                write!(f, "Insufficient shards for recovery: need at least {}, but only {} available", required, available)
            }
            ErasureError::CorruptedShard { shard_idx, expected, actual } => {
                write!(f, "Shard {} corrupted: CRC32C expected 0x{:08x}, got 0x{:08x}", shard_idx, expected, actual)
            }
            ErasureError::OriginalChecksumMismatch { expected, actual } => {
                write!(f, "Reconstructed Hyper-Tile CRC32C mismatch: expected 0x{:08x}, got 0x{:08x}", expected, actual)
            }
            ErasureError::SingularMatrix => write!(f, "Reconstruction matrix is singular in GF(2^8)"),
            ErasureError::Io(err) => write!(f, "Erasure coding IO error: {}", err),
        }
    }
}

impl std::error::Error for ErasureError {}

impl From<io::Error> for ErasureError {
    fn from(err: io::Error) -> Self {
        ErasureError::Io(err.to_string())
    }
}

/// Galois Field GF(2^8) with primitive polynomial 0x11D (x^8 + x^4 + x^3 + x^2 + 1).
pub struct GF256 {
    exp: [u8; 512],
    log: [u8; 256],
}

impl GF256 {
    pub const fn init() -> Self {
        let mut exp = [0u8; 512];
        let mut log = [0u8; 256];
        let mut val = 1u16;
        let mut i = 0;
        while i < 255 {
            exp[i] = val as u8;
            exp[i + 255] = val as u8;
            log[val as usize] = i as u8;
            val <<= 1;
            if val >= 256 {
                val ^= 0x11d;
            }
            i += 1;
        }
        exp[510] = exp[0];
        exp[511] = exp[1];
        Self { exp, log }
    }

    #[inline(always)]
    pub fn add(&self, a: u8, b: u8) -> u8 {
        a ^ b
    }

    #[inline(always)]
    pub fn sub(&self, a: u8, b: u8) -> u8 {
        a ^ b
    }

    #[inline(always)]
    pub fn mul(&self, a: u8, b: u8) -> u8 {
        if a == 0 || b == 0 {
            0
        } else {
            let idx = self.log[a as usize] as usize + self.log[b as usize] as usize;
            self.exp[idx]
        }
    }

    #[inline(always)]
    pub fn div(&self, a: u8, b: u8) -> u8 {
        if b == 0 {
            panic!("Division by zero in GF(2^8)");
        }
        if a == 0 {
            0
        } else {
            let idx = 255 + self.log[a as usize] as usize - self.log[b as usize] as usize;
            self.exp[idx]
        }
    }

    #[inline(always)]
    pub fn inv(&self, a: u8) -> u8 {
        if a == 0 {
            panic!("Cannot invert 0 in GF(2^8)");
        }
        self.exp[255 - self.log[a as usize] as usize]
    }
}

pub static GF: GF256 = GF256::init();

/// Magic bytes for S3A Shard files (`b"S3AS"`).
pub const SHARD_MAGIC: [u8; 4] = *b"S3AS";

/// 32-byte sector-aligned header for an individual Erasure-Coded Shard.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct TileShardHeader {
    pub magic: [u8; 4],
    pub shard_idx: u16,
    pub k: u16,
    pub m: u16,
    pub reserved: u16,
    pub original_len: u32,
    pub original_crc32c: u32,
    pub shard_crc32c: u32,
    pub padding: [u8; 8],
}

impl TileShardHeader {
    pub fn verify(&self, payload: &[u8]) -> Result<(), ErasureError> {
        if self.magic != SHARD_MAGIC {
            return Err(ErasureError::Io("Invalid shard magic bytes".into()));
        }
        let actual_crc = crc32c::crc32c(payload);
        if actual_crc != self.shard_crc32c {
            return Err(ErasureError::CorruptedShard {
                shard_idx: self.shard_idx,
                expected: self.shard_crc32c,
                actual: actual_crc,
            });
        }
        Ok(())
    }
}

/// An individual Erasure Coded Shard with header and payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileShard {
    pub header: TileShardHeader,
    pub payload: Vec<u8>,
}

impl TileShard {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(32 + self.payload.len());
        out.extend_from_slice(bytes_of(&self.header));
        out.extend_from_slice(&self.payload);
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ErasureError> {
        if bytes.len() < 32 {
            return Err(ErasureError::Io("Shard buffer smaller than 32-byte header".into()));
        }
        let header: TileShardHeader = *from_bytes(&bytes[..32]);
        let payload = bytes[32..].to_vec();
        header.verify(&payload)?;
        Ok(Self { header, payload })
    }
}

/// Reed-Solomon Codec for S3A Hyper-Tiles using Cauchy Systematic Generator Matrices.
pub struct ReedSolomonCodec {
    pub k: usize,
    pub m: usize,
    matrix: Vec<Vec<u8>>, // (K + M) x K systematic generator matrix
}

impl ReedSolomonCodec {
    /// Creates a new Reed-Solomon Codec with K data shards and M parity shards.
    pub fn new(k: usize, m: usize) -> Result<Self, ErasureError> {
        if k == 0 || m == 0 || k + m > 256 {
            return Err(ErasureError::InvalidParameters { k, m });
        }

        // Build systematic generator matrix G of size (K + M) x K
        // Top K rows: Identity Matrix I_K
        // Bottom M rows: Cauchy Matrix C where C[i][j] = 1 / (x_i ^ y_j)
        // Let x_i = i for i in 0..M, y_j = M + j for j in 0..K.
        // Since x_i != y_j, x_i ^ y_j != 0.
        let mut matrix = vec![vec![0u8; k]; k + m];

        // Identity for data rows
        for i in 0..k {
            matrix[i][i] = 1;
        }

        // Cauchy matrix for parity rows
        for i in 0..m {
            let x = i as u8;
            for j in 0..k {
                let y = (m + j) as u8;
                let diff = x ^ y;
                matrix[k + i][j] = GF.inv(diff);
            }
        }

        Ok(Self { k, m, matrix })
    }

    /// Storage overhead multiplier (e.g. 1.25 for 8+2, 1.50 for 4+2).
    pub fn storage_overhead_ratio(&self) -> f64 {
        (self.k + self.m) as f64 / (self.k as f64)
    }

    /// Slices and encodes any data buffer (e.g. 128 KB Hyper-Tile) into K + M shards.
    pub fn encode(&self, data: &[u8]) -> Result<Vec<TileShard>, ErasureError> {
        if data.is_empty() {
            return Err(ErasureError::DataEmpty);
        }

        let original_len = data.len() as u32;
        let original_crc32c = crc32c::crc32c(data);

        // Compute shard size (rounded up to align evenly across K shards)
        let shard_size = (data.len() + self.k - 1) / self.k;

        // Populate K data shards
        let mut data_shards: Vec<Vec<u8>> = Vec::with_capacity(self.k);
        for i in 0..self.k {
            let start = i * shard_size;
            let end = (start + shard_size).min(data.len());
            let mut shard = vec![0u8; shard_size];
            if start < data.len() {
                shard[..end - start].copy_from_slice(&data[start..end]);
            }
            data_shards.push(shard);
        }

        // Generate M parity shards
        let mut parity_shards: Vec<Vec<u8>> = Vec::with_capacity(self.m);
        for i in 0..self.m {
            let row = self.k + i;
            let mut p_shard = vec![0u8; shard_size];
            for b in 0..shard_size {
                let mut sum = 0u8;
                for j in 0..self.k {
                    let coef = self.matrix[row][j];
                    let val = data_shards[j][b];
                    sum ^= GF.mul(coef, val);
                }
                p_shard[b] = sum;
            }
            parity_shards.push(p_shard);
        }

        // Wrap all K + M shards with TileShardHeader
        let mut result = Vec::with_capacity(self.k + self.m);

        for (idx, payload) in data_shards.into_iter().enumerate() {
            let shard_crc32c = crc32c::crc32c(&payload);
            let header = TileShardHeader {
                magic: SHARD_MAGIC,
                shard_idx: idx as u16,
                k: self.k as u16,
                m: self.m as u16,
                reserved: 0,
                original_len,
                original_crc32c,
                shard_crc32c,
                padding: [0u8; 8],
            };
            result.push(TileShard { header, payload });
        }

        for (idx, payload) in parity_shards.into_iter().enumerate() {
            let shard_crc32c = crc32c::crc32c(&payload);
            let header = TileShardHeader {
                magic: SHARD_MAGIC,
                shard_idx: (self.k + idx) as u16,
                k: self.k as u16,
                m: self.m as u16,
                reserved: 0,
                original_len,
                original_crc32c,
                shard_crc32c,
                padding: [0u8; 8],
            };
            result.push(TileShard { header, payload });
        }

        Ok(result)
    }

    /// Reconstructs the original data buffer from ANY K available shards out of K+M.
    pub fn decode(&self, shards: &[Option<TileShard>]) -> Result<Vec<u8>, ErasureError> {
        // Collect available shards
        let mut available: Vec<&TileShard> = Vec::new();
        for shard_opt in shards {
            if let Some(shard) = shard_opt {
                // Verify shard payload integrity first
                shard.header.verify(&shard.payload)?;
                available.push(shard);
            }
        }

        if available.len() < self.k {
            return Err(ErasureError::InsufficientShards {
                required: self.k,
                available: available.len(),
            });
        }

        // Take exactly the first K available shards
        let chosen = &available[..self.k];
        let original_len = chosen[0].header.original_len as usize;
        let original_crc32c = chosen[0].header.original_crc32c;
        let shard_size = chosen[0].payload.len();

        // Fast path: if the first K available shards are exactly the K data shards in order
        let is_all_data = chosen.iter().enumerate().all(|(i, s)| s.header.shard_idx as usize == i);
        if is_all_data {
            let mut reconstructed = Vec::with_capacity(self.k * shard_size);
            for s in chosen {
                reconstructed.extend_from_slice(&s.payload);
            }
            reconstructed.truncate(original_len);
            let actual_crc = crc32c::crc32c(&reconstructed);
            if actual_crc != original_crc32c {
                return Err(ErasureError::OriginalChecksumMismatch {
                    expected: original_crc32c,
                    actual: actual_crc,
                });
            }
            return Ok(reconstructed);
        }

        // Submatrix inversion path:
        // Extract K x K submatrix from generator matrix corresponding to available shard indices
        let mut submatrix = vec![vec![0u8; self.k]; self.k];
        for (i, shard) in chosen.iter().enumerate() {
            let row_idx = shard.header.shard_idx as usize;
            submatrix[i].copy_from_slice(&self.matrix[row_idx]);
        }

        let inv_submatrix = invert_matrix(&submatrix, self.k)?;

        // Reconstruct the K original data shards
        let mut reconstructed_data_shards: Vec<Vec<u8>> = vec![vec![0u8; shard_size]; self.k];
        for b in 0..shard_size {
            for i in 0..self.k {
                let mut sum = 0u8;
                for j in 0..self.k {
                    let coef = inv_submatrix[i][j];
                    let val = chosen[j].payload[b];
                    sum ^= GF.mul(coef, val);
                }
                reconstructed_data_shards[i][b] = sum;
            }
        }

        // Concatenate data shards and truncate to original length
        let mut reconstructed = Vec::with_capacity(self.k * shard_size);
        for shard in reconstructed_data_shards {
            reconstructed.extend_from_slice(&shard);
        }
        reconstructed.truncate(original_len);

        // Verify CRC32C against original
        let actual_crc = crc32c::crc32c(&reconstructed);
        if actual_crc != original_crc32c {
            return Err(ErasureError::OriginalChecksumMismatch {
                expected: original_crc32c,
                actual: actual_crc,
            });
        }

        Ok(reconstructed)
    }
}

/// Inverts a K x K matrix in GF(2^8) using Gaussian Elimination with partial pivoting.
fn invert_matrix(matrix: &[Vec<u8>], k: usize) -> Result<Vec<Vec<u8>>, ErasureError> {
    // Augmented matrix [A | I_K] of size K x 2K
    let mut aug = vec![vec![0u8; 2 * k]; k];
    for r in 0..k {
        aug[r][..k].copy_from_slice(&matrix[r]);
        aug[r][k + r] = 1;
    }

    for c in 0..k {
        // Find pivot
        let mut pivot_row = c;
        while pivot_row < k && aug[pivot_row][c] == 0 {
            pivot_row += 1;
        }
        if pivot_row == k {
            return Err(ErasureError::SingularMatrix);
        }

        // Swap pivot row if needed
        if pivot_row != c {
            aug.swap(c, pivot_row);
        }

        // Scale pivot row so aug[c][c] == 1
        let pivot = aug[c][c];
        let inv_pivot = GF.inv(pivot);
        for j in 0..2 * k {
            aug[c][j] = GF.mul(aug[c][j], inv_pivot);
        }

        // Eliminate column c in all other rows
        for r in 0..k {
            if r != c {
                let factor = aug[r][c];
                if factor != 0 {
                    for j in 0..2 * k {
                        aug[r][j] ^= GF.mul(factor, aug[c][j]);
                    }
                }
            }
        }
    }

    // Extract right K x K matrix
    let mut inv = vec![vec![0u8; k]; k];
    for r in 0..k {
        inv[r].copy_from_slice(&aug[r][k..2 * k]);
    }

    Ok(inv)
}

/// Saves an array of shards to disk in the specified directory.
pub fn save_shards_to_dir(shards: &[TileShard], dir: &Path) -> Result<Vec<PathBuf>, ErasureError> {
    fs::create_dir_all(dir)?;
    let mut paths = Vec::with_capacity(shards.len());
    for shard in shards {
        let filename = format!("shard_{:02}.s3as", shard.header.shard_idx);
        let path = dir.join(filename);
        let mut file = File::create(&path)?;
        file.write_all(&shard.to_bytes())?;
        paths.push(path);
    }
    Ok(paths)
}

/// Loads shards from a directory.
pub fn load_shards_from_dir(dir: &Path) -> Result<Vec<TileShard>, ErasureError> {
    let mut shards = Vec::new();
    let entries = fs::read_dir(dir)?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("s3as") {
            let bytes = fs::read(&path)?;
            let shard = TileShard::from_bytes(&bytes)?;
            shards.push(shard);
        }
    }
    shards.sort_by_key(|s| s.header.shard_idx);
    Ok(shards)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gf256_arithmetic() {
        assert_eq!(GF.add(42, 42), 0);
        assert_eq!(GF.sub(15, 15), 0);
        assert_eq!(GF.mul(0, 100), 0);
        assert_eq!(GF.mul(1, 100), 100);

        // Test inverse: a * inv(a) == 1 for all a in 1..256
        for a in 1..=255u8 {
            let inv_a = GF.inv(a);
            assert_eq!(GF.mul(a, inv_a), 1, "Failed inverse for {}", a);
        }

        // Test division: div(mul(a, b), b) == a
        for a in 1..=50u8 {
            for b in 1..=50u8 {
                let prod = GF.mul(a, b);
                assert_eq!(GF.div(prod, b), a);
            }
        }
    }

    #[test]
    fn test_reed_solomon_4_2_all_shards_present() {
        let codec = ReedSolomonCodec::new(4, 2).unwrap();
        assert_eq!(codec.storage_overhead_ratio(), 1.50);

        // Generate 128 KB sample data (representative Hyper-Tile)
        let mut sample_data = vec![0u8; 131_072];
        for (i, b) in sample_data.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }

        let shards = codec.encode(&sample_data).unwrap();
        assert_eq!(shards.len(), 6);

        // Decode with all 6 shards available
        let available: Vec<Option<TileShard>> = shards.into_iter().map(Some).collect();
        let recovered = codec.decode(&available).unwrap();

        assert_eq!(recovered.len(), sample_data.len());
        assert_eq!(recovered, sample_data);
    }

    #[test]
    fn test_reed_solomon_4_2_survives_two_lost_data_shards() {
        let codec = ReedSolomonCodec::new(4, 2).unwrap();
        let sample_data = b"S3A Hyper-Tile Storage Engine - 1.25x - 1.50x Durability without 3x Bloat!".to_vec();

        let shards = codec.encode(&sample_data).unwrap();
        assert_eq!(shards.len(), 6);

        // Drop shard 0 and shard 1 (lose 2 data shards!)
        let mut available: Vec<Option<TileShard>> = shards.into_iter().map(Some).collect();
        available[0] = None;
        available[1] = None;

        let recovered = codec.decode(&available).unwrap();
        assert_eq!(recovered, sample_data);
    }

    #[test]
    fn test_reed_solomon_8_2_efficiency_and_recovery() {
        let codec = ReedSolomonCodec::new(8, 2).unwrap();
        // 8+2 has only 1.25x overhead (25%)!
        assert_eq!(codec.storage_overhead_ratio(), 1.25);

        let mut sample_data = vec![0u8; 131_072];
        for (i, b) in sample_data.iter_mut().enumerate() {
            *b = ((i * 7) % 255) as u8;
        }

        let shards = codec.encode(&sample_data).unwrap();
        assert_eq!(shards.len(), 10);

        // Drop shard 2 (data) and shard 9 (parity) - 2 drive failures!
        let mut available: Vec<Option<TileShard>> = shards.into_iter().map(Some).collect();
        available[2] = None;
        available[9] = None;

        let recovered = codec.decode(&available).unwrap();
        assert_eq!(recovered, sample_data);
    }

    #[test]
    fn test_reed_solomon_fails_when_too_many_shards_lost() {
        let codec = ReedSolomonCodec::new(4, 2).unwrap();
        let sample_data = vec![42u8; 1024];

        let shards = codec.encode(&sample_data).unwrap();
        let mut available: Vec<Option<TileShard>> = shards.into_iter().map(Some).collect();

        // Drop 3 shards (exceeding M=2)
        available[0] = None;
        available[1] = None;
        available[2] = None;

        let err = codec.decode(&available).unwrap_err();
        match err {
            ErasureError::InsufficientShards { required, available } => {
                assert_eq!(required, 4);
                assert_eq!(available, 3);
            }
            other => panic!("Expected InsufficientShards, got {:?}", other),
        }
    }

    #[test]
    fn test_reed_solomon_corrupted_shard_detection() {
        let codec = ReedSolomonCodec::new(4, 2).unwrap();
        let sample_data = vec![123u8; 2048];

        let mut shards = codec.encode(&sample_data).unwrap();
        // Tamper with payload of shard 1
        shards[1].payload[5] ^= 0xFF;

        let available: Vec<Option<TileShard>> = shards.into_iter().map(Some).collect();
        let err = codec.decode(&available).unwrap_err();
        match err {
            ErasureError::CorruptedShard { shard_idx, .. } => {
                assert_eq!(shard_idx, 1);
            }
            other => panic!("Expected CorruptedShard, got {:?}", other),
        }
    }
}
