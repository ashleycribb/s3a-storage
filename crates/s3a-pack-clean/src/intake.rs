//! Duplicate-safe intake: dedup index, scoped load labels, redelivery + dead letters,
//! and signed provenance events, on top of raw-seal-first cleaning.
//!
//! Order: label check -> fetch (with redelivery) -> dedup -> seal raw -> clean -> write
//! clean (with redelivery) -> commit (dedup entries, label, events). Archives are written
//! before the log commit; a crash in between leaves unreferenced archives that a re-run
//! reuses (same content address) or ignores. Rulebook problems go to the cleaned version's
//! review queue and are never retried.
use crate::intake_log::{DeadLetter, DedupEntry, IntakeLog, LabelEntry, Record};
use crate::{clean, raw, rulebook::Rulebook, SourceMeta};
use ed25519_dalek::SigningKey;
use std::io;
use std::path::{Path, PathBuf};

/// Retry policy. Default: no retries. Only transient I/O errors are ever retried.
#[derive(Debug, Clone, Copy, Default)]
pub struct Redelivery {
    pub max_retries: u32,
}

pub fn is_transient(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::Interrupted
            | io::ErrorKind::TimedOut
            | io::ErrorKind::WouldBlock
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::NotConnected
            | io::ErrorKind::BrokenPipe
    )
}

/// Pluggable I/O so connectors (and tests) can supply fetch/write behaviour.
pub trait IntakeIo {
    fn fetch(&mut self, src: &Path, _attempt: u32) -> io::Result<Vec<u8>> {
        std::fs::read(src)
    }
    fn write_clean(
        &mut self,
        out: &Path,
        cv: &clean::CleanedVersion,
        _attempt: u32,
    ) -> io::Result<()> {
        clean::write(out, cv)
    }
}
pub struct FsIo;
impl IntakeIo for FsIo {}

pub struct Request<'a> {
    pub state_dir: &'a Path,
    pub files: Vec<PathBuf>,
    pub meta: SourceMeta,
    pub label: Option<(String, String)>,
    pub captured_at: u64,
    pub redelivery: Redelivery,
}

#[derive(Debug)]
pub enum Outcome {
    /// New data sealed and cleaned.
    Committed {
        raw_snapshot_hash: String,
        raw_archive: PathBuf,
        clean_archive: PathBuf,
        cleaned_hash: String,
        duplicates: Vec<DedupEntry>,
        dead_letters: Vec<DeadLetter>,
    },
    /// Every fetched item was already ingested for this connector: no-op.
    Duplicate {
        existing: Vec<DedupEntry>,
        dead_letters: Vec<DeadLetter>,
    },
    /// Nothing could be fetched; everything went to the dead-letter queue.
    AllDeadLettered(Vec<DeadLetter>),
}

#[derive(Debug)]
pub enum IntakeError {
    LabelReused(LabelEntry),
    /// Another intake run holds `<state>/intake.lock`.
    Locked(PathBuf),
    Io(io::Error),
}
impl std::fmt::Display for IntakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IntakeError::LabelReused(l) => write!(
                f,
                "label '{}' in scope '{}' was already committed (raw snapshot {})",
                l.label, l.scope, l.raw_snapshot_hash
            ),
            IntakeError::Locked(p) => write!(
                f,
                "intake state is locked by another run ({}); retry when it finishes",
                p.display()
            ),
            IntakeError::Io(e) => write!(f, "{e}"),
        }
    }
}
impl From<io::Error> for IntakeError {
    fn from(e: io::Error) -> Self {
        IntakeError::Io(e)
    }
}

fn with_retry<T>(
    policy: Redelivery,
    mut f: impl FnMut(u32) -> io::Result<T>,
) -> Result<T, (io::Error, u32)> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        match f(attempt) {
            Ok(v) => return Ok(v),
            Err(e) if is_transient(&e) && attempt <= policy.max_retries => continue,
            Err(e) => return Err((e, attempt)),
        }
    }
}

pub fn log_path(state_dir: &Path) -> PathBuf {
    state_dir.join("intake.log")
}

