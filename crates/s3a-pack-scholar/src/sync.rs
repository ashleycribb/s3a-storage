use bytemuck::{bytes_of, cast_slice, Pod, Zeroable};
use s3a_core::{MarkRecord, S3AError, TileType};

use crate::pack::ScholarTileType;
use crate::records::{
    AcademicPaperRecord, AiTraceRecord, CitationFingerprintRecord, HumanLrsRecord,
    ResearchGraphEdgeRecord, ScholarNoteClaimRecord, SrsWorkspaceRecord,
};
use crate::snapshot::SrsSnapshotBundle;

/// 64-byte Header for lightweight Scholar Research Snapshot sync deltas.
///
/// Enables sub-millisecond, sub-kilobyte incremental synchronization of agent and human
/// decisions without exporting or transmitting 768 KB+ full snapshot containers.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct SrsSyncDeltaHeader {
    pub magic: [u8; 8],                // b"S3ASYNC1" (offset 0..8)
    pub schema_version: u32,           // Version 1 (offset 8..12)
    pub flags: u32,                    // Sync flags (offset 12..16)
    pub session_uuid: [u64; 2],        // 128-bit active session UUID (offset 16..32)
    pub base_snapshot_uuid: [u64; 2],  // Parent snapshot UUID (offset 32..48)
    pub delta_seq: u64,                // Monotonic delta sequence counter (offset 48..56)
    pub record_count: u32,             // Number of entries in delta (offset 56..60)
    pub crc32c_checksum: u32,          // Hardware CRC32C over header and payload (offset 60..64)
    pub created_at_sec: u64,           // Timestamp in epoch seconds (offset 64..72)
    pub _reserved: [u64; 2],           // Reserved for future extensions (offset 72..88)
    pub _padding: u64,                 // Alignment padding (offset 88..96)
}

impl SrsSyncDeltaHeader {
    pub const MAGIC: [u8; 8] = *b"S3ASYNC1";
    pub const HEADER_SIZE: usize = std::mem::size_of::<Self>();

    pub fn new(
        session_uuid: [u64; 2],
        base_snapshot_uuid: [u64; 2],
        delta_seq: u64,
        record_count: u32,
    ) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        #[cfg(target_arch = "wasm32")]
        let now = 0u64;

        Self::with_timestamp(session_uuid, base_snapshot_uuid, delta_seq, record_count, now)
    }

    pub fn with_timestamp(
        session_uuid: [u64; 2],
        base_snapshot_uuid: [u64; 2],
        delta_seq: u64,
        record_count: u32,
        created_at_sec: u64,
    ) -> Self {
        Self {
            magic: Self::MAGIC,
            schema_version: 1,
            flags: 0,
            session_uuid,
            base_snapshot_uuid,
            delta_seq,
            record_count,
            crc32c_checksum: 0,
            created_at_sec,
            _reserved: [0u64; 2],
            _padding: 0,
        }
    }

    /// Computes hardware CRC32C of the header (excluding crc32c_checksum) combined with entry bytes.
    pub fn compute_checksum(&self, entry_bytes: &[u8]) -> u32 {
        let mut copy = *self;
        copy.crc32c_checksum = 0;
        let mut crc = crc32c::crc32c(bytes_of(&copy));
        crc = crc32c::crc32c_append(crc, entry_bytes);
        crc
    }
}

/// Tagged record entry within a sync delta packet (168 bytes total, supporting up to 160-byte records).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct SrsDeltaEntry {
    pub tile_type: u32,
    pub _padding: u32,
    pub record_words: [u64; 20],
}

impl SrsDeltaEntry {
    pub const ENTRY_SIZE: usize = std::mem::size_of::<Self>();

    pub fn from_record<T: Pod>(tile_type: u32, record: &T) -> Result<Self, S3AError> {
        let size = std::mem::size_of::<T>();
        if size > 160 {
            return Err(S3AError::BufferTooSmall {
                required: 160,
                provided: size,
            });
        }
        let mut entry = Self {
            tile_type,
            _padding: 0,
            record_words: [0u64; 20],
        };
        bytemuck::bytes_of_mut(&mut entry.record_words)[..size].copy_from_slice(bytes_of(record));
        Ok(entry)
    }

