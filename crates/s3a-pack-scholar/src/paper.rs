use bytemuck::{Pod, Zeroable};
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use crate::pack::ScholarTileType;
use crate::records::AcademicPaperRecord;

/// 64-byte binary blob header preceding variable-length paper record payloads
/// in `ScholarTileType::ACADEMIC_PAPERS` (Tile 12, layout version 2).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct ScholarPaperBlobHeader {
    pub key_hash: u64,         // 64-bit hash of canonical key (offset 0..8)
    pub sha256_hash: [u8; 32], // SHA-256 hash of paper record JSON (offset 8..40)
    pub total_bytes: u32,      // Total bytes of UTF-8 JSON payload (offset 40..44)
    pub chunk_index: u16,      // 0-based chunk index (offset 44..46)
    pub total_chunks: u16,     // Total chunks (offset 46..48)
    pub chunk_bytes: u32,      // Bytes in this chunk (offset 48..52)
    pub chunk_crc32: u32,      // CRC32C of this chunk's payload (offset 52..56)
    pub schema_version: u32,   // Schema layout version (2 for v2) (offset 56..60)
    pub _reserved: u32,        // Padding to 64 bytes (offset 60..64)
}

impl Default for ScholarPaperBlobHeader {
    fn default() -> Self {
        Self {
            key_hash: 0,
            sha256_hash: [0u8; 32],
            total_bytes: 0,
            chunk_index: 0,
            total_chunks: 1,
            chunk_bytes: 0,
            chunk_crc32: 0,
            schema_version: 2,
            _reserved: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScholarAuthor {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orcid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub affiliation: Option<String>,
}

impl ScholarAuthor {
    pub fn new(name: &str, orcid: Option<&str>) -> Self {
        Self {
            name: name.nfc().collect(),
            orcid: orcid.map(|s| s.nfc().collect()),
            affiliation: None,
        }
    }
}

/// Versioned, self-contained scholar paper record.
///
/// Holds all bibliographic, provenance, and content metadata necessary to cite and
/// read an academic work across multi-decade preservation horizons.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScholarPaperRecord {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doi: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<ScholarAuthor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abstract_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub venue: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_date: Option<String>,
}

/// Accepts exactly `YYYY-MM-DDTHH:MM:SS[.fraction]` followed by `Z` or `+00:00`: upper-case
/// `T`/`Z`, no surrounding whitespace, a real calendar date (leap years included). `-00:00`
/// (RFC 3339 "offset unknown") and any other offset are rejected.
pub fn validate_utc_iso8601(s: &str) -> std::io::Result<()> {
    let bad = |why: &str| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("date '{s}' is not a UTC ISO-8601 timestamp (YYYY-MM-DDTHH:MM:SSZ): {why}"),
        )
    };
    let body = if let Some(b) = s.strip_suffix('Z') {
        b
    } else if let Some(b) = s.strip_suffix("+00:00") {
        b
    } else {
        return Err(bad("must end in 'Z' or '+00:00'"));
    };
    let (main, frac) = match body.split_once('.') {
        Some((m, f)) => (m, Some(f)),
        None => (body, None),
    };
    if let Some(f) = frac {
        if f.is_empty() || !f.bytes().all(|c| c.is_ascii_digit()) {
            return Err(bad("fractional seconds must be digits"));
        }
    }
    let b = main.as_bytes();
    if b.len() != 19
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
    {
        return Err(bad("layout must be YYYY-MM-DDTHH:MM:SS"));
    }
    let num = |r: std::ops::Range<usize>| -> std::io::Result<u32> {
        let part = &main[r];
        if !part.bytes().all(|c| c.is_ascii_digit()) {
            return Err(bad("non-digit in a numeric field"));
        }
        Ok(part.parse().expect("ascii digits"))
    };
    let (year, month, day) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hour, min, sec) = (num(11..13)?, num(14..16)?, num(17..19)?);
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return Err(bad("month out of range")),
    };
    if year == 0 || day == 0 || day > days_in_month {
        return Err(bad("day out of range"));
    }
    if hour > 23 || min > 59 || sec > 60 {
        return Err(bad("time out of range"));
    }
    Ok(())
}

#[cfg(test)]
mod date_tests {
    use super::validate_utc_iso8601 as v;

    #[test]
    fn accepts_utc_forms() {
        for ok in [
            "2017-06-12T00:00:00Z",
            "2024-02-29T23:59:59.123Z",
            "2017-06-12T00:00:00+00:00",
        ] {
            assert!(v(ok).is_ok(), "{ok}");
        }
    }

