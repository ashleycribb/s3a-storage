//! A real `add-paper` path for the Scholar pack (usability fix B2).
//!
//! Before this module, the only way to get a paper into Tile 12
//! (`AcademicPaperRecord`) was `scholar ingest`, which requires NDJSON files
//! whose fields are already numeric hashes (`paper_id`, `hilbert_index`,
//! `topic_x/y/z`, `doi_prefix_hash`, `venue_id`). A researcher had to
//! hand-compute every hash and coordinate. This module does all of that
//! from a DOI (plus optional human metadata and an optional content file):
//!
//! * `paper_id` = FNV-1a hash of the normalised DOI — the same
//!   [`crate::xapi::hash_iri`] used by
//!   [`crate::records::CitationFingerprintRecord::from_content`] and by the
//!   `serve` ingest path, so a paper added here joins with the fingerprint
//!   and claim records that reference it by that hash.
//! * `doi_prefix_hash` = FNV-1a hash of the DOI prefix (the part before the
//!   first `/`, e.g. `10.1000`).
//! * `topic_x/y/z` + `hilbert_index`: the 3D Skilling Hilbert curve from
//!   [`s3a_core::point_to_hilbert_3d`], quantised exactly the way the
//!   harness `register_paper` already does it — each axis clamped to
//!   `[0, 1]`, scaled by 1023, `bits = 10`. No new curve is invented.
//!
//! ## How topic coordinates are derived (a documented default, not semantics)
//!
//! No topic-coordinate derivation existed anywhere in the codebase before
//! this: the harness takes caller-supplied coordinates, `serve` hard-codes
//! `(0.5, 0.5, 0.5)`, and the ingest fixture hand-writes coordinates that
//! are not even on the same scale. So this module defines one, explicitly:
//!
//! * If the caller supplies `--topic x,y,z`, those coordinates are used
//!   (clamped to `[0, 1]`). This is the only way to place a paper
//!   semantically today.
//! * Otherwise, coordinates are a deterministic hash spread: with a title,
//!   each axis is the mean over the title's lowercase tokens of
//!   `hash_iri(token + salt)` mapped to `[0, 1)`, so papers sharing title
//!   vocabulary land nearer each other; without a title, each axis is
//!   `hash_iri(doi + salt)` mapped to `[0, 1)`.
//!
//! That is a spread, not a semantic embedding: it puts papers somewhere
//! reproducible on the curve, it does not cluster by research topic. Real
//! topic placement needs embeddings or keywords and is a future design
//! decision, not something this module pretends to do.
//!
//! ## What is NOT stored
//!
//! `AcademicPaperRecord` holds hashes only — there is no title, author, or
//! DOI string field in the 64-byte record, so the title supplied here
//! influences placement only and is not recoverable from the archive
//! afterwards. If a `--file` is given, its bytes are hashed (SHA-256) into
//! a [`crate::records::CitationFingerprintRecord`] alongside the paper, so
//! the content can later be re-verified against the stored hash; the file
//! itself is never copied into the archive.
//!
//! ## Dossiers
//!
//! A dossier is a small JSON file carrying the same human metadata the
//! flags carry (`doi`, `title`, `year`, `venue_id`, `citation_count`,
//! `open_access`, `topic`). CLI flags override dossier fields. There is no
//! dossier format defined anywhere in `s3a-storage` today (the Navigator
//! SPEC's `paper_identity` is a viewer-side wiki-stem identity, not a
//! stored record), so this JSON shape is the new, minimal convention and
//! is documented here rather than inherited.

use std::io;
use std::path::Path;

use s3a_core::{point_to_hilbert_3d, S3ACoordinate, ADDON_EDU};
use s3a_engine::{MmapReader, QuerySieve, S3ACrudEngine};

use crate::ext::{ScholarCrudExt, ScholarQueryExt};
use crate::records::{
    AcademicPaperRecord, CitationFingerprintRecord, CONTENT_KIND_HTML, CONTENT_KIND_PDF,
};
use crate::xapi::hash_iri;

