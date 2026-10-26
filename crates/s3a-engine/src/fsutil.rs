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
    let stem = live
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("archive");
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
/// 1. Source file data and metadata are flushed to durable storage (`sync_all`) before
///    replacement. A sync failure is returned as an error and the destination is left
///    untouched (see ADR 0001).
/// 2. Atomic rename on POSIX; on Windows, a single `MoveFileExW(MOVEFILE_REPLACE_EXISTING |
///    MOVEFILE_WRITE_THROUGH)` with no backup/rename fallback window.
/// 3. On POSIX, the parent directory is fsynced after the rename so the new directory entry
///    itself is durable; a failure to sync the directory is returned as an error (the rename
///    has already happened at that point, so the destination holds the new contents).
/// 4. Destination is never left missing or partially overwritten if replacement fails.
pub fn atomic_replace<P1: AsRef<Path>, P2: AsRef<Path>>(from: P1, to: P2) -> io::Result<()> {
    atomic_replace_with(from.as_ref(), to.as_ref(), &|f: &File| f.sync_all())
}

/// Implementation of [`atomic_replace`] with an injectable file-sync function, so tests can
/// simulate `fsync` failures that are otherwise impossible to trigger deterministically.
fn atomic_replace_with(
    from: &Path,
    to: &Path,
    sync_file: &dyn Fn(&File) -> io::Result<()>,
) -> io::Result<()> {
    atomic_replace_with_rename(from, to, sync_file, &platform_rename)
}

/// POSIX: `rename(2)` is atomic and replaces an existing destination.
#[cfg(not(windows))]
fn platform_rename(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)
}

/// Windows: `MoveFileExW(MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)`.
/// Declared directly against kernel32 to avoid adding a dependency.
#[cfg(windows)]
fn platform_rename(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    fn wide(p: &Path) -> Vec<u16> {
        p.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }
    let (f, t) = (wide(from), wide(to));
    // SAFETY: both buffers are valid NUL-terminated UTF-16 strings that outlive the call.
    let ok = unsafe {
        MoveFileExW(
            f.as_ptr(),
            t.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// [`atomic_replace_with`] with an injectable replace step, so tests can simulate a failed
/// rename/MoveFileExW and check the destination still holds the old contents.
fn atomic_replace_with_rename(
    from: &Path,
    to: &Path,
    sync_file: &dyn Fn(&File) -> io::Result<()>,
    rename: &dyn Fn(&Path, &Path) -> io::Result<()>,
) -> io::Result<()> {
    if !from.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Source file does not exist: {}", from.display()),
        ));
    }

    // Flush and sync source file data & metadata to disk. Errors are propagated:
    // renaming an unsynced file over the live archive could leave a torn file after
    // power loss.
    {
        let f = File::open(from)?;
        sync_file(&f)?;
    }

    // Single atomic replace step. On Windows this is MoveFileExW with
    // MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH: the destination is never removed
    // first, so it always holds either the old or the new contents, and the call does not
    // return until the move is flushed (Windows has no directory fsync; WRITE_THROUGH is
    // the documented durability barrier). Errors are returned; there is no fallback path.
    #[cfg(windows)]
    {
        rename(from, to)
    }

    #[cfg(not(windows))]
    {
        rename(from, to)?;
        #[cfg(unix)]
        {
            let parent = match to.parent() {
                Some(p) if !p.as_os_str().is_empty() => p,
                _ => Path::new("."),
            };
            let dir = File::open(parent)?;
            sync_file(&dir)?;
        }
        Ok(())
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
                if (file_name.starts_with(&compact_prefix)
                    || file_name.starts_with(&tmp_prefix)
                    || file_name.starts_with(&old_prefix))
                    && path != live
                    && fs::remove_file(&path).is_ok()
                {
                    cleaned += 1;
                }
            }
        }
    }

    Ok(cleaned)
}

