//! Integration round-trip tests for S3A Technical Data Package (TDP): PDF and STEP.

use std::fs;
use std::path::Path;
use tempfile::tempdir;

use s3a_engine::{MmapReader, TileWriter};
use s3a_pack_tdp::pdf::create_test_pdf;
use s3a_pack_tdp::store::{store_pdf, store_step};
use s3a_pack_tdp::{
    detect_mime_and_ext, export_bom_csv, export_bom_json, extract_pdf_metadata, parse_step,
};

#[test]
fn test_tdp_fixtures_roundtrip() {
    let fixture_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    fs::create_dir_all(&fixture_dir).unwrap();

    let pdf_fixture_path = fixture_dir.join("sample.pdf");
    if !pdf_fixture_path.exists() {
        let pdf_data = create_test_pdf("S3A Gearbox TDP Technical Drawing");
        fs::write(&pdf_fixture_path, &pdf_data).unwrap();
    }

    let step_fixture_path = fixture_dir.join("sample.step");
    assert!(step_fixture_path.exists(), "sample.step fixture must exist");

    let pdf_bytes = fs::read(&pdf_fixture_path).expect("Read sample.pdf");
    let step_bytes = fs::read(&step_fixture_path).expect("Read sample.step");

    // 1. Verify MIME detection
    assert_eq!(detect_mime_and_ext(&pdf_bytes), ("application/pdf", "pdf"));
    assert_eq!(
        detect_mime_and_ext(&step_bytes),
        ("application/step", "step")
    );

    // 2. Test PDF Metadata Extraction
    let pdf_meta = extract_pdf_metadata("drawings/gearbox.pdf", &pdf_bytes).unwrap();
    assert_eq!(pdf_meta.page_count, 1);
    assert_eq!(
        pdf_meta.title.as_deref(),
        Some("S3A Gearbox TDP Technical Drawing")
    );
    assert_eq!(pdf_meta.mime_type, "application/pdf");
    assert_eq!(pdf_meta.size_bytes, pdf_bytes.len());
    assert!(!pdf_meta.sha256.is_empty());

    // 3. Test STEP AP242 Parsing & BOM Generation
    let step_tree = parse_step(&step_bytes).unwrap();
    assert_eq!(step_tree.root_part_number.as_deref(), Some("ASM-1000"));
    assert_eq!(
        step_tree.header.originating_system.as_deref(),
        Some("Creo Parametric")
    );
    assert_eq!(step_tree.parts.len(), 4); // Root + 3 components (shaft, bearing, seal)

    let bom_csv = export_bom_csv(&step_tree);
    assert!(bom_csv.contains("part_number,name,parent,quantity"));
    assert!(bom_csv.contains("ASM-1000,Main Gearbox Assembly,,1"));
    assert!(bom_csv.contains("BRG-6205,Deep Groove Ball Bearing,ASM-1000,2"));
    assert!(bom_csv.contains("SHAFT-001,Input Drive Shaft,ASM-1000,1"));
    assert!(bom_csv.contains("SEAL-35X50,Radial Shaft Seal Ring,ASM-1000,1"));

    let bom_json = export_bom_json(&step_tree).unwrap();
    assert!(bom_json.contains("\"part_number\": \"ASM-1000\""));
    assert!(bom_json.contains("\"part_number\": \"BRG-6205\""));

    // 4. Test S3A Vault Storage & Round-trip
    let tmp = tempdir().unwrap();
    let vault_path = tmp.path().join("tdp_test.s3a");
    TileWriter::create(&vault_path).unwrap();

    // Store PDF
    let stored_pdf_meta = store_pdf(&vault_path, "drawings/gearbox.pdf", &pdf_bytes).unwrap();
    assert_eq!(stored_pdf_meta.sha256, pdf_meta.sha256);

    // Store STEP
    let stored_step_tree = store_step(&vault_path, "cad/gearbox.step", &step_bytes).unwrap();
    assert_eq!(
        stored_step_tree.root_part_number,
        step_tree.root_part_number
    );

    // Verify records exist in vault via MmapReader
    let reader = MmapReader::open(&vault_path).unwrap();
    assert!(reader.tile_count() >= 4); // 2 primary + 2 sidecars/metadata
}
