//! Graph Query Integration Tests (Prompt 003).
//!
//! Tests the 3 canonical graph use cases:
//! 1. Assembly tree (part -> child part)
//! 2. Citation graph (paper -> cites -> paper), including Scholar Tile 13 mapping
//! 3. Agent grants (agent -> acts_for -> person)
//! 4. 100,000-edge performance benchmark test.

use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use s3a_core::graph::GenericGraphEdge;
use s3a_engine::graph::{append_edges, GraphIndex};
use s3a_engine::ql::{execute_query, QueryResult};
use s3a_engine::TileWriter;

fn tmp_archive_path(test_name: &str) -> PathBuf {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let p = std::env::temp_dir().join(format!("s3a_graph_test_{}_{}.s3a", test_name, now));
    let _ = fs::remove_file(&p);
    p
}

#[test]
fn test_usecase_1_assembly_tree() {
    let path = tmp_archive_path("assembly");

    // Assembly hierarchy:
    // GEARBOX-001 -> SHAFT-MAIN-10 -> BEARING-101 -> BALL-STEEL-01
    // GEARBOX-001 -> HOUSING-TOP-20
    let edges = vec![
        GenericGraphEdge::new(
            "GEARBOX-001",
            "SHAFT-MAIN-10",
            "has_part",
            r#"{"qty":1,"material":"steel"}"#,
            [1u8; 32],
            1700000000,
        ),
        GenericGraphEdge::new(
            "GEARBOX-001",
            "HOUSING-TOP-20",
            "has_part",
            r#"{"qty":1,"material":"aluminum"}"#,
            [1u8; 32],
            1700000001,
        ),
        GenericGraphEdge::new(
            "SHAFT-MAIN-10",
            "BEARING-101",
            "has_part",
            r#"{"qty":2,"material":"alloy"}"#,
            [1u8; 32],
            1700000002,
        ),
        GenericGraphEdge::new(
            "BEARING-101",
            "BALL-STEEL-01",
            "has_part",
            r#"{"qty":16,"diameter_mm":8.0}"#,
            [1u8; 32],
            1700000003,
        ),
    ];

    append_edges(&path, &edges).expect("append assembly edges");

    // Rebuild index from disk
    let index = GraphIndex::build_from_file(&path).expect("build index");
    assert_eq!(index.len(), 4);

    // 1. NEIGHBORS
    let neighbors = index.neighbors("GEARBOX-001");
    assert_eq!(neighbors.len(), 2);
    let targets: Vec<&str> = neighbors.iter().map(|e| e.to.as_str()).collect();
    assert!(targets.contains(&"SHAFT-MAIN-10"));
    assert!(targets.contains(&"HOUSING-TOP-20"));

    // 2. TRAVERSE with depth
    let (visited, traversed) = index.traverse("GEARBOX-001", Some("has_part"), 3);
    assert_eq!(traversed.len(), 4);
    assert_eq!(
        visited,
        vec![
            "GEARBOX-001",
            "SHAFT-MAIN-10",
            "HOUSING-TOP-20",
            "BEARING-101",
            "BALL-STEEL-01"
        ]
    );

    // 3. PATH
    let (path_nodes, path_edges) = index
        .find_path("GEARBOX-001", "BALL-STEEL-01", Some("has_part"))
        .expect("path must exist");
    assert_eq!(
        path_nodes,
        vec![
            "GEARBOX-001",
            "SHAFT-MAIN-10",
            "BEARING-101",
            "BALL-STEEL-01"
        ]
    );
    assert_eq!(path_edges.len(), 3);

    // 4. Test through S3A-QL query engine
    let ql_neighbors = format!(
        "NEIGHBORS \"GEARBOX-001\" FROM \"{}\"",
        path.to_string_lossy()
    );
    match execute_query(&ql_neighbors).expect("query neighbors") {
        QueryResult::GraphNeighbors { node, neighbors } => {
            assert_eq!(node, "GEARBOX-001");
            assert_eq!(neighbors.len(), 2);
        }
        other => panic!("unexpected result: {:?}", other),
    }

    let ql_path = format!(
        "PATH \"GEARBOX-001\" TO \"BALL-STEEL-01\" VIA \"has_part\" FROM \"{}\"",
        path.to_string_lossy()
    );
    match execute_query(&ql_path).expect("query path") {
        QueryResult::GraphPath {
            from,
            to,
            found,
            nodes,
            edges,
        } => {
            assert_eq!(from, "GEARBOX-001");
            assert_eq!(to, "BALL-STEEL-01");
            assert!(found);
            assert_eq!(nodes.len(), 4);
            assert_eq!(edges.len(), 3);
        }
        other => panic!("unexpected result: {:?}", other),
    }

    let _ = fs::remove_file(&path);
}

