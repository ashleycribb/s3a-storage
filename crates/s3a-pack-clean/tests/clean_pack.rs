use ed25519_dalek::SigningKey;
use s3a_pack_clean::{clean, raw, QueueTag, Rulebook, RulebookUpdate, SourceMeta};
use std::path::{Path, PathBuf};

const RB: &[u8] = include_bytes!("../examples/rulebook.yaml");
const INVOICES: &str = "invoice_id,date,amount,currency\n\
 INV-1 , 10/21/2026,12.50,usd\n\
INV-2,2026-10-22,8,$\n\
INV-3,13/40/2026,5,USD\n\
INV-4,10/22/2026,3,GBP\n";

fn key(b: u8) -> SigningKey {
    SigningKey::from_bytes(&[b; 32])
}
fn meta() -> SourceMeta {
    SourceMeta {
        connector: "file".into(),
        org: Some("example-bakery".into()),
        user: Some("ashley".into()),
        device: Some("aura-gw-1".into()),
        location: Some("front office".into()),
    }
}

fn setup(dir: &Path) -> (PathBuf, raw::RawSnapshot) {
    std::fs::write(dir.join("invoice_oct.csv"), INVOICES).unwrap();
    std::fs::write(dir.join("payroll.xlsx"), b"PK\x03\x04 not really").unwrap();
    let raw_out = dir.join("raw.s3a");
    let files = [dir.join("invoice_oct.csv"), dir.join("payroll.xlsx")];
    let refs: Vec<&Path> = files.iter().map(PathBuf::as_path).collect();
    let snap = raw::intake(&refs, &meta(), 1_790_000_000, &raw_out).unwrap();
    (raw_out, snap)
}

#[test]
fn end_to_end_rules_citation_and_queue() {
    let d = tempfile::tempdir().unwrap();
    let (raw_out, snap) = setup(d.path());
    let (snap2, blobs) = raw::load(&raw_out).unwrap();
    assert_eq!(snap, snap2);
    let rb = Rulebook::parse(RB).unwrap();
    let cv = clean::clean(&snap2, &blobs, &rb, RB, &key(1));
    clean::verify(&cv, &snap2).unwrap();

    // Two good rows cleaned by deterministic rules.
    assert_eq!(cv.body.records.len(), 2);
    let r1 = &cv.body.records[0];
    assert_eq!(r1.fields["invoice_id"], "INV-1");
    assert_eq!(r1.fields["date"], "2026-10-21");
    assert_eq!(r1.fields["currency"], "USD");
    assert_eq!(cv.body.records[1].fields["currency"], "USD");
    // Every change names rule id+version, maker and approval basis.
    assert!(cv.body.changes.iter().any(|c| c.rule_id == "date-us"
        && c.rule_version == 1
        && c.made_by == "rule-engine"
        && c.approved_by.starts_with("rulebook:2026.10.1@sha256:")));

    // Every cleaned record traces to its raw fingerprint and source metadata.
    for r in &cv.body.records {
        let it = snap2.item(&r.raw_item_sha256).expect("cites raw item");
        assert_eq!(r.raw_snapshot_hash, snap2.snapshot_hash);
        assert_eq!(
            it.item_sha256,
            s3a_pack_clean::sha256_hex(&blobs[&it.item_sha256])
        );
        assert_eq!(it.meta.device.as_deref(), Some("aura-gw-1"));
        assert_eq!(it.captured_at, 1_790_000_000);
    }

    // Unknown file tagged UNCLASSIFIED (needs review); bad rows quarantined NEEDS_RULE.
    let q = &cv.body.review_queue;
    let unc = q.iter().find(|e| e.tag == QueueTag::Unclassified).unwrap();
    assert!(unc.source_path.ends_with("payroll.xlsx"));
    assert_eq!(unc.mark, s3a_core::MARK_NEEDS_REVIEW);
    let bad: Vec<_> = q.iter().filter(|e| e.tag == QueueTag::NeedsRule).collect();
    assert_eq!(bad.len(), 2, "{q:?}");
    assert!(bad
        .iter()
        .all(|e| e.mark == s3a_core::MARK_QUARANTINE && e.row.is_some()));
    assert!(bad.iter().any(|e| e.reason.contains("GBP")));
}