    pub fn record_bytes(&self) -> &[u8] {
        bytemuck::bytes_of(&self.record_words)
    }
}

/// Errors returned during delta sync validation or application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncError {
    BufferTooSmall { required: usize, provided: usize },
    InvalidMagic,
    UnsupportedVersion(u32),
    ChecksumMismatch { expected: u32, actual: u32 },
    InvalidPayloadSize,
    SessionMismatch,
    BaseSnapshotMismatch,
    SequenceGap { expected: u64, actual: u64 },
    ForbiddenTileInDelta(u32),
    UnknownTileType(u32),
    UnauthorizedMark { mark_kind: u32, action: u8 },
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncError::BufferTooSmall { required, provided } => {
                write!(f, "Delta buffer too small: required {}, provided {}", required, provided)
            }
            SyncError::InvalidMagic => write!(f, "Invalid sync delta magic: expected b\"S3ASYNC1\""),
            SyncError::UnsupportedVersion(v) => write!(f, "Unsupported sync delta version: {}", v),
            SyncError::ChecksumMismatch { expected, actual } => {
                write!(f, "Sync delta checksum mismatch: expected 0x{:08X}, computed 0x{:08X}", expected, actual)
            }
            SyncError::InvalidPayloadSize => write!(f, "Invalid sync delta payload size for record count"),
            SyncError::SessionMismatch => write!(f, "Sync delta session UUID does not match target session"),
            SyncError::BaseSnapshotMismatch => write!(f, "Sync delta base snapshot UUID does not match target bundle"),
            SyncError::SequenceGap { expected, actual } => {
                write!(f, "Sync delta sequence gap: expected {}, actual {}", expected, actual)
            }
            SyncError::ForbiddenTileInDelta(tt) => {
                write!(f, "Tile type {} is strictly forbidden in client sync deltas", tt)
            }
            SyncError::UnknownTileType(tt) => write!(f, "Unknown or unhandled tile type {} in sync delta", tt),
            SyncError::UnauthorizedMark { mark_kind, action } => {
                write!(f, "Unauthorized mark operation: mark_kind {}, action {}", mark_kind, action)
            }
        }
    }
}

impl std::error::Error for SyncError {}

/// High-performance sync delta packet containing a verified batch of mutated records.
#[derive(Debug, Clone, PartialEq)]
pub struct SrsSyncDeltaPacket {
    pub header: SrsSyncDeltaHeader,
    pub entries: Vec<SrsDeltaEntry>,
}

impl SrsSyncDeltaPacket {
    /// Creates a new empty sync delta packet.
    pub fn new(session_uuid: [u64; 2], base_snapshot_uuid: [u64; 2], delta_seq: u64) -> Self {
        Self {
            header: SrsSyncDeltaHeader::new(session_uuid, base_snapshot_uuid, delta_seq, 0),
            entries: Vec::new(),
        }
    }

    /// Creates a new empty sync delta packet with explicit creation timestamp.
    pub fn with_timestamp(
        session_uuid: [u64; 2],
        base_snapshot_uuid: [u64; 2],
        delta_seq: u64,
        created_at_sec: u64,
    ) -> Self {
        Self {
            header: SrsSyncDeltaHeader::with_timestamp(session_uuid, base_snapshot_uuid, delta_seq, 0, created_at_sec),
            entries: Vec::new(),
        }
    }

    /// Appends a 64-byte Pod record to the delta packet.
    pub fn push_record<T: Pod>(&mut self, tile_type: u32, record: &T) -> Result<(), S3AError> {
        let entry = SrsDeltaEntry::from_record(tile_type, record)?;
        self.entries.push(entry);
        self.header.record_count = self.entries.len() as u32;
        Ok(())
    }

