use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(500);

fn temp_archive_path(label: &str) -> PathBuf {
    let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "s3a_cli_test_{}_{}_{}.s3a",
        label,
        std::process::id(),
        id
    ));
    p
}

struct TempArchiveGuard(PathBuf);
impl Drop for TempArchiveGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn test_cli_help_and_stable_usage() {
    let bin = env!("CARGO_BIN_EXE_s3a-cli");
    let output = Command::new(bin)
        .output()
        .expect("Failed to execute s3a-cli");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("STABLE CORE OPS & MAINTENANCE:"));
    assert!(stdout.contains("DEVELOPER REPL, WIRE SERVER & PROTOCOLS:"));
    assert!(stdout.contains("DOMAIN PACK & LAKEHOUSE INTEGRATIONS:"));
    assert!(stdout.contains("BENCHMARKS & STRESS HARNESSES (EXPERIMENTAL):"));
    assert!(stdout.contains("s3a-cli status [--json]"));
    assert!(stdout.contains("s3a-cli doctor [--json]"));
    assert!(stdout.contains("s3a-cli recover"));
    assert!(stdout.contains("s3a-cli query [--json]"));
}

#[test]
fn test_cli_query_and_json_output() {
    let bin = env!("CARGO_BIN_EXE_s3a-cli");
    let p = temp_archive_path("query_json");
    let guard = TempArchiveGuard(p.clone());
    let path_str = p.to_str().unwrap();

    // 1. Insert via CLI query
    let insert_ql = format!(
        "INSERT TELEMETRY (1000, 10, 20, 99.5) INTO \"{}\"",
        path_str
    );
    let out_insert = Command::new(bin)
        .args(["query", "--json", &insert_ql])
        .output()
        .expect("insert query failed");
    assert!(out_insert.status.success());
    let str_insert = String::from_utf8_lossy(&out_insert.stdout);
    assert!(str_insert.contains("\"ok\":true"));
    assert!(str_insert.contains("\"type\":\"insert\""));

    // 2. Query Telemetry via CLI --json
    let sift_ql = format!("SIFT TELEMETRY FROM \"{}\" WHERE SENSOR_ID = 10", path_str);
    let out_sift = Command::new(bin)
        .args(["query", "--json", &sift_ql])
        .output()
        .expect("sift query failed");
    assert!(out_sift.status.success());
    let str_sift = String::from_utf8_lossy(&out_sift.stdout);
    assert!(str_sift.contains("\"ok\":true"));
    assert!(str_sift.contains("\"count\":1"));

    // 3. Status --json
    let out_status = Command::new(bin)
        .args(["status", "--json", path_str])
        .output()
        .expect("status failed");
    assert!(out_status.status.success());
    let str_status = String::from_utf8_lossy(&out_status.stdout);
    assert!(str_status.contains("\"ok\":true"));
    assert!(str_status.contains("\"tile_count\":1"));
    assert!(str_status.contains("\"active_slot\":"));

    // 4. Doctor --json
    let out_doc = Command::new(bin)
        .args(["doctor", "--json", path_str])
        .output()
        .expect("doctor failed");
    assert!(out_doc.status.success());
    let str_doc = String::from_utf8_lossy(&out_doc.stdout);
    assert!(str_doc.contains("\"ok\":true"));
    assert!(str_doc.contains("\"health\":\"HEALTHY\""));
    assert!(str_doc.contains("\"tile_crc_errors\":0"));

    // 5. Verify --json
    let out_verify = Command::new(bin)
        .args(["verify", "--json", path_str])
        .output()
        .expect("verify failed");
    assert!(out_verify.status.success());
    let str_verify = String::from_utf8_lossy(&out_verify.stdout);
    assert!(str_verify.contains("\"ok\":true"));
    assert!(str_verify.contains("\"status\":\"verified\""));

    // 6. Recover
    let out_recover = Command::new(bin)
        .args(["recover", path_str])
        .output()
        .expect("recover failed");
    assert!(out_recover.status.success());
    let str_recover = String::from_utf8_lossy(&out_recover.stdout);
    assert!(str_recover.contains("Recovery SUCCESS"));

    drop(guard);
}

#[test]
fn test_cli_seal_anchor_upgrade_and_verify() {
    let out_v_json = anchor_confirm_flow(false);
    assert_eq!(
        out_v_json.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out_v_json.stdout)
    );
    let str_v_json = String::from_utf8_lossy(&out_v_json.stdout);
    assert!(str_v_json.contains("\"status\":\"confirmed\""));
    assert!(str_v_json.contains("\"block_height\":899123"));
    assert!(!str_v_json.contains("\"match\":\"no\""), "{}", str_v_json);
    assert!(str_v_json.contains("\"anchors_ok\":true"));
}

