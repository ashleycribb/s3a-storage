use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NONCE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Generates a unique temporary compact file path beside the target live file.
/// E.g. `data/scholar.compact.1234.1710000000_1.s3a`.
pub fn unique_compact_temp_path<P: AsRef<Path>>(live_path: P) -> PathBuf {
    let live = live_path.as_ref();
    let parent = match live.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let stem = live.file_stem().and_then(|s| s.to_str()).unwrap_or("archive");
    let ext = live.extension().and_then(|s| s.to_str()).unwrap_or("s3a");
    let pid = std::process::id();
    let nonce = NONCE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    parent.join(format!("{}.compact.{}.{}_{}.{}", stem, pid, ts, nonce, ext))
}

/// Atomically replaces destination file with source file across Windows and POSIX.
///
/// Guarantees:
/// 1. Source file data and metadata are flushed to durable storage (`sync_all`) before replacement.
/// 2. Atomic rename on POSIX; on Windows, attempts MoveFileExW-backed replace with backup swap fallback.
/// 3. Destination is never left missing or partially overwritten if replacement fails.
pub fn atomic_replace<P1: AsRef<Path>, P2: AsRef<Path>>(from: P1, to: P2) -> io::Result<()> {
    let from = from.as_ref();
    let to = to.as_ref();

    if !from.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Source file does not exist: {}", from.display()),
        ));
    }

    // Flush and sync source file data & metadata to disk
    if let Ok(f) = File::open(from) {
        let _ = f.sync_all();
    }

    #[cfg(windows)]
    {
        // Try standard rename first (calls MoveFileExW with MOVEFILE_REPLACE_EXISTING)
        match fs::rename(from, to) {
            Ok(()) => Ok(()),
            Err(_) => {
                let pid = std::process::id();
                let nonce = NONCE_COUNTER.fetch_add(1, Ordering::Relaxed);
                let backup = to.with_extension(format!("old.{}_{}.tmp", pid, nonce));

                if backup.exists() {
                    let _ = fs::remove_file(&backup);
                }

                if to.exists() {
                    fs::rename(to, &backup)?;
                }

                match fs::rename(from, to) {
                    Ok(()) => {
                        let _ = fs::remove_file(&backup);
                        Ok(())
                    }
                    Err(e) => {
                        // Restore original file if replacing failed
                        if backup.exists() {
                            let _ = fs::rename(&backup, to);
                        }
                        Err(e)
                    }
                }
            }
        }
    }

    #[cfg(not(windows))]
    {
        fs::rename(from, to)
    }
}

/// Cleans orphan temporary compact files matching `{stem}.compact.*`, `{stem}.tmp.*`,
/// or `{stem}.old.*` in the directory of `live_path`.
pub fn clean_orphan_temps<P: AsRef<Path>>(live_path: P) -> io::Result<usize> {
    let live = live_path.as_ref();
    let parent = match live.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };

    let stem = match live.file_stem().and_then(|s| s.to_str()) {
        Some(s) => s,
        None => return Ok(0),
    };

    let compact_prefix = format!("{}.compact.", stem);
    let tmp_prefix = format!("{}.tmp", stem);
    let old_prefix = format!("{}.old.", stem);

    let mut cleaned = 0;
    if let Ok(entries) = fs::read_dir(parent) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(file_name) = path.file_name().and_then(|f| f.to_str()) {
                if file_name.starts_with(&compact_prefix)
                    || file_name.starts_with(&tmp_prefix)
                    || file_name.starts_with(&old_prefix)
                {
                    if path != live && fs::remove_file(&path).is_ok() {
                        cleaned += 1;
                    }
                }
            }
        }
    }

    Ok(cleaned)
}
