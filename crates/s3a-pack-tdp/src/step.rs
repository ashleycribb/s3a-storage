//! ISO 10303-21 (STEP) AP242 Product and Assembly Structure Parser.
//!
//! Extracts STEP header metadata and parses product definition and assembly structures
//! (`PRODUCT`, `PRODUCT_DEFINITION_FORMATION`, `PRODUCT_DEFINITION`,
//! `NEXT_ASSEMBLY_USAGE_OCCURRENCE`, `QUANTIFIED_ASSEMBLY_COMPONENT_USAGE`)
//! into a clean, canonical `StepAssemblyTree` and flat BOM records.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Header metadata from a STEP file (ISO 10303-21 HEADER section).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct StepHeader {
    pub file_name: Option<String>,
    pub timestamp: Option<String>,
    pub author: Option<String>,
    pub organization: Option<String>,
    pub preprocessor_version: Option<String>,
    pub originating_system: Option<String>,
    pub authorization: Option<String>,
    pub schemas: Vec<String>,
}

/// A single BOM component record in the assembly tree.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StepAssemblyPartRecord {
    pub part_number: String,
    pub name: String,
    pub parent: Option<String>,
    pub quantity: f64,
}

/// Complete hierarchical assembly tree representation of an AP242 STEP model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StepAssemblyTree {
    pub header: StepHeader,
    pub root_part_number: Option<String>,
    pub parts: Vec<StepAssemblyPartRecord>,
}

impl StepAssemblyTree {
    /// Formats the assembly tree as a standard CSV Bill of Materials (BOM).
    pub fn to_bom_csv(&self) -> String {
        let mut csv = String::from("part_number,name,parent,quantity\n");
        for p in &self.parts {
            let parent_str = p.parent.as_deref().unwrap_or("");
            // Escape CSV commas/quotes if needed
            let part_no = escape_csv_field(&p.part_number);
            let name = escape_csv_field(&p.name);
            let parent = escape_csv_field(parent_str);
            csv.push_str(&format!("{},{},{},{}\n", part_no, name, parent, p.quantity));
        }
        csv
    }

    /// Formats the assembly tree as JSON BOM.
    pub fn to_bom_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(&self.parts)
    }
}

fn escape_csv_field(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

// ---------------------------------------------------------------------------
// ISO 10303-21 Lexer & Parser
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum StepValue {
    String(String),
    EntityRef(u64),
    Number(f64),
    Enum(String),
    List(Vec<StepValue>),
    Typed(String, Vec<StepValue>),
    Null,
}

impl StepValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            StepValue::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_entity_ref(&self) -> Option<u64> {
        match self {
            StepValue::EntityRef(id) => Some(*id),
            _ => None,
        }
    }

    pub fn as_number(&self) -> Option<f64> {
        match self {
            StepValue::Number(n) => Some(*n),
            StepValue::Typed(_, args) => args.first().and_then(|v| v.as_number()),
            _ => None,
        }
    }
}

/// Strips comments (`/* ... */`) while respecting single-quoted string literals.
fn strip_step_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    let mut in_string = false;

    while i < chars.len() {
        if in_string {
            out.push(chars[i]);
            if chars[i] == '\'' {
                if i + 1 < chars.len() && chars[i + 1] == '\'' {
                    out.push('\'');
                    i += 2;
                    continue;
                } else {
                    in_string = false;
                }
            }
            i += 1;
        } else if chars[i] == '\'' {
            in_string = true;
            out.push('\'');
            i += 1;
        } else if chars[i] == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2; // skip */
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// Splits input into statements by `;` outside string literals.
fn split_step_statements(input: &str) -> Vec<String> {
    let mut stmts = Vec::new();
    let mut current = String::new();
    let mut in_string = false;
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        if in_string {
            current.push(c);
            if c == '\'' {
                if i + 1 < chars.len() && chars[i + 1] == '\'' {
                    current.push('\'');
                    i += 2;
                    continue;
                } else {
                    in_string = false;
                }
            }
        } else if c == '\'' {
            in_string = true;
            current.push(c);
        } else if c == ';' {
            let trimmed = current.trim().to_string();
            if !trimmed.is_empty() {
                stmts.push(trimmed);
            }
            current.clear();
        } else {
            current.push(c);
        }
        i += 1;
    }
    stmts
}

