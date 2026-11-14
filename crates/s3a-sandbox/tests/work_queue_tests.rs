use std::sync::{Arc, Barrier};
use std::thread::{sleep, spawn};
use std::time::Duration;

use rand_core::{OsRng, RngCore};
use serde_json::json;
use tempfile::tempdir;

use s3a_sandbox::{
    compute_record_signature, JobOutcome, JobRecord, JobState, SandboxWorker, WorkQueue,
    RECORD_TYPE_JOB,
};

fn create_sample_job(kind: &str, inputs: serde_json::Value) -> JobRecord {
    let mut job_id = [0u8; 16];
    OsRng.fill_bytes(&mut job_id);

    let mut requested_by = [0u8; 16];
    OsRng.fill_bytes(&mut requested_by);

    let mut grant = [0u8; 16];
    OsRng.fill_bytes(&mut grant);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut job = JobRecord {
        id: job_id,
        kind: kind.to_string(),
        inputs,
        requested_by,
        grant,
        created_at: now,
        signature: [0u8; 64],
    };
    job.signature = compute_record_signature(
        RECORD_TYPE_JOB,
        &job_id,
        &serde_json::to_vec(&job.inputs).unwrap_or_default(),
        &requested_by,
    );
    job
}

#[test]
fn test_claim_lease_expiry() {
    let dir = tempdir().unwrap();
    let vault_path = dir.path().join("vault.s3a");
    let queue = WorkQueue::new(&vault_path);

    let job = create_sample_job(
        "render_step_preview",
        json!({
            "step_content": "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('test.stp','2026-11-13',('Author'),('Org'),'','','');\nFILE_SCHEMA(('AP242_MANAGED_MODEL_BASED_3D_ENGINEERING_MIM_LF'));\nENDSEC;\nDATA;\n#10=PRODUCT('PRT-001','Bracket','',(#11));\n#11=PRODUCT_CONTEXT('',#12,'mechanical');\n#12=APPLICATION_CONTEXT('core data');\nENDSEC;\nEND-ISO-10303-21;\n"
        }),
    );
    let job_id = queue.enqueue(&job).unwrap();

    let worker1_id = [1u8; 16];
    let worker2_id = [2u8; 16];

    // Worker 1 claims job with 1 second lease
    let claimed = queue.claim_next(&worker1_id, 1, None).unwrap();
    assert!(claimed.is_some());
    assert_eq!(claimed.unwrap().id, job_id);

    // Immediately, worker 2 attempts to claim: should be None because lease is active
    let attempt = queue.claim_next(&worker2_id, 1, None).unwrap();
    assert!(attempt.is_none());

    // Wait 2 seconds for lease to expire
    sleep(Duration::from_secs(2));

    // Worker 2 attempts to claim again: now lease has expired, so Worker 2 claims it!
    let claimed2 = queue.claim_next(&worker2_id, 10, None).unwrap();
    assert!(claimed2.is_some());
    assert_eq!(claimed2.unwrap().id, job_id);

    let view = queue.get_job(&job_id).unwrap().unwrap();
    assert_eq!(view.worker_id, worker2_id);
    assert_eq!(view.retry_count, 2); // First claim = 1, retry claim = 2
}

#[test]
fn test_crash_mid_job() {
    let dir = tempdir().unwrap();
    let vault_path = dir.path().join("vault.s3a");
    let queue = WorkQueue::new(&vault_path);

    let job = create_sample_job(
        "extract_pdf_text",
        json!({
            "pdf_content": "%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>\nendobj\n4 0 obj\n<< /Length 44 >>\nstream\nBT /F1 12 Tf 72 712 Td (Hello S3A Sandbox) Tj ET\nendstream\nendobj\nxref\n0 5\n0000000000 65535 f \n0000000009 00000 n \n0000000058 00000 n \n0000000115 00000 n \n0000000204 00000 n \ntrailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n299\n%%EOF\n"
        }),
    );
    let job_id = queue.enqueue(&job).unwrap();

    let worker1_id = [10u8; 16];
    let worker2_id = [20u8; 16];

    // Worker 1 claims job with 1 second lease
    let claimed = queue.claim_next(&worker1_id, 1, None).unwrap();
    assert!(claimed.is_some());

    // Worker 1 simulates a crash (drops without writing result)
    drop(claimed);

    // Lease expires after 2 seconds
    sleep(Duration::from_secs(2));

    // Worker 2 polls and recovers the job
    let worker2 = SandboxWorker::new(queue.clone()).with_worker_id(worker2_id);
    let outcome = worker2.poll_and_run_once().unwrap();
    assert!(outcome.is_some());

    match outcome.unwrap() {
        JobOutcome::Completed {
            job_id: res_id,
            output,
        } => {
            assert_eq!(res_id, job_id);
            assert!(output.get("text").is_some());
        }
        JobOutcome::Failed { error, .. } => {
            panic!(
                "Expected job to complete on recovery, failed with: {}",
                error
            );
        }
    }

    let view = queue.get_job(&job_id).unwrap().unwrap();
    assert_eq!(view.state, JobState::Completed);
    assert_eq!(view.worker_id, worker2_id);
    assert_eq!(view.retry_count, 2);
}