    /// Exports the delta packet to a compact binary byte vector with hardware CRC32C.
    pub fn export_bytes(&self) -> Vec<u8> {
        let entry_slice: &[u8] = cast_slice(&self.entries);
        let mut header = self.header;
        header.record_count = self.entries.len() as u32;
        header.crc32c_checksum = header.compute_checksum(entry_slice);

        let total_size = SrsSyncDeltaHeader::HEADER_SIZE + entry_slice.len();
        let mut out = Vec::with_capacity(total_size);
        out.extend_from_slice(bytes_of(&header));
        out.extend_from_slice(entry_slice);
        out
    }

    /// Parses and verifies a sync delta packet from raw bytes.
    pub fn parse_bytes(bytes: &[u8]) -> Result<Self, SyncError> {
        if bytes.len() < SrsSyncDeltaHeader::HEADER_SIZE {
            return Err(SyncError::BufferTooSmall {
                required: SrsSyncDeltaHeader::HEADER_SIZE,
                provided: bytes.len(),
            });
        }

        let header: SrsSyncDeltaHeader = bytemuck::pod_read_unaligned(&bytes[..SrsSyncDeltaHeader::HEADER_SIZE]);
        if header.magic != SrsSyncDeltaHeader::MAGIC {
            return Err(SyncError::InvalidMagic);
        }
        if header.schema_version != 1 {
            return Err(SyncError::UnsupportedVersion(header.schema_version));
        }

        let expected_payload_len = (header.record_count as usize) * SrsDeltaEntry::ENTRY_SIZE;
        let expected_total_len = SrsSyncDeltaHeader::HEADER_SIZE + expected_payload_len;
        if bytes.len() != expected_total_len {
            return Err(SyncError::InvalidPayloadSize);
        }

        let payload_bytes = &bytes[SrsSyncDeltaHeader::HEADER_SIZE..];
        let computed_checksum = header.compute_checksum(payload_bytes);
        if header.crc32c_checksum != computed_checksum {
            return Err(SyncError::ChecksumMismatch {
                expected: header.crc32c_checksum,
                actual: computed_checksum,
            });
        }

        let entries: Vec<SrsDeltaEntry> = cast_slice(payload_bytes).to_vec();

        Ok(Self { header, entries })
    }

