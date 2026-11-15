//! Acceptance 009 follow-up (6b): a retraction targets exactly one edge id.
//!
//! Written before the fix. Do not edit. THIS FILE DECLARES NEW API that does not exist at
//! b9daf82, so on b9daf82 it fails to compile (expected). Required signatures:
//!
//!   impl ScholarGraphEdge {
//!       /// Stable id of this stored edge: lowercase hex SHA-256 of its canonical JSON
//!       /// (all fields, NFC), so two edges with the same relation but different provenance
//!       /// have different ids.
//!       pub fn edge_id(&self) -> String;
//!       /// Append-only retraction of the single edge `target_edge_id`.
//!       pub fn retract(target_edge_id: &str, provenance_actor: &str, provenance_timestamp: &str) -> Self;
//!   }
//!
//! `add_to_bytes` of a retraction whose target id is not in the archive returns Err.

use s3a_pack_scholar::ScholarGraphEdge;

#[test]
fn retraction_hides_only_the_targeted_edge() {
    let e1 = ScholarGraphEdge::new("a", "b", "cites", "alice", "2026-01-01T00:00:00Z");
    let e2 = ScholarGraphEdge::new("a", "b", "cites", "bob", "2026-01-02T00:00:00Z");
    assert_ne!(
        e1.edge_id(),
        e2.edge_id(),
        "same relation, different provenance => different ids"
    );
    let mut b = e1.add_to_bytes(&[]).unwrap();
    b = e2.add_to_bytes(&b).unwrap();
    b = ScholarGraphEdge::retract(&e1.edge_id(), "curator", "2026-01-03T00:00:00Z")
        .add_to_bytes(&b)
        .unwrap();
    let left = ScholarGraphEdge::list_from_bytes(&b, "a", Some("cites")).unwrap();
    assert_eq!(left.len(), 1, "only e1 is retracted");
    assert_eq!(left[0].edge_id(), e2.edge_id());
    assert_eq!(left[0].provenance_actor.as_deref(), Some("bob"));
}

#[test]
fn retracting_an_unknown_edge_id_is_an_error() {
    let b = ScholarGraphEdge::new("a", "b", "cites", "alice", "2026-01-01T00:00:00Z")
        .add_to_bytes(&[])
        .unwrap();
    let r = ScholarGraphEdge::retract(&"0".repeat(64), "curator", "2026-01-03T00:00:00Z")
        .add_to_bytes(&b);
    assert!(r.is_err(), "a retraction must name an edge that exists");
}

#[test]
fn edge_id_survives_reopen_from_bytes() {
    let e = ScholarGraphEdge::new("a", "b", "cites", "alice", "2026-01-01T00:00:00Z");
    let b = e.add_to_bytes(&[]).unwrap();
    let back = ScholarGraphEdge::list_from_bytes(&b, "a", None).unwrap();
    assert_eq!(back[0].edge_id(), e.edge_id());
}