/// Hilbert resolution used for paper placement: 10 bits per axis, matching
/// the harness `register_paper` precedent (1024 cells per axis).
pub const HILBERT_BITS: u32 = 10;
/// Grid size per axis implied by [`HILBERT_BITS`]: `2^10 = 1024`.
pub const HILBERT_GRID: u32 = 1 << HILBERT_BITS;

/// Per-axis salts for the deterministic hash-spread topic derivation.
const TOPIC_SALTS: [&str; 3] = ["topic-x", "topic-y", "topic-z"];

/// Human-supplied metadata for one paper. Only `doi` is required; every
/// other field has a documented default (see [`add_paper`]).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AddPaperInput {
    /// DOI in any common form: `10.1000/182`, `doi:10.1000/182`,
    /// `https://doi.org/10.1000/182`. Normalised before hashing.
    pub doi: String,
    /// Paper title. Influences the hash-spread topic coordinates only; it
    /// is NOT stored in the record (there is no title field).
    pub title: Option<String>,
    /// Publication year. `None` stores 0, rendered as "year unknown".
    pub year: Option<u16>,
    /// Numeric venue id. `None` stores 0 (unknown). There is no
    /// venue-name registry, so names cannot be resolved to ids here.
    pub venue_id: Option<u16>,
    /// Citation count, default 0.
    pub citation_count: u32,
    /// Open-access flag, default false (paywalled).
    pub open_access: bool,
    /// Explicit topic coordinates in `[0, 1]`; overrides the hash-spread
    /// derivation when present (values are clamped).
    pub topic: Option<[f32; 3]>,
    /// Ingest timestamp (epoch seconds). `None` uses the current time.
    pub timestamp_sec: Option<u64>,
}

impl AddPaperInput {
    /// Builds an input from a dossier JSON document. Fields:
    /// `doi` (required string), `title`, `year`, `venue_id`,
    /// `citation_count`, `open_access`, `topic` (array of 3 numbers).
    /// Unknown fields are ignored. CLI flags override these values after
    /// this call by overwriting the corresponding fields.
    pub fn from_dossier_json(json: &str) -> Result<Self, String> {
        let v: serde_json::Value =
            serde_json::from_str(json).map_err(|e| format!("dossier is not valid JSON: {e}"))?;
        let obj = v
            .as_object()
            .ok_or_else(|| "dossier JSON must be an object".to_string())?;
        let mut input = AddPaperInput::default();
        input.doi = obj
            .get("doi")
            .and_then(|d| d.as_str())
            .ok_or_else(|| "dossier is missing required string field \"doi\"".to_string())?
            .to_string();
        input.title = obj
            .get("title")
            .and_then(|t| t.as_str())
            .map(|t| t.to_string());
        input.year = obj.get("year").and_then(|y| y.as_u64()).map(|y| y as u16);
        input.venue_id = obj
            .get("venue_id")
            .and_then(|y| y.as_u64())
            .map(|y| y as u16);
        input.citation_count = obj
            .get("citation_count")
            .and_then(|c| c.as_u64())
            .unwrap_or(0) as u32;
        input.open_access = obj
            .get("open_access")
            .and_then(|o| o.as_bool())
            .unwrap_or(false);
        if let Some(topic) = obj.get("topic").and_then(|t| t.as_array()) {
            if topic.len() != 3 {
                return Err("dossier \"topic\" must be an array of exactly 3 numbers".into());
            }
            let mut coords = [0.0f32; 3];
            for (i, n) in topic.iter().enumerate() {
                coords[i] = n
                    .as_f64()
                    .ok_or_else(|| "dossier \"topic\" entries must be numbers".to_string())?
                    as f32;
            }
            input.topic = Some(coords);
        }
        Ok(input)
    }
}

