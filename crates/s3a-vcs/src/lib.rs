//! S3A Git-Style Version Control (S3A-VCS).
//!
//! Provides commit sealing, branches, tags, log, diff, merge, vault sync, and git export
//! for S3A archives.

pub mod errors;
pub mod frontier;
pub mod git_export;
pub mod records;
pub mod repo;
pub mod sync;

pub use errors::VcsError;
pub use frontier::{Frontier, LineageMode};
pub use git_export::export_git;
pub use records::{
    ConflictRecordDetail, VcsCommitRecord, VcsMergeResult, VcsRefRecord, VcsResolutionRecord,
    REF_KIND_BRANCH, REF_KIND_TAG, RESOLUTION_CHOICE_SOURCE, RESOLUTION_CHOICE_SYNTHESIS,
    RESOLUTION_CHOICE_TARGET, VCS_COMMIT_MESSAGE_MAX, VCS_RECORD_ID_MAX, VCS_REF_NAME_MAX,
};
pub use repo::{TileTypeDiff, VcsDiffReport, VcsRepo};
pub use sync::{sync_push, VcsSyncInventory};

pub fn version() -> &'static str {
    "0.1.0"
}
