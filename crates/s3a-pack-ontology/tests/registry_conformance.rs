//! Conformance tests verifying that `docs/spec/registry.md` Tile 16 and Tile 17
//! field tables match the implemented `OntologyConceptRecord` and
//! `OntologyAxiomRecord` layouts in `s3a_pack_ontology::records`.

use s3a_pack_ontology::records::{OntologyAxiomRecord, OntologyConceptRecord};
use std::mem::{align_of, offset_of, size_of};

const REGISTRY_MD: &str = include_str!("../../../docs/spec/registry.md");

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedField {
    field_id: u16,
    name: String,
    field_type_code: u16,
    byte_offset: u32,
    byte_len: u32,
}

fn parse_tile_table(md: &str, tile_id: u32) -> Vec<ParsedField> {
    let normalized = md.replace("\r\n", "\n");
    let header_prefix = format!("#### Tile {tile_id}:");

    let section = normalized
        .split("#### Tile ")
        .find(|s| {
            let trimmed = s.trim_start();
            trimmed.starts_with(&format!("{tile_id}:"))
                || trimmed.starts_with(&format!("{tile_id} "))
        })
        .unwrap_or_else(|| panic!("Missing section '{header_prefix}' in registry.md"));

    let (_, body) = section
        .split_once('\n')
        .unwrap_or_else(|| panic!("Malformed section for Tile {tile_id}"));

    let mut fields = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            break;
        }
        if !trimmed.starts_with('|') {
            continue;
        }
        let cols: Vec<&str> = trimmed
            .split('|')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if cols.len() < 5 {
            continue;
        }
        if cols[0].contains("Field ID") || cols[0].starts_with(':') || cols[0].starts_with('-') {
            continue;
        }

        let field_id = match cols[0].parse::<u16>() {
            Ok(id) => id,
            Err(_) => continue,
        };
        let name = cols[1].trim_matches('`').trim().to_string();
        let field_type_code = parse_type_code(cols[2]);
        let byte_offset = cols[3]
            .parse::<u32>()
            .unwrap_or_else(|e| panic!("Invalid byte_offset '{}': {e}", cols[3]));
        let byte_len = cols[4]
            .parse::<u32>()
            .unwrap_or_else(|e| panic!("Invalid byte_len '{}': {e}", cols[4]));

        fields.push(ParsedField {
            field_id,
            name,
            field_type_code,
            byte_offset,
            byte_len,
        });
    }

    fields
}

fn parse_type_code(col: &str) -> u16 {
    if let Some(open) = col.find('(') {
        if let Some(close) = col.find(')') {
            return col[open + 1..close]
                .trim()
                .parse::<u16>()
                .unwrap_or_else(|e| panic!("Invalid type code in '{col}': {e}"));
        }
    }
    panic!("Expected type code in parentheses in '{col}'");
}

fn expected_tile_16_fields() -> Vec<ParsedField> {
    vec![
        ParsedField {
            field_id: 1,
            name: "concept_id".to_string(),
            field_type_code: 4, // U64
            byte_offset: offset_of!(OntologyConceptRecord, concept_id) as u32,
            byte_len: 8,
        },
        ParsedField {
            field_id: 2,
            name: "parent_id".to_string(),
            field_type_code: 4, // U64
            byte_offset: offset_of!(OntologyConceptRecord, parent_id) as u32,
            byte_len: 8,
        },
        ParsedField {
            field_id: 3,
            name: "ontology_id".to_string(),
            field_type_code: 3, // U32
            byte_offset: offset_of!(OntologyConceptRecord, ontology_id) as u32,
            byte_len: 4,
        },
        ParsedField {
            field_id: 4,
            name: "concept_kind".to_string(),
            field_type_code: 2, // U16
            byte_offset: offset_of!(OntologyConceptRecord, concept_kind) as u32,
            byte_len: 2,
        },
        ParsedField {
            field_id: 5,
            name: "xapi_profile_id".to_string(),
            field_type_code: 2, // U16
            byte_offset: offset_of!(OntologyConceptRecord, xapi_profile_id) as u32,
            byte_len: 2,
        },
        ParsedField {
            field_id: 6,
            name: "hilbert_coord".to_string(),
            field_type_code: 3, // U32
            byte_offset: offset_of!(OntologyConceptRecord, hilbert_coord) as u32,
            byte_len: 4,
        },
        ParsedField {
            field_id: 7,
            name: "flags".to_string(),
            field_type_code: 3, // U32
            byte_offset: offset_of!(OntologyConceptRecord, flags) as u32,
            byte_len: 4,
        },
        ParsedField {
            field_id: 8,
            name: "reserved".to_string(),
            field_type_code: 13, // BYTES
            byte_offset: offset_of!(OntologyConceptRecord, _reserved) as u32,
            byte_len: 32,
        },
    ]
}