    #[test]
    fn rejects_non_utc_and_impossible_dates() {
        for bad in [
            "2017-06-12T00:00:00-00:00",
            "2017-06-12T00:00:00+02:00",
            "2017-06-12t00:00:00z",
            " 2017-06-12T00:00:00Z",
            "2017-06-12T00:00:00Z ",
            "2017-04-31T00:00:00Z",
            "2023-02-29T00:00:00Z",
            "2017-6-12T00:00:00Z",
            "2017-06-12T24:00:00Z",
            "2017-06-12T00:00:00.Z",
            "+017-06-12T00:00:00Z",
            "2017-06-12 00:00:00Z",
        ] {
            assert!(v(bad).is_err(), "{bad}");
        }
    }
}

impl ScholarPaperRecord {
    pub fn new(key: &str, title: &str) -> Self {
        Self {
            key: key.nfc().collect(),
            doi: None,
            aliases: Vec::new(),
            title: Some(title.nfc().collect()),
            authors: Vec::new(),
            abstract_text: None,
            venue: None,
            year: None,
            language: None,
            license: None,
            source_url: None,
            published_date: None,
        }
    }

    pub fn validate(&self) -> std::io::Result<()> {
        match &self.title {
            Some(t) if !t.trim().is_empty() => {}
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "paper title cannot be empty",
                ));
            }
        }
        if let Some(d) = &self.published_date {
            validate_utc_iso8601(d)?;
        }
        Ok(())
    }

    /// Enforces UTF-8 NFC normalization on all text fields.
    pub fn normalize(&mut self) {
        self.key = self.key.nfc().collect();
        if let Some(d) = &self.doi {
            self.doi = Some(d.nfc().collect());
        }
        self.aliases = self.aliases.iter().map(|a| a.nfc().collect()).collect();
        if let Some(t) = &self.title {
            self.title = Some(t.nfc().collect());
        }
        for a in &mut self.authors {
            a.name = a.name.nfc().collect();
            if let Some(o) = &a.orcid {
                a.orcid = Some(o.nfc().collect());
            }
            if let Some(aff) = &a.affiliation {
                a.affiliation = Some(aff.nfc().collect());
            }
        }
        if let Some(ab) = &self.abstract_text {
            self.abstract_text = Some(ab.nfc().collect());
        }
        if let Some(v) = &self.venue {
            self.venue = Some(v.nfc().collect());
        }
        if let Some(l) = &self.language {
            self.language = Some(l.nfc().collect());
        }
        if let Some(lic) = &self.license {
            self.license = Some(lic.nfc().collect());
        }
        if let Some(u) = &self.source_url {
            self.source_url = Some(u.nfc().collect());
        }
        if let Some(pd) = &self.published_date {
            self.published_date = Some(pd.nfc().collect());
        }
    }

    /// Appends this paper record into an archive, splitting large payloads across
    /// continuation hyper-tiles (Tile 12) if necessary.
    pub fn put_to_writer(&self, writer: &mut s3a_engine::TileWriter) -> std::io::Result<()> {
        self.validate()?;
        let mut normalized = self.clone();
        normalized.normalize();

        let json_bytes = serde_json::to_vec(&normalized)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        let mut hasher = sha2::Sha256::default();
        use sha2::Digest;
        hasher.update(&json_bytes);
        let sha256_hash: [u8; 32] = hasher.finalize().into();

        let key_hash = crc32c::crc32c(normalized.key.as_bytes()) as u64;
        let total_bytes = json_bytes.len() as u32;

        let max_chunk_capacity =
            s3a_core::HYPER_TILE_PAYLOAD_SIZE - std::mem::size_of::<ScholarPaperBlobHeader>();
        let total_chunks =
            ((json_bytes.len() + max_chunk_capacity - 1) / max_chunk_capacity).max(1) as u16;

        for chunk_idx in 0..total_chunks {
            let start = chunk_idx as usize * max_chunk_capacity;
            let end = (start + max_chunk_capacity).min(json_bytes.len());
            let chunk_slice = &json_bytes[start..end];
            let chunk_crc = crc32c::crc32c(chunk_slice);

            let blob_hdr = ScholarPaperBlobHeader {
                key_hash,
                sha256_hash,
                total_bytes,
                chunk_index: chunk_idx,
                total_chunks,
                chunk_bytes: chunk_slice.len() as u32,
                chunk_crc32: chunk_crc,
                schema_version: 2,
                _reserved: 0,
            };

            let mut tile_payload = Vec::with_capacity(
                std::mem::size_of::<ScholarPaperBlobHeader>() + chunk_slice.len(),
            );
            tile_payload.extend_from_slice(bytemuck::bytes_of(&blob_hdr));
            tile_payload.extend_from_slice(chunk_slice);

            let mut hdr = s3a_core::HyperTileHeader::new(0, ScholarTileType::ACADEMIC_PAPERS);
            hdr.record_layout_version = 2;
            hdr.payload_bytes = tile_payload.len() as u32;
            hdr.data_crc32 = crc32c::crc32c(&tile_payload);

            writer.write_raw_hyper_tile(&hdr, &tile_payload)?;
        }

        Ok(())
    }

    /// Appends this paper record into an in-memory archive byte buffer, returning the updated bytes.
    pub fn put_to_bytes(&self, archive_bytes: &[u8]) -> std::io::Result<Vec<u8>> {
        self.validate()?;
        let mut normalized = self.clone();
        normalized.normalize();

        let json_bytes = serde_json::to_vec(&normalized)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        let mut hasher = sha2::Sha256::default();
        use sha2::Digest;
        hasher.update(&json_bytes);
        let sha256_hash: [u8; 32] = hasher.finalize().into();

        let key_hash = crc32c::crc32c(normalized.key.as_bytes()) as u64;
        let total_bytes = json_bytes.len() as u32;

        let max_chunk_capacity =
            s3a_core::HYPER_TILE_PAYLOAD_SIZE - std::mem::size_of::<ScholarPaperBlobHeader>();
        let total_chunks =
            ((json_bytes.len() + max_chunk_capacity - 1) / max_chunk_capacity).max(1) as u16;

        let mut out = archive_bytes.to_vec();

        for chunk_idx in 0..total_chunks {
            let start = chunk_idx as usize * max_chunk_capacity;
            let end = (start + max_chunk_capacity).min(json_bytes.len());
            let chunk_slice = &json_bytes[start..end];
            let chunk_crc = crc32c::crc32c(chunk_slice);

            let blob_hdr = ScholarPaperBlobHeader {
                key_hash,
                sha256_hash,
                total_bytes,
                chunk_index: chunk_idx,
                total_chunks,
                chunk_bytes: chunk_slice.len() as u32,
                chunk_crc32: chunk_crc,
                schema_version: 2,
                _reserved: 0,
            };

            let mut tile_payload = Vec::with_capacity(
                std::mem::size_of::<ScholarPaperBlobHeader>() + chunk_slice.len(),
            );
            tile_payload.extend_from_slice(bytemuck::bytes_of(&blob_hdr));
            tile_payload.extend_from_slice(chunk_slice);

            out = s3a_engine::append_tile_to_buffer(
                &out,
                ScholarTileType::ACADEMIC_PAPERS,
                2,
                &tile_payload,
            )?;
        }

        Ok(out)
    }

    /// Retrieves a paper record matching `target_key` from raw archive bytes.
    pub fn get_from_bytes(
        archive_bytes: &[u8],
        target_key: &str,
    ) -> std::io::Result<Option<ScholarPaperRecord>> {
        if archive_bytes.len() < s3a_core::FILE_HEADER_SIZE {
            return Ok(None);
        }
        let slot_a: s3a_core::FileHeader = bytemuck::pod_read_unaligned(
            &archive_bytes[s3a_core::FILE_HEADER_SLOT_A_OFFSET
                ..s3a_core::FILE_HEADER_SLOT_A_OFFSET + s3a_core::FILE_HEADER_SLOT_SIZE],
        );
        let slot_b: s3a_core::FileHeader = bytemuck::pod_read_unaligned(
            &archive_bytes[s3a_core::FILE_HEADER_SLOT_B_OFFSET
                ..s3a_core::FILE_HEADER_SLOT_B_OFFSET + s3a_core::FILE_HEADER_SLOT_SIZE],
        );
        let (active_hdr, _) = s3a_engine::resolve_active_header(&slot_a, &slot_b)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

        let tile_count = active_hdr.tile_count as usize;
        let mut headers: Vec<s3a_core::HyperTileHeader> = Vec::with_capacity(tile_count);
        let mut payloads: Vec<&[u8]> = Vec::with_capacity(tile_count);
        for i in 0..tile_count {
            let off = s3a_core::FILE_HEADER_SIZE + i * s3a_core::HYPER_TILE_SIZE;
            if off + s3a_core::HYPER_TILE_SIZE <= archive_bytes.len() {
                let hdr: s3a_core::HyperTileHeader = bytemuck::pod_read_unaligned(
                    &archive_bytes[off..off + s3a_core::HYPER_TILE_HEADER_SIZE],
                );
                let payload = &archive_bytes
                    [off + s3a_core::HYPER_TILE_HEADER_SIZE..off + s3a_core::HYPER_TILE_SIZE];
                headers.push(hdr);
                payloads.push(payload);
            }
        }
        let tiles = headers.iter().zip(payloads);
        Self::get_from_tiles(tiles, target_key)
    }

    /// Retrieves a paper record matching `target_key` (canonical key, DOI, or alias).
    ///
    /// Correctly decodes both version 2 rich chunked papers and version 1 legacy 64-byte records.
    /// A Tile 12 that fails its checks is an error, never silently skipped.
    pub fn get_from_reader(
        reader: &s3a_engine::MmapReader,
        target_key: &str,
    ) -> std::io::Result<Option<ScholarPaperRecord>> {
        let tile_count = reader.tile_count() as usize;
        let mut tiles = Vec::with_capacity(tile_count);
        for i in 0..tile_count {
            if let Some((hdr, payload)) = reader.get_tile(i) {
                tiles.push((hdr, payload));
            }
        }
        Self::get_from_tiles(tiles, target_key)
    }

    /// Decodes and finds the paper record matching `target_key` from a sequence of tiles.
    ///
    /// Returns `Err(InvalidData)` for any Tile 12 whose header CRC, payload CRC, chunk length,
    /// chunk CRC, chunk set, SHA-256 or JSON is inconsistent.
    pub fn get_from_tiles<'a>(
        tiles: impl IntoIterator<Item = (&'a s3a_core::HyperTileHeader, &'a [u8])>,
        target_key: &str,
    ) -> std::io::Result<Option<ScholarPaperRecord>> {
        let bad = |m: String| std::io::Error::new(std::io::ErrorKind::InvalidData, m);
        let norm_key: String = target_key.nfc().collect();

        // Group continuation chunks by sha256_hash
        type Chunks = (
            ScholarPaperBlobHeader,
            std::collections::BTreeMap<u16, Vec<u8>>,
        );
        let mut chunks_by_hash: std::collections::BTreeMap<[u8; 32], Chunks> =
            std::collections::BTreeMap::new();
        let mut order: Vec<[u8; 32]> = Vec::new();

        let mut legacy_candidates: Vec<ScholarPaperRecord> = Vec::new();

        for (hdr, payload) in tiles {
            if hdr.tile_type != ScholarTileType::ACADEMIC_PAPERS {
                continue;
            }
            if s3a_engine::check_tile_header_crc(hdr).is_err() {
                return Err(bad(format!(
                    "Tile 12 (id {}) header CRC mismatch",
                    hdr.tile_id
                )));
            }
            let plen = hdr.payload_bytes as usize;
            if plen > payload.len() {
                return Err(bad(format!(
                    "Tile 12 (id {}) payload_bytes {} exceeds tile payload {}",
                    hdr.tile_id,
                    plen,
                    payload.len()
                )));
            }
            let payload_data = &payload[..plen];
            if !(s3a_engine::is_legacy_tile(hdr) && hdr.data_crc32 == 0)
                && crc32c::crc32c(payload_data) != hdr.data_crc32
            {
                return Err(bad(format!(
                    "Tile 12 (id {}) payload CRC mismatch",
                    hdr.tile_id
                )));
            }

            if hdr.record_layout_version <= 1 {
                // Legacy 64-byte AcademicPaperRecord slice
                let records = bytemuck::try_cast_slice::<u8, AcademicPaperRecord>(payload_data)
                    .map_err(|e| bad(format!("Tile 12 v1 payload is not 64-byte records: {e}")))?;
                for r in records {
                    if r.is_tombstone() {
                        continue;
                    }
                    let matches = r.paper_id.to_string() == norm_key
                        || format!("0x{:016x}", r.paper_id) == norm_key
                        || (r.doi_prefix_hash != 0
                            && r.doi_prefix_hash == crc32c::crc32c(norm_key.as_bytes()) as u64);
                    if matches {
                        // v1 records hold no title, venue name, DOI or authors: those stay absent.
                        legacy_candidates.push(ScholarPaperRecord {
                            key: r.paper_id.to_string(),
                            doi: None,
                            aliases: Vec::new(),
                            title: None,
                            authors: Vec::new(),
                            abstract_text: None,
                            venue: None,
                            year: if r.year != 0 {
                                Some(r.year as u32)
                            } else {
                                None
                            },
                            language: None,
                            license: None,
                            source_url: None,
                            published_date: None,
                        });
                    }
                }
            } else if hdr.record_layout_version == 2 {
                let bsz = std::mem::size_of::<ScholarPaperBlobHeader>();
                if payload_data.len() < bsz {
                    return Err(bad(format!(
                        "Tile 12 (id {}) v2 payload shorter than blob header",
                        hdr.tile_id
                    )));
                }
                let blob_hdr: ScholarPaperBlobHeader =
                    bytemuck::pod_read_unaligned(&payload_data[..bsz]);
                let chunk_len = blob_hdr.chunk_bytes as usize;
                if bsz.checked_add(chunk_len) != Some(payload_data.len()) {
                    return Err(bad(format!(
                        "Tile 12 (id {}) chunk_bytes {} inconsistent with payload_bytes {}",
                        hdr.tile_id,
                        chunk_len,
                        payload_data.len()
                    )));
                }
                if blob_hdr.total_chunks == 0
                    || blob_hdr.chunk_index >= blob_hdr.total_chunks
                    || chunk_len > blob_hdr.total_bytes as usize
                {
                    return Err(bad(format!(
                        "Tile 12 (id {}) chunk index/count/length inconsistent",
                        hdr.tile_id
                    )));
                }
                let chunk_data = &payload_data[bsz..];
                if crc32c::crc32c(chunk_data) != blob_hdr.chunk_crc32 {
                    return Err(bad(format!(
                        "Tile 12 (id {}) chunk CRC mismatch",
                        hdr.tile_id
                    )));
                }
                if !chunks_by_hash.contains_key(&blob_hdr.sha256_hash) {
                    order.push(blob_hdr.sha256_hash);
                }
                let entry = chunks_by_hash
                    .entry(blob_hdr.sha256_hash)
                    .or_insert_with(|| (blob_hdr, std::collections::BTreeMap::new()));
                if entry.0.total_chunks != blob_hdr.total_chunks
                    || entry.0.total_bytes != blob_hdr.total_bytes
                {
                    return Err(bad(
                        "Tile 12 chunks of one paper disagree on totals".to_string()
                    ));
                }
                entry.1.insert(blob_hdr.chunk_index, chunk_data.to_vec());
            } else {
                return Err(bad(format!(
                    "Tile 12 (id {}) has unsupported layout version {}",
                    hdr.tile_id, hdr.record_layout_version
                )));
            }
        }

        // Reassemble v2 papers; the newest matching record wins.
        let mut found: Option<ScholarPaperRecord> = None;
        for sha in order {
            let (blob_hdr, chunk_map) = chunks_by_hash.remove(&sha).expect("ordered key present");
            if chunk_map.len() != blob_hdr.total_chunks as usize {
                return Err(bad(format!(
                    "Tile 12 paper is missing chunks ({} of {})",
                    chunk_map.len(),
                    blob_hdr.total_chunks
                )));
            }
            let mut full_bytes = Vec::with_capacity(blob_hdr.total_bytes as usize);
            for (_, data) in chunk_map {
                full_bytes.extend_from_slice(&data);
            }
            if full_bytes.len() != blob_hdr.total_bytes as usize {
                return Err(bad("Tile 12 reassembled length != total_bytes".to_string()));
            }
            use sha2::Digest;
            let actual_sha: [u8; 32] = sha2::Sha256::digest(&full_bytes).into();
            if actual_sha != sha {
                return Err(bad("Tile 12 reassembled paper SHA-256 mismatch".to_string()));
            }
            let paper = serde_json::from_slice::<ScholarPaperRecord>(&full_bytes)
                .map_err(|e| bad(format!("Tile 12 paper JSON invalid: {e}")))?;
            let matches = paper.key == norm_key
                || paper.doi.as_deref() == Some(norm_key.as_str())
                || paper.aliases.iter().any(|a| a == &norm_key);
            if matches {
                found = Some(paper);
            }
        }
        if found.is_some() {
            return Ok(found);
        }
        Ok(legacy_candidates.pop())
    }
}