#[test]
fn replay_is_deterministic() {
    let d = tempfile::tempdir().unwrap();
    let (raw_out, _) = setup(d.path());
    let (snap, blobs) = raw::load(&raw_out).unwrap();
    let rb = Rulebook::parse(RB).unwrap();
    let a = clean::clean(&snap, &blobs, &rb, RB, &key(1));
    let b = clean::clean(&snap, &blobs, &rb, RB, &key(2));
    assert_eq!(
        a.cleaned_hash, b.cleaned_hash,
        "hash is independent of signer"
    );
    let (snap3, blobs3) = raw::load(&raw_out).unwrap();
    assert_eq!(
        a.cleaned_hash,
        clean::clean(&snap3, &blobs3, &rb, RB, &key(1)).cleaned_hash
    );
    // Archive round-trip keeps the same hash.
    let out = d.path().join("clean.s3a");
    clean::write(&out, &a).unwrap();
    assert_eq!(clean::read(&out).unwrap(), a);
    // A different rulebook version gives a different hash.
    let rb2_bytes = String::from_utf8(RB.to_vec())
        .unwrap()
        .replace("2026.10.1", "2026.10.2");
    let rb2 = Rulebook::parse(rb2_bytes.as_bytes()).unwrap();
    assert_ne!(
        a.cleaned_hash,
        clean::body_hash(&clean::clean_body(
            &snap,
            &blobs,
            &rb2,
            rb2_bytes.as_bytes()
        ))
    );
}

#[test]
fn raw_untouched_after_cleaning_and_never_overwritten() {
    let d = tempfile::tempdir().unwrap();
    let (raw_out, snap) = setup(d.path());
    let before = std::fs::read(&raw_out).unwrap();
    let (s, blobs) = raw::load(&raw_out).unwrap();
    let rb = Rulebook::parse(RB).unwrap();
    let cv = clean::clean(&s, &blobs, &rb, RB, &key(1));
    clean::write(&d.path().join("clean.s3a"), &cv).unwrap();
    assert_eq!(std::fs::read(&raw_out).unwrap(), before);
    // Raw bytes are byte-identical to the source file.
    let src = std::fs::read(d.path().join("invoice_oct.csv")).unwrap();
    let it = snap
        .items
        .iter()
        .find(|i| i.file_name == "invoice_oct.csv")
        .unwrap();
    assert_eq!(blobs[&it.item_sha256], src);
    // Sealed archives are never overwritten.
    let refs = [d.path().join("invoice_oct.csv")];
    let refs: Vec<&Path> = refs.iter().map(PathBuf::as_path).collect();
    assert!(raw::intake(&refs, &meta(), 1, &raw_out).is_err());
    assert!(clean::write(&d.path().join("clean.s3a"), &cv).is_err());
}

#[test]
fn tampered_provenance_fails_verify() {
    let d = tempfile::tempdir().unwrap();
    let (raw_out, _) = setup(d.path());
    let (snap, blobs) = raw::load(&raw_out).unwrap();
    let rb = Rulebook::parse(RB).unwrap();
    let cv = clean::clean(&snap, &blobs, &rb, RB, &key(1));

    let mut t = cv.clone();
    t.body.changes[0].approved_by = "someone-else".into();
    assert!(clean::verify(&t, &snap)
        .unwrap_err()
        .contains("hash mismatch"));

    let mut t = cv.clone();
    t.body.records[0]
        .fields
        .insert("amount".into(), "999".into());
    assert!(clean::verify(&t, &snap).is_err());

    // Re-hashing after tampering does not help: signature fails.
    let mut t = cv.clone();
    t.body.review_queue.clear();
    t.cleaned_hash = clean::body_hash(&t.body);
    assert!(clean::verify(&t, &snap).unwrap_err().contains("signature"));

    // Citing a raw item that is not in the snapshot fails.
    let mut t = cv.clone();
    t.body.records[0].raw_item_sha256 = "00".repeat(32);
    t.cleaned_hash = clean::body_hash(&t.body);
    let k = key(1);
    use ed25519_dalek::Signer;
    t.signature = hex::encode(k.sign(t.cleaned_hash.as_bytes()).to_bytes());
    assert!(clean::verify(&t, &snap)
        .unwrap_err()
        .contains("unknown raw item"));

    // Tampered raw archive bytes are detected on load.
    let mut b = std::fs::read(&raw_out).unwrap();
    let pos = b.windows(5).position(|w| w == b"INV-2").unwrap();
    b[pos] = b'X';
    std::fs::write(&raw_out, b).unwrap();
    assert!(raw::load(&raw_out).is_err());
}