#[test]
fn test_cli_anchor_wrong_merkle_root_exits_4() {
    let out = anchor_confirm_flow(true);
    assert_eq!(out.status.code(), Some(4));
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("Bitcoin block Merkle root mismatch"), "{}", s);
    assert!(s.contains("\"anchors_ok\":false"));
}

/// Seal, anchor to two mock calendars, confirm anchor #1 in a mock block, upgrade,
/// and return `verify --anchor --json`. `wrong_root` puts a different Merkle root in the header.
fn anchor_confirm_flow(wrong_root: bool) -> std::process::Output {
    use s3a_engine::anchor::mock_calendar::MockOtsCalendar;

    let bin = env!("CARGO_BIN_EXE_s3a-cli");
    let raw = temp_archive_path("anchor_raw");
    let sealed = temp_archive_path("anchor_sealed");
    let _g1 = TempArchiveGuard(raw.clone());
    let _g2 = TempArchiveGuard(sealed.clone());

    let raw_str = raw.to_str().unwrap();
    let sealed_str = sealed.to_str().unwrap();

    // 1. Create raw archive
    let insert_ql = format!(
        "INSERT TELEMETRY (2000, 11, 22, 123.45) INTO \"{}\"",
        raw_str
    );
    let out_insert = Command::new(bin)
        .args(["query", "--json", &insert_ql])
        .output()
        .expect("insert failed");
    assert!(out_insert.status.success());

    // 2. Seal archive
    let out_seal = Command::new(bin)
        .args(["seal", raw_str, sealed_str])
        .output()
        .expect("seal failed");
    assert!(out_seal.status.success());
    let str_seal = String::from_utf8_lossy(&out_seal.stdout);
    assert!(str_seal.contains("Sealed S3A archive created successfully"));

    // 3. Start mock calendars
    let cal1 = MockOtsCalendar::start();
    let cal2 = MockOtsCalendar::start();
    let cals = format!("{},{}", cal1.url(), cal2.url());

    // 4. Anchor to mock calendars
    let out_anchor = Command::new(bin)
        .args(["anchor", sealed_str, "--calendars", &cals])
        .output()
        .expect("anchor failed");
    assert!(out_anchor.status.success());
    let str_anchor = String::from_utf8_lossy(&out_anchor.stdout);
    assert!(str_anchor.contains("Successfully anchored"));

    // 5. Verify anchor in pending state
    let out_verify = Command::new(bin)
        .args(["verify", "--anchor", sealed_str])
        .output()
        .expect("verify anchor failed");
    assert!(out_verify.status.success());
    let str_verify = String::from_utf8_lossy(&out_verify.stdout);
    assert!(str_verify.contains("Trust Anchor Verification"));
    assert!(str_verify.contains("Status:      pending"));
    assert!(str_verify.contains("Match:       yes"));

    // 6. Confirm in calendar and upgrade anchor
    let rep = s3a_engine::verify_sealed_archive(&sealed, None).expect("verify");
    let detached =
        s3a_engine::anchor::DetachedTimestampFile::from_reader(&rep.anchors[0].proof_bytes[..])
            .expect("parse");
    let attestations = s3a_engine::anchor::collect_attestations(&detached.timestamp.first_step);
    let commitment = attestations
        .iter()
        .find_map(|(att, hash)| {
            if matches!(att, s3a_engine::anchor::Attestation::Pending { .. }) {
                Some(*hash)
            } else {
                None
            }
        })
        .expect("expected pending attestation");

    // The mock calendar attests the commitment directly (no ops after it), so the
    // block's Merkle root must equal the commitment, in header (internal) byte order,
    // as OpenTimestamps' BitcoinBlockHeaderAttestation checks `digest == hashMerkleRoot`.
    let block_height = 899123u64;
    let mut block_hash: [u8; 32] = commitment.try_into().expect("32-byte commitment");
    if wrong_root {
        block_hash[0] ^= 0xff;
    }
    let header = MockOtsCalendar::make_mock_block_header(block_hash);
    cal1.confirm(&hex::encode(commitment), block_height, header);
    cal2.confirm(&hex::encode(commitment), block_height, header);

    let upgrade_cal = rep.anchors[0].uri.clone().unwrap_or_else(|| cal1.url());
    let out_up = Command::new(bin)
        .args(["anchor", "upgrade", sealed_str, "--calendars", &upgrade_cal])
        .output()
        .expect("upgrade anchor failed");
    assert!(out_up.status.success());

    // 7. Verify anchor in confirmed state (--json)
    let out_v_json = Command::new(bin)
        .args([
            "verify",
            "--anchor",
            "--esplora",
            &cal1.url(),
            "--json",
            sealed_str,
        ])
        .output()
        .expect("verify json failed");
    out_v_json
}

