use bytemuck::{Pod, Zeroable};

pub const FLAG_ACTIVE: u32 = 0;
pub const FLAG_TOMBSTONE: u32 = 1 << 0;
pub const FLAG_PROVISIONAL_PROFILE: u32 = 1 << 15; // 0x8000: Synthesized JIT profile pending consolidation

// Standard Ontology Subsystems
pub const ONTOLOGY_CITO: u32 = 1;
pub const ONTOLOGY_PROV_O: u32 = 2;
pub const ONTOLOGY_IEEE_1872: u32 = 3;
pub const ONTOLOGY_PEDAGOGY: u32 = 4;
pub const ONTOLOGY_CUSTOM: u32 = 5;

// Concept Kinds
pub const KIND_CLASS: u16 = 1;
pub const KIND_VERB: u16 = 2;
pub const KIND_ACTIVITY_TYPE: u16 = 3;
pub const KIND_PROPERTY: u16 = 4;
pub const KIND_ANALOGY_SCAFFOLD: u16 = 5;

// Axiom Predicates
pub const PREDICATE_SUBCLASS_OF: u32 = 1;
pub const PREDICATE_HAS_APPROVED_ANALOGY: u32 = 2;
pub const PREDICATE_REQUIRES_COMPETENCY: u32 = 3;
pub const PREDICATE_DOMAIN: u32 = 4;
pub const PREDICATE_RANGE: u32 = 5;
pub const PREDICATE_DISJOINT_WITH: u32 = 6;
pub const PREDICATE_EXPLAINS: u32 = 7;

/// ONTOLOGY CONCEPT RECORD (64 bytes, 8-byte aligned).
/// Represents a normalized concept, class, verb, activity type, or pedagogical scaffold.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq, Default)]
pub struct OntologyConceptRecord {
    pub concept_id: u64,      // 64-bit hash of canonical URI (offset 0..8)
    pub parent_id: u64,       // Subsumption parent concept hash (0 if root) (offset 8..16)
    pub ontology_id: u32,     // Enclosing ontology ID (offset 16..20)
    pub concept_kind: u16,    // 1=Class, 2=Verb, 3=ActivityType, etc. (offset 20..22)
    pub xapi_profile_id: u16, // Associated xAPI profile ID (offset 22..24)
    pub hilbert_coord: u32,   // Topological interval index for SIMD subtree sifting (offset 24..28)
    pub flags: u32,           // Active, Tombstone, Provisional (offset 28..32)
    pub _reserved: [u64; 4],  // Padding to exact 64 bytes (offset 32..64)
}

impl OntologyConceptRecord {
    pub fn new(
        concept_id: u64,
        parent_id: u64,
        ontology_id: u32,
        concept_kind: u16,
        xapi_profile_id: u16,
        hilbert_coord: u32,
    ) -> Self {
        Self {
            concept_id,
            parent_id,
            ontology_id,
            concept_kind,
            xapi_profile_id,
            hilbert_coord,
            flags: FLAG_ACTIVE,
            _reserved: [0u64; 4],
        }
    }

    pub fn with_provisional(mut self) -> Self {
        self.flags |= FLAG_PROVISIONAL_PROFILE;
        self
    }

    pub fn is_provisional(&self) -> bool {
        (self.flags & FLAG_PROVISIONAL_PROFILE) != 0
    }

    pub fn is_tombstone(&self) -> bool {
        (self.flags & FLAG_TOMBSTONE) != 0
    }

    pub fn mark_tombstone(&mut self) {
        self.flags |= FLAG_TOMBSTONE;
    }
}

/// ONTOLOGY AXIOM RECORD (64 bytes, 8-byte aligned).
/// Encapsulates relationships, domain/range constraints, approved analogies, and xAPI rules.
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq, Default)]
pub struct OntologyAxiomRecord {
    pub subject_concept_id: u64,  // Subject concept hash (offset 0..8)
    pub object_concept_id: u64,   // Object concept hash (offset 8..16)
    pub predicate_id: u32,        // Predicate type (offset 16..20)
    pub axiom_type: u16, // 1=Hierarchical, 2=PedagogicalScaffold, 3=xAPIRule (offset 20..22)
    pub weight_fp16: u16, // Relevance / confidence weight (offset 22..24)
    pub target_tile_type: u16, // Target S3A tile type e.g. 10 (LRS), 12 (Paper) (offset 24..26)
    pub required_addon_mask: u16, // Required addon bitmask (offset 26..28)
    pub flags: u32,      // Active, Tombstone, Provisional (offset 28..32)
    pub _reserved: [u64; 4], // Padding to exact 64 bytes (offset 32..64)
}

impl OntologyAxiomRecord {
    pub fn new(
        subject_concept_id: u64,
        object_concept_id: u64,
        predicate_id: u32,
        axiom_type: u16,
        weight_fp16: u16,
        target_tile_type: u16,
        required_addon_mask: u16,
    ) -> Self {
        Self {
            subject_concept_id,
            object_concept_id,
            predicate_id,
            axiom_type,
            weight_fp16,
            target_tile_type,
            required_addon_mask,
            flags: FLAG_ACTIVE,
            _reserved: [0u64; 4],
        }
    }

    pub fn is_tombstone(&self) -> bool {
        (self.flags & FLAG_TOMBSTONE) != 0
    }

    pub fn mark_tombstone(&mut self) {
        self.flags |= FLAG_TOMBSTONE;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_sizes_and_alignments() {
        assert_eq!(size_of::<OntologyConceptRecord>(), 64);
        assert_eq!(std::mem::align_of::<OntologyConceptRecord>(), 8);

        assert_eq!(size_of::<OntologyAxiomRecord>(), 64);
        assert_eq!(std::mem::align_of::<OntologyAxiomRecord>(), 8);
    }
}
