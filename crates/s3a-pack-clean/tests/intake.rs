use ed25519_dalek::SigningKey;
use s3a_pack_clean::intake::{
    ingest, log_path, FsIo, IntakeError, IntakeIo, Outcome, Redelivery, Request,
};
use s3a_pack_clean::intake_log::{IntakeLog, Record};
use s3a_pack_clean::{Rulebook, SourceMeta};
use std::io;
use std::path::{Path, PathBuf};

const RB: &[u8] = include_bytes!("../examples/rulebook.yaml");
const CSV: &str = "invoice_id,date,amount,currency\nINV-1,10/21/2026,12.50,usd\n";

fn key() -> SigningKey {
    SigningKey::from_bytes(&[7; 32])
}
fn req<'a>(
    state: &'a Path,
    files: Vec<PathBuf>,
    label: Option<(&str, &str)>,
    retries: u32,
) -> Request<'a> {
    Request {
        state_dir: state,
        files,
        meta: SourceMeta {
            connector: "file".into(),
            org: None,
            user: None,
            device: None,
            location: None,
        },
        label: label.map(|(s, l)| (s.into(), l.into())),
        captured_at: 1_790_000_000,
        redelivery: Redelivery {
            max_retries: retries,
        },
    }
}
fn run(r: &Request, io_: &mut dyn IntakeIo) -> Result<Outcome, IntakeError> {
    ingest(r, &Rulebook::parse(RB).unwrap(), RB, &key(), io_)
}

#[test]
fn resend_same_bytes_is_noop_returning_existing() {
    let d = tempfile::tempdir().unwrap();
    let st = d.path().join("state");
    let f = d.path().join("a.csv");
    std::fs::write(&f, CSV).unwrap();
    let Outcome::Committed {
        raw_snapshot_hash, ..
    } = run(
        &req(&st, vec![f.clone()], Some(("bakery", "oct-1")), 0),
        &mut FsIo,
    )
    .unwrap()
    else {
        panic!()
    };
    // Same bytes under a different file name: still a duplicate for this connector.
    let g = d.path().join("copy.csv");
    std::fs::write(&g, CSV).unwrap();
    let before = std::fs::read(log_path(&st)).unwrap();
    match run(&req(&st, vec![g], Some(("bakery", "oct-2")), 0), &mut FsIo).unwrap() {
        Outcome::Duplicate { existing, .. } => {
            assert_eq!(existing[0].raw_snapshot_hash, raw_snapshot_hash)
        }
        o => panic!("{o:?}"),
    }
    assert_eq!(
        before,
        std::fs::read(log_path(&st)).unwrap(),
        "duplicate must not write"
    );
    assert_eq!(std::fs::read_dir(st.join("raw")).unwrap().count(), 1);
}

#[test]
fn same_label_different_bytes_refused() {
    let d = tempfile::tempdir().unwrap();
    let st = d.path().join("state");
    let f = d.path().join("a.csv");
    std::fs::write(&f, CSV).unwrap();
    run(
        &req(&st, vec![f.clone()], Some(("bakery", "oct")), 0),
        &mut FsIo,
    )
    .unwrap();
    std::fs::write(&f, format!("{CSV}INV-2,10/22/2026,1,USD\n")).unwrap();
    match run(
        &req(&st, vec![f.clone()], Some(("bakery", "oct")), 0),
        &mut FsIo,
    ) {
        Err(IntakeError::LabelReused(l)) => assert_eq!(l.label, "oct"),
        o => panic!("{o:?}"),
    }
    // Same label in another scope is fine.
    assert!(matches!(
        run(&req(&st, vec![f], Some(("cafe", "oct")), 0), &mut FsIo).unwrap(),
        Outcome::Committed { .. }
    ));
}