#[test]
fn test_cli_scholar_recheck_command() {
    use s3a_core::ADDON_EDU;
    use s3a_engine::S3ACrudEngine;
    use s3a_pack_scholar::{
        CitationFingerprintRecord, MockScholarServer, ScholarCrudExt, CONTENT_KIND_PDF,
    };

    let bin = env!("CARGO_BIN_EXE_s3a-cli");
    let server = MockScholarServer::start();
    let archive = temp_archive_path("scholar_recheck_cli");
    let _g = TempArchiveGuard(archive.clone());
    let archive_str = archive.to_str().unwrap();

    // 1. Create S3A archive with ADDON_EDU
    let engine = S3ACrudEngine::open_or_create(&archive).unwrap();
    engine.enable_addon(ADDON_EDU).unwrap();

    let valid_bytes = b"%PDF-1.4 Mock valid content for research paper";

    // 2. Insert fingerprints
    let fp1 = CitationFingerprintRecord::from_content(
        101,
        "10.1000/101",
        valid_bytes,
        CONTENT_KIND_PDF,
        &format!("{}/paper_ok.pdf", server.addr),
        1,
        1600000000,
    );
    let fp2 = CitationFingerprintRecord::from_content(
        102,
        "10.1000/102",
        b"%PDF-1.4 Content before tampering",
        CONTENT_KIND_PDF,
        &format!("{}/paper_changed.pdf", server.addr),
        1,
        1600000000,
    );
    let fp3 = CitationFingerprintRecord::from_content(
        103,
        "10.1000/retracted",
        valid_bytes,
        CONTENT_KIND_PDF,
        &format!("{}/paper_ok.pdf", server.addr),
        1,
        1600000000,
    );
    let fp4 = CitationFingerprintRecord::from_content(
        104,
        "10.1000/104",
        valid_bytes,
        CONTENT_KIND_PDF,
        "http://127.0.0.1:9/unreachable.pdf",
        1,
        1600000000,
    );

    engine
        .create_citation_fingerprints(&[fp1, fp2, fp3, fp4])
        .unwrap();

    // 3. Run s3a-cli scholar recheck in offline mode
    let out_offline = Command::new(bin)
        .args(["scholar", "recheck", archive_str])
        .output()
        .expect("scholar recheck offline failed");
    assert!(out_offline.status.success());
    let str_offline = String::from_utf8_lossy(&out_offline.stdout);
    assert!(str_offline.contains("Summary: 4 unchanged, 0 changed, 0 retracted, 0 unreachable."));

    // 4. Run s3a-cli scholar recheck in online mode against mock server
    let url1 = format!("101=10.1000/101,{}/paper_ok.pdf", server.addr);
    let url2 = format!("102=10.1000/102,{}/paper_changed.pdf", server.addr);
    let url3 = format!("103=10.1000/retracted,{}/paper_ok.pdf", server.addr);
    let url4 = "104=10.1000/104,http://127.0.0.1:9/unreachable.pdf".to_string();

    let out_online = Command::new(bin)
        .args([
            "scholar",
            "recheck",
            archive_str,
            "--online",
            "--endpoint",
            &server.addr,
            "--url",
            &url1,
            "--url",
            &url2,
            "--url",
            &url3,
            "--url",
            &url4,
        ])
        .output()
        .expect("scholar recheck online failed");
    assert!(out_online.status.success());
    let str_online = String::from_utf8_lossy(&out_online.stdout);
    assert!(str_online.contains("1 unchanged, 1 changed, 1 retracted, 1 unreachable"));
    assert!(str_online.contains("MARK_NEEDS_REVIEW"));
    assert!(str_online.contains("3 policy marks appended"));
}

