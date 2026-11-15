//! CI check: Every field defined in the tile-type registry / standard_schema_for_type
//! MUST match docs/spec/registry.md per tile type in a structured table comparison,
//! checking field name, type, and width (byte length) in BOTH directions.

use s3a_core::TileType;
use std::collections::BTreeMap;

const SPEC_REGISTRY_MD: &str = include_str!("../../../docs/spec/registry.md");

#[derive(Debug, Clone, PartialEq, Eq)]
struct SpecField {
    field_id: u16,
    name: String,
    field_type: u16,
    byte_offset: u32,
    byte_len: u32,
}

fn parse_spec_tables(md: &str) -> BTreeMap<u32, Vec<SpecField>> {
    let mut map: BTreeMap<u32, Vec<SpecField>> = BTreeMap::new();

    for section in md.split("#### Tile ") {
        let (first_line, rest) = match section.split_once('\n') {
            Some(pair) => pair,
            None => continue,
        };

        // Extract tile type ID(s) from first line: e.g. "0: `telemetry`" or "19 / 20: `marks`"
        let colon_idx = match first_line.find(':') {
            Some(idx) => idx,
            None => continue,
        };
        let ids_part = &first_line[..colon_idx];
        let mut tile_ids = Vec::new();
        for id_str in ids_part.split('/') {
            if let Ok(id) = id_str.trim().parse::<u32>() {
                tile_ids.push(id);
            }
        }
        if tile_ids.is_empty() {
            continue;
        }

        // Parse markdown table rows
        let mut fields = Vec::new();
        for line in rest.lines() {
            let trimmed = line.trim();
            if !trimmed.starts_with('|') {
                continue;
            }
            let cols: Vec<&str> = trimmed
                .split('|')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect();
            if cols.len() < 5 {
                continue;
            }
            // Skip header and separator lines
            if cols[0].contains("Field ID") || cols[0].starts_with(':') || cols[0].starts_with('-')
            {
                continue;
            }

            let field_id = match cols[0].parse::<u16>() {
                Ok(id) => id,
                Err(_) => continue,
            };

            let name = cols[1].trim_matches('`').trim().to_string();

            // Parse field type code from e.g. "U64 (4)" or "(4)" or "U64"
            let type_str = cols[2];
            let field_type = if let Some(open) = type_str.find('(') {
                let close = type_str
                    .find(')')
                    .unwrap_or_else(|| panic!("unbalanced field type '{}'", type_str));
                let code = type_str[open + 1..close]
                    .trim()
                    .parse::<u16>()
                    .unwrap_or_else(|_| {
                        panic!("Failed to parse field type digits from '{}'", type_str)
                    });
                let named = type_str[..open].trim();
                if !named.is_empty() {
                    assert_eq!(
                        parse_type_name(named),
                        code,
                        "field type name and code disagree in '{}'",
                        type_str
                    );
                }
                code
            } else {
                parse_type_name(type_str)
            };

            let byte_offset = cols[3].parse::<u32>().unwrap_or_else(|_| {
                panic!("unparsable byte offset '{}' in row: {}", cols[3], trimmed)
            });

            let byte_len = cols[4].parse::<u32>().unwrap_or_else(|_| {
                panic!("unparsable byte length '{}' in row: {}", cols[4], trimmed)
            });

            fields.push(SpecField {
                field_id,
                name,
                field_type,
                byte_offset,
                byte_len,
            });
        }

        for id in tile_ids {
            map.insert(id, fields.clone());
        }
    }

    map
}

fn parse_type_name(s: &str) -> u16 {
    let s = s.trim();
    match s {
        "U8" => 1,
        "U16" => 2,
        "U32" => 3,
        "U64" => 4,
        "I8" => 5,
        "I16" => 6,
        "I32" => 7,
        "I64" => 8,
        "F32" => 9,
        "F64" => 10,
        "STRING" => 11,
        "UUID" => 12,
        "BYTES" => 13,
        _ => panic!("Unknown field type name: {}", s),
    }
}

#[test]
fn structured_spec_fields_match_code_bidirectionally() {
    let spec_tables = parse_spec_tables(SPEC_REGISTRY_MD);

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

    let mut errors = Vec::new();

    // Every tile in the list above, plus every tile the spec documents with a field table.
    let mut all_types: std::collections::BTreeSet<u32> = check_types.iter().copied().collect();
    all_types.extend(spec_tables.keys().copied());

    for &t in &all_types {
        let schema = s3a_engine::standard_schema_for_type(t);
        let spec_fields = match spec_tables.get(&t) {
            Some(f) => f,
            None => {
                errors.push(format!(
                    "Tile {} ({}) missing from docs/spec/registry.md table",
                    t, schema.name
                ));
                continue;
            }
        };

        // Direction 1: Code -> Spec
        for code_f in &schema.fields {
            match spec_fields.iter().find(|sf| sf.name == code_f.name) {
                None => {
                    errors.push(format!(
                        "Tile {} ({}): code field '{}' not in spec table",
                        t, schema.name, code_f.name
                    ));
                }
                Some(sf) => {
                    if sf.field_type != code_f.field_type {
                        errors.push(format!(
                            "Tile {} ({}): field '{}' type mismatch: code={}, spec={}",
                            t, schema.name, code_f.name, code_f.field_type, sf.field_type
                        ));
                    }
                    if sf.byte_len != code_f.byte_len {
                        errors.push(format!(
                            "Tile {} ({}): field '{}' byte_len mismatch: code={}, spec={}",
                            t, schema.name, code_f.name, code_f.byte_len, sf.byte_len
                        ));
                    }
                    if sf.byte_offset != code_f.byte_offset || sf.field_id != code_f.field_id {
                        errors.push(format!(
                            "Tile {} ({}): field '{}' offset/id mismatch: code=({}, #{}), spec=({}, #{})",
                            t,
                            schema.name,
                            code_f.name,
                            code_f.byte_offset,
                            code_f.field_id,
                            sf.byte_offset,
                            sf.field_id
                        ));
                    }
                }
            }
        }

        // Direction 2: Spec -> Code
        for sf in spec_fields {
            match schema.fields.iter().find(|cf| cf.name == sf.name) {
                None => {
                    errors.push(format!(
                        "Tile {} ({}): spec table field '{}' not in code registry",
                        t, schema.name, sf.name
                    ));
                }
                Some(code_f) => {
                    if sf.field_type != code_f.field_type {
                        errors.push(format!(
                            "Tile {} ({}): field '{}' type mismatch: spec={}, code={}",
                            t, schema.name, sf.name, sf.field_type, code_f.field_type
                        ));
                    }
                    if sf.byte_len != code_f.byte_len {
                        errors.push(format!(
                            "Tile {} ({}): field '{}' byte_len mismatch: spec={}, code={}",
                            t, schema.name, sf.name, sf.byte_len, code_f.byte_len
                        ));
                    }
                }
            }
        }
    }

    assert!(
        errors.is_empty(),
        "Bidirectional spec table check failed:\n{}",
        errors.join("\n")
    );
}