#[test]
fn crash_reopen_keeps_index_and_drops_torn_tail() {
    let d = tempfile::tempdir().unwrap();
    let st = d.path().join("state");
    let f = d.path().join("a.csv");
    std::fs::write(&f, CSV).unwrap();
    run(&req(&st, vec![f.clone()], Some(("s", "l")), 0), &mut FsIo).unwrap();
    let lp = log_path(&st);
    let n = IntakeLog::open(&lp).unwrap().records.len();
    // Simulate a crash mid-append.
    let mut bytes = std::fs::read(&lp).unwrap();
    bytes.extend_from_slice(b"{\"prev\":\"00");
    std::fs::write(&lp, &bytes).unwrap();
    let log = IntakeLog::open(&lp).unwrap();
    assert!(log.recovered_torn_tail);
    assert_eq!(log.records.len(), n);
    assert!(log.label("s", "l").is_some());
    drop(log);
    assert!(matches!(
        run(&req(&st, vec![f], None, 0), &mut FsIo).unwrap(),
        Outcome::Duplicate { .. }
    ));
    // Mid-file tampering is an error, not a silent truncate.
    let s = std::fs::read_to_string(&lp).unwrap();
    std::fs::write(
        &lp,
        s.replacen("\"oct\"", "\"x\"", 1)
            .replacen("\"s\"", "\"t\"", 1),
    )
    .unwrap();
    assert!(IntakeLog::open(&lp).is_err());
}

#[test]
fn provenance_events_signed_and_chained() {
    let d = tempfile::tempdir().unwrap();
    let st = d.path().join("state");
    let f = d.path().join("a.csv");
    std::fs::write(&f, CSV).unwrap();
    run(&req(&st, vec![f], None, 0), &mut FsIo).unwrap();
    let log = IntakeLog::open(&log_path(&st)).unwrap();
    let steps: Vec<_> = log.events().map(|e| e.step.as_str()).collect();
    assert_eq!(steps, ["fetch", "seal_raw", "clean", "commit"]);
    for e in log.events() {
        e.verify_sig().unwrap();
    }
    let mut e = log.events().next().unwrap().clone();
    e.output_hash = "00".repeat(32);
    assert!(e.verify_sig().is_err());
}

struct Flaky {
    fails: u32,
    kind: io::ErrorKind,
    calls: u32,
}
impl IntakeIo for Flaky {
    fn fetch(&mut self, src: &Path, _a: u32) -> io::Result<Vec<u8>> {
        self.calls += 1;
        if self.calls <= self.fails {
            return Err(io::Error::new(self.kind, "connector hiccup"));
        }
        std::fs::read(src)
    }
}
struct BadSink(u32);
impl IntakeIo for BadSink {
    fn write_clean(
        &mut self,
        _o: &Path,
        _c: &s3a_pack_clean::CleanedVersion,
        _a: u32,
    ) -> io::Result<()> {
        self.0 += 1;
        Err(io::Error::new(io::ErrorKind::TimedOut, "sink down"))
    }
}

#[test]
fn dead_letter_after_retries() {
    let d = tempfile::tempdir().unwrap();
    let st = d.path().join("state");
    let f = d.path().join("a.csv");
    std::fs::write(&f, CSV).unwrap();
    // Transient failure recovers within the retry budget.
    let mut io_ = Flaky {
        fails: 2,
        kind: io::ErrorKind::TimedOut,
        calls: 0,
    };
    assert!(matches!(
        run(&req(&st, vec![f.clone()], None, 2), &mut io_).unwrap(),
        Outcome::Committed { .. }
    ));
    // Write-side failure after sealing: DLQ carries the sealed raw, reason and attempts.
    let st2 = d.path().join("state2");
    let mut sink = BadSink(0);
    let Outcome::AllDeadLettered(dl) =
        run(&req(&st2, vec![f.clone()], None, 3), &mut sink).unwrap()
    else {
        panic!()
    };
    assert_eq!(sink.0, 4);
    assert_eq!((dl[0].attempts, dl[0].mark), (4, s3a_core::MARK_QUARANTINE));
    assert!(dl[0].reason.contains("sink down"));
    assert!(Path::new(dl[0].raw_archive.as_ref().unwrap()).exists());
    assert_eq!(
        IntakeLog::open(&log_path(&st2))
            .unwrap()
            .dead_letters()
            .count(),
        1
    );
}

#[test]
fn default_no_retry_and_permanent_errors_never_retried() {
    let d = tempfile::tempdir().unwrap();
    let st = d.path().join("state");
    let f = d.path().join("a.csv");
    std::fs::write(&f, CSV).unwrap();
    let mut io_ = Flaky {
        fails: 1,
        kind: io::ErrorKind::TimedOut,
        calls: 0,
    };
    let Outcome::AllDeadLettered(dl) = run(&req(&st, vec![f.clone()], None, 0), &mut io_).unwrap()
    else {
        panic!()
    };
    assert_eq!((dl[0].attempts, io_.calls), (1, 1));
    let mut io_ = Flaky {
        fails: 9,
        kind: io::ErrorKind::PermissionDenied,
        calls: 0,
    };
    let Outcome::AllDeadLettered(dl) = run(&req(&st, vec![f], None, 5), &mut io_).unwrap() else {
        panic!()
    };
    assert_eq!((dl[0].attempts, dl[0].transient), (1, false));
    let log = IntakeLog::open(&log_path(&st)).unwrap();
    assert!(log
        .records
        .iter()
        .all(|r| matches!(r, Record::DeadLetter(_))));
}

