//! VCS Errors and failure modes.

use std::fmt;

#[derive(Debug)]
pub enum VcsError {
    Io(std::io::Error),
    Core(s3a_core::S3AError),
    Vault(s3a_vault::VaultError),
    InvalidSignature(String),
    RefNotFound(String),
    TagImmutable(String),
    CommitNotFound(String),
    NonFastForward {
        branch: String,
        target_seal: String,
        remote_seal: String,
    },
    MergeConflict {
        reason: String,
        conflicting_record_ids: Vec<String>,
        requires_human_judgment: bool,
    },
    ConflictNeedsHuman {
        conflicts: Vec<crate::records::ConflictRecordDetail>,
        message: String,
    },
    ConsentRestricted(String),
    Generic(String),
}

impl fmt::Display for VcsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VcsError::Io(e) => write!(f, "VCS IO error: {}", e),
            VcsError::Core(e) => write!(f, "VCS core error: {}", e),
            VcsError::Vault(e) => write!(f, "VCS vault error: {}", e),
            VcsError::InvalidSignature(msg) => write!(f, "Invalid cryptographic signature: {}", msg),
            VcsError::RefNotFound(r) => write!(f, "Reference not found: {}", r),
            VcsError::TagImmutable(t) => write!(f, "Tag '{}' is immutable and cannot be updated", t),
            VcsError::CommitNotFound(c) => write!(f, "Commit not found: {}", c),
            VcsError::NonFastForward { branch, target_seal, remote_seal } => write!(
                f,
                "Non-fast-forward push to '{}' rejected (local {}, remote {}). Pass --merge to merge.",
                branch, target_seal, remote_seal
            ),
            VcsError::MergeConflict { reason, conflicting_record_ids, requires_human_judgment } => write!(
                f,
                "Merge conflict (human_judgment={}): {}. Conflicting IDs: {:?}",
                requires_human_judgment, reason, conflicting_record_ids
            ),
            VcsError::ConflictNeedsHuman { conflicts, message } => write!(
                f,
                "Merge conflict requires human judgment: {}. Conflicts: {:?}",
                message, conflicts
            ),
            VcsError::ConsentRestricted(msg) => write!(f, "Action blocked by participant consent policy: {}", msg),
            VcsError::Generic(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for VcsError {}

impl From<std::io::Error> for VcsError {
    fn from(e: std::io::Error) -> Self {
        VcsError::Io(e)
    }
}

impl From<s3a_core::S3AError> for VcsError {
    fn from(e: s3a_core::S3AError) -> Self {
        VcsError::Core(e)
    }
}

impl From<s3a_vault::VaultError> for VcsError {
    fn from(e: s3a_vault::VaultError) -> Self {
        VcsError::Vault(e)
    }
}
