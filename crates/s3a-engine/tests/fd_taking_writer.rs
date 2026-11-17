//! Acceptance tests for the fd-taking TileWriter create/append and seal_archive
//! API (same file description the caller already holds LOCK_EX on).
//!
//! These constructors and `seal_archive_from_file` must not open the archive
//! path and must not reopen the file via `/proc/self/fd`. The path-based
//! `TileWriter::create` / `open_append` and `seal_archive` APIs stay unchanged.
//!
//! The fd-taking API is provided only on platforms that have file
//! descriptors/handles (`cfg(not(target_arch = "wasm32"))`). It is not
//! compiled, stubbed, or faked on wasm32.

#![cfg(not(target_arch = "wasm32"))]

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use s3a_core::TileType;
use s3a_engine::{
    seal_archive, seal_archive_from_file, verify_sealed_archive, verify_sealed_bytes,
    TelemetryRecord, TileWriter,
};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TestTempFile {
    path: PathBuf,
}

impl TestTempFile {
    fn new(suffix: &str) -> Self {
        let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let mut path = std::env::temp_dir();
        path.push(format!(
            "s3a_fd_taking_{}_{}_{}_{}.s3a",
            suffix,
            std::process::id(),
            ts,
            id
        ));
        let _ = fs::remove_file(&path);
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestTempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn rec(ts: u64) -> TelemetryRecord {
    TelemetryRecord::new(ts, 1, 1, 1.0)
}

fn open_rw_create(path: &Path) -> File {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
        .expect("create empty snapshot temp")
}

fn read_all(file: &mut File) -> Vec<u8> {
    file.flush().unwrap();
    file.sync_all().unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).unwrap();
    buf
}

/// A snapshot temp is created, appended, and sealed through the fd-taking
/// s3a-engine API on the same file description that holds LOCK_EX. A second
/// open of the path cannot take that lock while the first description holds it.
#[test]
fn snapshot_temp_create_append_seal_on_same_locked_file_description() {
    let temp = TestTempFile::new("same_fd");
    let file = open_rw_create(temp.path());
    file.try_lock()
        .expect("host LOCK_EX on the snapshot temp fd");

    let mut writer = TileWriter::create_from_file(file).expect("fd-taking create");
    writer
        .write_hyper_tile(TileType::TELEMETRY, &[rec(1000)], None, None)
        .expect("write first tile through the locked fd");
    let file = writer
        .into_file()
        .expect("recover the same file description");

    let mut writer = TileWriter::open_append_from_file(file).expect("fd-taking append");
    writer
        .write_hyper_tile(TileType::TELEMETRY, &[rec(2000)], None, None)
        .expect("write second tile through the locked fd");
    let mut file = writer
        .into_file()
        .expect("recover the same file description");

    seal_archive_from_file(&mut file, 16, 2).expect("fd-taking seal on the locked fd");

    let contention = OpenOptions::new()
        .read(true)
        .write(true)
        .open(temp.path())
        .expect("independent open of the snapshot path");
    assert!(
        contention.try_lock().is_err(),
        "LOCK_EX must still be held on the original file description after create, append, and seal"
    );
    drop(contention);

    let report =
        verify_sealed_archive(temp.path(), None).expect("path verify of the sealed snapshot");
    assert!(report.is_sealed, "archive must be sealed");
    assert_eq!(report.tile_count, 2, "both appended tiles must be sealed");

    let bytes = read_all(&mut file);
    let from_fd = verify_sealed_bytes(&bytes).expect("verify bytes read from the same fd");
    assert!(from_fd.is_sealed);
    assert_eq!(from_fd.tile_count, 2);
    assert_eq!(from_fd.file_uuid, report.file_uuid);
}

/// After the temp is opened and locked, the path is removed (or replaced).
/// Create, append, and seal must still succeed, which they cannot if they
/// open or openat the snapshot path.
#[test]
fn snapshot_temp_create_append_seal_do_not_open_the_path() {
    let temp = TestTempFile::new("no_path_open");
    let file = open_rw_create(temp.path());
    file.try_lock()
        .expect("host LOCK_EX on the snapshot temp fd");

    let planted = PathBuf::from(format!("{}.planted", temp.path().display()));
    let _ = fs::remove_file(&planted);

    // Move the directory entry out of the way (and, on Windows, plant a dummy
    // at the original path). A later open/openat of the original path cannot
    // reach the inode we already hold.
    #[cfg(unix)]
    {
        fs::remove_file(temp.path()).expect("unlink snapshot path while the fd is held");
    }
    #[cfg(windows)]
    {
        fs::rename(temp.path(), &planted).expect("rename snapshot path while the handle is held");
        fs::write(temp.path(), b"not-an-s3a-archive").expect("plant dummy at the original path");
    }

    let mut writer = TileWriter::create_from_file(file).expect("create without the path");
    writer
        .write_hyper_tile(TileType::TELEMETRY, &[rec(1000)], None, None)
        .expect("write without the path");
    let file = writer.into_file().expect("same file description");
    let mut writer = TileWriter::open_append_from_file(file).expect("append without the path");
    writer
        .write_hyper_tile(TileType::TELEMETRY, &[rec(2000)], None, None)
        .expect("second write without the path");
    let mut file = writer.into_file().expect("same file description");
    seal_archive_from_file(&mut file, 16, 2).expect("seal without the path");

    let bytes = read_all(&mut file);
    let report = verify_sealed_bytes(&bytes).expect("sealed bytes on the original fd");
    assert!(report.is_sealed);
    assert_eq!(report.tile_count, 2);

    #[cfg(windows)]
    {
        let dummy = fs::read(temp.path()).expect("planted dummy must be untouched");
        assert_eq!(
            dummy, b"not-an-s3a-archive",
            "fd-taking API must not have opened and overwritten the original path"
        );
        let _ = fs::remove_file(temp.path());
        let _ = fs::remove_file(&planted);
    }
}

/// Linux: after the temp fd is open and locked, further open/openat/openat2
/// /creat syscalls are rejected. That includes a `/proc/self/fd` reopen, which
/// would be a second file description.
#[cfg(target_os = "linux")]
#[test]
fn snapshot_temp_create_append_seal_do_not_reopen_via_proc_self_fd() {
    if std::env::var_os("S3A_ENGINE_FD_SECCOMP_CHILD").is_some() {
        match no_reopen_under_seccomp() {
            Ok(()) => std::process::exit(0),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
    }

    let exe = std::env::current_exe().expect("test executable");
    let output = std::process::Command::new(exe)
        .env("S3A_ENGINE_FD_SECCOMP_CHILD", "1")
        .env("RUST_TEST_THREADS", "1")
        .args([
            "--exact",
            "snapshot_temp_create_append_seal_do_not_reopen_via_proc_self_fd",
            "--nocapture",
        ])
        .output()
        .expect("spawn seccomp child");
    assert!(
        output.status.success(),
        "seccomp child failed (create/append/seal must not open() or reopen /proc/self/fd): status={:?} stdout={} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(target_os = "linux")]
fn no_reopen_under_seccomp() -> Result<(), String> {
    let temp = TestTempFile::new("no_proc_fd");
    let file = open_rw_create(temp.path());
    file.try_lock().map_err(|e| format!("host LOCK_EX: {e}"))?;
    // Path gone so a path open cannot succeed even if seccomp were skipped.
    fs::remove_file(temp.path()).map_err(|e| format!("unlink: {e}"))?;

    install_open_eperm_seccomp().map_err(|e| format!("seccomp: {e}"))?;

    let mut writer = TileWriter::create_from_file(file)
        .map_err(|e| format!("create_from_file under seccomp (must not open/reopen): {e}"))?;
    writer
        .write_hyper_tile(TileType::TELEMETRY, &[rec(1000)], None, None)
        .map_err(|e| format!("write under seccomp: {e}"))?;
    let file = writer
        .into_file()
        .map_err(|e| format!("into_file under seccomp: {e}"))?;
    let mut writer = TileWriter::open_append_from_file(file)
        .map_err(|e| format!("open_append_from_file under seccomp: {e}"))?;
    writer
        .write_hyper_tile(TileType::TELEMETRY, &[rec(2000)], None, None)
        .map_err(|e| format!("append write under seccomp: {e}"))?;
    let mut file = writer
        .into_file()
        .map_err(|e| format!("into_file after append under seccomp: {e}"))?;
    seal_archive_from_file(&mut file, 16, 2)
        .map_err(|e| format!("seal_archive_from_file under seccomp: {e}"))?;

    let bytes = read_all(&mut file);
    let report = verify_sealed_bytes(&bytes).map_err(|e| format!("verify_sealed_bytes: {e}"))?;
    if !report.is_sealed {
        return Err("archive was not sealed".into());
    }
    if report.tile_count != 2 {
        return Err(format!("expected 2 tiles, got {}", report.tile_count));
    }
    Ok(())
}

/// Install a process-wide seccomp filter that returns EPERM for open, openat,
/// openat2, and creat. Must only be used in a child process that then exits.
#[cfg(target_os = "linux")]
fn install_open_eperm_seccomp() -> Result<(), String> {
    const BPF_LD: u16 = 0x00;
    const BPF_W: u16 = 0x00;
    const BPF_ABS: u16 = 0x20;
    const BPF_JMP: u16 = 0x05;
    const BPF_JEQ: u16 = 0x10;
    const BPF_K: u16 = 0x00;
    const BPF_RET: u16 = 0x06;
    const SECCOMP_RET_ALLOW: u32 = 0x7fff0000;
    const SECCOMP_RET_ERRNO: u32 = 0x00050000;
    const EPERM: u32 = 1;
    const SECCOMP_SET_MODE_FILTER: u32 = 1;
    const PR_SET_NO_NEW_PRIVS: i32 = 38;

    // seccomp_data.nr at 0, .arch at 4.
    const ARCH_X86_64: u32 = 0xc000003e;
    const ARCH_AARCH64: u32 = 0xc00000b7;

    #[cfg(target_arch = "x86_64")]
    const EXPECTED_ARCH: u32 = ARCH_X86_64;
    #[cfg(target_arch = "aarch64")]
    const EXPECTED_ARCH: u32 = ARCH_AARCH64;
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    compile_error!("seccomp test only written for x86_64 and aarch64");

    // Linux syscall numbers.
    #[cfg(target_arch = "x86_64")]
    const DENIED: &[u32] = &[2, 85, 257, 437]; // open, creat, openat, openat2
    #[cfg(target_arch = "aarch64")]
    const DENIED: &[u32] = &[56, 437]; // openat, openat2
    #[cfg(target_arch = "x86_64")]
    const SYS_SECCOMP: i64 = 317;
    #[cfg(target_arch = "aarch64")]
    const SYS_SECCOMP: i64 = 277;

    #[repr(C)]
    struct SockFilter {
        code: u16,
        jt: u8,
        jf: u8,
        k: u32,
    }
    #[repr(C)]
    struct SockFprog {
        len: u16,
        filter: *const SockFilter,
    }

    extern "C" {
        fn prctl(option: i32, ...) -> i32;
        fn syscall(n: i64, ...) -> i64;
    }

    fn ld_abs(k: u32) -> SockFilter {
        SockFilter {
            code: BPF_LD | BPF_W | BPF_ABS,
            jt: 0,
            jf: 0,
            k,
        }
    }
    fn jeq(k: u32, jt: u8, jf: u8) -> SockFilter {
        SockFilter {
            code: BPF_JMP | BPF_JEQ | BPF_K,
            jt,
            jf,
            k,
        }
    }
    fn ret(k: u32) -> SockFilter {
        SockFilter {
            code: BPF_RET | BPF_K,
            jt: 0,
            jf: 0,
            k,
        }
    }

    let mut filter = Vec::new();
    filter.push(ld_abs(4)); // arch
                            // If arch mismatches, skip the allow-for-wrong-arch jump by killing via errno.
    filter.push(jeq(EXPECTED_ARCH, 1, 0));
    filter.push(ret(SECCOMP_RET_ERRNO | EPERM));
    filter.push(ld_abs(0)); // nr
    for (i, nr) in DENIED.iter().enumerate() {
        let remaining = (DENIED.len() - i) as u8;
        // If equal, jump to the deny ret at the end (remaining compare jumps + the allow ret).
        filter.push(jeq(*nr, remaining, 0));
    }
    filter.push(ret(SECCOMP_RET_ALLOW));
    filter.push(ret(SECCOMP_RET_ERRNO | EPERM));

    let prog = SockFprog {
        len: filter.len() as u16,
        filter: filter.as_ptr(),
    };

    unsafe {
        if prctl(PR_SET_NO_NEW_PRIVS, 1i64, 0, 0, 0) != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let rc = syscall(
            SYS_SECCOMP,
            SECCOMP_SET_MODE_FILTER as i64,
            0i64,
            &prog as *const SockFprog,
        );
        if rc != 0 {
            return Err(format!(
                "seccomp filter rejected: {}",
                std::io::Error::last_os_error()
            ));
        }
    }
    // Keep filter alive until the syscall has copied it; then it is in the kernel.
    drop(filter);
    Ok(())
}

/// Existing path-based APIs keep their behavior: create, open_append, and
/// seal_archive still take paths; opening a sealed archive with open_append
/// does not modify it until the first append.
#[test]
fn path_based_create_open_append_and_seal_archive_still_work() {
    let raw = TestTempFile::new("path_raw");
    let sealed = TestTempFile::new("path_sealed");

    let mut writer = TileWriter::create(raw.path()).expect("path create");
    writer
        .write_hyper_tile(TileType::TELEMETRY, &[rec(1000)], None, None)
        .expect("path write");
    drop(writer);

    seal_archive(raw.path(), sealed.path(), 16, 2).expect("path seal_archive");
    let report = verify_sealed_archive(sealed.path(), None).expect("verify path-sealed archive");
    assert!(report.is_sealed);
    assert_eq!(report.tile_count, 1);

    let before = fs::read(sealed.path()).expect("read sealed bytes");
    let writer =
        TileWriter::open_append(sealed.path()).expect("path open_append of sealed archive");
    drop(writer);
    let after = fs::read(sealed.path()).expect("re-read after open_append without a write");
    assert_eq!(
        before, after,
        "open_append on a sealed archive must not modify it until the first write"
    );
}
