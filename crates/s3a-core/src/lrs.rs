//! S3A Core LRS Binary Storage Structures and Records.
//!
//! Provides the core data structures for conformant xAPI 1.0.3 Learning Record Store (LRS)
//! indexing, statement body blob storage, document stores (State, Agent Profile, Activity Profile),
//! and attachment storage.
//!
//! Strictly adheres to the S3A schema evolution contract (Pod/Zeroable, 8-byte aligned, FLAG_ACTIVE = 0).

use bytemuck::{Pod, Zeroable};

// Core LRS Tile Types
pub const TILE_STATEMENT_BODY: u32 = 21;
pub const TILE_STATEMENT_INDEX: u32 = 22;
pub const TILE_DOCUMENT_STORE: u32 = 23;
pub const TILE_ATTACHMENT_BLOB: u32 = 24;

// Core ELR Projection Tile Types (IEEE P2997 Subset)
pub const TILE_ELR_PERSON: u32 = 25;
pub const TILE_ELR_ORGANIZATION: u32 = 26;
pub const TILE_ELR_LEARNING_RESOURCE: u32 = 27;
pub const TILE_ELR_LEARNING_EVENT: u32 = 28;
pub const TILE_ELR_COMPETENCY_ASSERTION: u32 = 29;
pub const TILE_ELR_CREDENTIAL_ASSERTION: u32 = 30;
pub const TILE_ELR_GOAL: u32 = 31;

// ELR Record Flags (Preserving FLAG_ACTIVE = 0)
pub const ELR_FLAG_ACTIVE: u32 = 0;
pub const ELR_FLAG_TOMBSTONE: u32 = 1 << 0;

// Learning Status (ELRR rules.md & schema.md)
pub const ELR_STATUS_ATTEMPTED: u32 = 1;
pub const ELR_STATUS_COMPLETED: u32 = 2;
pub const ELR_STATUS_PASSED: u32 = 3;
pub const ELR_STATUS_FAILED: u32 = 4;
pub const ELR_STATUS_SATISFIED: u32 = 5;
pub const ELR_STATUS_REGISTERED: u32 = 6;
pub const ELR_STATUS_SCHEDULED: u32 = 7;

// Goal Types & Statuses
pub const ELR_GOAL_TYPE_SELF: u32 = 1;
pub const ELR_GOAL_TYPE_ASSIGNED: u32 = 2;
pub const ELR_GOAL_TYPE_RECOMMENDED: u32 = 3;

pub const ELR_GOAL_STATUS_ACTIVE: u32 = 0;
pub const ELR_GOAL_STATUS_REMOVED: u32 = 1;
pub const ELR_GOAL_STATUS_COMPLETED: u32 = 2;

// Statement Index Record Flags (Preserving FLAG_ACTIVE = 0)
pub const STATEMENT_FLAG_ACTIVE: u32 = 0;
pub const STATEMENT_FLAG_VOIDED: u32 = 1 << 0;
pub const STATEMENT_FLAG_VOIDING: u32 = 1 << 1;
pub const STATEMENT_FLAG_ATTACHMENTS: u32 = 1 << 2;

// Document Kinds
pub const DOC_KIND_STATE: u32 = 1;
pub const DOC_KIND_AGENT_PROFILE: u32 = 2;
pub const DOC_KIND_ACTIVITY_PROFILE: u32 = 3;

// Document Flags
pub const DOC_FLAG_ACTIVE: u32 = 0;
pub const DOC_FLAG_DELETED: u32 = 1 << 0;
pub const DOC_FLAG_IS_JSON: u32 = 1 << 1;

/// EXACT 64-BYTE STATEMENT INDEX RECORD (8-byte aligned, Pod/Zeroable).
///
/// Encapsulates fast O(1) multi-attribute indexing for statement ID (UUID),
/// timestamps (stored and statement), actor, verb, activity, registration,
/// and direct pointer to the canonical statement body in `TileType::STATEMENT_BODY`.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct StatementIndexRecord {
    pub statement_id: [u64; 2],   // 128-bit statement UUID (offset 0..16)
    pub stored_timestamp_ms: u64, // Epoch ms when statement was stored (offset 16..24)
    pub timestamp_ms: u64,        // Statement timestamp in epoch ms (offset 24..32)
    pub actor_hash: u64,          // 64-bit hash of actor IFI (offset 32..40)
    pub verb_hash: u32,           // 32-bit hash or verb_id of verb IRI (offset 40..44)
    pub activity_hash: u32,       // 32-bit hash of object activity ID (offset 44..48)
    pub registration_hash: u32,   // 32-bit hash of registration UUID (0 if none) (offset 48..52)
    pub body_tile_id: u32,        // HyperTile ID containing the body blob (offset 52..56)
    pub body_offset: u32, // Byte offset in tile payload where StatementBlobHeader begins (offset 56..60)
    pub flags: u32,       // Active, voided, voiding, attachments, etc. (offset 60..64)
}

