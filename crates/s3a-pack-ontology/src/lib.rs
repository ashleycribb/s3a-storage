//! S3A Domain Pack for Web Ontologies (OWL), Axiom Grounding, and xAPI Profile Verification.
//!
//! Provides first-class binary representations for:
//! - Tile 16: Normalized Ontology Concept Record (`OntologyConceptRecord`, 64 bytes)
//! - Tile 17: Ontology Axiom & Rule Record (`OntologyAxiomRecord`, 64 bytes)

pub mod ext;
pub mod pack;
pub mod records;

pub use ext::OntologyCrudExt;
pub use pack::{register_pack_types, OntologyPack, OntologyTileType, ONTOLOGY_DESCRIPTORS};
pub use records::*;

#[cfg(test)]
mod tests {
    use super::*;
    use s3a_core::ADDON_ONTOLOGY;
    use s3a_engine::S3ACrudEngine;

    struct TempArchive {
        path: std::path::PathBuf,
    }

    impl TempArchive {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let mut path = std::env::temp_dir();
            path.push(format!(
                "s3a_ontology_test_{}_{}_{}.s3a",
                std::process::id(),
                ts,
                id
            ));
            Self { path }
        }
    }

    impl Drop for TempArchive {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    #[test]
    fn test_ontology_pack_metadata() {
        use s3a_core::S3AAddon;
        let pack = OntologyPack;
        assert_eq!(pack.addon_id(), ADDON_ONTOLOGY);
        assert_eq!(pack.name(), "s3a-pack-ontology");
        assert_eq!(pack.tile_range(), 16..=17);
        assert_eq!(pack.descriptors().len(), 2);
    }

    #[test]
    fn test_ontology_write_gate_fails_without_addon() {
        let temp = TempArchive::new();
        let engine = S3ACrudEngine::open_or_create(&temp.path).unwrap();

        let concept =
            OntologyConceptRecord::new(0x1234567890ABCDEF, 0, ONTOLOGY_CITO, KIND_VERB, 1, 100);

        // Attempting to write without ADDON_ONTOLOGY must fail
        let res = engine.create_ontology_concepts(&[concept]);
        assert!(
            res.is_err(),
            "Write must fail closed when ADDON_ONTOLOGY is missing"
        );
    }

    #[test]
    fn test_ontology_concept_and_analogy_roundtrip() {
        let temp = TempArchive::new();
        let engine = S3ACrudEngine::open_or_create(&temp.path).unwrap();
        engine.enable_addon(ADDON_ONTOLOGY).unwrap();

        // 1. Register base concept: IdealGasLaw
        let concept_hash = 0xAA00BB11CC22DD33;
        let concept =
            OntologyConceptRecord::new(concept_hash, 0, ONTOLOGY_PEDAGOGY, KIND_CLASS, 1, 250);
        let c_tile = engine.create_ontology_concepts(&[concept]).unwrap();
        assert_eq!(c_tile, 1);

        // 2. Register approved analogy scaffold: BouncingTennisBalls
        let analogy_hash = 0x5566778899AABBCC;
        let axiom = OntologyAxiomRecord::new(
            concept_hash,
            analogy_hash,
            PREDICATE_HAS_APPROVED_ANALOGY,
            2,      // PedagogicalScaffold
            0x3C00, // 1.0 in FP16
            10,     // HumanLrs
            ADDON_ONTOLOGY as u16,
        );
        let a_tile = engine.create_ontology_axioms(&[axiom]).unwrap();
        assert_eq!(a_tile, 2);

        // 3. Resolve concept by ID
        let resolved = engine.resolve_concept_by_id(concept_hash).unwrap();
        assert!(resolved.is_some());
        let (rec, coord) = resolved.unwrap();
        assert_eq!(rec.concept_id, concept_hash);
        assert_eq!(coord.tile_id, 0);
        assert_eq!(coord.record_offset, 0);

        // 4. Find approved analogies
        let analogies = engine.find_approved_analogies(concept_hash).unwrap();
        assert_eq!(analogies.len(), 1);
        assert_eq!(analogies[0].0.object_concept_id, analogy_hash);
        assert_eq!(analogies[0].0.predicate_id, PREDICATE_HAS_APPROVED_ANALOGY);
    }
}