#[test]
fn concurrent_intake_blocked_by_state_lock() {
    use s3a_pack_clean::intake::StateLock;
    let d = tempfile::tempdir().unwrap();
    let st = d.path().join("state");
    let f = d.path().join("a.csv");
    std::fs::write(&f, CSV).unwrap();
    let held = StateLock::acquire(&st).unwrap();
    match run(&req(&st, vec![f.clone()], None, 0), &mut FsIo) {
        Err(IntakeError::Locked(p)) => assert!(p.ends_with("intake.lock")),
        o => panic!("{o:?}"),
    }
    assert!(matches!(
        s3a_pack_clean::intake::gc(&st, false),
        Err(IntakeError::Locked(_))
    ));
    drop(held);
    // Lock file still exists but the OS lock is gone: not stale.
    assert!(st.join("intake.lock").exists());
    assert!(matches!(
        run(&req(&st, vec![f], None, 0), &mut FsIo).unwrap(),
        Outcome::Committed { .. }
    ));
}

#[test]
fn gc_dry_run_vs_apply() {
    use s3a_pack_clean::intake::gc;
    let d = tempfile::tempdir().unwrap();
    let st = d.path().join("state");
    let f = d.path().join("a.csv");
    std::fs::write(&f, CSV).unwrap();
    let Outcome::Committed {
        raw_archive,
        clean_archive,
        ..
    } = run(&req(&st, vec![f], None, 0), &mut FsIo).unwrap()
    else {
        panic!()
    };
    let orphan_raw = st.join("raw").join("staging-deadbeef.s3a");
    let orphan_clean = st.join("clean").join(format!("{}.s3a", "ab".repeat(32)));
    std::fs::write(&orphan_raw, b"x").unwrap();
    std::fs::write(&orphan_clean, b"y").unwrap();
    let dry = gc(&st, false).unwrap();
    assert_eq!(dry.len(), 2);
    assert!(orphan_raw.exists() && orphan_clean.exists());
    let applied = gc(&st, true).unwrap();
    assert_eq!(applied, dry);
    assert!(!orphan_raw.exists() && !orphan_clean.exists());
    assert!(
        raw_archive.exists() && clean_archive.exists(),
        "referenced archives untouched"
    );
    assert!(gc(&st, false).unwrap().is_empty());
}

#[test]
fn log_verify_is_read_only_and_repair_truncates() {
    let d = tempfile::tempdir().unwrap();
    let st = d.path().join("state");
    let f = d.path().join("a.csv");
    std::fs::write(&f, CSV).unwrap();
    run(&req(&st, vec![f], None, 0), &mut FsIo).unwrap();
    let lp = log_path(&st);
    let mut bytes = std::fs::read(&lp).unwrap();
    bytes.extend_from_slice(b"{\"prev\":\"00");
    std::fs::write(&lp, &bytes).unwrap();
    let dir = st.display().to_string();
    let args = |c: &str| vec![c.to_string(), dir.clone()];
    assert_eq!(s3a_pack_clean::cli::run(&args("log-verify")), 4);
    assert_eq!(
        std::fs::read(&lp).unwrap(),
        bytes,
        "log-verify must not modify the log"
    );
    let l = IntakeLog::inspect(&lp).unwrap();
    assert_eq!((l.recovered_torn_tail, l.torn_tail_bytes), (true, 11));
    assert_eq!(s3a_pack_clean::cli::run(&args("log-repair")), 0);
    assert_eq!(std::fs::read(&lp).unwrap().len(), bytes.len() - 11);
    assert_eq!(s3a_pack_clean::cli::run(&args("log-verify")), 0);
    // Missing log: inspect does not create one.
    let empty = d.path().join("nope");
    assert!(IntakeLog::inspect(&log_path(&empty)).is_err());
    assert!(!empty.exists());
}