    /// Applies all records from this delta packet directly into an `SrsSnapshotBundle`.
    ///
    /// Fail-closed guarantees:
    /// 1. Verifies session UUID and base snapshot UUID match bundle.
    /// 2. Replayed deltas (delta_seq <= last_applied_delta_seq) are idempotent no-ops returning Ok(0).
    /// 3. Sequence gaps (delta_seq > last_applied_delta_seq + 1) are rejected with `SyncError::SequenceGap`.
    /// 4. AGENT_REGISTRY entries are strictly forbidden in deltas (`SyncError::ForbiddenTileInDelta`).
    /// 5. Unknown tile types are rejected (`SyncError::UnknownTileType`).
    /// 6. Mark mutations are restricted: only NEEDS_REVIEW and CONSENT_WITHDRAWN can be SET,
    ///    and clearing LEGAL_HOLD, QUARANTINE, or CONSENT_WITHDRAWN is rejected (`SyncError::UnauthorizedMark`).
    /// 7. Transactional: validates ALL entries first before mutating any bundle records.
    pub fn apply_to_bundle(&self, bundle: &mut SrsSnapshotBundle) -> Result<usize, SyncError> {
        if self.header.session_uuid != bundle.session_uuid {
            return Err(SyncError::SessionMismatch);
        }
        if self.header.base_snapshot_uuid != bundle.base_snapshot_uuid {
            return Err(SyncError::BaseSnapshotMismatch);
        }

        // Idempotency: replaying an already applied or past delta is a no-op returning Ok(0)
        if self.header.delta_seq <= bundle.last_applied_delta_seq {
            return Ok(0);
        }

        // Monotonic sequence verification: reject sequence gaps
        let expected_seq = bundle.last_applied_delta_seq + 1;
        if self.header.delta_seq > expected_seq {
            return Err(SyncError::SequenceGap {
                expected: expected_seq,
                actual: self.header.delta_seq,
            });
        }

        // Phase 1: All-or-nothing validation of all entries
        for entry in &self.entries {
            if entry.tile_type == ScholarTileType::AGENT_REGISTRY {
                return Err(SyncError::ForbiddenTileInDelta(entry.tile_type));
            }

            match entry.tile_type {
                ScholarTileType::HUMAN_LRS
                | ScholarTileType::AI_TRACE_LOG
                | ScholarTileType::ACADEMIC_PAPERS
                | ScholarTileType::RESEARCH_GRAPH
                | ScholarTileType::SRS_WORKSPACE
                | ScholarTileType::NOTE_CLAIMS => {
                    // Valid domain pack tile types
                }
                TileType::MARKS => {
                    let r: MarkRecord = bytemuck::pod_read_unaligned(&entry.record_bytes()[..64]);
                    if r.set_or_clear == s3a_core::MARK_SET {
                        if r.mark_kind != s3a_core::MARK_NEEDS_REVIEW
                            && r.mark_kind != s3a_core::MARK_CONSENT_WITHDRAWN
                        {
                            return Err(SyncError::UnauthorizedMark {
                                 mark_kind: r.mark_kind,
                                 action: r.set_or_clear,
                             });
                         }
                     } else if r.set_or_clear == s3a_core::MARK_CLEAR {
                         if r.mark_kind == s3a_core::MARK_LEGAL_HOLD
                             || r.mark_kind == s3a_core::MARK_QUARANTINE
                             || r.mark_kind == s3a_core::MARK_CONSENT_WITHDRAWN
                         {
                             return Err(SyncError::UnauthorizedMark {
                                 mark_kind: r.mark_kind,
                                 action: r.set_or_clear,
                             });
                         }
                     } else {
                         return Err(SyncError::UnauthorizedMark {
                             mark_kind: r.mark_kind,
                             action: r.set_or_clear,
                         });
                     }
                 }
                 other => {
                     return Err(SyncError::UnknownTileType(other));
                 }
             }
         }
 
         // Phase 2: Transactional application (only reached if all entries passed validation)
         let mut count = 0;
         for entry in &self.entries {
             match entry.tile_type {
                 ScholarTileType::HUMAN_LRS => {
                     let r: HumanLrsRecord = bytemuck::pod_read_unaligned(&entry.record_bytes()[..64]);
                     bundle.human_decisions.push(r);
                     count += 1;
                 }
                 ScholarTileType::AI_TRACE_LOG => {
                     let r: AiTraceRecord = bytemuck::pod_read_unaligned(&entry.record_bytes()[..64]);
                     bundle.ai_traces.push(r);
                     count += 1;
                 }
                 ScholarTileType::ACADEMIC_PAPERS => {
                     let r: AcademicPaperRecord = bytemuck::pod_read_unaligned(&entry.record_bytes()[..64]);
                     bundle.papers.push(r);
                     count += 1;
                 }
                 ScholarTileType::RESEARCH_GRAPH => {
                     let r: ResearchGraphEdgeRecord = bytemuck::pod_read_unaligned(&entry.record_bytes()[..64]);
                     bundle.evidence_edges.push(r);
                     count += 1;
                 }
                 ScholarTileType::SRS_WORKSPACE => {
                     let r: SrsWorkspaceRecord = bytemuck::pod_read_unaligned(&entry.record_bytes()[..64]);
                     bundle.workspace = Some(r);
                     count += 1;
                 }
                 ScholarTileType::NOTE_CLAIMS => {
                     let r: ScholarNoteClaimRecord = bytemuck::pod_read_unaligned(&entry.record_bytes()[..160]);
                     bundle.claims.push(r);
                     count += 1;
                 }
                 ScholarTileType::CITATION_FINGERPRINT => {
                     let _r: CitationFingerprintRecord = bytemuck::pod_read_unaligned(&entry.record_bytes()[..96]);
                     count += 1;
                 }
                 TileType::MARKS => {
                     let r: MarkRecord = bytemuck::pod_read_unaligned(&entry.record_bytes()[..64]);
                     bundle.policy_marks.push(r);
                     count += 1;
                 }
                 _ => unreachable!(),
             }
         }

        bundle.last_applied_delta_seq = self.header.delta_seq;
        bundle.manifest.total_records += count as u64;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::records::{AgentRegistryRecord, SrsManifestHeader};

    #[test]
    fn test_sync_delta_export_and_parse_roundtrip() {
        let session = [0x1111_2222_3333_4444, 0x5555_6666_7777_8888];
        let base = [0xAAAA_BBBB_CCCC_DDDD, 0xEEEE_FFFF_0000_1111];

        let mut delta = SrsSyncDeltaPacket::new(session, base, 1);

        let human = HumanLrsRecord::new(
            session,
            1600000000,
            0xFEEDFACE,
            2,
            0.95,
            s3a_core::S3ACoordinate::new(0, 11, 0),
            0x12345678,
        );
        delta.push_record(ScholarTileType::HUMAN_LRS, &human).unwrap();

        let ai = AiTraceRecord::new(
            session,
            1600000001,
            0xCAFECAFE,
            1,
            0.99,
            s3a_core::S3ACoordinate::new(0, 10, 0),
            42,
        );
        delta.push_record(ScholarTileType::AI_TRACE_LOG, &ai).unwrap();

        let bytes = delta.export_bytes();
        // 96 bytes header + 2 * 168 bytes entries = 432 bytes total (sub-kilobyte!)
        assert_eq!(bytes.len(), 432);

        let parsed = SrsSyncDeltaPacket::parse_bytes(&bytes).expect("parse delta bytes");
        assert_eq!(parsed.header.session_uuid, session);
        assert_eq!(parsed.header.delta_seq, 1);
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed.entries[0].tile_type, ScholarTileType::HUMAN_LRS);
        assert_eq!(parsed.entries[1].tile_type, ScholarTileType::AI_TRACE_LOG);

        let parsed_human: HumanLrsRecord = bytemuck::pod_read_unaligned(&parsed.entries[0].record_bytes()[..64]);
        assert_eq!(parsed_human.actor_hash, 0xFEEDFACE);
        assert_eq!(parsed_human.decision_score, 0.95);
    }

