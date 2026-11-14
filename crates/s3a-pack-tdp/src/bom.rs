//! Bill of Materials (BOM) Export for Technical Data Packages.

use crate::step::{parse_step, StepAssemblyTree};

/// Exports an assembly tree to standard CSV Bill of Materials.
pub fn export_bom_csv(tree: &StepAssemblyTree) -> String {
    tree.to_bom_csv()
}

/// Exports an assembly tree to JSON Bill of Materials.
pub fn export_bom_json(tree: &StepAssemblyTree) -> Result<String, serde_json::Error> {
    tree.to_bom_json()
}

/// Parses raw STEP bytes and exports its BOM as CSV.
pub fn export_bom_csv_from_bytes(step_bytes: &[u8]) -> Result<String, String> {
    let tree = parse_step(step_bytes)?;
    Ok(export_bom_csv(&tree))
}

/// Parses raw STEP bytes and exports its BOM as JSON.
pub fn export_bom_json_from_bytes(step_bytes: &[u8]) -> Result<String, String> {
    let tree = parse_step(step_bytes)?;
    export_bom_json(&tree).map_err(|e| format!("Failed to serialize BOM JSON: {}", e))
}