fn expected_tile_17_fields() -> Vec<ParsedField> {
    vec![
        ParsedField {
            field_id: 1,
            name: "subject_concept_id".to_string(),
            field_type_code: 4, // U64
            byte_offset: offset_of!(OntologyAxiomRecord, subject_concept_id) as u32,
            byte_len: 8,
        },
        ParsedField {
            field_id: 2,
            name: "object_concept_id".to_string(),
            field_type_code: 4, // U64
            byte_offset: offset_of!(OntologyAxiomRecord, object_concept_id) as u32,
            byte_len: 8,
        },
        ParsedField {
            field_id: 3,
            name: "predicate_id".to_string(),
            field_type_code: 3, // U32
            byte_offset: offset_of!(OntologyAxiomRecord, predicate_id) as u32,
            byte_len: 4,
        },
        ParsedField {
            field_id: 4,
            name: "axiom_type".to_string(),
            field_type_code: 2, // U16
            byte_offset: offset_of!(OntologyAxiomRecord, axiom_type) as u32,
            byte_len: 2,
        },
        ParsedField {
            field_id: 5,
            name: "weight_fp16".to_string(),
            field_type_code: 2, // U16
            byte_offset: offset_of!(OntologyAxiomRecord, weight_fp16) as u32,
            byte_len: 2,
        },
        ParsedField {
            field_id: 6,
            name: "target_tile_type".to_string(),
            field_type_code: 2, // U16
            byte_offset: offset_of!(OntologyAxiomRecord, target_tile_type) as u32,
            byte_len: 2,
        },
        ParsedField {
            field_id: 7,
            name: "required_addon_mask".to_string(),
            field_type_code: 2, // U16
            byte_offset: offset_of!(OntologyAxiomRecord, required_addon_mask) as u32,
            byte_len: 2,
        },
        ParsedField {
            field_id: 8,
            name: "flags".to_string(),
            field_type_code: 3, // U32
            byte_offset: offset_of!(OntologyAxiomRecord, flags) as u32,
            byte_len: 4,
        },
        ParsedField {
            field_id: 9,
            name: "reserved".to_string(),
            field_type_code: 13, // BYTES
            byte_offset: offset_of!(OntologyAxiomRecord, _reserved) as u32,
            byte_len: 32,
        },
    ]
}

#[test]
fn implemented_layouts_match_offset_of() {
    assert_eq!(size_of::<OntologyConceptRecord>(), 64);
    assert_eq!(align_of::<OntologyConceptRecord>(), 8);
    assert_eq!(offset_of!(OntologyConceptRecord, concept_id), 0);
    assert_eq!(offset_of!(OntologyConceptRecord, parent_id), 8);
    assert_eq!(offset_of!(OntologyConceptRecord, ontology_id), 16);
    assert_eq!(offset_of!(OntologyConceptRecord, concept_kind), 20);
    assert_eq!(offset_of!(OntologyConceptRecord, xapi_profile_id), 22);
    assert_eq!(offset_of!(OntologyConceptRecord, hilbert_coord), 24);
    assert_eq!(offset_of!(OntologyConceptRecord, flags), 28);
    assert_eq!(offset_of!(OntologyConceptRecord, _reserved), 32);

    assert_eq!(size_of::<OntologyAxiomRecord>(), 64);
    assert_eq!(align_of::<OntologyAxiomRecord>(), 8);
    assert_eq!(offset_of!(OntologyAxiomRecord, subject_concept_id), 0);
    assert_eq!(offset_of!(OntologyAxiomRecord, object_concept_id), 8);
    assert_eq!(offset_of!(OntologyAxiomRecord, predicate_id), 16);
    assert_eq!(offset_of!(OntologyAxiomRecord, axiom_type), 20);
    assert_eq!(offset_of!(OntologyAxiomRecord, weight_fp16), 22);
    assert_eq!(offset_of!(OntologyAxiomRecord, target_tile_type), 24);
    assert_eq!(offset_of!(OntologyAxiomRecord, required_addon_mask), 26);
    assert_eq!(offset_of!(OntologyAxiomRecord, flags), 28);
    assert_eq!(offset_of!(OntologyAxiomRecord, _reserved), 32);
}

#[test]
fn different_registry_tables_parse_to_different_field_lists() {
    let snippet_a = "\
#### Tile 16: `ontology_concept` (Record Size: 64 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `concept_id` | U64 (4) | 0 | 8 | false | ID |
";
    let snippet_b = "\
#### Tile 16: `ontology_concept` (Record Size: 64 B)
| Field ID | Name | Field Type | Byte Offset | Byte Length | Null Allowed | Description |
|:---:|:---|:---:|:---:|:---:|:---:|:---|
| 1 | `parent_id` | U64 (4) | 8 | 8 | false | Parent |
";
    assert_ne!(
        parse_tile_table(snippet_a, 16),
        parse_tile_table(snippet_b, 16)
    );
}

#[test]
fn live_registry_tile_16_and_17_tables_are_not_identical() {
    let t16 = parse_tile_table(REGISTRY_MD, 16);
    let t17 = parse_tile_table(REGISTRY_MD, 17);
    assert!(!t16.is_empty());
    assert!(!t17.is_empty());
    assert_ne!(t16, t17);
}

#[test]
fn registry_md_tile_16_fields_match_ontology_concept_record() {
    let actual = parse_tile_table(REGISTRY_MD, 16);
    let expected = expected_tile_16_fields();
    assert_eq!(actual, expected);
}

#[test]
fn registry_md_tile_17_fields_match_ontology_axiom_record() {
    let actual = parse_tile_table(REGISTRY_MD, 17);
    let expected = expected_tile_17_fields();
    assert_eq!(actual, expected);
}