    #[test]
    fn test_sync_delta_corrupted_checksum_fails() {
        let session = [1, 2];
        let mut delta = SrsSyncDeltaPacket::new(session, [0, 0], 1);
        let human = HumanLrsRecord::default();
        delta.push_record(ScholarTileType::HUMAN_LRS, &human).unwrap();

        let mut bytes = delta.export_bytes();
        // Corrupt a byte in the payload
        let last_idx = bytes.len() - 1;
        bytes[last_idx] ^= 0xFF;

        let res = SrsSyncDeltaPacket::parse_bytes(&bytes);
        assert!(matches!(res, Err(SyncError::ChecksumMismatch { .. })));
    }

    fn create_test_bundle(session: [u64; 2], base: [u64; 2]) -> SrsSnapshotBundle {
        let manifest = SrsManifestHeader::new(session, base, [0, 0], session, 1600000000, 1, 0);
        SrsSnapshotBundle {
            manifest,
            session_uuid: session,
            base_snapshot_uuid: base,
            last_applied_delta_seq: 0,
            workspace: None,
            papers: Vec::new(),
            evidence_edges: Vec::new(),
            human_decisions: Vec::new(),
            ai_traces: Vec::new(),
            agents: Vec::new(),
            claims: Vec::new(),
            policy_marks: Vec::new(),
        }
    }