impl Default for StatementIndexRecord {
    fn default() -> Self {
        Self {
            statement_id: [0, 0],
            stored_timestamp_ms: 0,
            timestamp_ms: 0,
            actor_hash: 0,
            verb_hash: 0,
            activity_hash: 0,
            registration_hash: 0,
            body_tile_id: 0,
            body_offset: 0,
            flags: STATEMENT_FLAG_ACTIVE,
        }
    }
}

impl StatementIndexRecord {
    pub fn new(
        statement_id: [u64; 2],
        stored_timestamp_ms: u64,
        timestamp_ms: u64,
        actor_hash: u64,
        verb_hash: u32,
        activity_hash: u32,
        registration_hash: u32,
        body_tile_id: u32,
        body_offset: u32,
        flags: u32,
    ) -> Self {
        Self {
            statement_id,
            stored_timestamp_ms,
            timestamp_ms,
            actor_hash,
            verb_hash,
            activity_hash,
            registration_hash,
            body_tile_id,
            body_offset,
            flags,
        }
    }

    pub fn is_voided(&self) -> bool {
        (self.flags & STATEMENT_FLAG_VOIDED) != 0
    }

    pub fn is_voiding(&self) -> bool {
        (self.flags & STATEMENT_FLAG_VOIDING) != 0
    }

    pub fn has_attachments(&self) -> bool {
        (self.flags & STATEMENT_FLAG_ATTACHMENTS) != 0
    }

    pub fn mark_voided(&mut self) {
        self.flags |= STATEMENT_FLAG_VOIDED;
    }

    pub fn mark_voiding(&mut self) {
        self.flags |= STATEMENT_FLAG_VOIDING;
    }
}

/// EXACT 64-BYTE STATEMENT BLOB HEADER (8-byte aligned, Pod/Zeroable).
///
/// Precedes statement JSON data within a `TileType::STATEMENT_BODY` (Tile 21) hyper-tile.
/// Supports both compact single-chunk statement JSON and multi-chunk statement payloads.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct StatementBlobHeader {
    pub body_hash_xxh3: [u8; 16], // 128-bit hash (xxh3_128 of canonical statement JSON) (offset 0..16)
    pub sha256_hash: [u8; 32], // 256-bit SHA-256 hash of canonical statement JSON (offset 16..48)
    pub total_bytes: u32, // Total uncompressed byte length of this statement JSON (offset 48..52)
    pub chunk_index: u16, // 0 for single chunk, or chunk index if split (offset 52..54)
    pub total_chunks: u16, // 1 for single chunk, or total chunks (offset 54..56)
    pub chunk_bytes: u32, // Byte length of data payload immediately following this header (offset 56..60)
    pub chunk_crc32: u32, // CRC32C checksum of data payload following this header (offset 60..64)
}

impl Default for StatementBlobHeader {
    fn default() -> Self {
        Self {
            body_hash_xxh3: [0u8; 16],
            sha256_hash: [0u8; 32],
            total_bytes: 0,
            chunk_index: 0,
            total_chunks: 1,
            chunk_bytes: 0,
            chunk_crc32: 0,
        }
    }
}

impl StatementBlobHeader {
    pub fn new(
        body_hash_xxh3: [u8; 16],
        sha256_hash: [u8; 32],
        total_bytes: u32,
        chunk_index: u16,
        total_chunks: u16,
        chunk_bytes: u32,
        chunk_crc32: u32,
    ) -> Self {
        Self {
            body_hash_xxh3,
            sha256_hash,
            total_bytes,
            chunk_index,
            total_chunks,
            chunk_bytes,
            chunk_crc32,
        }
    }
}

/// EXACT 64-BYTE DOCUMENT RECORD HEADER (8-byte aligned, Pod/Zeroable).
///
/// Precedes State, Agent Profile, and Activity Profile documents in `TileType::DOCUMENT_STORE` (Tile 23).
/// The variable key string and content payload follow immediately after this header.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct DocumentRecordHeader {
    pub doc_kind: u32, // DOC_KIND_STATE, DOC_KIND_AGENT_PROFILE, etc. (offset 0..4)
    pub flags: u32,    // DOC_FLAG_ACTIVE, DOC_FLAG_DELETED (offset 4..8)
    pub key_hash: u64, // 64-bit hash of compound key for fast O(1) lookup (offset 8..16)
    pub etag_hash: [u8; 16], // 128-bit hash representing document ETag (offset 16..32)
    pub timestamp_ms: u64, // Last-Modified timestamp in epoch ms (offset 32..40)
    pub content_len: u32, // Byte length of document payload (offset 40..44)
    pub key_len: u32,  // Byte length of UTF-8 key string (offset 44..48)
    pub content_crc32: u32, // CRC32C of document content payload (offset 48..52)
    pub _reserved: [u32; 3], // Zero-padded alignment to 64 bytes (offset 52..64)
}