pub fn ingest(
    req: &Request,
    rb: &Rulebook,
    rb_bytes: &[u8],
    key: &SigningKey,
    io_: &mut dyn IntakeIo,
) -> Result<Outcome, IntakeError> {
    let _lock = StateLock::acquire(req.state_dir)?;
    let mut log = IntakeLog::open(&log_path(req.state_dir))?;
    if let Some((scope, label)) = &req.label {
        if let Some(l) = log.label(scope, label) {
            return Err(IntakeError::LabelReused(l.clone()));
        }
    }
    let conn = req.meta.connector.clone();
    let mut dls = vec![];
    let mut fetched = vec![];
    let mut dups = vec![];
    for f in &req.files {
        match with_retry(req.redelivery, |a| io_.fetch(f, a)) {
            Ok(bytes) => {
                let sha = crate::sha256_hex(&bytes);
                if let Some(d) = log.dedup(&sha, &conn) {
                    dups.push(d.clone());
                } else if !fetched
                    .iter()
                    .any(|(_, b): &(PathBuf, Vec<u8>)| crate::sha256_hex(b) == sha)
                {
                    fetched.push((f.clone(), bytes));
                }
            }
            Err((e, attempts)) => {
                let d = DeadLetter {
                    mark: s3a_core::MARK_QUARANTINE,
                    step: "fetch".into(),
                    source_path: f.display().to_string(),
                    item_sha256: None,
                    raw_archive: None,
                    reason: e.to_string(),
                    attempts,
                    transient: is_transient(&e),
                    at: req.captured_at,
                };
                log.append(Record::DeadLetter(d.clone()))?;
                dls.push(d);
            }
        }
    }
    if fetched.is_empty() {
        return Ok(if dups.is_empty() {
            Outcome::AllDeadLettered(dls)
        } else {
            Outcome::Duplicate {
                existing: dups,
                dead_letters: dls,
            }
        });
    }

    // Seal raw (content-addressed path, so a crash re-run reuses it).
    std::fs::create_dir_all(req.state_dir.join("raw"))?;
    std::fs::create_dir_all(req.state_dir.join("clean"))?;
    let staging = req.state_dir.join("raw").join(format!(
        "staging-{}.s3a",
        crate::sha256_hex(&crate::canonical(&(
            req.captured_at,
            fetched
                .iter()
                .map(|(p, b)| (p.display().to_string(), crate::sha256_hex(b)))
                .collect::<Vec<_>>()
        )))
    ));
    let snap = if staging.exists() {
        raw::load(&staging)?.0
    } else {
        raw::intake_bytes(&fetched, &req.meta, req.captured_at, &staging)?
    };
    let raw_archive = req
        .state_dir
        .join("raw")
        .join(format!("{}.s3a", snap.snapshot_hash));
    if !raw_archive.exists() {
        std::fs::rename(&staging, &raw_archive)?;
    } else if staging.exists() {
        std::fs::remove_file(&staging)?;
    }
    let (snap, blobs) = raw::load(&raw_archive)?;

    // Clean (deterministic; rulebook problems land in the review queue, never retried).
    let cv = clean::clean(&snap, &blobs, rb, rb_bytes, key);
    let clean_archive = req
        .state_dir
        .join("clean")
        .join(format!("{}.s3a", cv.cleaned_hash));
    if !clean_archive.exists() {
        if let Err((e, attempts)) =
            with_retry(req.redelivery, |a| io_.write_clean(&clean_archive, &cv, a))
        {
            for it in &snap.items {
                let d = DeadLetter {
                    mark: s3a_core::MARK_QUARANTINE,
                    step: "write_clean".into(),
                    source_path: it.source_path.clone(),
                    item_sha256: Some(it.item_sha256.clone()),
                    raw_archive: Some(raw_archive.display().to_string()),
                    reason: e.to_string(),
                    attempts,
                    transient: is_transient(&e),
                    at: req.captured_at,
                };
                log.append(Record::DeadLetter(d.clone()))?;
                dls.push(d);
            }
            log.event(
                key,
                "seal_raw",
                snap.items.iter().map(|i| i.item_sha256.clone()).collect(),
                snap.snapshot_hash.clone(),
                req.captured_at,
            )?;
            return Ok(Outcome::AllDeadLettered(dls));
        }
    }

    // Commit: events, dedup entries, then the label last.
    let at = req.captured_at;
    log.event(
        key,
        "fetch",
        snap.items
            .iter()
            .map(|i| crate::sha256_hex(i.source_path.as_bytes()))
            .collect(),
        crate::sha256_hex(&crate::canonical(
            &snap
                .items
                .iter()
                .map(|i| &i.item_sha256)
                .collect::<Vec<_>>(),
        )),
        at,
    )?;
    log.event(
        key,
        "seal_raw",
        snap.items.iter().map(|i| i.item_sha256.clone()).collect(),
        snap.snapshot_hash.clone(),
        at,
    )?;
    log.event(
        key,
        "clean",
        vec![snap.snapshot_hash.clone(), cv.body.rulebook_sha256.clone()],
        cv.cleaned_hash.clone(),
        at,
    )?;
    for it in &snap.items {
        if log.dedup(&it.item_sha256, &conn).is_none() {
            log.append(Record::Dedup(DedupEntry {
                item_sha256: it.item_sha256.clone(),
                connector_id: conn.clone(),
                raw_snapshot_hash: snap.snapshot_hash.clone(),
                raw_archive: raw_archive.display().to_string(),
            }))?;
        }
    }
    if let Some((scope, label)) = &req.label {
        log.append(Record::Label(LabelEntry {
            scope: scope.clone(),
            label: label.clone(),
            raw_snapshot_hash: snap.snapshot_hash.clone(),
            committed_at: at,
        }))?;
    }
    log.event(
        key,
        "commit",
        vec![snap.snapshot_hash.clone(), cv.cleaned_hash.clone()],
        log.last_hash().to_string(),
        at,
    )?;
    Ok(Outcome::Committed {
        raw_snapshot_hash: snap.snapshot_hash,
        raw_archive,
        clean_archive,
        cleaned_hash: cv.cleaned_hash,
        duplicates: dups,
        dead_letters: dls,
    })
}