    #[test]
    fn test_apply_delta_rejects_agent_registry() {
        let session = [0x1111, 0x2222];
        let base = [0x3333, 0x4444];
        let mut bundle = create_test_bundle(session, base);

        let mut delta = SrsSyncDeltaPacket::new(session, base, 1);
        let agent = AgentRegistryRecord::default();
        delta.push_record(ScholarTileType::AGENT_REGISTRY, &agent).unwrap();

        let res = delta.apply_to_bundle(&mut bundle);
        assert_eq!(res, Err(SyncError::ForbiddenTileInDelta(ScholarTileType::AGENT_REGISTRY)));
        assert_eq!(bundle.agents.len(), 0);
    }

    #[test]
    fn test_apply_delta_rejects_unknown_tile() {
        let session = [0x1111, 0x2222];
        let base = [0x3333, 0x4444];
        let mut bundle = create_test_bundle(session, base);

        let mut delta = SrsSyncDeltaPacket::new(session, base, 1);
        let human = HumanLrsRecord::default();
        delta.push_record(99, &human).unwrap();

        let res = delta.apply_to_bundle(&mut bundle);
        assert_eq!(res, Err(SyncError::UnknownTileType(99)));
    }

    #[test]
    fn test_apply_delta_mark_restrictions() {
        use s3a_core::{
            MARK_CLEAR, MARK_CONSENT_WITHDRAWN, MARK_LEGAL_HOLD, MARK_NEEDS_REVIEW,
            MARK_QUARANTINE, MARK_SET,
        };
        let session = [0x1111, 0x2222];
        let base = [0x3333, 0x4444];

        // 1. Setting LEGAL_HOLD via delta must be rejected
        let mut bundle = create_test_bundle(session, base);
        let mut delta = SrsSyncDeltaPacket::new(session, base, 1);
        let m_illegal_set = MarkRecord::new(10, 1, 0, 1, MARK_LEGAL_HOLD, MARK_SET, 100, 1);
        delta.push_record(TileType::MARKS, &m_illegal_set).unwrap();
        assert_eq!(
            delta.apply_to_bundle(&mut bundle),
            Err(SyncError::UnauthorizedMark { mark_kind: MARK_LEGAL_HOLD, action: MARK_SET })
        );

        // 2. Clearing LEGAL_HOLD, QUARANTINE, or CONSENT_WITHDRAWN must be rejected
        for bad_clear in [MARK_LEGAL_HOLD, MARK_QUARANTINE, MARK_CONSENT_WITHDRAWN] {
            let mut delta_clear = SrsSyncDeltaPacket::new(session, base, 1);
            let m_clear = MarkRecord::new(10, 1, 0, 1, bad_clear, MARK_CLEAR, 100, 1);
            delta_clear.push_record(TileType::MARKS, &m_clear).unwrap();
            assert_eq!(
                delta_clear.apply_to_bundle(&mut bundle),
                Err(SyncError::UnauthorizedMark { mark_kind: bad_clear, action: MARK_CLEAR })
            );
        }

        // 3. Setting NEEDS_REVIEW or CONSENT_WITHDRAWN is permitted
        let mut delta_ok = SrsSyncDeltaPacket::new(session, base, 1);
        let m_review = MarkRecord::new(10, 1, 0, 1, MARK_NEEDS_REVIEW, MARK_SET, 100, 1);
        let m_consent = MarkRecord::new(10, 1, 1, 1, MARK_CONSENT_WITHDRAWN, MARK_SET, 101, 1);
        delta_ok.push_record(TileType::MARKS, &m_review).unwrap();
        delta_ok.push_record(TileType::MARKS, &m_consent).unwrap();

        assert_eq!(delta_ok.apply_to_bundle(&mut bundle), Ok(2));
        assert_eq!(bundle.policy_marks.len(), 2);
    }