/// The outcome of a successful [`add_paper`] call.
#[derive(Debug, Clone)]
pub struct AddPaperOutcome {
    /// The DOI exactly as normalised (lowercase, no `doi:`/`https://doi.org/` prefix).
    pub doi_normalized: String,
    /// The stored paper record (hashes + coordinates, no strings).
    pub record: AcademicPaperRecord,
    /// Coordinate of the paper record as the query sieve reports it
    /// (level 0, 0-based tile index, record offset 0). This is read back
    /// from the archive after the write, so it is the coordinate a later
    /// `SIFT`/`FETCH` will actually return — not the 1-based tile id the
    /// writer returns (the harness `register_paper` uses that id directly,
    /// which is off by one against sieve coordinates).
    pub coordinate: S3ACoordinate,
    /// The stored citation fingerprint, if a content file was supplied.
    pub fingerprint: Option<CitationFingerprintRecord>,
    /// SHA-256 of the supplied content file, if one was given.
    pub content_sha256: Option<[u8; 32]>,
    /// SHA-256 of the supplied content file as lowercase hex, if given.
    pub content_sha256_hex: Option<String>,
}

/// Normalises a DOI for hashing: trims whitespace, strips a leading
/// `doi:`, `https://doi.org/`, `http://doi.org/`, `https://dx.doi.org/` or
/// `http://dx.doi.org/` prefix (case-insensitive), and lowercases.
/// Returns an error unless the result looks like a DOI (`10.<prefix>/<suffix>`).
pub fn normalize_doi(raw: &str) -> Result<String, String> {
    let mut doi = raw.trim().to_string();
    let lower = doi.to_ascii_lowercase();
    for prefix in [
        "https://dx.doi.org/",
        "http://dx.doi.org/",
        "https://doi.org/",
        "http://doi.org/",
        "doi:",
    ] {
        if lower.starts_with(prefix) {
            doi = doi[prefix.len()..].to_string();
            break;
        }
    }
    let doi = doi.trim().to_ascii_lowercase();
    if doi.is_empty() {
        return Err("DOI is empty".to_string());
    }
    if doi.contains(char::is_whitespace) {
        return Err(format!("DOI contains whitespace: {doi:?}"));
    }
    let Some((prefix, suffix)) = doi.split_once('/') else {
        return Err(format!(
            "DOI must contain a '/' separating prefix and suffix: {raw:?}"
        ));
    };
    if !prefix.starts_with("10.") || prefix.len() <= 3 || suffix.is_empty() {
        return Err(format!(
            "DOI must start with \"10.\" and have a non-empty suffix: {raw:?}"
        ));
    }
    Ok(doi)
}

/// The DOI prefix: the part before the first `/` (e.g. `10.1000`).
pub fn doi_prefix(normalized_doi: &str) -> &str {
    normalized_doi.split('/').next().unwrap_or(normalized_doi)
}

/// Maps a 64-bit hash uniformly into `[0, 1)` using its top 53 bits.
pub fn unit_from_hash(hash: u64) -> f32 {
    ((hash >> 11) as f64 / 9007199254740992.0f64) as f32
}

/// Derives the hash-spread topic coordinates for a paper (see the module
/// docs). With a title, each axis is the mean of the salted token hashes;
/// without one, each axis is the salted DOI hash. Deterministic for the
/// same DOI + title.
///
/// The salt goes FIRST (`salt + "\0" + token`): FNV-1a avalanches poorly
/// for a difference in the last byte of the input, so a trailing salt
/// leaves all three axes nearly identical (every paper collapses onto
/// the curve's main diagonal). A leading salt is mixed through every
/// subsequent byte's multiply step, so the axes spread.
pub fn derive_topic_coords(normalized_doi: &str, title: Option<&str>) -> [f32; 3] {
    let tokens: Vec<String> = title
        .map(|t| {
            t.split(|c: char| !c.is_alphanumeric())
                .filter(|w| !w.is_empty())
                .map(|w| w.to_ascii_lowercase())
                .collect()
        })
        .unwrap_or_default();
    let mut coords = [0.0f32; 3];
    for (axis, salt) in TOPIC_SALTS.iter().enumerate() {
        if tokens.is_empty() {
            coords[axis] = unit_from_hash(hash_iri(&format!("{salt}\0{normalized_doi}")));
        } else {
            let sum: f64 = tokens
                .iter()
                .map(|tok| unit_from_hash(hash_iri(&format!("{salt}\0{tok}"))) as f64)
                .sum();
            coords[axis] = (sum / tokens.len() as f64) as f32;
        }
    }
    coords
}