/// Exclusive OS file lock on `<state>/intake.lock`, held until dropped.
/// The kernel releases it when the process exits, so a crashed run never leaves a
/// stale lock; the lock file's mere existence means nothing.
pub struct StateLock {
    _file: std::fs::File,
}
impl StateLock {
    pub fn acquire(state_dir: &Path) -> Result<Self, IntakeError> {
        std::fs::create_dir_all(state_dir)?;
        let p = state_dir.join("intake.lock");
        let f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&p)?;
        match f.try_lock() {
            Ok(()) => Ok(StateLock { _file: f }),
            Err(std::fs::TryLockError::WouldBlock) => Err(IntakeError::Locked(p)),
            Err(std::fs::TryLockError::Error(e)) => Err(IntakeError::Io(e)),
        }
    }
}

/// An archive under `<state>/raw` or `<state>/clean` not referenced by the log.
#[derive(Debug, Clone, PartialEq)]
pub struct Orphan {
    pub path: PathBuf,
    pub bytes: u64,
}

/// List (and with `apply`, delete) unreferenced archives. Takes the state lock.
/// Referenced: raw archives named by dedup or dead-letter records; clean archives
/// whose cleaned hash appears in a signed `clean` event.
pub fn gc(state_dir: &Path, apply: bool) -> Result<Vec<Orphan>, IntakeError> {
    let _lock = StateLock::acquire(state_dir)?;
    let log = IntakeLog::inspect(&log_path(state_dir))?;
    if log.recovered_torn_tail {
        return Err(IntakeError::Io(io::Error::new(
            io::ErrorKind::InvalidData,
            "intake log has a torn tail; run `clean log-repair` first",
        )));
    }
    let refs = log.referenced_archives();
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let raw_refs: std::collections::BTreeSet<PathBuf> = refs
        .iter()
        .filter(|r| !r.starts_with("clean:"))
        .map(|r| canon(Path::new(r)))
        .collect();
    let mut out = vec![];
    for sub in ["raw", "clean"] {
        let dir = state_dir.join(sub);
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<_> = rd.filter_map(Result::ok).map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            if !p.is_file() || p.extension().and_then(|e| e.to_str()) != Some("s3a") {
                continue;
            }
            let referenced = if sub == "raw" {
                raw_refs.contains(&canon(&p))
            } else {
                let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                refs.contains(&format!("clean:{stem}"))
            };
            if !referenced {
                out.push(Orphan {
                    bytes: std::fs::metadata(&p)?.len(),
                    path: p,
                });
            }
        }
    }
    if apply {
        for o in &out {
            std::fs::remove_file(&o.path)?;
        }
    }
    Ok(out)
}