    #[test]
    fn test_apply_delta_session_and_base_mismatches() {
        let session = [0x1111, 0x2222];
        let base = [0x3333, 0x4444];
        let mut bundle = create_test_bundle(session, base);

        // Session mismatch
        let delta_bad_session = SrsSyncDeltaPacket::new([0x9999, 0x8888], base, 1);
        assert_eq!(
            delta_bad_session.apply_to_bundle(&mut bundle),
            Err(SyncError::SessionMismatch)
        );

        // Base snapshot mismatch
        let delta_bad_base = SrsSyncDeltaPacket::new(session, [0x9999, 0x8888], 1);
        assert_eq!(
            delta_bad_base.apply_to_bundle(&mut bundle),
            Err(SyncError::BaseSnapshotMismatch)
        );
    }

    #[test]
    fn test_apply_delta_idempotency_and_sequence_gaps() {
        let session = [0x1111, 0x2222];
        let base = [0x3333, 0x4444];
        let mut bundle = create_test_bundle(session, base);

        // 1. Sequence gap: delta_seq = 2 when expecting 1
        let mut delta_gap = SrsSyncDeltaPacket::new(session, base, 2);
        let human = HumanLrsRecord::default();
        delta_gap.push_record(ScholarTileType::HUMAN_LRS, &human).unwrap();
        assert_eq!(
            delta_gap.apply_to_bundle(&mut bundle),
            Err(SyncError::SequenceGap { expected: 1, actual: 2 })
        );
        assert_eq!(bundle.human_decisions.len(), 0);

        // 2. Apply delta 1 successfully
        let mut delta_1 = SrsSyncDeltaPacket::new(session, base, 1);
        delta_1.push_record(ScholarTileType::HUMAN_LRS, &human).unwrap();
        assert_eq!(delta_1.apply_to_bundle(&mut bundle), Ok(1));
        assert_eq!(bundle.human_decisions.len(), 1);
        assert_eq!(bundle.last_applied_delta_seq, 1);

        // 3. Replay delta 1: must be idempotent no-op returning Ok(0)
        assert_eq!(delta_1.apply_to_bundle(&mut bundle), Ok(0));
        assert_eq!(bundle.human_decisions.len(), 1); // No duplicates!
        assert_eq!(bundle.last_applied_delta_seq, 1);

        // 4. Apply delta 2 successfully
        let mut delta_2 = SrsSyncDeltaPacket::new(session, base, 2);
        let ai = AiTraceRecord::default();
        delta_2.push_record(ScholarTileType::AI_TRACE_LOG, &ai).unwrap();
        assert_eq!(delta_2.apply_to_bundle(&mut bundle), Ok(1));
        assert_eq!(bundle.ai_traces.len(), 1);
        assert_eq!(bundle.last_applied_delta_seq, 2);

        // 5. Replay old delta 1 or 2: still idempotent no-op
        assert_eq!(delta_1.apply_to_bundle(&mut bundle), Ok(0));
        assert_eq!(delta_2.apply_to_bundle(&mut bundle), Ok(0));
        assert_eq!(bundle.human_decisions.len(), 1);
        assert_eq!(bundle.ai_traces.len(), 1);
    }

    #[test]
    fn test_apply_delta_all_or_nothing_transactional_safety() {
        let session = [0x1111, 0x2222];
        let base = [0x3333, 0x4444];
        let mut bundle = create_test_bundle(session, base);

        let mut delta = SrsSyncDeltaPacket::new(session, base, 1);
        // Entry 1: Valid human decision
        let human = HumanLrsRecord::default();
        delta.push_record(ScholarTileType::HUMAN_LRS, &human).unwrap();
        // Entry 2: Forbidden agent registry
        let agent = AgentRegistryRecord::default();
        delta.push_record(ScholarTileType::AGENT_REGISTRY, &agent).unwrap();

        // Must fail closed on entry 2 and NOT apply entry 1
        let res = delta.apply_to_bundle(&mut bundle);
        assert_eq!(res, Err(SyncError::ForbiddenTileInDelta(ScholarTileType::AGENT_REGISTRY)));
        assert_eq!(bundle.human_decisions.len(), 0);
        assert_eq!(bundle.last_applied_delta_seq, 0);
    }
}
