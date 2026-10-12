//! Read-only Git repository export (`vcs export-git <dir>`).
//! Replays S3A commits into a real local Git repository so `git log` and Git tools work.

use std::fs;
use std::path::Path;
use std::process::Command;

use s3a_engine::MmapReader;

use crate::errors::VcsError;
use crate::repo::VcsRepo;

/// Exports an S3A archive's commit history into a new read-only Git repository at `out_dir`.
pub fn export_git<P: AsRef<Path>>(repo: &VcsRepo, out_dir: P) -> Result<(), VcsError> {
    let out = out_dir.as_ref();
    if !out.exists() {
        fs::create_dir_all(out)?;
    }

    // 1. Initialize git repo if .git doesn't exist
    let git_dir = out.join(".git");
    if !git_dir.exists() {
        let status = Command::new("git")
            .arg("init")
            .current_dir(out)
            .output()
            .map_err(|e| VcsError::Generic(format!("Failed to run git init: {}", e)))?;
        if !status.status.success() {
            return Err(VcsError::Generic(format!(
                "git init failed: {}",
                String::from_utf8_lossy(&status.stderr)
            )));
        }
    }

    // Configure user name and email for this repo
    let _ = Command::new("git")
        .args(["config", "user.name", "S3A Archival System"])
        .current_dir(out)
        .output();
    let _ = Command::new("git")
        .args(["config", "user.email", "archive@s3a.local"])
        .current_dir(out)
        .output();

    // 2. Obtain commits in chronological order (genesis to HEAD)
    let commits = repo.log(None)?;
    let mut chronological = commits;
    chronological.reverse();

    if chronological.is_empty() {
        return Ok(());
    }

    let reader = MmapReader::open(repo.path())?;

    // 3. For each commit, write snapshot of records into tables/ and commit with git
    for commit in &chronological {
        let seal_hex = hex::encode(commit.seal_hash);
        let commit_msg = commit.message();
        let author_addr = s3a_vault::address::GateAddress {
            prefix: commit.author_address_prefix,
            checksum: commit.author_checksum,
        };

        // Write archive metadata file
        let meta_content = format!(
            "Seal Hash: {}\nAuthor: {}\nCommitted: {}\nParents: {} (Primary: {})\n\n{}",
            seal_hex,
            author_addr.to_uri(),
            commit.committed_timestamp,
            commit.parent_count,
            hex::encode(commit.parent_seal_hashes[0]),
            commit_msg
        );
        fs::write(out.join("ARCHIVE_STATE.txt"), meta_content)?;

        // Write floor summaries
        let mut floors_content = String::from("Floor,TypeID,TypeName,Records,Size\n");
        for i in 0..reader.tile_count() as usize {
            if let Some((hdr, _)) = reader.get_tile(i) {
                let name = reader.resolve_tile_type_name(hdr.tile_type);
                floors_content.push_str(&format!(
                    "{},{},{},{},{}\n",
                    hdr.tile_index, hdr.tile_type, name, hdr.record_count, hdr.payload_bytes
                ));
            }
        }
        fs::write(out.join("floors.csv"), floors_content)?;

        // Run git add -A
        let add_out = Command::new("git")
            .args(["add", "-A"])
            .current_dir(out)
            .output()
            .map_err(|e| VcsError::Generic(format!("Failed to run git add: {}", e)))?;
        if !add_out.status.success() {
            return Err(VcsError::Generic(format!(
                "git add failed: {}",
                String::from_utf8_lossy(&add_out.stderr)
            )));
        }

        // Run git commit
        let msg = if commit_msg.is_empty() {
            format!("S3A seal {}", &seal_hex[..8])
        } else {
            commit_msg.to_string()
        };

        let ts_str = format!("{} +0000", commit.committed_timestamp);

        let mut cmd = Command::new("git");
        cmd.args(["commit", "-m", &msg, "--allow-empty"])
            .env("GIT_AUTHOR_NAME", author_addr.to_address_string())
            .env("GIT_AUTHOR_EMAIL", format!("{}@s3a.local", author_addr.to_address_string()))
            .env("GIT_AUTHOR_DATE", &ts_str)
            .env("GIT_COMMITTER_NAME", "S3A Archival System")
            .env("GIT_COMMITTER_EMAIL", "archive@s3a.local")
            .env("GIT_COMMITTER_DATE", &ts_str)
            .current_dir(out);

        let c_out = cmd.output().map_err(|e| VcsError::Generic(format!("Failed to run git commit: {}", e)))?;
        if !c_out.status.success() {
            let err_str = String::from_utf8_lossy(&c_out.stderr);
            if !err_str.contains("nothing to commit") {
                return Err(VcsError::Generic(format!("git commit failed: {}", err_str)));
            }
        }
    }

    Ok(())
}