impl Default for DocumentRecordHeader {
    fn default() -> Self {
        Self {
            doc_kind: 0,
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
    pub fn is_deleted(&self) -> bool {
        (self.flags & DOC_FLAG_DELETED) != 0
    }
}

/// EXACT 64-BYTE ATTACHMENT BLOB HEADER (8-byte aligned, Pod/Zeroable).
///
/// Precedes binary attachment blobs in `TileType::ATTACHMENT_BLOB` (Tile 24),
/// addressed by SHA-256 hash.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct AttachmentBlobHeader {
    pub sha2: [u8; 32], // 256-bit SHA-256 hash matching statement attachment sha2 (offset 0..32)
    pub content_type_hash: u32, // 32-bit hash of MIME type (offset 32..36)
    pub chunk_index: u16, // Chunk offset if split across tiles (offset 36..38)
    pub total_chunks: u16, // Total chunks (offset 38..40)
    pub total_bytes: u32, // Full attachment size in bytes (offset 40..44)
    pub chunk_bytes: u32, // Byte length of this chunk payload (offset 44..48)
    pub chunk_crc32: u32, // CRC32C checksum of this chunk (offset 48..52)
    pub _reserved: [u32; 3], // Zero-padded alignment to 64 bytes (offset 52..64)
}

impl Default for AttachmentBlobHeader {
    fn default() -> Self {
        Self {
            sha2: [0u8; 32],
            content_type_hash: 0,
            chunk_index: 0,
            total_chunks: 1,
            total_bytes: 0,
            chunk_bytes: 0,
            chunk_crc32: 0,
            _reserved: [0u32; 3],
        }
    }
}

/// EXACT 96-BYTE ELR RECORD HEADER (8-byte aligned, Pod/Zeroable).
///
/// Precedes projected learner records across Tiles 25..=31:
/// Person (25), Organization (26), LearningResource (27),
/// LearningEvent (28), CompetencyAssertion (29), CredentialAssertion (30), Goal (31).
///
/// Followed immediately by `payload_len` bytes of UTF-8 JSON containing entity attributes.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq)]
pub struct ElrRecordHeader {
    pub entity_id: [u64; 2],           // 128-bit entity UUID (offset 0..16)
    pub source_statement_id: [u64; 2], // 128-bit source statement UUID (provenance) (offset 16..32)
    pub related_id: [u64; 2], // 128-bit related entity UUID (person_id, resource_id, etc.) (offset 32..48)
    pub timestamp_ms: u64,    // Timestamp in epoch ms (offset 48..56)
    pub flags: u32,           // ELR_FLAG_ACTIVE = 0, ELR_FLAG_TOMBSTONE = 1 << 0 (offset 56..60)
    pub status: u32,          // ELR_STATUS_* or ELR_GOAL_STATUS_* (offset 60..64)
    pub numeric_value: f32,   // score_scaled / confidence (offset 64..68)
    pub payload_len: u32,     // byte length of following UTF-8 JSON payload (offset 68..72)
    pub payload_crc32: u32,   // hardware CRC32C of payload (offset 72..76)
    pub sub_type: u32,        // ELR_GOAL_TYPE_* or qualification_type (offset 76..80)
    pub _reserved: [u64; 2],  // padding to 96 bytes (offset 80..96)
}

impl Default for ElrRecordHeader {
    fn default() -> Self {
        Self {
            entity_id: [0, 0],
            source_statement_id: [0, 0],
            related_id: [0, 0],
            timestamp_ms: 0,
            flags: ELR_FLAG_ACTIVE,
            status: 0,
            numeric_value: 0.0,
            payload_len: 0,
            payload_crc32: 0,
            sub_type: 0,
            _reserved: [0, 0],
        }
    }
}

impl ElrRecordHeader {
    pub fn is_tombstone(&self) -> bool {
        (self.flags & ELR_FLAG_TOMBSTONE) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::size_of;

    #[test]
    fn test_lrs_record_sizes_and_alignments() {
        assert_eq!(size_of::<StatementIndexRecord>(), 64);
        assert_eq!(core::mem::align_of::<StatementIndexRecord>(), 8);

        assert_eq!(size_of::<StatementBlobHeader>(), 64);
        assert_eq!(core::mem::align_of::<StatementBlobHeader>(), 8);

        assert_eq!(size_of::<DocumentRecordHeader>(), 64);
        assert_eq!(core::mem::align_of::<DocumentRecordHeader>(), 8);

        assert_eq!(size_of::<AttachmentBlobHeader>(), 64);
        assert_eq!(core::mem::align_of::<AttachmentBlobHeader>(), 8);

        assert_eq!(size_of::<ElrRecordHeader>(), 96);
        assert_eq!(core::mem::align_of::<ElrRecordHeader>(), 8);
    }
}