/// Tokenizer for an argument list `(...)`
struct ArgParser<'a> {
    chars: &'a [char],
    pos: usize,
}

impl<'a> ArgParser<'a> {
    fn new(chars: &'a [char]) -> Self {
        Self { chars, pos: 0 }
    }

    fn skip_whitespace(&mut self) {
        while self.pos < self.chars.len() && self.chars[self.pos].is_whitespace() {
            self.pos += 1;
        }
    }

    fn parse_value(&mut self) -> Option<StepValue> {
        self.skip_whitespace();
        if self.pos >= self.chars.len() {
            return None;
        }

        let c = self.chars[self.pos];

        if c == '\'' {
            // String literal
            self.pos += 1;
            let mut s = String::new();
            while self.pos < self.chars.len() {
                if self.chars[self.pos] == '\'' {
                    if self.pos + 1 < self.chars.len() && self.chars[self.pos + 1] == '\'' {
                        s.push('\'');
                        self.pos += 2;
                    } else {
                        self.pos += 1;
                        break;
                    }
                } else {
                    s.push(self.chars[self.pos]);
                    self.pos += 1;
                }
            }
            Some(StepValue::String(s))
        } else if c == '#' {
            // Entity reference #123
            self.pos += 1;
            let start = self.pos;
            while self.pos < self.chars.len() && self.chars[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
            let s: String = self.chars[start..self.pos].iter().collect();
            s.parse::<u64>().ok().map(StepValue::EntityRef)
        } else if c == '(' {
            // List (...)
            self.pos += 1;
            let mut items = Vec::new();
            loop {
                self.skip_whitespace();
                if self.pos >= self.chars.len() {
                    break;
                }
                if self.chars[self.pos] == ')' {
                    self.pos += 1;
                    break;
                }
                if let Some(val) = self.parse_value() {
                    items.push(val);
                }
                self.skip_whitespace();
                if self.pos < self.chars.len() && self.chars[self.pos] == ',' {
                    self.pos += 1;
                }
            }
            Some(StepValue::List(items))
        } else if c == '.' {
            // Enum .VAL.
            self.pos += 1;
            let start = self.pos;
            while self.pos < self.chars.len() && self.chars[self.pos] != '.' {
                self.pos += 1;
            }
            let s: String = self.chars[start..self.pos].iter().collect();
            if self.pos < self.chars.len() && self.chars[self.pos] == '.' {
                self.pos += 1;
            }
            Some(StepValue::Enum(s))
        } else if c == '$' || c == '*' {
            self.pos += 1;
            Some(StepValue::Null)
        } else if c.is_ascii_digit() || c == '-' || c == '+' {
            // Number
            let start = self.pos;
            while self.pos < self.chars.len()
                && (self.chars[self.pos].is_ascii_digit()
                    || self.chars[self.pos] == '.'
                    || self.chars[self.pos] == 'e'
                    || self.chars[self.pos] == 'E'
                    || self.chars[self.pos] == '-'
                    || self.chars[self.pos] == '+')
            {
                self.pos += 1;
            }
            let s: String = self.chars[start..self.pos].iter().collect();
            s.parse::<f64>().ok().map(StepValue::Number)
        } else if c.is_alphabetic() || c == '_' {
            // Typed call: e.g. COUNT_MEASURE(4.0)
            let start = self.pos;
            while self.pos < self.chars.len()
                && (self.chars[self.pos].is_alphanumeric() || self.chars[self.pos] == '_')
            {
                self.pos += 1;
            }
            let type_name: String = self.chars[start..self.pos].iter().collect();
            self.skip_whitespace();
            if self.pos < self.chars.len() && self.chars[self.pos] == '(' {
                if let Some(StepValue::List(args)) = self.parse_value() {
                    Some(StepValue::Typed(type_name, args))
                } else {
                    Some(StepValue::Enum(type_name))
                }
            } else {
                Some(StepValue::Enum(type_name))
            }
        } else {
            self.pos += 1;
            None
        }
    }

    fn parse_all(&mut self) -> Vec<StepValue> {
        let mut list = Vec::new();
        while self.pos < self.chars.len() {
            self.skip_whitespace();
            if self.pos >= self.chars.len() {
                break;
            }
            if self.chars[self.pos] == ',' {
                self.pos += 1;
                continue;
            }
            if let Some(val) = self.parse_value() {
                list.push(val);
            } else {
                self.pos += 1;
            }
        }
        list
    }
}

fn parse_argument_list(raw_args: &str) -> Vec<StepValue> {
    let chars: Vec<char> = raw_args.chars().collect();
    let mut parser = ArgParser::new(&chars);
    parser.parse_all()
}

// ---------------------------------------------------------------------------
// High-Level STEP AP242 Extraction
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct ParsedEntity {
    id: u64,
    type_name: String,
    args: Vec<StepValue>,
}

#[derive(Debug, Clone)]
struct ProductRecord {
    part_number: String,
    name: String,
}

/// Parses an AP242 / ISO 10303-21 STEP file from raw bytes into `StepAssemblyTree`.
pub fn parse_step(bytes: &[u8]) -> Result<StepAssemblyTree, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|e| format!("STEP file is not valid UTF-8/ASCII: {}", e))?;
    let cleaned = strip_step_comments(text);
    let stmts = split_step_statements(&cleaned);

    let mut in_header = false;
    let mut in_data = false;

    let mut header = StepHeader::default();
    let mut entities: HashMap<u64, ParsedEntity> = HashMap::new();

    for stmt in &stmts {
        let upper = stmt.to_uppercase();
        if upper == "HEADER" {
            in_header = true;
            in_data = false;
            continue;
        } else if upper == "DATA" {
            in_header = false;
            in_data = true;
            continue;
        } else if upper == "ENDSEC" {
            in_header = false;
            in_data = false;
            continue;
        }

        if in_header {
            parse_header_statement(stmt, &mut header);
        } else if in_data {
            if let Some(ent) = parse_data_statement(stmt) {
                entities.insert(ent.id, ent);
            }
        }
    }

    build_assembly_tree(header, entities)
}

