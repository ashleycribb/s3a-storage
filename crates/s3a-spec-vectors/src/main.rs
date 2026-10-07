use s3a_spec_vectors::{generate_all_vectors, verify_conformance_vectors};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("============================================================");
    println!("S3A Spec Conformance Vector Generator (v1.1-draft)");
    println!("============================================================");

    // Default output path is ./spec-vectors
    let out_dir = PathBuf::from("spec-vectors");
    println!("Generating conformance vectors into: {}", out_dir.display());

    let cases = generate_all_vectors(&out_dir)?;
    println!("Generated {} test vector cases successfully:", cases.len());
    for case in &cases {
        println!("  - {}.s3a / {}.expected.json", case, case);
    }

    println!("\nExecuting runner verification contract over generated vectors...");
    let verified =
        verify_conformance_vectors(&out_dir).map_err(|e| format!("Verification failed: {}", e))?;
    println!(
        "Verified {}/{} test cases against S3A v1.1 specification gates: ALL PASSED",
        verified,
        cases.len()
    );

    println!("============================================================");
    println!("Successfully generated spec-vectors/ and verified runner contract.");
    Ok(())
}
