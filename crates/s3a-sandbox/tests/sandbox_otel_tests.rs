//! Tests for OpenTelemetry (OTel) Export in s3a-sandbox.

#[cfg(not(feature = "otel"))]
#[test]
#[allow(clippy::assertions_on_constants)]
fn test_sandbox_otel_feature_disabled_by_default() {
    assert!(!cfg!(feature = "otel"));
}

#[cfg(feature = "otel")]
mod sandbox_otel_tests {
    use s3a_sandbox::otel::{clear_global_exporter, set_global_exporter, InMemorySpanExporter};
    use s3a_sandbox::records::JobRecord;
    use s3a_sandbox::runner::execute_job_in_sandbox;
    use serde_json::json;
    use std::sync::Arc;

    #[test]
    fn test_sandbox_job_emits_span_matching_record_hash() {
        clear_global_exporter();
        let exporter = Arc::new(InMemorySpanExporter::new());
        set_global_exporter(Arc::clone(&exporter) as _);

        let valid_step = "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('Test'),'2;1');\nFILE_NAME('test.stp','2026-11-14',('Author'),('Org'),'','','');\nFILE_SCHEMA(('AP242_MANAGED_MODEL_BASED_3D_ENGINEERING_MIM_LF'));\nENDSEC;\nDATA;\n#1=PRODUCT_DEFINITION('part1','test',#2,#3);\nENDSEC;\nEND-ISO-10303-21;\n";
        let job = JobRecord {
            id: [0x77; 16],
            kind: "render_step_preview".to_string(),
            inputs: json!({
                "step_content": valid_step,
            }),
            requested_by: [0x11; 16],
            grant: [0x22; 16],
            created_at: 1792200000,
            signature: [0x99; 64],
        };

        let expected_hash = job.sha256_digest();

        // Run the job (which executes in-process or via sandbox)
        let res = execute_job_in_sandbox(&job);
        assert!(res.is_ok(), "Job must succeed: {:?}", res);

        // Verify span was emitted and matches job hash
        let spans = exporter.spans();
        assert!(!spans.is_empty(), "Job execution must emit an OTel span");

        let job_span = spans
            .iter()
            .find(|s| s.name == "job:render_step_preview")
            .expect("Must find 'job:render_step_preview' span");

        assert_eq!(
            job_span.record_hash(),
            Some(expected_hash.as_str()),
            "Span record hash must match the job's sha256_digest"
        );
        assert_eq!(
            job_span.agent_id(),
            Some(hex::encode(job.requested_by).as_str())
        );
        assert_eq!(job_span.job_id(), Some(hex::encode(job.id).as_str()));
    }
}
