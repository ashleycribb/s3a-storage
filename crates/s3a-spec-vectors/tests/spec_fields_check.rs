//! CI check: Every field defined in the tile-type registry / standard_schema_for_type
//! MUST appear in docs/spec (specifically docs/spec/registry.md).
//! Fails if code defines a field that the spec doesn't document.

use s3a_core::TileType;

const SPEC_REGISTRY_MD: &str = include_str!("../../../docs/spec/registry.md");

#[test]
fn all_code_registry_fields_appear_in_docs_spec() {
    let check_types = [
        TileType::TELEMETRY,
        TileType::EMBEDDING,
        3,  // wearable_micro
        4,  // robotics_kinematic
        5,  // gis_survey_mesh
        6,  // data_availability
        10, // human_lrs
        11, // ai_trace_log
        12, // academic_papers
        13, // research_graph
        14, // srs_workspace
        15, // srs_manifest
        16, // ontology_concept
        17, // ontology_axiom
        18, // agent_registry
        TileType::MARKS,
        20, // note_claims
        TileType::STATEMENT_BODY,
        TileType::STATEMENT_INDEX,
        TileType::DOCUMENT_STORE,
        TileType::ATTACHMENT_BLOB,
        32, // citation_fingerprint
        TileType::RECORD,
        TileType::BLOB,
        TileType::PROVENANCE,
        TileType::COLUMNAR,
        TileType::ARRAY,
        TileType::CONSENT,
        TileType::ERASURE,
        TileType::SCHEMA,
        TileType::RECORD_ROOT,
        TileType::VCS_COMMIT,
        TileType::VCS_REF,
        TileType::VCS_RESOLUTION,
        TileType::GATE_DIRECTORY,
        TileType::GATE_MAILBOX,
        TileType::GRAPH_EDGE,
    ];

    let mut missing = Vec::new();

    for &t in &check_types {
        let schema = s3a_engine::standard_schema_for_type(t);
        for f in &schema.fields {
            let backticked = format!("`{}`", f.name);
            if !SPEC_REGISTRY_MD.contains(&backticked) && !SPEC_REGISTRY_MD.contains(&f.name) {
                missing.push(format!(
                    "Tile {} ({}): field '{}' not found in docs/spec/registry.md",
                    t, schema.name, f.name
                ));
            }
        }
    }

    assert!(
        missing.is_empty(),
        "Code defines fields that docs/spec/registry.md does not document:\n{}",
        missing.join("\n")
    );
}