fn parse_header_statement(stmt: &str, header: &mut StepHeader) {
    if let Some(open_paren) = stmt.find('(') {
        let name = stmt[..open_paren].trim().to_uppercase();
        let close_paren = stmt.rfind(')').unwrap_or(stmt.len());
        let raw_args = &stmt[open_paren + 1..close_paren];
        let args = parse_argument_list(raw_args);

        match name.as_str() {
            "FILE_NAME" => {
                if let Some(val) = args.first().and_then(|v| v.as_str()) {
                    header.file_name = Some(val.to_string());
                }
                if let Some(val) = args.get(1).and_then(|v| v.as_str()) {
                    header.timestamp = Some(val.to_string());
                }
                if let Some(StepValue::List(authors)) = args.get(2) {
                    let author_names: Vec<String> = authors
                        .iter()
                        .filter_map(|a| a.as_str().map(|s| s.to_string()))
                        .collect();
                    if !author_names.is_empty() {
                        header.author = Some(author_names.join(", "));
                    }
                } else if let Some(val) = args.get(2).and_then(|v| v.as_str()) {
                    header.author = Some(val.to_string());
                }
                if let Some(StepValue::List(orgs)) = args.get(3) {
                    let org_names: Vec<String> = orgs
                        .iter()
                        .filter_map(|a| a.as_str().map(|s| s.to_string()))
                        .collect();
                    if !org_names.is_empty() {
                        header.organization = Some(org_names.join(", "));
                    }
                } else if let Some(val) = args.get(3).and_then(|v| v.as_str()) {
                    header.organization = Some(val.to_string());
                }
                if let Some(val) = args.get(4).and_then(|v| v.as_str()) {
                    header.preprocessor_version = Some(val.to_string());
                }
                if let Some(val) = args.get(5).and_then(|v| v.as_str()) {
                    header.originating_system = Some(val.to_string());
                }
                if let Some(val) = args.get(6).and_then(|v| v.as_str()) {
                    header.authorization = Some(val.to_string());
                }
            }
            "FILE_SCHEMA" => {
                if let Some(StepValue::List(schemas)) = args.first() {
                    for s in schemas {
                        if let Some(schema_name) = s.as_str() {
                            header.schemas.push(schema_name.to_string());
                        }
                    }
                } else if let Some(s) = args.first().and_then(|v| v.as_str()) {
                    header.schemas.push(s.to_string());
                }
            }
            _ => {}
        }
    }
}

