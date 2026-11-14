//! S3A Core Plain Document and Key-Value Binary Storage Structures.
//!
//! Provides the primary columnar Hyper-Tile structures for storing plain key-value records,
//! documents, and JSON objects in `TileType::DOCUMENT_STORE` (Tile 23).
//!
//! Strictly adheres to the S3A schema evolution contract (Pod/Zeroable, 8-byte aligned, FLAG_ACTIVE = 0).

use bytemuck::{Pod, Zeroable};
use xxhash_rust::xxh3::Xxh3;

// Document & Key-Value Record Flags (Preserving FLAG_ACTIVE = 0)
pub const DOC_FLAG_ACTIVE: u32 = 0;
pub const DOC_FLAG_DELETED: u32 = 1 << 0;
pub const DOC_FLAG_IS_JSON: u32 = 1 << 1;

// Core Document Kinds (Range 16..31 / 0x10..0x1F)
// Reserved separate from LRS xAPI documents (which use kinds 1..3: State = 1, Agent Profile = 2, Activity Profile = 3).
pub const DOC_KIND_GENERIC_KV: u32 = 16;
pub const DOC_KIND_JSON: u32 = 17;
pub const DOC_KIND_BINARY: u32 = 18;

/// Computes the canonical 64-bit XXH3 compound key hash matching S3A engine document indexing.
/// Key is hashed as `"{doc_kind}\0{key}"`.
pub fn compute_key_hash(doc_kind: u32, key: &str) -> u64 {
    let mut hasher = Xxh3::new();
    let mut n = doc_kind;
    if n == 0 {
        hasher.update(b"0\0");
    } else {
        let mut digits = [0u8; 11];
        let mut idx = 11;
        while n > 0 {
            idx -= 1;
            digits[idx] = b'0' + (n % 10) as u8;
            n /= 10;
        }
        hasher.update(&digits[idx..]);
        hasher.update(b"\0");
    }
    hasher.update(key.as_bytes());
    hasher.digest()
}

/// EXACT 64-BYTE DOCUMENT / KEY-VALUE RECORD HEADER (8-byte aligned, Pod/Zeroable).
///
/// Precedes variable-length keys and content payloads in `TileType::DOCUMENT_STORE` (Tile 23).
/// Layout:
/// - 64-byte fixed header (`DocumentRecordHeader`)
/// - `key_len` bytes of UTF-8 key string
/// - `content_len` bytes of document content payload
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct DocumentRecordHeader {
    pub doc_kind: u32, // Document kind (16 = KV, 17 = JSON, 18 = BINARY, etc.) (offset 0..4)
    pub flags: u32,    // DOC_FLAG_ACTIVE, DOC_FLAG_DELETED, etc. (offset 4..8)
    pub key_hash: u64, // 64-bit hash of compound key for fast O(1) lookup (offset 8..16)
    pub etag_hash: [u8; 16], // 128-bit hash representing document ETag or version (offset 16..32)
    pub timestamp_ms: u64, // Last-Modified timestamp in epoch ms (offset 32..40)
    pub content_len: u32, // Byte length of document payload (offset 40..44)
    pub key_len: u32,  // Byte length of UTF-8 key string (offset 44..48)
    pub content_crc32: u32, // CRC32C of document content payload (offset 48..52)
    pub _reserved: [u32; 3], // Zero-padded alignment to 64 bytes (offset 52..64)
}

/// Type alias for Key-Value use cases.
pub type KvRecordHeader = DocumentRecordHeader;

impl Default for DocumentRecordHeader {
    fn default() -> Self {
        Self {
            doc_kind: DOC_KIND_GENERIC_KV,
            flags: DOC_FLAG_ACTIVE,
            key_hash: 0,
            etag_hash: [0u8; 16],
            timestamp_ms: 0,
            content_len: 0,
            key_len: 0,
            content_crc32: 0,
            _reserved: [0u32; 3],
        }
    }
}

impl DocumentRecordHeader {
    #[inline]
    pub fn new(doc_kind: u32, key: &str, content: &[u8], timestamp_ms: u64, is_json: bool) -> Self {
        let key_bytes = key.as_bytes();
        let mut flags = DOC_FLAG_ACTIVE;
        let mut kind = doc_kind;
        if is_json {
            flags |= DOC_FLAG_IS_JSON;
            if kind == DOC_KIND_GENERIC_KV {
                kind = DOC_KIND_JSON;
            }
        }

        Self {
            doc_kind: kind,
            flags,
            key_hash: compute_key_hash(kind, key),
            etag_hash: [0u8; 16],
            timestamp_ms,
            content_len: content.len() as u32,
            key_len: key_bytes.len() as u32,
            content_crc32: crc32c::crc32c(content),
            _reserved: [0u32; 3],
        }
    }

    #[inline]
    pub fn is_deleted(&self) -> bool {
        (self.flags & DOC_FLAG_DELETED) != 0
    }

    #[inline]
    pub fn is_json(&self) -> bool {
        (self.flags & DOC_FLAG_IS_JSON) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{align_of, size_of};
    use xxhash_rust::xxh3::xxh3_64;

    #[test]
    fn test_document_record_header_contract() {
        assert_eq!(size_of::<DocumentRecordHeader>(), 64);
        assert_eq!(align_of::<DocumentRecordHeader>(), 8);
        assert_eq!(DOC_FLAG_ACTIVE, 0);

        let key = "user:1001";
        let content = b"{\"name\":\"Alice\"}";
        let hdr = DocumentRecordHeader::new(DOC_KIND_GENERIC_KV, key, content, 1700000000000, true);
        assert_eq!(hdr.doc_kind, DOC_KIND_JSON);
        assert_eq!(hdr.key_len, key.len() as u32);
        assert_eq!(hdr.content_len, content.len() as u32);
        assert!(hdr.is_json());
        assert!(!hdr.is_deleted());
        assert_eq!(hdr.key_hash, compute_key_hash(DOC_KIND_JSON, key));
    }

    #[test]
    fn test_compute_key_hash_matches_xxh3_compound() {
        for kind in [0, 1, 16, 42, 9999] {
            let key = "test:compound:key:123";
            let expected_bytes = format!("{}\0{}", kind, key);
            let expected_hash = xxh3_64(expected_bytes.as_bytes());
            let actual_hash = compute_key_hash(kind, key);
            assert_eq!(
                actual_hash, expected_hash,
                "compute_key_hash must match xxh3_64 on '{}\\0{}'",
                kind, key
            );
        }
    }

    #[test]
    fn test_core_kinds_do_not_overlap_lrs() {
        assert!(DOC_KIND_GENERIC_KV > 3);
        assert!(DOC_KIND_JSON > 3);
        assert!(DOC_KIND_BINARY > 3);
    }
}