#[test]
fn test_two_workers_no_double_claim() {
    let dir = tempdir().unwrap();
    let vault_path = Arc::new(dir.path().join("vault.s3a"));
    let queue = WorkQueue::new(&*vault_path);

    let job = create_sample_job(
        "render_step_preview",
        json!({
            "step_content": "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('test.stp','2026-11-13',('Author'),('Org'),'','','');\nFILE_SCHEMA(('AP242_MANAGED_MODEL_BASED_3D_ENGINEERING_MIM_LF'));\nENDSEC;\nDATA;\n#10=PRODUCT('PRT-001','Bracket','',(#11));\n#11=PRODUCT_CONTEXT('',#12,'mechanical');\n#12=APPLICATION_CONTEXT('core data');\nENDSEC;\nEND-ISO-10303-21;\n"
        }),
    );
    let job_id = queue.enqueue(&job).unwrap();

    let barrier = Arc::new(Barrier::new(2));

    let queue_a = WorkQueue::new(&*vault_path);
    let barrier_a = barrier.clone();
    let handle_a = spawn(move || {
        barrier_a.wait();
        let worker_a_id = [101u8; 16];
        queue_a.claim_next(&worker_a_id, 30, None).unwrap()
    });

    let queue_b = WorkQueue::new(&*vault_path);
    let barrier_b = barrier.clone();
    let handle_b = spawn(move || {
        barrier_b.wait();
        let worker_b_id = [102u8; 16];
        queue_b.claim_next(&worker_b_id, 30, None).unwrap()
    });

    let res_a = handle_a.join().unwrap();
    let res_b = handle_b.join().unwrap();

    // Exactly one worker must have claimed the job, and the other got None!
    let claimed_count = (res_a.is_some() as usize) + (res_b.is_some() as usize);
    assert_eq!(
        claimed_count, 1,
        "Expected exactly one worker to claim the job, but got {} claims",
        claimed_count
    );

    let view = queue.get_job(&job_id).unwrap().unwrap();
    assert_eq!(view.state, JobState::Claimed);
}

#[test]
fn test_render_step_preview_e2e() {
    let dir = tempdir().unwrap();
    let vault_path = dir.path().join("vault.s3a");
    let queue = WorkQueue::new(&vault_path);

    let step_content = "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('bracket.stp','2026-11-13',('Engineer'),('SpaceCorp'),'','','');\nFILE_SCHEMA(('AP242_MANAGED_MODEL_BASED_3D_ENGINEERING_MIM_LF'));\nENDSEC;\nDATA;\n#10=PRODUCT('PRT-100','Engine Bracket','',(#11));\n#11=PRODUCT_CONTEXT('',#12,'mechanical');\n#12=APPLICATION_CONTEXT('core data');\n#20=PRODUCT_DEFINITION_FORMATION('REV-A','Initial',#10);\n#30=PRODUCT_DEFINITION('DEF-1','bracket def',#20,#11);\nENDSEC;\nEND-ISO-10303-21;\n";

    let job = create_sample_job(
        "render_step_preview",
        json!({
            "step_content": step_content,
            "format": "gltf"
        }),
    );
    let job_id = queue.enqueue(&job).unwrap();

    let worker = SandboxWorker::new(queue.clone());
    let outcome = worker.poll_and_run_once().unwrap();
    assert!(outcome.is_some());

    let view = queue.get_job(&job_id).unwrap().unwrap();
    assert_eq!(view.state, JobState::Completed);
    let res = view.result.unwrap();
    assert!(res.success);
    assert_eq!(res.output["format"], "gltf");
    assert!(res.output["gltf"]["asset"]["version"] == "2.0");
    assert_eq!(res.output["part_count"], 1);
}