fn parse_data_statement(stmt: &str) -> Option<ParsedEntity> {
    // Statement: #10 = TYPE(arg1, arg2...)
    let eq_idx = stmt.find('=')?;
    let left = stmt[..eq_idx].trim();
    if !left.starts_with('#') {
        return None;
    }
    let id: u64 = left[1..].trim().parse().ok()?;

    let right = stmt[eq_idx + 1..].trim();
    let open_paren = right.find('(')?;
    let type_name = right[..open_paren].trim().to_uppercase();
    let close_paren = right.rfind(')')?;
    let raw_args = &right[open_paren + 1..close_paren];
    let args = parse_argument_list(raw_args);

    Some(ParsedEntity {
        id,
        type_name,
        args,
    })
}

fn build_assembly_tree(
    header: StepHeader,
    entities: HashMap<u64, ParsedEntity>,
) -> Result<StepAssemblyTree, String> {
    let mut products: HashMap<u64, ProductRecord> = HashMap::new();
    let mut formations: HashMap<u64, u64> = HashMap::new(); // formation_id -> product_id
    let mut product_definitions: HashMap<u64, u64> = HashMap::new(); // pd_id -> formation_id

    // Index all PRODUCT, FORMATION, and PRODUCT_DEFINITION records
    for ent in entities.values() {
        match ent.type_name.as_str() {
            "PRODUCT" => {
                let part_no = ent
                    .args
                    .first()
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let name = ent
                    .args
                    .get(1)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                products.insert(
                    ent.id,
                    ProductRecord {
                        part_number: if part_no.is_empty() {
                            name.clone()
                        } else {
                            part_no
                        },
                        name,
                    },
                );
            }
            "PRODUCT_DEFINITION_FORMATION"
            | "PRODUCT_DEFINITION_FORMATION_WITH_SPECIFIED_SOURCE" => {
                // Usually arg 2 is the #product_id reference
                if let Some(prod_ref) = ent.args.get(2).and_then(|v| v.as_entity_ref()) {
                    formations.insert(ent.id, prod_ref);
                }
            }
            "PRODUCT_DEFINITION" => {
                // Usually arg 2 is the #formation_id reference
                if let Some(form_ref) = ent.args.get(2).and_then(|v| v.as_entity_ref()) {
                    product_definitions.insert(ent.id, form_ref);
                }
            }
            _ => {}
        }
    }

    // Helper to resolve an entity reference to a ProductRecord
    let resolve_product = |ent_ref: u64| -> Option<ProductRecord> {
        if let Some(p) = products.get(&ent_ref) {
            return Some(p.clone());
        }
        if let Some(form_ref) = product_definitions.get(&ent_ref) {
            if let Some(prod_ref) = formations.get(form_ref) {
                if let Some(p) = products.get(prod_ref) {
                    return Some(p.clone());
                }
            }
        }
        if let Some(prod_ref) = formations.get(&ent_ref) {
            if let Some(p) = products.get(prod_ref) {
                return Some(p.clone());
            }
        }
        None
    };

    // Helper to resolve quantity from an argument or measure reference
    let resolve_quantity = |val: Option<&StepValue>| -> f64 {
        match val {
            Some(StepValue::Number(n)) => *n,
            Some(StepValue::Typed(_, args)) => {
                args.first().and_then(|v| v.as_number()).unwrap_or(1.0)
            }
            Some(StepValue::EntityRef(ref_id)) => {
                if let Some(ent) = entities.get(ref_id) {
                    for arg in &ent.args {
                        if let Some(n) = arg.as_number() {
                            return n;
                        }
                        if let StepValue::Typed(_, sub_args) = arg {
                            if let Some(n) = sub_args.first().and_then(|v| v.as_number()) {
                                return n;
                            }
                        }
                    }
                }
                1.0
            }
            _ => 1.0,
        }
    };

    // Parse assembly relationships: NEXT_ASSEMBLY_USAGE_OCCURRENCE & QUANTIFIED_ASSEMBLY_COMPONENT_USAGE
    // Map: (parent_part_number, child_part_number) -> (name, quantity)
    let mut occurrences: Vec<(ProductRecord, ProductRecord, f64)> = Vec::new();
    let mut children_parts: HashSet<String> = HashSet::new();

    for ent in entities.values() {
        match ent.type_name.as_str() {
            "NEXT_ASSEMBLY_USAGE_OCCURRENCE"
            | "PROMISSORY_USAGE_OCCURRENCE"
            | "SPECIFIED_HIGHER_USAGE_OCCURRENCE" => {
                // Args: id, name, description, #relating_pd, #related_pd, ...
                let parent_ref = ent.args.get(3).and_then(|v| v.as_entity_ref());
                let child_ref = ent.args.get(4).and_then(|v| v.as_entity_ref());

                if let (Some(pref), Some(cref)) = (parent_ref, child_ref) {
                    if let (Some(parent_prod), Some(child_prod)) =
                        (resolve_product(pref), resolve_product(cref))
                    {
                        children_parts.insert(child_prod.part_number.clone());
                        occurrences.push((parent_prod, child_prod, 1.0));
                    }
                }
            }
            "QUANTIFIED_ASSEMBLY_COMPONENT_USAGE" => {
                // Args: id, name, description, #relating_pd, #related_pd, reference_designator, #quantity_measure
                let parent_ref = ent.args.get(3).and_then(|v| v.as_entity_ref());
                let child_ref = ent.args.get(4).and_then(|v| v.as_entity_ref());
                let qty = resolve_quantity(ent.args.get(6));

                if let (Some(pref), Some(cref)) = (parent_ref, child_ref) {
                    if let (Some(parent_prod), Some(child_prod)) =
                        (resolve_product(pref), resolve_product(cref))
                    {
                        children_parts.insert(child_prod.part_number.clone());
                        occurrences.push((parent_prod, child_prod, qty));
                    }
                }
            }
            _ => {}
        }
    }

    // Determine root product:
    // Any product that is a parent or declared in products, but not in children_parts
    let mut candidate_roots: Vec<ProductRecord> = products
        .values()
        .filter(|p| !children_parts.contains(&p.part_number))
        .cloned()
        .collect();

    candidate_roots.sort_by(|a, b| a.part_number.cmp(&b.part_number));
    let root = candidate_roots.first().cloned();

    // Aggregate part records for BOM
    // We want the root part first (quantity = 1.0, parent = None)
    let mut parts: Vec<StepAssemblyPartRecord> = Vec::new();

    if let Some(ref r) = root {
        parts.push(StepAssemblyPartRecord {
            part_number: r.part_number.clone(),
            name: r.name.clone(),
            parent: None,
            quantity: 1.0,
        });
    }

    // Aggregate child quantities by (parent_part_number, child_part_number)
    let mut child_map: BTreeMap<(String, String), (String, f64)> = BTreeMap::new();
    for (parent, child, qty) in occurrences {
        let key = (parent.part_number.clone(), child.part_number.clone());
        let entry = child_map.entry(key).or_insert((child.name.clone(), 0.0));
        entry.1 += qty;
    }

    for ((parent_part, child_part), (name, qty)) in child_map {
        parts.push(StepAssemblyPartRecord {
            part_number: child_part,
            name,
            parent: Some(parent_part),
            quantity: qty,
        });
    }

    Ok(StepAssemblyTree {
        header,
        root_part_number: root.map(|r| r.part_number),
        parts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_AP242_STEP: &str = r#"ISO-10303-21;
HEADER;
FILE_DESCRIPTION(('AP242 STEP Assembly Model'), '2;1');
FILE_NAME('gearbox_asm.stp', '2026-03-01T12:00:00', ('Ashley C.'), ('S3A Corp'), 'Creo 10.0', 'PTC Creo', 'Approved');
FILE_SCHEMA(('AP242_MANAGED_MODEL_BASED_3D_ENGINEERING_MIM_LF'));
ENDSEC;
DATA;
/* Top-level Gearbox Assembly */
#10=PRODUCT('ASM-1000', 'Main Gearbox Assembly', '', (#11));
#11=PRODUCT_CONTEXT('', #12, 'mechanical');
#12=APPLICATION_CONTEXT('core data for automotive mechanical design processes');
#20=PRODUCT_DEFINITION_FORMATION('1', '', #10);
#30=PRODUCT_DEFINITION('design', '', #20, #31);
#31=PRODUCT_DEFINITION_CONTEXT('part definition', #12, 'design');

/* Component 1: Input Shaft */
#40=PRODUCT('SHAFT-001', 'Input Drive Shaft', '', (#11));
#41=PRODUCT_DEFINITION_FORMATION('1', '', #40);
#42=PRODUCT_DEFINITION('design', '', #41, #31);

/* Component 2: Ball Bearing 6205 */
#50=PRODUCT('BRG-6205', 'Deep Groove Ball Bearing', '', (#11));
#51=PRODUCT_DEFINITION_FORMATION('1', '', #50);
#52=PRODUCT_DEFINITION('design', '', #51, #31);

/* Usages */
#60=NEXT_ASSEMBLY_USAGE_OCCURRENCE('NAUO1', 'Shaft In Gearbox', '', #30, #42, $);
#70=QUANTIFIED_ASSEMBLY_COMPONENT_USAGE('QACU1', 'Bearings In Gearbox', '', #30, #52, $, #71);
#71=MEASURE_WITH_UNIT(COUNT_MEASURE(2.0), #72);
#72=UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(0.01), #73, 'accuracy', 'distance');
#73=( CONVERSION_BASED_UNIT('MILLIMETRE', #74) LENGTH_UNIT() NAMED_UNIT(#75) );
ENDSEC;
END-ISO-10303-21;
"#;

    #[test]
    fn test_parse_step_ap242_assembly() {
        let tree = parse_step(SAMPLE_AP242_STEP.as_bytes()).expect("Failed to parse STEP");

        // Verify Header
        assert_eq!(tree.header.file_name.as_deref(), Some("gearbox_asm.stp"));
        assert_eq!(tree.header.author.as_deref(), Some("Ashley C."));
        assert_eq!(tree.header.organization.as_deref(), Some("S3A Corp"));
        assert_eq!(tree.header.originating_system.as_deref(), Some("PTC Creo"));
        assert_eq!(
            tree.header.schemas,
            vec!["AP242_MANAGED_MODEL_BASED_3D_ENGINEERING_MIM_LF"]
        );

        // Verify Root
        assert_eq!(tree.root_part_number.as_deref(), Some("ASM-1000"));

        // Verify BOM parts
        assert_eq!(tree.parts.len(), 3);

        // Root
        assert_eq!(tree.parts[0].part_number, "ASM-1000");
        assert_eq!(tree.parts[0].name, "Main Gearbox Assembly");
        assert_eq!(tree.parts[0].parent, None);
        assert_eq!(tree.parts[0].quantity, 1.0);

        // Bearing
        let brg = tree
            .parts
            .iter()
            .find(|p| p.part_number == "BRG-6205")
            .unwrap();
        assert_eq!(brg.parent.as_deref(), Some("ASM-1000"));
        assert_eq!(brg.quantity, 2.0);

        // Shaft
        let shaft = tree
            .parts
            .iter()
            .find(|p| p.part_number == "SHAFT-001")
            .unwrap();
        assert_eq!(shaft.parent.as_deref(), Some("ASM-1000"));
        assert_eq!(shaft.quantity, 1.0);

        // Verify CSV export
        let csv = tree.to_bom_csv();
        assert!(csv.contains("part_number,name,parent,quantity"));
        assert!(csv.contains("ASM-1000,Main Gearbox Assembly,,1"));
        assert!(csv.contains("BRG-6205,Deep Groove Ball Bearing,ASM-1000,2"));

        // Verify JSON export
        let json = tree.to_bom_json().unwrap();
        assert!(json.contains("\"part_number\": \"ASM-1000\""));
        assert!(json.contains("\"quantity\": 2.0"));
    }
}