#[test]
fn ambiguous_date_needs_rule() {
    let d = tempfile::tempdir().unwrap();
    let rb_s = String::from_utf8(RB.to_vec()).unwrap().replace(
        r#"["MM/DD/YYYY", "YYYY-MM-DD"]"#,
        r#"["MM/DD/YYYY", "DD/MM/YYYY"]"#,
    );
    std::fs::write(
        d.path().join("invoice_a.csv"),
        "invoice_id,date,amount,currency\nA,03/04/2026,1,USD\nB,25/04/2026,1,USD\n",
    )
    .unwrap();
    let f = d.path().join("invoice_a.csv");
    let snap = raw::intake(&[f.as_path()], &meta(), 1, &d.path().join("r.s3a")).unwrap();
    let (_, blobs) = raw::load(&d.path().join("r.s3a")).unwrap();
    let rb = Rulebook::parse(rb_s.as_bytes()).unwrap();
    let body = clean::clean_body(&snap, &blobs, &rb, rb_s.as_bytes());
    assert_eq!(body.records.len(), 1); // 25/04 is unambiguous
    assert!(body.review_queue[0].reason.contains("ambiguous date"));
}

#[test]
fn rulebook_update_must_be_signed_by_human_approver() {
    let human = key(7);
    let agent = key(8);
    let hp = hex::encode(human.verifying_key().to_bytes());
    let ap = hex::encode(agent.verifying_key().to_bytes());
    let base = String::from_utf8(RB.to_vec())
        .unwrap()
        .replace("approvers: []", &format!("approvers: [\"{hp}\"]"))
        .replace("agents: []", &format!("agents: [\"{ap}\"]"));
    let next = base
        .replace("2026.10.1", "2026.10.2")
        .replace("[USD, EUR]", "[USD, EUR, GBP]");
    let u = RulebookUpdate::sign(base.as_bytes(), next.as_bytes(), "accept GBP", &human).unwrap();
    u.verify(base.as_bytes(), next.as_bytes()).unwrap();
    // Agent key is refused even with a valid signature.
    let ua = RulebookUpdate::sign(base.as_bytes(), next.as_bytes(), "x", &agent).unwrap();
    assert!(ua
        .verify(base.as_bytes(), next.as_bytes())
        .unwrap_err()
        .contains("AI agent"));
    // Unknown key refused; edited bytes refused; version must change.
    let other = RulebookUpdate::sign(base.as_bytes(), next.as_bytes(), "x", &key(9)).unwrap();
    assert!(other.verify(base.as_bytes(), next.as_bytes()).is_err());
    assert!(u
        .verify(base.as_bytes(), next.replace("GBP", "JPY").as_bytes())
        .is_err());
    let same = base.replace("[USD, EUR]", "[USD]");
    let us = RulebookUpdate::sign(base.as_bytes(), same.as_bytes(), "x", &human).unwrap();
    assert!(us.verify(base.as_bytes(), same.as_bytes()).is_err());
}

#[test]
fn cli_end_to_end() {
    let d = tempfile::tempdir().unwrap();
    let p = |n: &str| d.path().join(n).display().to_string();
    std::fs::write(p("invoice_oct.csv"), INVOICES).unwrap();
    std::fs::write(p("notes.txt"), "hello").unwrap();
    std::fs::write(p("rb.yaml"), RB).unwrap();
    std::fs::write(p("k.hex"), "11".repeat(32)).unwrap();
    let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let run = s3a_pack_clean::cli::run;
    assert_eq!(
        run(&a(&[
            "ingest",
            &p("invoice_oct.csv"),
            &p("notes.txt"),
            "--rulebook",
            &p("rb.yaml"),
            "--raw-out",
            &p("raw.s3a"),
            "--clean-out",
            &p("clean.s3a"),
            "--key",
            &p("k.hex"),
            "--org",
            "bakery"
        ])),
        0
    );
    assert_eq!(
        run(&a(&[
            "verify",
            &p("raw.s3a"),
            &p("clean.s3a"),
            "--rulebook",
            &p("rb.yaml")
        ])),
        0
    );
    assert_eq!(run(&a(&["queue", &p("clean.s3a")])), 0);
    // Replay with a different rulebook fails verify.
    std::fs::write(
        p("rb2.yaml"),
        String::from_utf8(RB.to_vec())
            .unwrap()
            .replace("2026.10.1", "9"),
    )
    .unwrap();
    assert_eq!(
        run(&a(&[
            "verify",
            &p("raw.s3a"),
            &p("clean.s3a"),
            "--rulebook",
            &p("rb2.yaml")
        ])),
        2
    );
}