#[test]
fn test_render_step_preview_png() {
    let dir = tempdir().unwrap();
    let vault_path = dir.path().join("vault.s3a");
    let queue = WorkQueue::new(&vault_path);

    let step_content = "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('sample.stp','2026-11-13',('Dev'),('S3A'),'','','');\nFILE_SCHEMA(('AP242_MANAGED_MODEL_BASED_3D_ENGINEERING_MIM_LF'));\nENDSEC;\nDATA;\n#10=PRODUCT('PRT-200','Shaft','',(#11));\n#11=PRODUCT_CONTEXT('',#12,'mechanical');\n#12=APPLICATION_CONTEXT('core data');\nENDSEC;\nEND-ISO-10303-21;\n";

    let job = create_sample_job(
        "render_step_preview",
        json!({
            "step_content": step_content,
            "format": "png"
        }),
    );
    let job_id = queue.enqueue(&job).unwrap();

    let worker = SandboxWorker::new(queue.clone());
    let outcome = worker.poll_and_run_once().unwrap();
    assert!(outcome.is_some());

    let view = queue.get_job(&job_id).unwrap().unwrap();
    let res = view.result.unwrap();
    assert_eq!(res.output["format"], "png");
    let hex_str = res.output["png_hex"].as_str().unwrap();
    let png_bytes = hex::decode(hex_str).unwrap();
    // Verify PNG magic
    assert_eq!(
        &png_bytes[0..8],
        &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]
    );
}

#[test]
fn test_extract_pdf_text_e2e() {
    let dir = tempdir().unwrap();
    let vault_path = dir.path().join("vault.s3a");
    let queue = WorkQueue::new(&vault_path);

    let pdf_bytes = s3a_pack_tdp::create_test_pdf("S3A Work Queue PaperLab");

    let job = create_sample_job(
        "extract_pdf_text",
        json!({
            "pdf_content": String::from_utf8_lossy(&pdf_bytes).to_string()
        }),
    );
    let job_id = queue.enqueue(&job).unwrap();

    let worker = SandboxWorker::new(queue.clone());
    let outcome = worker.poll_and_run_once().unwrap();
    assert!(outcome.is_some());

    let view = queue.get_job(&job_id).unwrap().unwrap();
    assert_eq!(view.state, JobState::Completed);
    let res = view.result.unwrap();
    assert!(res.success);
    assert_eq!(res.output["page_count"], 1);
}

#[test]
fn test_sandboxes_never_see_vault_keys() {
    let dir = tempdir().unwrap();
    let vault_path = dir.path().join("vault.s3a");
    let queue = WorkQueue::new(&vault_path);

    // Set a secret ambient environment variable in the host process
    std::env::set_var("S3A_VAULT_SECRET_KEY", "super_secret_master_key_9999");
    std::env::set_var("VAULT_ADMIN_TOKEN", "token_xyz_never_leak");

    let step_content = "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('test.stp','2026-11-13',('Author'),('Org'),'','','');\nFILE_SCHEMA(('AP242_MANAGED_MODEL_BASED_3D_ENGINEERING_MIM_LF'));\nENDSEC;\nDATA;\n#10=PRODUCT('PRT-001','Bracket','',(#11));\n#11=PRODUCT_CONTEXT('',#12,'mechanical');\n#12=APPLICATION_CONTEXT('core data');\nENDSEC;\nEND-ISO-10303-21;\n";

    let job = create_sample_job(
        "render_step_preview",
        json!({
            "step_content": step_content,
            "format": "gltf"
        }),
    );
    let job_id = queue.enqueue(&job).unwrap();

    let worker = SandboxWorker::new(queue.clone());
    let outcome = worker.poll_and_run_once().unwrap();
    assert!(outcome.is_some());

    let view = queue.get_job(&job_id).unwrap().unwrap();
    assert_eq!(view.state, JobState::Completed);

    // Clean up host env
    std::env::remove_var("S3A_VAULT_SECRET_KEY");
    std::env::remove_var("VAULT_ADMIN_TOKEN");
}