/// Turns topic coordinates in `[0, 1]` into a Skilling Hilbert index:
/// each axis is clamped, scaled to the 1024-cell grid, and passed to
/// [`point_to_hilbert_3d`] with `bits = 10` — the harness precedent.
pub fn topic_coords_to_hilbert(topic: [f32; 3]) -> u64 {
    point_to_hilbert_3d(
        (topic[0].clamp(0.0, 1.0) * (HILBERT_GRID - 1) as f32) as u32,
        (topic[1].clamp(0.0, 1.0) * (HILBERT_GRID - 1) as f32) as u32,
        (topic[2].clamp(0.0, 1.0) * (HILBERT_GRID - 1) as f32) as u32,
        HILBERT_BITS,
    )
}

/// Infers the citation-fingerprint content kind from a file name:
/// `.pdf` → PDF, `.html`/`.htm` → HTML, anything else → 0 (unknown).
pub fn content_kind_for_file_name(file_name: &str) -> u32 {
    let lower = file_name.to_ascii_lowercase();
    if lower.ends_with(".pdf") {
        CONTENT_KIND_PDF
    } else if lower.ends_with(".html") || lower.ends_with(".htm") {
        CONTENT_KIND_HTML
    } else {
        0
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Builds the [`AcademicPaperRecord`] (and, when `file_bytes` is given, the
/// [`CitationFingerprintRecord`]) for an input, computing every hash and
/// coordinate. Pure: writes nothing. `timestamp_sec` is the ingest time
/// used for both records.
pub fn build_paper_record(
    input: &AddPaperInput,
    file_bytes: Option<&[u8]>,
    file_kind: u32,
    timestamp_sec: u64,
) -> Result<(AcademicPaperRecord, Option<CitationFingerprintRecord>), String> {
    let doi = normalize_doi(&input.doi)?;
    let paper_id = hash_iri(&doi);
    let doi_prefix_hash = hash_iri(doi_prefix(&doi));
    let topic = match input.topic {
        Some(t) => [
            t[0].clamp(0.0, 1.0),
            t[1].clamp(0.0, 1.0),
            t[2].clamp(0.0, 1.0),
        ],
        None => derive_topic_coords(&doi, input.title.as_deref()),
    };
    let hilbert_index = topic_coords_to_hilbert(topic);
    let record = AcademicPaperRecord::new(
        paper_id,
        timestamp_sec,
        hilbert_index,
        topic[0],
        topic[1],
        topic[2],
        input.citation_count,
        input.year.unwrap_or(0),
        input.venue_id.unwrap_or(0),
        u16::from(input.open_access),
        0,
        doi_prefix_hash,
    );
    let fingerprint = file_bytes.map(|bytes| {
        CitationFingerprintRecord::from_content(
            paper_id,
            &doi,
            bytes,
            file_kind,
            &format!("https://doi.org/{doi}"),
            1,
            timestamp_sec,
        )
    });
    Ok((record, fingerprint))
}

/// Finds a paper already in `archive_path` with the given normalised DOI
/// hash, returning its record and coordinate, or `None`.
fn find_paper_by_id(
    archive_path: &Path,
    paper_id: u64,
) -> Option<(AcademicPaperRecord, S3ACoordinate)> {
    let reader = MmapReader::open(archive_path).ok()?;
    let sieve = QuerySieve::new(&reader);
    sieve
        .query_academic_papers_3d([-f32::INFINITY; 3], [f32::INFINITY; 3], 0, u16::MAX)
        .into_iter()
        .find(|(rec, _)| rec.paper_id == paper_id)
}

/// Adds one paper to the archive at `archive_path` (created if missing,
/// with `ADDON_EDU` enabled), computing all hashes and Hilbert coordinates
/// from the DOI and optional metadata. `file_bytes`/`file_kind` describe
/// an optional content file (e.g. the paper PDF): its SHA-256 is stored as
/// a citation fingerprint alongside the paper; the bytes themselves are
/// never written to the archive.
///
/// Fails (without writing anything) when the DOI is malformed or a paper
/// with the same DOI is already in the archive — duplicates are refused
/// rather than silently stored twice.
pub fn add_paper<P: AsRef<Path>>(
    archive_path: P,
    input: &AddPaperInput,
    file_bytes: Option<&[u8]>,
    file_kind: u32,
) -> io::Result<AddPaperOutcome> {
    let archive_path = archive_path.as_ref();
    let doi =
        normalize_doi(&input.doi).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let paper_id = hash_iri(&doi);
    if archive_path.exists() {
        if let Some((_, coord)) = find_paper_by_id(archive_path, paper_id) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "paper with DOI {doi} is already in {archive} (paper_id 0x{paper_id:016x} at {coord}); refusing to add a duplicate",
                    archive = archive_path.display(),
                ),
            ));
        }
    }
    let timestamp_sec = input.timestamp_sec.unwrap_or_else(now_secs);
    let (record, fingerprint) = build_paper_record(input, file_bytes, file_kind, timestamp_sec)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let content_sha256 = fingerprint.as_ref().map(|fp| fp.content_sha256);
    let content_sha256_hex = fingerprint.as_ref().map(|fp| fp.content_sha256_hex());

    let engine = S3ACrudEngine::open_or_create(archive_path)?;
    engine.enable_addon(ADDON_EDU)?;
    let _tile_id = engine.create_academic_papers(&[record])?;
    if let Some(fp) = &fingerprint {
        engine.create_citation_fingerprints(std::slice::from_ref(fp))?;
    }
    drop(engine);
    // Read the coordinate back through the query path instead of trusting
    // the writer's 1-based tile id: sieve/FETCH coordinates use the
    // 0-based tile index, and this also verifies the write landed.
    let (_, coordinate) = find_paper_by_id(archive_path, paper_id).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "paper with DOI {doi} was written but could not be read back from {archive}",
                archive = archive_path.display()
            ),
        )
    })?;
    Ok(AddPaperOutcome {
        doi_normalized: doi,
        record,
        coordinate,
        fingerprint,
        content_sha256,
        content_sha256_hex,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use s3a_core::hilbert_to_point_3d;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_path(label: &str) -> std::path::PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut p = std::env::temp_dir();
        p.push(format!(
            "s3a_add_paper_test_{}_{}_{}.s3a",
            label,
            std::process::id(),
            id
        ));
        p
    }

    struct Guard(std::path::PathBuf);
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn test_normalize_doi_strips_prefixes_and_lowercases() {
        assert_eq!(
            normalize_doi("https://doi.org/10.1000/ABC-123").unwrap(),
            "10.1000/abc-123"
        );
        assert_eq!(normalize_doi("DOI:10.1234/xyz").unwrap(), "10.1234/xyz");
        assert_eq!(
            normalize_doi("  http://dx.doi.org/10.5555/12345678  ").unwrap(),
            "10.5555/12345678"
        );
        assert_eq!(normalize_doi("10.1000/182").unwrap(), "10.1000/182");
    }

    #[test]
    fn test_normalize_doi_rejects_malformed() {
        for bad in [
            "",
            "not-a-doi",
            "11.1000/abc",
            "10./abc",
            "10.1000/",
            "10.1000/a b",
        ] {
            assert!(normalize_doi(bad).is_err(), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn test_build_paper_record_hashes_and_hilbert_roundtrip() {
        let input = AddPaperInput {
            doi: "https://doi.org/10.1000/182".into(),
            title: Some("Hilbert curves for literature review".into()),
            year: Some(2024),
            venue_id: Some(7),
            citation_count: 25,
            open_access: true,
            topic: None,
            timestamp_sec: Some(1_700_000_000),
        };
        let (rec, fp) = build_paper_record(&input, None, 0, 1_700_000_000).unwrap();
        assert!(fp.is_none());
        // paper_id is the FNV-1a hash of the normalised DOI, doi_prefix_hash of its prefix.
        assert_eq!(rec.paper_id, hash_iri("10.1000/182"));
        assert_eq!(rec.doi_prefix_hash, hash_iri("10.1000"));
        assert_eq!(rec.year, 2024);
        assert_eq!(rec.venue_id, 7);
        assert_eq!(rec.citation_count, 25);
        assert_eq!(rec.open_access_flag, 1);
        assert_eq!(rec.timestamp_sec, 1_700_000_000);
        // Topic coords are on [0, 1] and the Hilbert index round-trips to
        // the same quantised grid cell through the existing Skilling curve.
        assert!((0.0..1.0).contains(&rec.topic_x));
        assert!((0.0..1.0).contains(&rec.topic_y));
        assert!((0.0..1.0).contains(&rec.topic_z));
        let (x, y, z) = hilbert_to_point_3d(rec.hilbert_index, HILBERT_BITS);
        assert_eq!(x, (rec.topic_x.clamp(0.0, 1.0) * 1023.0) as u32);
        assert_eq!(y, (rec.topic_y.clamp(0.0, 1.0) * 1023.0) as u32);
        assert_eq!(z, (rec.topic_z.clamp(0.0, 1.0) * 1023.0) as u32);
    }

    #[test]
    fn test_explicit_topic_overrides_derivation_and_clamps() {
        let input = AddPaperInput {
            doi: "10.1000/182".into(),
            topic: Some([0.25, 1.5, -0.5]),
            ..Default::default()
        };
        let (rec, _) = build_paper_record(&input, None, 0, 100).unwrap();
        assert_eq!([rec.topic_x, rec.topic_y, rec.topic_z], [0.25, 1.0, 0.0]);
        let (x, y, z) = hilbert_to_point_3d(rec.hilbert_index, HILBERT_BITS);
        assert_eq!((x, y, z), (255, 1023, 0));
    }

    #[test]
    fn test_derive_topic_coords_deterministic_and_title_sensitive() {
        let a = derive_topic_coords("10.1000/182", Some("Hilbert curves for review"));
        let b = derive_topic_coords("10.1000/182", Some("Hilbert curves for review"));
        assert_eq!(a, b);
        let no_title = derive_topic_coords("10.1000/182", None);
        assert_ne!(a, no_title);
        let other_title = derive_topic_coords("10.1000/182", Some("Quantum error correction"));
        assert_ne!(a, other_title);
        for c in a.iter().chain(no_title.iter()) {
            assert!((0.0..1.0).contains(c));
        }
        // The three axes must actually spread: a trailing salt left them
        // identical (all papers on the curve's main diagonal), because
        // FNV-1a barely avalanches a last-byte difference into the top
        // bits. Regression test for that diagonal collapse.
        for coords in [a, no_title, other_title] {
            let spread =
                coords.iter().fold(0.0f32, |m, c| m.max(*c)) - coords.iter().fold(1.0f32, |m, c| m.min(*c));
            assert!(
                spread > 0.01,
                "topic axes must spread, got {coords:?} (spread {spread})"
            );
        }
    }

    #[test]
    fn test_dossier_json_parsing_and_missing_doi() {
        let input = AddPaperInput::from_dossier_json(
            r#"{"doi":"10.1000/182","title":"Some Title","year":2023,"venue_id":3,"citation_count":4,"open_access":true,"topic":[0.1,0.2,0.3]}"#,
        )
        .unwrap();
        assert_eq!(input.doi, "10.1000/182");
        assert_eq!(input.title.as_deref(), Some("Some Title"));
        assert_eq!(input.year, Some(2023));
        assert_eq!(input.venue_id, Some(3));
        assert_eq!(input.citation_count, 4);
        assert!(input.open_access);
        assert_eq!(input.topic, Some([0.1, 0.2, 0.3]));
        assert!(AddPaperInput::from_dossier_json(r#"{"title":"No DOI"}"#).is_err());
        assert!(AddPaperInput::from_dossier_json(r#"{"doi":"10.1/x","topic":[0.1,0.2]}"#).is_err());
    }

    #[test]
    fn test_add_paper_writes_and_reads_back_with_fingerprint() {
        let path = temp_path("roundtrip");
        let _guard = Guard(path.clone());
        let pdf = b"%PDF-1.4 fake paper bytes for hashing";
        let input = AddPaperInput {
            doi: "10.1000/182".into(),
            title: Some("A paper about Hilbert curves".into()),
            year: Some(2024),
            venue_id: Some(1),
            citation_count: 3,
            open_access: true,
            topic: None,
            timestamp_sec: Some(1_700_000_001),
        };
        let out = add_paper(&path, &input, Some(pdf), CONTENT_KIND_PDF).unwrap();
        assert_eq!(out.doi_normalized, "10.1000/182");
        assert_eq!(out.coordinate.tile_id, 0);
        assert_eq!(out.coordinate.record_offset, 0);
        // Fingerprint joins on paper_id and its SHA-256 matches the file bytes.
        let fp = out.fingerprint.expect("fingerprint must be stored");
        assert_eq!(fp.paper_id, out.record.paper_id);
        assert_eq!(fp.doi_hash, out.record.paper_id);
        assert_eq!(fp.content_kind_str(), "pdf");
        use sha2::Digest;
        let expected: [u8; 32] = sha2::Sha256::digest(pdf).into();
        assert_eq!(fp.content_sha256, expected);
        assert_eq!(out.content_sha256, Some(expected));

        // Read back through the normal query path.
        let reader = MmapReader::open(&path).unwrap();
        let sieve = QuerySieve::new(&reader);
        let papers =
            sieve.query_academic_papers_3d([-f32::INFINITY; 3], [f32::INFINITY; 3], 0, u16::MAX);
        assert_eq!(papers.len(), 1);
        assert_eq!(papers[0].0, out.record);
        assert_eq!(papers[0].1, out.coordinate);
        let fps = sieve.query_citation_fingerprints(Some(out.record.paper_id));
        assert_eq!(fps.len(), 1);
        assert_eq!(fps[0].0.content_sha256, expected);
    }

    #[test]
    fn test_add_paper_refuses_duplicate_doi() {
        let path = temp_path("dup");
        let _guard = Guard(path.clone());
        let input = AddPaperInput {
            doi: "10.1000/182".into(),
            timestamp_sec: Some(100),
            ..Default::default()
        };
        add_paper(&path, &input, None, 0).unwrap();
        // Same DOI in a different surface form is the same paper.
        let dup = AddPaperInput {
            doi: "https://doi.org/10.1000/182".into(),
            timestamp_sec: Some(200),
            ..Default::default()
        };
        let err = add_paper(&path, &dup, None, 0).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        // Exactly one paper is stored.
        let reader = MmapReader::open(&path).unwrap();
        let sieve = QuerySieve::new(&reader);
        assert_eq!(
            sieve
                .query_academic_papers_3d([-f32::INFINITY; 3], [f32::INFINITY; 3], 0, u16::MAX)
                .len(),
            1
        );
    }

    #[test]
    fn test_add_paper_rejects_bad_doi_without_writing() {
        let path = temp_path("baddoi");
        let _guard = Guard(path.clone());
        let input = AddPaperInput {
            doi: "not-a-doi".into(),
            ..Default::default()
        };
        assert!(add_paper(&path, &input, None, 0).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn test_content_kind_for_file_name() {
        assert_eq!(content_kind_for_file_name("paper.PDF"), CONTENT_KIND_PDF);
        assert_eq!(content_kind_for_file_name("page.html"), CONTENT_KIND_HTML);
        assert_eq!(content_kind_for_file_name("notes.txt"), 0);
    }
}
