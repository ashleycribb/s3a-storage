//! PDF Metadata Extraction and Serialization for Technical Data Packages (TDP).

use lopdf::Document;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Metadata extracted from a PDF document stored in an S3A vault.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PdfMetadata {
    pub key: String,
    pub mime_type: String,
    pub size_bytes: usize,
    pub page_count: usize,
    pub title: Option<String>,
    pub sha256: String,
}

/// Computes the SHA-256 checksum of raw bytes, returning lowercase hex string.
pub fn compute_sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Extracts metadata from raw PDF bytes using pure-Rust `lopdf`.
pub fn extract_pdf_metadata(key: &str, bytes: &[u8]) -> Result<PdfMetadata, String> {
    let size_bytes = bytes.len();
    let sha256 = compute_sha256(bytes);

    let doc = Document::load_mem(bytes)
        .map_err(|e| format!("Failed to parse PDF document with lopdf: {}", e))?;

    let page_count = doc.get_pages().len();

    let title = if let Ok(info_ref) = doc.trailer.get(b"Info") {
        let info_dict = match info_ref {
            lopdf::Object::Reference(id) => {
                doc.get_object(*id).ok().and_then(|obj| obj.as_dict().ok())
            }
            lopdf::Object::Dictionary(dict) => Some(dict),
            _ => None,
        };
        info_dict.and_then(|dict| {
            dict.get(b"Title").ok().and_then(|obj| match obj {
                lopdf::Object::String(bytes, _) => {
                    // Try UTF-8 first, fallback to ISO-8859-1 / lossy
                    String::from_utf8(bytes.clone())
                        .ok()
                        .or_else(|| Some(String::from_utf8_lossy(bytes).to_string()))
                }
                _ => None,
            })
        })
    } else {
        None
    };

    Ok(PdfMetadata {
        key: key.to_string(),
        mime_type: "application/pdf".to_string(),
        size_bytes,
        page_count,
        title,
        sha256,
    })
}

/// Generates a minimal 1-page valid PDF in memory for tests and fixtures.
pub fn create_test_pdf(title: &str) -> Vec<u8> {
    use lopdf::content::Content;
    use lopdf::{dictionary, Document, Object, Stream};

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });
    let resources_id = doc.add_object(dictionary! {
        "Font" => dictionary! {
            "F1" => font_id,
        },
    });
    let content = Content { operations: vec![] };
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
        "Resources" => resources_id,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    });
    let pages = dictionary! {
        "Type" => "Pages",
        "Kids" => vec![page_id.into()],
        "Count" => 1,
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);

    let info_id = doc.add_object(dictionary! {
        "Title" => Object::string_literal(title),
        "Producer" => Object::string_literal("S3A Tests"),
    });
    doc.trailer.set("Info", info_id);

    let mut buf = Vec::new();
    doc.save_to(&mut buf).unwrap();
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_pdf_metadata() {
        let pdf_bytes = create_test_pdf("Turbine Assembly Drawing");
        let meta = extract_pdf_metadata("drawings/turbine.pdf", &pdf_bytes).unwrap();

        assert_eq!(meta.key, "drawings/turbine.pdf");
        assert_eq!(meta.mime_type, "application/pdf");
        assert_eq!(meta.page_count, 1);
        assert_eq!(meta.title.as_deref(), Some("Turbine Assembly Drawing"));
        assert_eq!(meta.size_bytes, pdf_bytes.len());
        assert_eq!(meta.sha256.len(), 64);
    }
}