#[cfg(test)]
mod atomic_replace_tests {
    use super::*;
    use std::cell::Cell;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "s3a_a5_{}_{}_{}",
            tag,
            std::process::id(),
            NONCE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn atomic_replace_success_replaces_contents() {
        let d = tmpdir("ok");
        let to = d.join("live.s3a");
        let from = d.join("live.compact.tmp.s3a");
        fs::write(&to, b"old contents").unwrap();
        fs::write(&from, b"new contents").unwrap();
        atomic_replace(&from, &to).unwrap();
        assert_eq!(fs::read(&to).unwrap(), b"new contents");
        assert!(!from.exists());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn atomic_replace_creates_missing_destination() {
        let d = tmpdir("new");
        let to = d.join("live.s3a");
        let from = d.join("src.s3a");
        fs::write(&from, b"fresh").unwrap();
        atomic_replace(&from, &to).unwrap();
        assert_eq!(fs::read(&to).unwrap(), b"fresh");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn atomic_replace_missing_source_is_not_found() {
        let d = tmpdir("missing");
        let err = atomic_replace(d.join("nope"), d.join("live.s3a")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn file_sync_failure_propagates_and_leaves_destination_untouched() {
        let d = tmpdir("syncfail");
        let to = d.join("live.s3a");
        let from = d.join("src.s3a");
        fs::write(&to, b"old contents").unwrap();
        fs::write(&from, b"new contents").unwrap();
        let fail = |_: &File| -> io::Result<()> { Err(io::Error::other("injected fsync failure")) };
        let err = atomic_replace_with(&from, &to, &fail).unwrap_err();
        assert!(err.to_string().contains("injected fsync failure"));
        assert_eq!(fs::read(&to).unwrap(), b"old contents");
        assert!(from.exists(), "source must not be consumed on failure");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn rename_failure_leaves_old_destination_and_source_intact() {
        let d = tmpdir("renamefail");
        let to = d.join("live.s3a");
        let from = d.join("src.s3a");
        fs::write(&to, b"old contents").unwrap();
        fs::write(&from, b"new contents").unwrap();
        let fail = |_: &Path, _: &Path| -> io::Result<()> {
            Err(io::Error::other("injected rename failure"))
        };
        let err =
            atomic_replace_with_rename(&from, &to, &|f: &File| f.sync_all(), &fail).unwrap_err();
        assert!(err.to_string().contains("injected rename failure"));
        assert_eq!(fs::read(&to).unwrap(), b"old contents");
        assert!(from.exists(), "source must not be consumed on failure");
        // A subsequent real replace yields exactly the new contents.
        atomic_replace(&from, &to).unwrap();
        assert_eq!(fs::read(&to).unwrap(), b"new contents");
        assert!(!from.exists());
        let leftovers: Vec<_> = fs::read_dir(&d).unwrap().collect();
        assert_eq!(leftovers.len(), 1, "no backup/temp files left behind");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn repeated_replace_always_leaves_old_or_new() {
        let d = tmpdir("repeat");
        let to = d.join("live.s3a");
        fs::write(&to, b"v0").unwrap();
        for i in 1..=20u32 {
            let from = d.join(format!("src{}.s3a", i));
            fs::write(&from, format!("v{}", i)).unwrap();
            atomic_replace(&from, &to).unwrap();
            assert_eq!(fs::read(&to).unwrap(), format!("v{}", i).as_bytes());
        }
        fs::remove_dir_all(&d).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn directory_sync_failure_propagates() {
        let d = tmpdir("dirsyncfail");
        let to = d.join("live.s3a");
        let from = d.join("src.s3a");
        fs::write(&to, b"old").unwrap();
        fs::write(&from, b"new").unwrap();
        // First sync (the file) succeeds, second (the parent directory) fails.
        let calls = Cell::new(0u32);
        let sync = |f: &File| -> io::Result<()> {
            calls.set(calls.get() + 1);
            if calls.get() == 2 {
                assert!(
                    f.metadata()?.is_dir(),
                    "second sync must target the directory"
                );
                Err(io::Error::other("injected dir fsync failure"))
            } else {
                f.sync_all()
            }
        };
        let err = atomic_replace_with(&from, &to, &sync).unwrap_err();
        assert!(err.to_string().contains("injected dir fsync failure"));
        assert_eq!(calls.get(), 2);
        // Rename already happened: target is fully new, never torn.
        assert_eq!(fs::read(&to).unwrap(), b"new");
        fs::remove_dir_all(&d).unwrap();
    }

    const CHILD_ENV: &str = "S3A_A5_CRASH_CHILD_DIR";
    const SIZE: usize = 256 * 1024;

    /// Child half of the crash test: endlessly writes a temp file and atomically
    /// replaces the target, alternating between two full-size payloads, until killed.
    #[test]
    fn crash_child_writer_loop() {
        let Ok(dir) = std::env::var(CHILD_ENV) else {
            return;
        };
        let dir = PathBuf::from(dir);
        let to = dir.join("live.s3a");
        let mut i: u64 = 0;
        loop {
            let byte = if i.is_multiple_of(2) { b'A' } else { b'B' };
            let tmp = unique_compact_temp_path(&to);
            // Write in chunks so a kill can land mid-write of the temp file.
            let mut f = File::create(&tmp).unwrap();
            for _ in 0..(SIZE / 4096) {
                io::Write::write_all(&mut f, &[byte; 4096]).unwrap();
            }
            drop(f);
            atomic_replace(&tmp, &to).unwrap();
            i += 1;
        }
    }

    /// Crash-injection test: SIGKILL a child process at random points while it is
    /// writing / replacing; the target must always be entirely old or entirely new.
    #[cfg(unix)]
    #[test]
    fn crash_injection_target_is_fully_old_or_fully_new() {
        use std::process::Command;
        let exe = std::env::current_exe().unwrap();
        let mut saw_b = false;
        for round in 0..20u64 {
            let d = tmpdir("crash");
            let to = d.join("live.s3a");
            fs::write(&to, vec![b'A'; SIZE]).unwrap();
            let mut child = Command::new(&exe)
                .args([
                    "--exact",
                    "fsutil::atomic_replace_tests::crash_child_writer_loop",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(CHILD_ENV, &d)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(20 + (round * 37) % 180));
            child.kill().unwrap(); // SIGKILL: no destructors, no cleanup
            child.wait().unwrap();
            let data = fs::read(&to).unwrap();
            assert_eq!(
                data.len(),
                SIZE,
                "round {round}: target torn (len {})",
                data.len()
            );
            let first = data[0];
            assert!(
                first == b'A' || first == b'B',
                "round {round}: unexpected byte"
            );
            assert!(
                data.iter().all(|&b| b == first),
                "round {round}: mixed contents"
            );
            saw_b |= first == b'B';
            // Orphan temps may remain; they are cleaned by clean_orphan_temps.
            let _ = clean_orphan_temps(&to);
            fs::remove_dir_all(&d).unwrap();
        }
        assert!(
            saw_b,
            "child never completed a replace; test did not exercise rename"
        );
    }
}
