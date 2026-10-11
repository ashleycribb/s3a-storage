use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum SandboxError {
    PathTraversalDetected(String),
    OutsideRoot(String),
    ReadOnlyMode,
    Io(std::io::Error),
}

impl std::error::Error for SandboxError {}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SandboxError::PathTraversalDetected(p) => {
                write!(f, "Sandbox violation: path traversal detected in '{}'", p)
            }
            SandboxError::OutsideRoot(p) => {
                write!(f, "Sandbox violation: path '{}' is outside sandbox root", p)
            }
            SandboxError::ReadOnlyMode => write!(
                f,
                "Operation rejected: harness is running in --read-only mode"
            ),
            SandboxError::Io(e) => write!(f, "I/O error: {}", e),
        }
    }
}

impl From<std::io::Error> for SandboxError {
    fn from(e: std::io::Error) -> Self {
        SandboxError::Io(e)
    }
}

#[derive(Debug, Clone)]
pub struct Sandbox {
    root_dir: PathBuf,
    exports_dir: PathBuf,
    default_db_path: PathBuf,
    read_only: bool,
}

impl Sandbox {
    pub fn new<P: AsRef<Path>>(root: P, read_only: bool) -> Result<Self, SandboxError> {
        let root_path = root.as_ref();
        if !root_path.exists() {
            std::fs::create_dir_all(root_path)?;
        }
        let canonical_root = root_path.canonicalize()?;
        let exports_dir = canonical_root.join("exports");
        if !exports_dir.exists() {
            std::fs::create_dir_all(&exports_dir)?;
        }
        let canonical_exports = exports_dir.canonicalize()?;
        let default_db_path = canonical_root.join("scholar_lab.s3a");

        Ok(Self {
            root_dir: canonical_root,
            exports_dir: canonical_exports,
            default_db_path,
            read_only,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root_dir
    }

    pub fn exports_dir(&self) -> &Path {
        &self.exports_dir
    }

    pub fn default_db(&self) -> &Path {
        &self.default_db_path
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Validates and resolves an export snapshot file name.
    /// Strictly guarantees that all write targets reside INSIDE <root>/exports/
    /// Rejects any path traversal (".."), directory separators, drive letters, or absolute paths.
    pub fn resolve_export_snapshot_path(
        &self,
        filename: Option<&str>,
    ) -> Result<PathBuf, SandboxError> {
        if self.read_only {
            return Err(SandboxError::ReadOnlyMode);
        }

        let fname_owned;
        let fname = match filename {
            Some(f) if !f.trim().is_empty() => {
                let s = f.trim();
                // Strict rejection of path traversal, absolute paths, and directory separators
                if s.contains("..")
                    || s.contains('/')
                    || s.contains('\\')
                    || s.contains(':')
                    || s.contains('\0')
                {
                    return Err(SandboxError::PathTraversalDetected(s.to_string()));
                }
                s
            }
            _ => {
                fname_owned = format!("snapshot_{}.s3a", crate::generate_uuid_v7());
                &fname_owned
            }
        };

        let target = self.exports_dir.join(fname);
        if target.parent() != Some(&self.exports_dir) {
            return Err(SandboxError::PathTraversalDetected(fname.to_string()));
        }

        if target.exists() {
            let canonical = target.canonicalize()?;
            if !canonical.starts_with(&self.exports_dir) {
                return Err(SandboxError::OutsideRoot(fname.to_string()));
            }
        }

        Ok(target)
    }

    /// Generates a unique, sandbox-confined export snapshot path using a real UUIDv7.
    pub fn create_export_snapshot_path(&self) -> Result<PathBuf, SandboxError> {
        self.resolve_export_snapshot_path(None)
    }

    /// Validates and resolves a snapshot path for intake.
    /// Strictly confined to snapshots located in <root>/exports/ or <root>.
    pub fn resolve_intake_snapshot_path(&self, path_str: &str) -> Result<PathBuf, SandboxError> {
        let trimmed = path_str.trim();
        if trimmed.contains("..")
            || trimmed.contains('/')
            || trimmed.contains('\\')
            || trimmed.contains(':')
            || trimmed.contains('\0')
        {
            return Err(SandboxError::PathTraversalDetected(trimmed.to_string()));
        }

        let in_exports = self.exports_dir.join(trimmed);
        let candidate = if in_exports.exists() {
            in_exports
        } else {
            self.root_dir.join(trimmed)
        };

        if !candidate.exists() {
            return Err(SandboxError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Snapshot file not found: '{}'", trimmed),
            )));
        }

        let canonical = candidate.canonicalize()?;
        if !canonical.starts_with(&self.root_dir) {
            return Err(SandboxError::OutsideRoot(trimmed.to_string()));
        }

        Ok(canonical)
    }

    /// Verifies if write operations are permitted.
    pub fn check_write_permitted(&self) -> Result<(), SandboxError> {
        if self.read_only {
            Err(SandboxError::ReadOnlyMode)
        } else {
            Ok(())
        }
    }
}
