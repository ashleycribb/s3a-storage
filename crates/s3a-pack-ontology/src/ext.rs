use std::io;
use s3a_core::{S3ACoordinate, HYPER_TILE_PAYLOAD_SIZE};
use s3a_engine::{MmapReader, S3ACrudEngine, TileWriter};
use crate::pack::OntologyTileType;
use crate::records::{OntologyAxiomRecord, OntologyConceptRecord, PREDICATE_HAS_APPROVED_ANALOGY};

/// High-Level CRUD Extension for S3A Ontology & Axiom Registry.
pub trait OntologyCrudExt {
    fn create_ontology_concepts(&self, records: &[OntologyConceptRecord]) -> io::Result<u64>;
    fn create_ontology_axioms(&self, records: &[OntologyAxiomRecord]) -> io::Result<u64>;
    fn resolve_concept_by_id(&self, concept_id: u64) -> io::Result<Option<(OntologyConceptRecord, S3ACoordinate)>>;
    fn find_approved_analogies(&self, concept_id: u64) -> io::Result<Vec<(OntologyAxiomRecord, S3ACoordinate)>>;
    fn query_axioms_by_subject(&self, subject_id: u64) -> io::Result<Vec<(OntologyAxiomRecord, S3ACoordinate)>>;
}

impl OntologyCrudExt for S3ACrudEngine {
    fn create_ontology_concepts(&self, records: &[OntologyConceptRecord]) -> io::Result<u64> {
        if records.is_empty() {
            return Ok(0);
        }
        let capacity = HYPER_TILE_PAYLOAD_SIZE / std::mem::size_of::<OntologyConceptRecord>();
        if records.len() > capacity {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Records exceed single tile capacity"));
        }

        let filter_keys: Vec<u64> = records.iter().map(|r| r.concept_id).collect();

        let mut writer = TileWriter::open_append(self.file_path())?;
        writer.write_hyper_tile_ext(
            OntologyTileType::ONTOLOGY_CONCEPT,
            records,
            None,
            None,
            Some(&filter_keys),
        )
    }

    fn create_ontology_axioms(&self, records: &[OntologyAxiomRecord]) -> io::Result<u64> {
        if records.is_empty() {
            return Ok(0);
        }
        let capacity = HYPER_TILE_PAYLOAD_SIZE / std::mem::size_of::<OntologyAxiomRecord>();
        if records.len() > capacity {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Records exceed single tile capacity"));
        }

        let mut filter_keys: Vec<u64> = Vec::with_capacity(records.len() * 2);
        for r in records {
            filter_keys.push(r.subject_concept_id);
            filter_keys.push(r.object_concept_id);
        }

        let mut writer = TileWriter::open_append(self.file_path())?;
        writer.write_hyper_tile_ext(
            OntologyTileType::ONTOLOGY_AXIOM,
            records,
            None,
            None,
            Some(&filter_keys),
        )
    }

    fn resolve_concept_by_id(&self, concept_id: u64) -> io::Result<Option<(OntologyConceptRecord, S3ACoordinate)>> {
        let reader = MmapReader::open(self.file_path())?;
        let tile_count = reader.tile_count();

        for i in 0..tile_count {
            let (header, payload) = match reader.get_tile(i as usize) {
                Some(val) => val,
                None => continue,
            };

            if header.tile_type != OntologyTileType::ONTOLOGY_CONCEPT {
                continue;
            }

            let records: &[OntologyConceptRecord] = bytemuck::cast_slice(payload);
            for (rec_idx, rec) in records.iter().enumerate() {
                if rec.concept_id == concept_id && !rec.is_tombstone() {
                    let coord = S3ACoordinate::new(0, i as u32, rec_idx as u32);
                    return Ok(Some((*rec, coord)));
                }
            }
        }
        Ok(None)
    }

    fn find_approved_analogies(&self, concept_id: u64) -> io::Result<Vec<(OntologyAxiomRecord, S3ACoordinate)>> {
        let reader = MmapReader::open(self.file_path())?;
        let tile_count = reader.tile_count();
        let mut results = Vec::new();

        for i in 0..tile_count {
            let (header, payload) = match reader.get_tile(i as usize) {
                Some(val) => val,
                None => continue,
            };

            if header.tile_type != OntologyTileType::ONTOLOGY_AXIOM {
                continue;
            }

            let records: &[OntologyAxiomRecord] = bytemuck::cast_slice(payload);
            for (rec_idx, rec) in records.iter().enumerate() {
                if rec.subject_concept_id == concept_id
                    && rec.predicate_id == PREDICATE_HAS_APPROVED_ANALOGY
                    && !rec.is_tombstone()
                {
                    let coord = S3ACoordinate::new(0, i as u32, rec_idx as u32);
                    results.push((*rec, coord));
                }
            }
        }
        Ok(results)
    }

    fn query_axioms_by_subject(&self, subject_id: u64) -> io::Result<Vec<(OntologyAxiomRecord, S3ACoordinate)>> {
        let reader = MmapReader::open(self.file_path())?;
        let tile_count = reader.tile_count();
        let mut results = Vec::new();

        for i in 0..tile_count {
            let (header, payload) = match reader.get_tile(i as usize) {
                Some(val) => val,
                None => continue,
            };

            if header.tile_type != OntologyTileType::ONTOLOGY_AXIOM {
                continue;
            }

            let records: &[OntologyAxiomRecord] = bytemuck::cast_slice(payload);
            for (rec_idx, rec) in records.iter().enumerate() {
                if rec.subject_concept_id == subject_id && !rec.is_tombstone() {
                    let coord = S3ACoordinate::new(0, i as u32, rec_idx as u32);
                    results.push((*rec, coord));
                }
            }
        }
        Ok(results)
    }
}
