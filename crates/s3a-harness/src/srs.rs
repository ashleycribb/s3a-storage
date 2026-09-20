use std::path::Path;
use std::io;
use s3a_core::{SrsManifestHeader, SrsWorkspaceRecord};
use s3a_engine::{S3ACrudEngine, SrsSnapshotBundle};

/// High-level manager for creating, packing, verifying, and mounting Scholar Research Snapshots (.snapshot.s3a).
pub struct SrsContainerManager;

impl SrsContainerManager {
    /// Packages a live S3A academic project database into an immutable SRS snapshot.
    pub fn package_project<P: AsRef<Path>, Q: AsRef<Path>>(
        live_db_path: P,
        out_snapshot_path: Q,
        project_uuid: [u64; 2],
        snapshot_uuid: [u64; 2],
        session_uuid: [u64; 2],
        workspace: Option<&SrsWorkspaceRecord>,
    ) -> io::Result<SrsSnapshotBundle> {
        let engine = S3ACrudEngine::open_or_create(live_db_path)?;
        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
        let manifest = SrsManifestHeader::new(
            project_uuid,
            snapshot_uuid,
            [0, 0],
            session_uuid,
            ts,
            0,
            0,
        );

        engine.export_srs_snapshot(out_snapshot_path, &manifest, workspace, Some(project_uuid))
    }

    /// Mounts an SRS snapshot file via zero-copy mmap with full CRC32C verification in < 1ms.
    pub fn mount_snapshot<P: AsRef<Path>>(snapshot_path: P) -> io::Result<SrsSnapshotBundle> {
        S3ACrudEngine::intake_srs_snapshot(snapshot_path)
    }
}
