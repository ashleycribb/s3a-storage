//! Acceptance 009 follow-up (1): `rebuild_indexes` must restore every query from data tiles alone.
//!
//! Written before the fix. Do not edit. Each test first damages or deletes the index tiles
//! (22 STATEMENT_INDEX, 83 RECORD_ROOT), proves the damage is visible, then runs
//! `s3a_engine::rebuild_indexes` and requires query results identical to the undamaged archive.
//! A no-op `rebuild_indexes` fails every test here.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use s3a_core::{
    FileHeader, HyperTileHeader, TileType, FILE_HEADER_SIZE, FILE_HEADER_SLOT_A_OFFSET,
    FILE_HEADER_SLOT_B_OFFSET, FILE_HEADER_SLOT_SIZE, HYPER_TILE_HEADER_SIZE, HYPER_TILE_SIZE,
};
use s3a_engine::lrs_store::{uuid_to_u64_pair, LrsStore, StatementFilter};
use s3a_engine::sealed::{seal_archive_v2, verify_sealed_archive, SealOptionsV2};

static N: AtomicU64 = AtomicU64::new(0);
struct Tmp(PathBuf);
impl Tmp {
    fn new(tag: &str) -> Self {
        let n = N.fetch_add(1, Ordering::Relaxed);
        Tmp(std::env::temp_dir().join(format!(
            "acc009_rebuild_{}_{}_{}.s3a",
            tag,
            std::process::id(),
            n
        )))
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

const IDS: [&str; 4] = [
    "11111111-1111-4111-8111-111111111111",
    "22222222-2222-4222-8222-222222222222",
    "33333333-3333-4333-8333-333333333333",
    "44444444-4444-4444-8444-444444444444",
];
const REG: &str = "99999999-9999-4999-8999-999999999999";

fn stmt(id: &str, verb: &str, obj: serde_json::Value, ts: &str, reg: bool) -> String {
    let mut s = serde_json::json!({
        "id": id,
        "actor": {"objectType": "Agent", "account": {"homePage": "https://example.org", "name": "learner-a"}},
        "verb": {"id": verb, "display": {"en-US": "v"}},
        "object": obj,
        "timestamp": ts
    });
    if reg {
        s["context"] = serde_json::json!({"registration": REG});
    }
    s.to_string()
}

/// Builds an LRS archive with three statements and one voiding statement (voids IDS[1]).
fn build_lrs(path: &Path) {
    let lrs = LrsStore::open_or_create(path).unwrap();
    let act = |n: &str| serde_json::json!({"objectType": "Activity", "id": format!("https://example.org/act/{n}")});
    lrs.store_statement(
        &stmt(
            IDS[0],
            "http://adlnet.gov/expapi/verbs/completed",
            act("a"),
            "2026-01-01T10:00:00Z",
            true,
        ),
        &[],
    )
    .unwrap();
    lrs.store_statement(
        &stmt(
            IDS[1],
            "http://adlnet.gov/expapi/verbs/attempted",
            act("b"),
            "2026-01-02T10:00:00Z",
            false,
        ),
        &[],
    )
    .unwrap();
    lrs.store_statement(
        &stmt(
            IDS[2],
            "http://adlnet.gov/expapi/verbs/completed",
            act("c"),
            "2026-01-03T10:00:00Z",
            true,
        ),
        &[],
    )
    .unwrap();
    lrs.store_statement(
        &stmt(
            IDS[3],
            "http://adlnet.gov/expapi/verbs/voided",
            serde_json::json!({"objectType": "StatementRef", "id": IDS[1]}),
            "2026-01-04T10:00:00Z",
            false,
        ),
        &[],
    )
    .unwrap();
}

/// Every statement query the store offers, as comparable strings.
fn snapshot(path: &Path) -> Vec<String> {
    let lrs = LrsStore::open_or_create(path).unwrap();
    let mut out = Vec::new();
    let base = StatementFilter {
        limit: 1000,
        format: "exact".into(),
        ..Default::default()
    };
    let mut q = |label: &str, f: StatementFilter| {
        let page = lrs.query_statements(f);
        out.push(format!(
            "{label}: {:?}",
            page.map(|p| p.statements).map_err(|e| e.to_string())
        ));
    };
    q("all", base.clone());
    q(
        "all-asc",
        StatementFilter {
            ascending: true,
            ..base.clone()
        },
    );
    q(
        "verb-completed",
        StatementFilter {
            verb_iri: Some("http://adlnet.gov/expapi/verbs/completed".into()),
            ..base.clone()
        },
    );
    q(
        "registration",
        StatementFilter {
            registration: Some(uuid_to_u64_pair(REG).unwrap()),
            ..base.clone()
        },
    );
    q(
        "since-jan3",
        StatementFilter {
            since_ms: Some(1_767_398_400_000),
            ..base.clone()
        },
    );
    q(
        "activity-c",
        StatementFilter {
            activity_id: Some("https://example.org/act/c".into()),
            ..base.clone()
        },
    );
    q(
        "ids",
        StatementFilter {
            format: "ids".into(),
            ..base.clone()
        },
    );
    for id in IDS {
        let pair = uuid_to_u64_pair(id).unwrap();
        for voided in [false, true] {
            let r = lrs.get_statement(pair, voided);
            out.push(format!(
                "get {id} voided={voided}: {:?}",
                r.map(|(rec, json, ..)| (bytemuck::bytes_of(&rec).to_vec(), json))
                    .map_err(|e| e.to_string())
            ));
        }
    }
    out
}

/// Compares two query snapshots and reports only the differing entries (truncated).
fn assert_same(before: &[String], after: &[String], what: &str) {
    let diffs: Vec<String> = before
        .iter()
        .zip(after.iter())
        .filter(|(a, b)| a != b)
        .map(|(a, b)| {
            format!(
                "\n  before: {}\n  after:  {}",
                &a[..a.len().min(220)],
                &b[..b.len().min(220)]
            )
        })
        .collect();
    assert!(
        diffs.is_empty() && before.len() == after.len(),
        "{what}: {} differing queries:{}",
        diffs.len(),
        diffs.join("")
    );
}

#[test]
fn precondition_statement_queries_return_statements_after_reopen() {
    // Parity is only meaningful if queries return data on a reopened, undamaged archive.
    let t = Tmp::new("reopen");
    build_lrs(&t.0);
    let snap = snapshot(&t.0);
    assert!(
        snap[0].contains(IDS[0]) && snap[0].contains(IDS[2]),
        "query_statements on a reopened archive must return the stored statements, got: {}",
        &snap[0][..snap[0].len().min(200)]
    );
}

fn headers(bytes: &[u8]) -> Vec<HyperTileHeader> {
    let a: FileHeader = bytemuck::pod_read_unaligned(
        &bytes[FILE_HEADER_SLOT_A_OFFSET..FILE_HEADER_SLOT_A_OFFSET + FILE_HEADER_SLOT_SIZE],
    );
    let b: FileHeader = bytemuck::pod_read_unaligned(
        &bytes[FILE_HEADER_SLOT_B_OFFSET..FILE_HEADER_SLOT_B_OFFSET + FILE_HEADER_SLOT_SIZE],
    );
    let (h, _) = s3a_engine::resolve_active_header(&a, &b).unwrap();
    (0..h.tile_count as usize)
        .map(|i| {
            let off = FILE_HEADER_SIZE + i * HYPER_TILE_SIZE;
            bytemuck::pod_read_unaligned(&bytes[off..off + HYPER_TILE_HEADER_SIZE])
        })
        .collect()
}

/// Physically removes every tile whose type is in `drop` and recommits both header slots.
fn delete_tiles(path: &Path, drop: &[u32]) -> usize {
    let bytes = fs::read(path).unwrap();
    let hs = headers(&bytes);
    let mut out = bytes[..FILE_HEADER_SIZE].to_vec();
    let mut kept = 0u32;
    let mut removed = 0;
    for (i, h) in hs.iter().enumerate() {
        if drop.contains(&h.tile_type) {
            removed += 1;
            continue;
        }
        let off = FILE_HEADER_SIZE + i * HYPER_TILE_SIZE;
        let mut tile = bytes[off..off + HYPER_TILE_SIZE].to_vec();
        let mut nh = *h;
        nh.tile_index = kept;
        nh.header_crc32 = s3a_engine::compute_tile_header_crc32(&nh);
        tile[..HYPER_TILE_HEADER_SIZE].copy_from_slice(bytemuck::bytes_of(&nh));
        out.extend_from_slice(&tile);
        kept += 1;
    }
    // Keep any seal trailer (directory + end cap) that followed the tiles: removing tiles
    // in front of it must not also silently strip the seal.
    out.extend_from_slice(&bytes[FILE_HEADER_SIZE + hs.len() * HYPER_TILE_SIZE..]);
    for off in [FILE_HEADER_SLOT_A_OFFSET, FILE_HEADER_SLOT_B_OFFSET] {
        let mut fh: FileHeader =
            bytemuck::pod_read_unaligned(&out[off..off + FILE_HEADER_SLOT_SIZE]);
        if fh.magic == [0; 4] {
            continue;
        }
        fh.tile_count = kept;
        fh.generation += 1;
        fh.header_crc32 = s3a_engine::compute_header_crc32(&fh);
        out[off..off + FILE_HEADER_SLOT_SIZE].copy_from_slice(bytemuck::bytes_of(&fh));
    }
    fs::write(path, &out).unwrap();
    removed
}

/// Flips payload bytes of every tile whose type is in `types` (data CRC no longer matches).
fn corrupt_tiles(path: &Path, types: &[u32]) -> usize {
    let mut bytes = fs::read(path).unwrap();
    let hs = headers(&bytes);
    let mut n = 0;
    for (i, h) in hs.iter().enumerate() {
        if types.contains(&h.tile_type) {
            let p = FILE_HEADER_SIZE + i * HYPER_TILE_SIZE + HYPER_TILE_HEADER_SIZE;
            for b in &mut bytes[p..p + 64] {
                *b ^= 0xA5;
            }
            n += 1;
        }
    }
    fs::write(path, &bytes).unwrap();
    n
}

#[test]
fn rebuild_after_deleting_statement_index_restores_all_queries() {
    let t = Tmp::new("del22");
    build_lrs(&t.0);
    let before = snapshot(&t.0);
    assert!(
        delete_tiles(&t.0, &[TileType::STATEMENT_INDEX]) > 0,
        "fixture must contain tile 22"
    );
    assert_ne!(
        snapshot(&t.0),
        before,
        "deleting tile 22 must be visible before rebuild (test precondition)"
    );
    s3a_engine::rebuild_indexes(&t.0).expect("rebuild_indexes");
    assert_same(
        &before,
        &snapshot(&t.0),
        "statement queries after rebuild (tile 22 deleted)",
    );
}

#[test]
fn rebuild_after_corrupting_statement_index_restores_all_queries() {
    let t = Tmp::new("cor22");
    build_lrs(&t.0);
    let before = snapshot(&t.0);
    assert!(corrupt_tiles(&t.0, &[TileType::STATEMENT_INDEX]) > 0);
    assert_ne!(
        snapshot(&t.0),
        before,
        "corrupting tile 22 must be visible before rebuild (test precondition)"
    );
    s3a_engine::rebuild_indexes(&t.0).expect("rebuild_indexes");
    assert_same(
        &before,
        &snapshot(&t.0),
        "statement queries after rebuild (tile 22 corrupted)",
    );
}

fn sealed_lrs(tag: &str) -> (Tmp, Tmp) {
    let src = Tmp::new(&format!("{tag}_src"));
    let out = Tmp::new(&format!("{tag}_sealed"));
    build_lrs(&src.0);
    let opts = SealOptionsV2 {
        record_roots: true,
        about: true,
        ..Default::default()
    };
    seal_archive_v2(&src.0, &out.0, &opts).expect("seal with record roots");
    (src, out)
}

/// Record-root query surface: for every data tile (by tile_id) its sealed root, plus the check result.
fn roots(path: &Path) -> (bool, bool, Vec<(u64, u32, u64, [u8; 32])>) {
    let r = s3a_engine::MmapReader::open(path).unwrap();
    let rep = s3a_engine::record_roots::check(&r);
    let mut v: Vec<_> = s3a_engine::record_roots::read_entries(&r)
        .into_iter()
        .filter(|e| {
            e.tile_type != TileType::STATEMENT_INDEX && e.tile_type != TileType::RECORD_ROOT
        })
        .map(|e| (e.tile_id, e.tile_type, e.record_count, e.root))
        .collect();
    v.sort();
    (rep.present, rep.ok(), v)
}

#[test]
fn rebuild_after_deleting_both_index_tiles_on_sealed_archive_restores_roots_and_seal() {
    let (_src, s) = sealed_lrs("del2283");
    let q_before = snapshot(&s.0);
    let r_before = roots(&s.0);
    assert!(
        r_before.0 && r_before.1 && !r_before.2.is_empty(),
        "precondition: sealed file has valid record roots"
    );
    assert!(delete_tiles(&s.0, &[TileType::STATEMENT_INDEX, TileType::RECORD_ROOT]) >= 2);
    assert_ne!(roots(&s.0), r_before, "precondition: deletion visible");
    s3a_engine::rebuild_indexes(&s.0).expect("rebuild_indexes on a sealed archive");
    assert_same(
        &q_before,
        &snapshot(&s.0),
        "sealed: statement queries after rebuild (22+83 deleted)",
    );
    assert_eq!(
        roots(&s.0),
        r_before,
        "record-root tile 83 must be rebuilt with identical data-tile roots"
    );
    let v = verify_sealed_archive(&s.0, None).expect("verify after rebuild");
    assert!(v.is_sealed && v.warnings.is_empty() && v.repaired_tiles.is_empty(),
        "after rebuild the archive must still verify as sealed with no warnings (re-sealed), got {:?}", v.warnings);
}

#[test]
fn rebuild_after_corrupting_both_index_tiles_on_sealed_archive_restores_roots() {
    let (_src, s) = sealed_lrs("cor2283");
    let q_before = snapshot(&s.0);
    let r_before = roots(&s.0);
    assert!(corrupt_tiles(&s.0, &[TileType::STATEMENT_INDEX, TileType::RECORD_ROOT]) >= 2);
    s3a_engine::rebuild_indexes(&s.0).expect("rebuild_indexes");
    assert_same(
        &q_before,
        &snapshot(&s.0),
        "sealed: statement queries after rebuild (22+83 corrupted)",
    );
    assert_eq!(roots(&s.0), r_before);
}

#[test]
fn rebuild_refuses_corrupt_statement_body_instead_of_indexing_it() {
    let t = Tmp::new("corbody");
    build_lrs(&t.0);
    corrupt_tiles(&t.0, &[TileType::STATEMENT_BODY]);
    let before = fs::read(&t.0).unwrap();
    let r = s3a_engine::rebuild_indexes(&t.0);
    assert!(
        r.is_err(),
        "a data tile failing its CRC must make rebuild_indexes return Err, not index it"
    );
    assert_eq!(
        fs::read(&t.0).unwrap(),
        before,
        "a failed rebuild must not modify the file"
    );
}
