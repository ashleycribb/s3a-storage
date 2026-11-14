use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn get_bin() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_s3a") {
        return std::path::PathBuf::from(p);
    }
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_s3a-cli") {
        return std::path::PathBuf::from(p);
    }
    panic!("Neither CARGO_BIN_EXE_s3a nor CARGO_BIN_EXE_s3a-cli is set");
}

fn temp_dir() -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("s3a_tdp_test_{}", nanos));
    fs::create_dir_all(&dir).expect("failed to create temp dir");
    dir
}

#[test]
fn test_tdp_cli_open_export_pdf_and_step() {
    let bin = get_bin();
    let tmp = temp_dir();
    let vault_path = tmp.join("tdp_test.s3a");

    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("s3a-pack-tdp/tests/fixtures");
    let pdf_fixture = fixtures_dir.join("sample.pdf");
    let step_fixture = fixtures_dir.join("sample.step");

    assert!(pdf_fixture.exists(), "PDF fixture must exist");
    assert!(step_fixture.exists(), "STEP fixture must exist");

    // 1. s3a init
    let init_out = Command::new(&bin)
        .args(["init", vault_path.to_str().unwrap()])
        .output()
        .expect("s3a init failed");
    assert!(init_out.status.success(), "s3a init must succeed");

    // 2. s3a put PDF
    let put_pdf = Command::new(&bin)
        .args([
            "put",
            vault_path.to_str().unwrap(),
            "drawings/turbine.pdf",
            &format!("@{}", pdf_fixture.to_str().unwrap()),
        ])
        .output()
        .expect("s3a put PDF failed");
    assert!(put_pdf.status.success(), "s3a put PDF must succeed");
    let put_pdf_stdout = String::from_utf8_lossy(&put_pdf.stdout);
    assert!(put_pdf_stdout.contains("OK: stored PDF"));

    // 3. s3a get PDF metadata sidecar
    let get_pdf_meta = Command::new(&bin)
        .args([
            "get",
            vault_path.to_str().unwrap(),
            "drawings/turbine.pdf.meta",
        ])
        .output()
        .expect("s3a get pdf meta failed");
    assert!(get_pdf_meta.status.success());
    let meta_str = String::from_utf8_lossy(&get_pdf_meta.stdout);
    assert!(meta_str.contains("\"mime_type\": \"application/pdf\""));
    assert!(meta_str.contains("\"page_count\": 1"));

    // 4. s3a put STEP
    let put_step = Command::new(&bin)
        .args([
            "put",
            vault_path.to_str().unwrap(),
            "cad/gearbox.step",
            &format!("@{}", step_fixture.to_str().unwrap()),
        ])
        .output()
        .expect("s3a put STEP failed");
    assert!(put_step.status.success(), "s3a put STEP must succeed");
    let put_step_stdout = String::from_utf8_lossy(&put_step.stdout);
    assert!(put_step_stdout.contains("OK: stored STEP"));

    // 5. s3a export BOM as CSV
    let export_bom_csv = Command::new(&bin)
        .args([
            "export",
            vault_path.to_str().unwrap(),
            "cad/gearbox.step",
            "--format",
            "csv",
        ])
        .output()
        .expect("s3a export BOM csv failed");
    assert!(export_bom_csv.status.success());
    let bom_csv_str = String::from_utf8_lossy(&export_bom_csv.stdout);
    assert!(bom_csv_str.contains("part_number,name,parent,quantity"));
    assert!(bom_csv_str.contains("ASM-1000,Main Gearbox Assembly,,1"));
    assert!(bom_csv_str.contains("BRG-6205,Deep Groove Ball Bearing,ASM-1000,2"));

    // 6. s3a export BOM as JSON
    let export_bom_json = Command::new(&bin)
        .args([
            "export",
            vault_path.to_str().unwrap(),
            "cad/gearbox.step",
            "--format",
            "json",
        ])
        .output()
        .expect("s3a export BOM json failed");
    assert!(export_bom_json.status.success());
    let bom_json_str = String::from_utf8_lossy(&export_bom_json.stdout);
    assert!(bom_json_str.contains("\"part_number\": \"ASM-1000\""));

    // 7. s3a export PDF payload to file
    let exported_pdf = tmp.join("exported_turbine.pdf");
    let export_pdf_file = Command::new(&bin)
        .args([
            "export",
            vault_path.to_str().unwrap(),
            "drawings/turbine.pdf",
            "-o",
            exported_pdf.to_str().unwrap(),
        ])
        .output()
        .expect("s3a export PDF to file failed");
    assert!(export_pdf_file.status.success());
    assert!(exported_pdf.exists());
    let original_pdf_bytes = fs::read(&pdf_fixture).unwrap();
    let exported_pdf_bytes = fs::read(&exported_pdf).unwrap();
    assert_eq!(exported_pdf_bytes, original_pdf_bytes);

    // 8. s3a export STEP payload to file
    let exported_step = tmp.join("exported_gearbox.step");
    let export_step_file = Command::new(&bin)
        .args([
            "export",
            vault_path.to_str().unwrap(),
            "cad/gearbox.step",
            "-o",
            exported_step.to_str().unwrap(),
        ])
        .output()
        .expect("s3a export STEP to file failed");
    assert!(export_step_file.status.success());
    assert!(exported_step.exists());
    let original_step_bytes = fs::read(&step_fixture).unwrap();
    let exported_step_bytes = fs::read(&exported_step).unwrap();
    assert_eq!(exported_step_bytes, original_step_bytes);

    // 9. s3a open
    let open_pdf = Command::new(&bin)
        .args(["open", vault_path.to_str().unwrap(), "drawings/turbine.pdf"])
        .output()
        .expect("s3a open failed");
    assert!(open_pdf.status.success(), "s3a open must succeed");
    let open_pdf_stdout = String::from_utf8_lossy(&open_pdf.stdout);
    assert!(open_pdf_stdout.contains("OK: opened") || open_pdf_stdout.contains("OK: wrote"));

    let _ = fs::remove_dir_all(&tmp);
}