#[test]
fn test_cli_scholar_proof_and_export_bib() {
    use s3a_core::ADDON_EDU;
    use s3a_engine::anchor::mock_calendar::MockOtsCalendar;
    use s3a_engine::S3ACrudEngine;
    use s3a_pack_scholar::{
        AcademicPaperRecord, CitationFingerprintRecord, ScholarCrudExt, ScholarNoteClaimRecord,
        CLAIM_KIND_EMPIRICAL, CONTENT_KIND_PDF,
    };

    let bin = env!("CARGO_BIN_EXE_s3a-cli");
    let raw = temp_archive_path("scholar_proof_raw");
    let sealed = temp_archive_path("scholar_proof_sealed");
    let _g1 = TempArchiveGuard(raw.clone());
    let _g2 = TempArchiveGuard(sealed.clone());
    let raw_str = raw.to_str().unwrap();
    let sealed_str = sealed.to_str().unwrap();

    let bib_out = std::env::temp_dir().join(format!("test_bib_{}.bib", std::process::id()));
    let proof_out =
        std::env::temp_dir().join(format!("test_bib_{}.bib.proof.json", std::process::id()));
    let _g3 = TempArchiveGuard(bib_out.clone());
    let _g4 = TempArchiveGuard(proof_out.clone());

    // 1. Create S3A archive with ADDON_EDU
    let engine = S3ACrudEngine::open_or_create(&raw).unwrap();
    engine.enable_addon(ADDON_EDU).unwrap();

    // 2. Insert paper and citation fingerprint
    let paper = AcademicPaperRecord::new(1001, 1600000000, 1, 0.1, 0.2, 0.3, 10, 2024, 1, 1, 0, 0);
    engine.create_academic_papers(&[paper]).unwrap();

    let fp = CitationFingerprintRecord::from_content(
        1001,
        "10.1000/1001",
        b"%PDF-1.4 Empirical research finding text data",
        CONTENT_KIND_PDF,
        "https://example.org/paper.pdf",
        2,
        1600000000,
    );
    engine.create_citation_fingerprints(&[fp]).unwrap();

    // 3. Insert claim with evidence link
    let quote = "Key finding on page 4";
    let claim = ScholarNoteClaimRecord::new(
        42,
        1600000000,
        [0, 0],
        0xDEADBEEF,
        0.98,
        CLAIM_KIND_EMPIRICAL,
        quote,
    )
    .with_evidence(
        1001, // citation_fingerprint_id
        4,
        120,
        quote.len() as u32,
        quote,
        0xDEADBEEF,
        [0x1122334455667788, 0x99AABBCCDDEEFF00],
    );
    engine.create_note_claims(&[claim]).unwrap();
    drop(engine);

    // 4. Seal archive
    let out_seal = Command::new(bin)
        .args(["seal", raw_str, sealed_str])
        .output()
        .expect("seal failed");
    assert!(out_seal.status.success());

    // 5. Anchor sealed archive
    let cal1 = MockOtsCalendar::start();
    let cal2 = MockOtsCalendar::start();
    let cals = format!("{},{}", cal1.url(), cal2.url());
    let out_anchor = Command::new(bin)
        .args(["anchor", sealed_str, "--calendars", &cals])
        .output()
        .expect("anchor failed");
    assert!(out_anchor.status.success());

    // 6. Test human-readable proof
    let out_proof = Command::new(bin)
        .args(["scholar", "proof", sealed_str, "--claim", "42"])
        .output()
        .expect("scholar proof failed");
    assert!(out_proof.status.success());
    let str_proof = String::from_utf8_lossy(&out_proof.stdout);
    assert!(str_proof.contains("Claim ID:              #42"));
    assert!(str_proof.contains("EMPIRICAL"));
    assert!(str_proof.contains("Passage Locator:       Page 4, Chars 120..141"));
    assert!(str_proof.contains("Source Fingerprint:    Paper 0x3e9 (pdf) [Crossref v2]"));
    assert!(str_proof.contains("Trust Anchor:          opentimestamps (pending)"));

    // 7. Test JSON-LD proof
    let out_jsonld = Command::new(bin)
        .args(["scholar", "proof", sealed_str, "--claim", "42", "--json-ld"])
        .output()
        .expect("scholar proof json-ld failed");
    assert!(out_jsonld.status.success());
    let str_jsonld = String::from_utf8_lossy(&out_jsonld.stdout);
    assert!(str_jsonld.contains("\"@context\""));
    assert!(str_jsonld.contains("\"prov:Entity\""));
    assert!(str_jsonld.contains("\"claimId\""));
    assert!(str_jsonld.contains("\"sourceFingerprint\""));
    assert!(str_jsonld.contains("\"snapshotAnchor\""));

    // 8. Test export-bib with companion proof
    let bib_str = bib_out.to_str().unwrap();
    let out_bib = Command::new(bin)
        .args([
            "scholar",
            "export-bib",
            sealed_str,
            "--out",
            bib_str,
            "--format",
            "bibtex",
        ])
        .output()
        .expect("export-bib failed");
    assert!(out_bib.status.success());
    assert!(bib_out.exists());
    assert!(proof_out.exists());

    let bib_contents = std::fs::read_to_string(&bib_out).unwrap();
    assert!(bib_contents.contains("@article{paper_1001,"));

    let proof_contents = std::fs::read_to_string(&proof_out).unwrap();
    assert!(proof_contents.contains("\"snapshot_fingerprint\""));
    assert!(proof_contents.contains("\"references\""));
    assert!(proof_contents.contains("10.1000/1001"));
}
