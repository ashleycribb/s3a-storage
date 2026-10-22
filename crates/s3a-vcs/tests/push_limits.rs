//! Push / bundle-size limits for gate:// sync (reject before writing).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use ed25519_dalek::SigningKey;
use s3a_core::{TelemetryRecord, TileType};
use s3a_engine::TileWriter;
use s3a_vcs::{sync_push, sync_push_with_limits, PushLimits, VcsError, VcsRepo};

static N: AtomicU64 = AtomicU64::new(0);

struct Tmp(PathBuf);
impl Tmp {
    fn new(s: &str) -> Self {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "s3a_pushlim_{}_{}_{}.s3a",
            s,
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        Tmp(p)
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn key() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32])
}

/// Source with two commits on main.
fn source() -> (Tmp, VcsRepo) {
    let t = Tmp::new("src");
    let mut w = TileWriter::create(&t.0).unwrap();
    w.write_hyper_tile(
        TileType::TELEMETRY,
        &[TelemetryRecord::new(1, 1, 1, 1.0)],
        None,
        None,
    )
    .unwrap();
    drop(w);
    let r = VcsRepo::open(&t.0).unwrap();
    r.commit("c1", "main", &key(), None).unwrap();
    let mut w = TileWriter::open_append(&t.0).unwrap();
    w.write_hyper_tile(
        TileType::TELEMETRY,
        &[TelemetryRecord::new(2, 2, 1, 2.0)],
        None,
        None,
    )
    .unwrap();
    drop(w);
    let r = VcsRepo::open(&t.0).unwrap();
    r.commit("c2", "main", &key(), None).unwrap();
    (t, r)
}

fn target() -> (Tmp, VcsRepo) {
    let t = Tmp::new("tgt");
    let r = VcsRepo::init(&t.0).unwrap();
    (t, r)
}

fn lim(b: u64, c: u64) -> PushLimits {
    PushLimits {
        max_bundle_bytes: b,
        max_commits: c,
    }
}

#[test]
fn defaults_are_sane_and_used_by_sync_push() {
    let d = PushLimits::default();
    assert_eq!(d.max_bundle_bytes, 64 * 1024 * 1024);
    assert_eq!(d.max_commits, 1000);
    let (_s, src) = source();
    let (_t, tgt) = target();
    assert!(sync_push(&src, &tgt, "main", false, None, &key()).unwrap() > 0);
}

#[test]
fn bundle_over_limit_rejected_without_writing_then_at_limit_ok() {
    let (_s, src) = source();
    let (t, tgt) = target();
    let before = std::fs::read(&t.0).unwrap();
    let err =
        sync_push_with_limits(&src, &tgt, "main", false, None, &key(), lim(1, 1000)).unwrap_err();
    let actual = match err {
        VcsError::PushTooLarge {
            what: "bundle bytes",
            actual,
            limit: 1,
        } => actual,
        e => panic!("unexpected {e:?}"),
    };
    assert!(err_msg(actual).contains("exceeds limit"));
    assert_eq!(
        std::fs::read(&t.0).unwrap(),
        before,
        "target must be untouched"
    );
    assert!(tgt.get_ref("main").unwrap().is_none());

    // Over by one byte still rejected; exactly at the limit succeeds.
    assert!(sync_push_with_limits(
        &src,
        &tgt,
        "main",
        false,
        None,
        &key(),
        lim(actual - 1, 1000)
    )
    .is_err());
    assert!(
        sync_push_with_limits(&src, &tgt, "main", false, None, &key(), lim(actual, 1000)).unwrap()
            > 0
    );
}

fn err_msg(actual: u64) -> String {
    VcsError::PushTooLarge {
        what: "bundle bytes",
        actual,
        limit: 1,
    }
    .to_string()
}

#[test]
fn commit_count_over_and_at_limit() {
    let (_s, src) = source();
    let n = src.list_commits().unwrap().len() as u64;
    assert!(n >= 2);
    let (t, tgt) = target();
    let before = std::fs::read(&t.0).unwrap();
    match sync_push_with_limits(
        &src,
        &tgt,
        "main",
        false,
        None,
        &key(),
        lim(u64::MAX, n - 1),
    ) {
        Err(VcsError::PushTooLarge {
            what: "commits",
            actual,
            ..
        }) => assert_eq!(actual, n),
        r => panic!("unexpected {r:?}"),
    }
    assert_eq!(std::fs::read(&t.0).unwrap(), before);
    assert!(
        sync_push_with_limits(&src, &tgt, "main", false, None, &key(), lim(u64::MAX, n)).unwrap()
            > 0
    );
    // Re-push: nothing new, passes even with zero limits.
    assert_eq!(
        sync_push_with_limits(&src, &tgt, "main", false, None, &key(), lim(0, 0)).unwrap(),
        0
    );
}

#[test]
fn short_hash_resolves_in_repo() {
    let (_s, src) = source();
    let head = src.get_ref("main").unwrap().unwrap().target_seal_hash;
    let full = hex::encode(head);
    assert_eq!(src.resolve_rev(&full[..7]).unwrap(), head);
    assert_eq!(src.resolve_rev("main").unwrap(), head);
    assert_eq!(src.log(Some(&full[..9])).unwrap()[0].seal_hash, head);
    assert_eq!(
        src.lineage(&full[..7], Default::default(), None).unwrap()[0].seal_hash,
        head
    );
    assert!(matches!(
        src.resolve_rev(&full[..6]),
        Err(VcsError::CommitNotFound(_))
    ));
}
