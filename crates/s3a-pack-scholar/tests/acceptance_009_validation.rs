//! Acceptance 009 follow-up (6): writers validate input; readers return Err on malformed bytes
//! and never panic; re-adding a retracted edge makes it visible again.
//!
//! Written before the fix. Do not edit. Uses only APIs that exist at b9daf82.
//! Date rule: RFC 3339 UTC with a literal `Z` (e.g. `2017-06-12T00:00:00Z`, optional fraction).

use s3a_core::{HyperTileHeader, FILE_HEADER_SIZE, HYPER_TILE_HEADER_SIZE};
use s3a_pack_scholar::{ScholarGraphEdge, ScholarPaperRecord};
use std::panic::{catch_unwind, AssertUnwindSafe};

const BAD_DATES: [&str; 6] = [
    "2017-06-12T00:00:00+02:00", // not UTC
    "2017-06-12 00:00:00",       // not ISO 8601 / RFC 3339
    "12/06/2017",
    "yesterday",
    "2017-13-40T00:00:00Z", // impossible date
    "",
];

#[test]
fn paper_rejects_non_utc_or_non_iso_published_date() {
    let mut ok = ScholarPaperRecord::new("k:ok", "A Title");
    ok.published_date = Some("2017-06-12T00:00:00Z".into());
    assert!(
        ok.put_to_bytes(&[]).is_ok(),
        "a UTC RFC 3339 date is accepted"
    );
    for d in BAD_DATES {
        let mut p = ScholarPaperRecord::new("k:bad", "A Title");
        p.published_date = Some(d.into());
        assert!(
            p.put_to_bytes(&[]).is_err(),
            "published_date {d:?} must be rejected"
        );
    }
}

#[test]
fn paper_rejects_empty_title() {
    for t in ["", "   "] {
        let p = ScholarPaperRecord::new("k:t", t);
        assert!(
            p.put_to_bytes(&[]).is_err(),
            "title {t:?} must be rejected (absent is not empty)"
        );
    }
}

#[test]
fn edge_rejects_empty_actor_and_bad_timestamp() {
    assert!(
        ScholarGraphEdge::new("a", "b", "cites", "curator", "2026-01-01T00:00:00Z")
            .add_to_bytes(&[])
            .is_ok()
    );
    for actor in ["", "  "] {
        let e = ScholarGraphEdge::new("a", "b", "cites", actor, "2026-01-01T00:00:00Z");
        assert!(
            e.add_to_bytes(&[]).is_err(),
            "provenance_actor {actor:?} must be rejected"
        );
    }
    for d in BAD_DATES {
        let e = ScholarGraphEdge::new("a", "b", "cites", "curator", d);
        assert!(
            e.add_to_bytes(&[]).is_err(),
            "provenance_timestamp {d:?} must be rejected"
        );
    }
}

/// Overwrites a u32 inside the first tile's payload and re-signs both tile CRCs, so the length
/// field is the only defect a reader can find.
fn set_u32_in_first_payload(bytes: &mut [u8], payload_off: usize, v: u32) {
    let t = FILE_HEADER_SIZE;
    let p = t + HYPER_TILE_HEADER_SIZE;
    bytes[p + payload_off..p + payload_off + 4].copy_from_slice(&v.to_le_bytes());
    let mut h: HyperTileHeader =
        bytemuck::pod_read_unaligned(&bytes[t..t + HYPER_TILE_HEADER_SIZE]);
    h.data_crc32 = crc32c::crc32c(&bytes[p..p + h.payload_bytes as usize]);
    h.header_crc32 = s3a_engine::compute_tile_header_crc32(&h);
    bytes[t..t + HYPER_TILE_HEADER_SIZE].copy_from_slice(bytemuck::bytes_of(&h));
}

#[test]
fn malformed_edge_length_returns_err_without_panic() {
    for bad in [u32::MAX, 200_000, 1] {
        let mut b = ScholarGraphEdge::new("a", "b", "cites", "x", "2026-01-01T00:00:00Z")
            .add_to_bytes(&[])
            .unwrap();
        set_u32_in_first_payload(&mut b, 16, bad); // ScholarEdgeBlobHeader.total_bytes @16
        let r = catch_unwind(AssertUnwindSafe(|| {
            ScholarGraphEdge::list_from_bytes(&b, "a", None)
        }));
        let r = r.unwrap_or_else(|_| {
            panic!("list_from_bytes panicked on total_bytes={bad}; it must return Err")
        });
        assert!(
            r.is_err(),
            "total_bytes={bad} is inconsistent with the tile; must be Err, got {:?}",
            r.map(|v| v.len())
        );
    }
}

#[test]
fn malformed_paper_chunk_length_returns_err_without_panic() {
    for bad in [u32::MAX, 200_000, 1] {
        let mut b = ScholarPaperRecord::new("k:len", "Title")
            .put_to_bytes(&[])
            .unwrap();
        set_u32_in_first_payload(&mut b, 48, bad); // ScholarPaperBlobHeader.chunk_bytes @48
        let r = catch_unwind(AssertUnwindSafe(|| {
            ScholarPaperRecord::get_from_bytes(&b, "k:len")
        }));
        let r = r.unwrap_or_else(|_| {
            panic!("get_from_bytes panicked on chunk_bytes={bad}; it must return Err")
        });
        assert!(r.is_err(), "chunk_bytes={bad} is inconsistent; must be Err (not Ok(None) or a partial paper), got {r:?}");
    }
}

#[test]
fn edge_readded_after_retraction_is_visible() {
    let mut b = ScholarGraphEdge::new("a", "b", "cites", "x", "2026-01-01T00:00:00Z")
        .add_to_bytes(&[])
        .unwrap();
    b = ScholarGraphEdge::new_retraction("a", "b", "cites", "x", "2026-01-02T00:00:00Z")
        .add_to_bytes(&b)
        .unwrap();
    assert_eq!(
        ScholarGraphEdge::list_from_bytes(&b, "a", None)
            .unwrap()
            .len(),
        0,
        "retracted edge is hidden"
    );
    b = ScholarGraphEdge::new("a", "b", "cites", "y", "2026-01-03T00:00:00Z")
        .add_to_bytes(&b)
        .unwrap();
    let after = ScholarGraphEdge::list_from_bytes(&b, "a", None).unwrap();
    assert_eq!(
        after.len(),
        1,
        "an edge added after the retraction is a new edge and must be visible"
    );
    assert_eq!(
        after[0].provenance_timestamp.as_deref(),
        Some("2026-01-03T00:00:00Z")
    );
}
