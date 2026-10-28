//! Read-only display layouts for the scholar pack's snapshot tiles (12, 14, 15).
//! Metadata only; see [`s3a_core::display_layout`]. 128-bit UUIDs are stored as two `u64` in
//! memory, so they are shown as raw bytes rather than guessed into canonical text.

use s3a_core::display_layout;
use s3a_core::display_layout::{ft, DisplayLayoutRegistry};
use s3a_core::schema::SchemaTypeDefinition;

use crate::pack::ScholarTileType;
use crate::records::{AcademicPaperRecord, SrsManifestHeader, SrsWorkspaceRecord};

/// Layouts for tile 12 (academic papers), 14 (workspace) and 15 (snapshot manifest).
pub fn display_layouts() -> Vec<SchemaTypeDefinition> {
    vec![
        display_layout!(
            AcademicPaperRecord,
            ScholarTileType::ACADEMIC_PAPERS,
            1,
            "academic_paper",
            "scholar",
            [
                (paper_id, ft::U64),
                (timestamp_sec, ft::U64),
                (hilbert_index, ft::U64),
                (topic_x, ft::F32),
                (topic_y, ft::F32),
                (topic_z, ft::F32),
                (citation_count, ft::U32),
                (year, ft::U16),
                (venue_id, ft::U16),
                (open_access_flag, ft::U16),
                (warning_count, ft::U16),
                (doi_prefix_hash, ft::U64),
                (flags, ft::U64),
            ]
        ),
        display_layout!(
            SrsWorkspaceRecord,
            ScholarTileType::SRS_WORKSPACE,
            1,
            "srs_workspace",
            "scholar",
            [
                (project_uuid, ft::BYTES),
                (question_hash, ft::U64),
                (collection_mask, ft::U64),
                (topic_centroid_xyz, ft::BYTES),
                (status, ft::U32),
                (pinned_works_count, ft::U32),
                (claims_count, ft::U32),
                (flags, ft::U64),
            ]
        ),
        display_layout!(
            SrsManifestHeader,
            ScholarTileType::SRS_MANIFEST,
            1,
            "srs_manifest",
            "scholar",
            [
                (magic, ft::STRING),
                (schema_version, ft::U32),
                (archive_flags, ft::U32),
                (project_uuid, ft::BYTES),
                (snapshot_uuid, ft::BYTES),
                (parent_snapshot_uuid, ft::BYTES),
                (origin_session_uuid, ft::BYTES),
                (created_at_sec, ft::U64),
                (tile_count, ft::U32),
                (crc32c_checksum, ft::U32),
                (total_records, ft::U64),
            ]
        ),
    ]
}

/// A validated registry of [`display_layouts`].
pub fn display_registry() -> DisplayLayoutRegistry {
    DisplayLayoutRegistry::from_layouts(display_layouts())
        .expect("built-in scholar display layouts are valid")
}
