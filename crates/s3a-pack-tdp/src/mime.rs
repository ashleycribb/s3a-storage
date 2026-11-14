//! MIME type and file extension detection from magic bytes and payloads.

/// Detects the MIME type and recommended file extension from payload bytes.
///
/// Returns `(mime_type, file_extension)`.
pub fn detect_mime_and_ext(bytes: &[u8]) -> (&'static str, &'static str) {
    if bytes.is_empty() {
        return ("application/octet-stream", "bin");
    }

    // PDF magic: %PDF-
    if bytes.starts_with(b"%PDF-") {
        return ("application/pdf", "pdf");
    }

    // PNG magic: 89 50 4E 47 0D 0A 1A 0A
    if bytes.len() >= 8 && &bytes[0..8] == b"\x89PNG\r\n\x1a\n" {
        return ("image/png", "png");
    }

    // JPEG magic: FF D8 FF
    if bytes.len() >= 3 && &bytes[0..3] == b"\xff\xd8\xff" {
        return ("image/jpeg", "jpg");
    }

    // STEP (ISO 10303-21): starts with ISO-10303-21; (allowing leading whitespace / comments)
    let prefix_sample = if bytes.len() > 1024 {
        &bytes[..1024]
    } else {
        bytes
    };
    if let Ok(prefix_str) = std::str::from_utf8(prefix_sample) {
        let trimmed = prefix_str.trim_start();
        if trimmed.starts_with("ISO-10303-21;")
            || trimmed.starts_with("/*") && trimmed.contains("ISO-10303-21;")
        {
            return ("application/step", "step");
        }
    }

    // JSON: starts with '{' or '[' (ignoring whitespace) and parses valid JSON
    let trimmed = bytes
        .iter()
        .copied()
        .skip_while(|b| b.is_ascii_whitespace())
        .collect::<Vec<u8>>();
    if !trimmed.is_empty()
        && (trimmed[0] == b'{' || trimmed[0] == b'[')
        && serde_json::from_slice::<serde_json::Value>(bytes).is_ok()
    {
        return ("application/json", "json");
    }

    // UTF-8 Text / CSV
    if let Ok(s) = std::str::from_utf8(bytes) {
        if s.contains(',') && s.contains('\n') {
            return ("text/csv", "csv");
        }
        return ("text/plain", "txt");
    }

    ("application/octet-stream", "bin")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_pdf() {
        let pdf = b"%PDF-1.7\n1 0 obj\n<<>>\nendobj\n%%EOF";
        assert_eq!(detect_mime_and_ext(pdf), ("application/pdf", "pdf"));
    }

    #[test]
    fn test_detect_step() {
        let step = b"ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;";
        assert_eq!(detect_mime_and_ext(step), ("application/step", "step"));

        let step_with_ws = b"  \r\n  ISO-10303-21;\nHEADER;";
        assert_eq!(
            detect_mime_and_ext(step_with_ws),
            ("application/step", "step")
        );
    }

    #[test]
    fn test_detect_json() {
        let json = b"  {\"name\": \"S3A\", \"version\": 1}  ";
        assert_eq!(detect_mime_and_ext(json), ("application/json", "json"));
    }

    #[test]
    fn test_detect_binary_unknown() {
        let bin = &[0x00, 0x11, 0x22, 0x33, 0xFF];
        assert_eq!(
            detect_mime_and_ext(bin),
            ("application/octet-stream", "bin")
        );
    }
}
