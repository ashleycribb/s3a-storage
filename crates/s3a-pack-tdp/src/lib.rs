//! S3A Technical Data Package (TDP) Pack: PDF Metadata, AP242 STEP Assembly Parsing, and BOM Export.

pub mod bom;
pub mod mime;
pub mod pdf;
pub mod step;
pub mod store;

pub use bom::{
    export_bom_csv, export_bom_csv_from_bytes, export_bom_json, export_bom_json_from_bytes,
};
pub use mime::detect_mime_and_ext;
pub use pdf::{compute_sha256, extract_pdf_metadata, PdfMetadata};
pub use step::{parse_step, StepAssemblyPartRecord, StepAssemblyTree, StepHeader};
pub use store::{store_pdf, store_step, write_document_record};
