//! S3A Standalone Research Agent Harness & Autonomous Execution Engine.
//! Replaces OpenHarness, Gno Vault, LightRAG, SQL LRS, and LearnMCP-xAPI into a unified,
//! zero-overhead Rust engine operating over S3A Hyper-Tiles and SRS Snapshots.

pub mod agent;
pub mod srs;
pub mod mcp;

pub use agent::{ResearchLoopEngine, ResearchAction, EpistemicConfidenceTier};
pub use srs::SrsContainerManager;
pub use mcp::S3AMcpDispatcher;

#[cfg(test)]
mod tests {
    use super::*;
    use s3a_engine::S3ACrudEngine;
    use s3a_core::SrsWorkspaceRecord;

    struct TempArchive {
        path: std::path::PathBuf,
    }

    impl TempArchive {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
            let mut path = std::env::temp_dir();
            path.push(format!("s3a_harness_test_{}_{}_{}.s3a", std::process::id(), ts, id));
            Self { path }
        }
    }

    impl Drop for TempArchive {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    #[test]
    fn test_harness_autonomous_research_loop_and_gates() {
        let live_db = TempArchive::new();
        let engine = S3ACrudEngine::open_or_create(&live_db.path).unwrap();

        let session_uuid = [0x11223344, 0x55667788];
        let loop_engine = ResearchLoopEngine::new(session_uuid, 42);

        // 1. Ingest paper
        let paper_coord = loop_engine.register_paper(&engine, 0x99998888, [0.2, 0.4, 0.6], 100, 2025, 1, true).unwrap();
        assert_eq!(paper_coord.level, 0);

        // 2. High confidence AI step (no gate)
        assert!(!loop_engine.requires_human_gate(0.85, false));
        let ai_coord = loop_engine.record_ai_step(&engine, 1, 0.85, s3a_core::S3ACoordinate::new(0, 0, 0), 555).unwrap();

        // 3. Low confidence AI step (triggers gate)
        assert!(loop_engine.requires_human_gate(0.45, false));

        // 4. Human oversight decision
        let human_coord = loop_engine.record_human_decision(&engine, 0x05E8_0001, 1, 1.0, ai_coord, 0x99998888 as u32).unwrap();
        assert_eq!(human_coord.level, 0);

        // 5. Epistemic tier mapping
        let tier = EpistemicConfidenceTier::from_score(0.85);
        assert_eq!(tier, EpistemicConfidenceTier::High);
        assert!(tier.epistemic_phrase().contains("Substantial"));
    }

    #[test]
    fn test_harness_mcp_dispatcher_json_rpc() {
        let live_db = TempArchive::new();
        let path_str = live_db.path.to_str().unwrap();

        // 1. Initialize
        let init_req = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#;
        let init_resp = S3AMcpDispatcher::handle_request(init_req, path_str).unwrap();
        assert!(init_resp.contains("s3a-academic-harness"));

        // 2. Tools List
        let list_req = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
        let list_resp = S3AMcpDispatcher::handle_request(list_req, path_str).unwrap();
        assert!(list_resp.contains("s3a_search_library"));
        assert!(list_resp.contains("s3a_get_synthesis_matrix"));
        assert!(list_resp.contains("s3a_export_srs"));
    }

    #[test]
    fn test_harness_srs_packaging_and_mounting() {
        let live_db = TempArchive::new();
        let snap_file = TempArchive::new();

        let engine = S3ACrudEngine::open_or_create(&live_db.path).unwrap();
        let loop_engine = ResearchLoopEngine::new([1, 2], 100);
        let _ = loop_engine.register_paper(&engine, 0x12345, [0.1, 0.2, 0.3], 50, 2024, 1, true).unwrap();

        let p_uuid = [0xCAFE, 0xBABE];
        let s_uuid = [0xDEAD, 0xBEEF];
        let ws = SrsWorkspaceRecord::new(p_uuid, 888, 1, [0.1, 0.2, 0.3], 1);

        // Package SRS snapshot
        let packaged = SrsContainerManager::package_project(&live_db.path, &snap_file.path, p_uuid, s_uuid, [0, 0], Some(&ws)).unwrap();
        assert_eq!(packaged.papers.len(), 1);
        assert!(packaged.workspace.is_some());

        // Zero-copy mount
        let mounted = SrsContainerManager::mount_snapshot(&snap_file.path).unwrap();
        assert_eq!(mounted.manifest.project_uuid, p_uuid);
        assert_eq!(mounted.papers.len(), 1);
        assert_eq!(mounted.papers[0].paper_id, 0x12345);
    }
}