#[test]
fn test_usecase_2_citation_graph_with_tile13_interop() {
    let path = tmp_archive_path("citations");

    // 1. Add modern Core Tile 87 edges
    let modern_edges = vec![
        GenericGraphEdge::new(
            "doi:10.1000/transformer",
            "doi:10.1000/attention",
            "cites",
            r#"{"context":"prior work"}"#,
            [0u8; 32],
            1500000000,
        ),
        GenericGraphEdge::new(
            "doi:10.1000/attention",
            "doi:10.1000/seq2seq",
            "cites",
            r#"{"context":"architecture"}"#,
            [0u8; 32],
            1400000000,
        ),
    ];
    append_edges(&path, &modern_edges).expect("append modern edges");

    // 2. Append legacy Scholar Tile 13 raw records into the same file to test interop without breaking Tile 13 files!
    #[repr(C, align(8))]
    #[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
    struct RawTile13Edge {
        subject_hash: u64,
        object_hash: u64,
        timestamp_sec: u64,
        hilbert_coord: u64,
        predicate_id: u32,
        weight: f32,
        flags: u64,
        _reserved: [u64; 2],
    }

    let legacy_record = RawTile13Edge {
        subject_hash: 0x1234_5678,
        object_hash: 0x9abc_def0,
        timestamp_sec: 1300000000,
        hilbert_coord: 42,
        predicate_id: 1, // cites
        weight: 0.95,
        flags: 0, // active
        _reserved: [0; 2],
    };

    let mut writer = TileWriter::open_append(&path).expect("open for legacy tile");
    writer
        .enable_addon(s3a_core::ADDON_EDU)
        .expect("enable ADDON_EDU");
    writer
        .write_hyper_tile::<RawTile13Edge>(13, &[legacy_record], None, None)
        .expect("write tile 13");

    // Register pack graph tile extractor for Tile 13
    s3a_engine::graph::register_graph_tile_extractor(std::sync::Arc::new(|tile_type, payload| {
        if tile_type == 13 {
            let chunk_size = std::mem::size_of::<RawTile13Edge>();
            let mut edges = Vec::new();
            for chunk in payload.chunks_exact(chunk_size) {
                let rec: RawTile13Edge = bytemuck::pod_read_unaligned(chunk);
                if (rec.flags & 1) == 0 {
                    let predicate = match rec.predicate_id {
                        1 => "cites",
                        _ => "related_to",
                    };
                    edges.push(s3a_engine::graph::IndexedEdge {
                        from: format!("0x{:016x}", rec.subject_hash),
                        to: format!("0x{:016x}", rec.object_hash),
                        edge_type: predicate.to_string(),
                        properties: format!(r#"{{"predicate_id":{}}}"#, rec.predicate_id),
                        signer: [0u8; 32],
                        timestamp: rec.timestamp_sec,
                    });
                }
            }
            edges
        } else {
            Vec::new()
        }
    }));

    // Rebuild index from archive
    let index = GraphIndex::build_from_file(&path).expect("build index");
    // 2 modern edges + 1 legacy Scholar Tile 13 edge
    assert_eq!(index.len(), 3);

    // Verify Tile 13 mapped edge is queryable
    let legacy_neighbors = index.neighbors("0x0000000012345678");
    assert_eq!(legacy_neighbors.len(), 1);
    assert_eq!(legacy_neighbors[0].to, "0x000000009abcdef0");
    assert_eq!(legacy_neighbors[0].edge_type, "cites");

    // Path on modern edges
    let (path_nodes, _) = index
        .find_path(
            "doi:10.1000/transformer",
            "doi:10.1000/seq2seq",
            Some("cites"),
        )
        .expect("path found");
    assert_eq!(
        path_nodes,
        vec![
            "doi:10.1000/transformer",
            "doi:10.1000/attention",
            "doi:10.1000/seq2seq"
        ]
    );

    let _ = fs::remove_file(&path);
}

#[test]
fn test_usecase_3_agent_grants_authorization_chain() {
    let path = tmp_archive_path("agent_grants");

    // Delegation chain:
    // worker-agent-42 -[acts_for]-> supervisor-agent-10 -[acts_for]-> dept-lead-02 -[acts_for]-> user-alice
    // rogue-agent-99 -[acts_for]-> evil-corp (disconnected from user-alice)
    let edges = vec![
        GenericGraphEdge::new(
            "worker-agent-42",
            "supervisor-agent-10",
            "acts_for",
            r#"{"scope":"compute:read","ttl_sec":3600}"#,
            [2u8; 32],
            1700000000,
        ),
        GenericGraphEdge::new(
            "supervisor-agent-10",
            "dept-lead-02",
            "acts_for",
            r#"{"scope":"compute:*","ttl_sec":86400}"#,
            [3u8; 32],
            1700000001,
        ),
        GenericGraphEdge::new(
            "dept-lead-02",
            "user-alice",
            "acts_for",
            r#"{"scope":"*","owner":true}"#,
            [4u8; 32],
            1700000002,
        ),
        GenericGraphEdge::new(
            "rogue-agent-99",
            "evil-corp",
            "acts_for",
            r#"{"scope":"unauthorized"}"#,
            [9u8; 32],
            1700000003,
        ),
    ];

    append_edges(&path, &edges).expect("append grant edges");

    let index = GraphIndex::build_from_file(&path).expect("build index");

    // Valid chain
    let (chain, _) = index
        .find_path("worker-agent-42", "user-alice", Some("acts_for"))
        .expect("authorization chain must exist");
    assert_eq!(
        chain,
        vec![
            "worker-agent-42",
            "supervisor-agent-10",
            "dept-lead-02",
            "user-alice"
        ]
    );

    // Invalid chain
    assert!(index
        .find_path("rogue-agent-99", "user-alice", Some("acts_for"))
        .is_none());

    let _ = fs::remove_file(&path);
}

#[test]
fn test_100k_edge_performance() {
    let path = tmp_archive_path("perf_100k");

    println!("Generating 100,000 graph edges...");
    let start_gen = Instant::now();
    let edge_count = 100_000;
    let mut edges = Vec::with_capacity(edge_count);

    // Create a 10-level deep tree with branches, plus cross-edges
    for i in 0..edge_count {
        let parent_id = i / 4;
        let child_id = i + 1;
        let edge = GenericGraphEdge::new(
            format!("node_{}", parent_id),
            format!("node_{}", child_id),
            if i % 5 == 0 { "cites" } else { "contains" },
            format!(r#"{{"idx":{}}}"#, i),
            [0u8; 32],
            1700000000 + (i as u64),
        );
        edges.push(edge);
    }
    println!("Generated 100k edges in {:?}", start_gen.elapsed());

    println!("Appending 100,000 edges to vault...");
    let start_write = Instant::now();
    append_edges(&path, &edges).expect("append 100k edges");
    let write_elapsed = start_write.elapsed();
    println!("Wrote 100,000 edges to archive in {:?}", write_elapsed);

    // Rebuild Adjacency Index from disk
    println!("Rebuilding adjacency index from disk across 100,000 edges...");
    let start_open = Instant::now();
    let index = GraphIndex::build_from_file(&path).expect("build 100k index");
    let open_elapsed = start_open.elapsed();
    println!("Adjacency index rebuilt in {:?}", open_elapsed);

    assert_eq!(index.len(), edge_count);
    // Index rebuild must complete well within target (under 2 seconds)
    assert!(
        open_elapsed.as_millis() < 2000,
        "Adjacency index build took {:?}, expected < 2000ms",
        open_elapsed
    );

    // Test NEIGHBORS query performance
    let start_nb = Instant::now();
    let neighbors = index.neighbors("node_0");
    let nb_elapsed = start_nb.elapsed();
    println!(
        "Neighbors query returned {} edges in {:?}",
        neighbors.len(),
        nb_elapsed
    );
    assert_eq!(neighbors.len(), 4);
    assert!(
        nb_elapsed.as_millis() < 50,
        "Neighbors query too slow: {:?}",
        nb_elapsed
    );

    // Test TRAVERSE query performance
    let start_tr = Instant::now();
    let (visited, traversed) = index.traverse("node_0", None, 3);
    let tr_elapsed = start_tr.elapsed();
    println!(
        "Traverse depth 3 visited {} nodes ({} edges) in {:?}",
        visited.len(),
        traversed.len(),
        tr_elapsed
    );
    assert!(visited.len() > 10);
    assert!(
        tr_elapsed.as_millis() < 50,
        "Traverse query too slow: {:?}",
        tr_elapsed
    );

    // Test PATH query performance across multiple hops
    let start_path = Instant::now();
    // node_0 -> node_1 -> node_5 -> node_21 -> node_85
    let path_opt = index.find_path("node_0", "node_85", None);
    let path_elapsed = start_path.elapsed();
    println!("Path query completed in {:?}", path_elapsed);
    let (path_nodes, _) = path_opt.expect("path should exist");
    assert_eq!(path_nodes.len(), 5);
    assert_eq!(path_nodes.first().unwrap(), "node_0");
    assert_eq!(path_nodes.last().unwrap(), "node_85");
    assert!(
        path_elapsed.as_millis() < 50,
        "Path query too slow: {:?}",
        path_elapsed
    );

    let _ = fs::remove_file(&path);
}
