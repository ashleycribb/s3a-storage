//! Acceptance 009 follow-up (2)(3)(4): the embedded schema tile must be enough to decode a v2
//! paper, the reader must never invent a registry, and the spec version must come from the file.
//!
//! Written before the fix. Do not edit.
//!
//! Contract asserted here (document it in docs/spec/registry.md, tile 12 layout v2):
//! the Tile 80 entry for type 12 has `type_version == 2` and these fields, by name, with the
//! offsets/widths of `ScholarPaperBlobHeader` (field type codes from registry.md: U16=2, U32=3,
//! U64=4, BYTES=13, STRING=11):
//!   key_hash U64 @0/8, sha256_hash BYTES @8/32, total_bytes U32 @40/4, chunk_index U16 @44/2,
//!   total_chunks U16 @46/2, chunk_bytes U32 @48/4, chunk_crc32 U32 @52/4,
//!   schema_version U32 @56/4, chunk_data STRING @64 (byte_len 0 = variable, UTF-8 JSON chunk).
//! The decoder below uses ONLY those embedded offsets plus s3a-core's generic schema codec;
//! it never calls the Scholar pack's readers or `standard_schema_for_type`.

use s3a_core::schema::SchemaTypeDefinition;
use s3a_core::{
    FileHeader, HyperTileHeader, TileType, FILE_HEADER_SIZE, FILE_HEADER_SLOT_A_OFFSET,
    FILE_HEADER_SLOT_B_OFFSET, FILE_HEADER_SLOT_SIZE, HYPER_TILE_HEADER_SIZE, HYPER_TILE_SIZE,
};
use s3a_engine::sealed::seal_buffer_with_manifest;
use s3a_pack_scholar::{ScholarAuthor, ScholarPaperRecord};
use sha2::Digest;

fn tiles(bytes: &[u8]) -> Vec<(HyperTileHeader, &[u8])> {
    let a: FileHeader = bytemuck::pod_read_unaligned(
        &bytes[FILE_HEADER_SLOT_A_OFFSET..FILE_HEADER_SLOT_A_OFFSET + FILE_HEADER_SLOT_SIZE],
    );
    let b: FileHeader = bytemuck::pod_read_unaligned(
        &bytes[FILE_HEADER_SLOT_B_OFFSET..FILE_HEADER_SLOT_B_OFFSET + FILE_HEADER_SLOT_SIZE],
    );
    let (h, _) = s3a_engine::resolve_active_header(&a, &b).unwrap();
    (0..h.tile_count as usize)
        .map(|i| {
            let off = FILE_HEADER_SIZE + i * HYPER_TILE_SIZE;
            let hdr: HyperTileHeader =
                bytemuck::pod_read_unaligned(&bytes[off..off + HYPER_TILE_HEADER_SIZE]);
            let p = &bytes[off + HYPER_TILE_HEADER_SIZE
                ..off + HYPER_TILE_HEADER_SIZE + hdr.payload_bytes as usize];
            (hdr, p)
        })
        .collect()
}

fn sample_paper() -> ScholarPaperRecord {
    let mut p = ScholarPaperRecord::new(
        "arxiv:2106.09685",
        "LoRA: Low-Rank Adaptation — Müller & Ångström",
    );
    p.doi = Some("10.48550/arXiv.2106.09685".into());
    p.authors = vec![
        ScholarAuthor::new("Edward J. Hu", None),
        ScholarAuthor::new("Jürgen Ñúñez", Some("0000-0002-1825-0097")),
    ];
    p.abstract_text = Some("Ünïcödé abstract. ".repeat(8000)); // ~170 KB: needs 2 continuation tiles
    p.year = Some(2021);
    p.published_date = Some("2021-06-17T00:00:00Z".into());
    p
}

fn field<'a>(
    def: &'a SchemaTypeDefinition,
    name: &str,
) -> &'a s3a_core::schema::SchemaFieldDefinition {
    def.fields
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| {
            panic!(
                "embedded schema for tile {} v{} has no field `{}`; fields present: {:?}",
                def.type_id,
                def.type_version,
                name,
                def.fields
                    .iter()
                    .map(|f| (&f.name, f.field_type, f.byte_offset, f.byte_len))
                    .collect::<Vec<_>>()
            )
        })
}

fn rd(p: &[u8], f: &s3a_core::schema::SchemaFieldDefinition) -> u64 {
    let s = &p[f.byte_offset as usize..f.byte_offset as usize + f.byte_len as usize];
    let mut b = [0u8; 8];
    b[..s.len()].copy_from_slice(s);
    u64::from_le_bytes(b)
}

/// Generic decoder: everything it knows about tile 12 comes from the archive's own Tile 80.
fn decode_papers_with_embedded_schema_only(archive: &[u8]) -> Vec<serde_json::Value> {
    let ts = tiles(archive);
    let schema_tiles: Vec<_> = ts
        .iter()
        .filter(|(h, _)| h.tile_type == TileType::SCHEMA)
        .collect();
    assert!(
        !schema_tiles.is_empty(),
        "sealed archive must embed a Schema tile (type 80)"
    );
    let (sh, sp) = schema_tiles[0];
    assert_eq!(
        s3a_engine::compute_tile_header_crc32(sh),
        sh.header_crc32,
        "schema tile header CRC"
    );
    assert_eq!(crc32c::crc32c(sp), sh.data_crc32, "schema tile data CRC");
    let defs = s3a_core::schema::decode_schema_payload(sp)
        .expect("decode embedded schema with s3a-core codec");

    let mut chunks: std::collections::BTreeMap<
        Vec<u8>,
        (u64, u64, std::collections::BTreeMap<u64, Vec<u8>>),
    > = Default::default();
    for (h, p) in ts.iter().filter(|(h, _)| h.tile_type == 12) {
        let def = defs
            .iter()
            .find(|d| d.type_id == 12 && d.type_version == h.record_layout_version)
            .unwrap_or_else(|| {
                panic!(
                    "no embedded schema entry for tile 12 layout v{}",
                    h.record_layout_version
                )
            });
        let (kh, sha, tot, idx, n, cb, crc, sv, data) = (
            field(def, "key_hash"),
            field(def, "sha256_hash"),
            field(def, "total_bytes"),
            field(def, "chunk_index"),
            field(def, "total_chunks"),
            field(def, "chunk_bytes"),
            field(def, "chunk_crc32"),
            field(def, "schema_version"),
            field(def, "chunk_data"),
        );
        for (f, t, o, l) in [
            (kh, 4, 0, 8),
            (sha, 13, 8, 32),
            (tot, 3, 40, 4),
            (idx, 2, 44, 2),
            (n, 2, 46, 2),
            (cb, 3, 48, 4),
            (crc, 3, 52, 4),
            (sv, 3, 56, 4),
        ] {
            assert_eq!(
                (f.field_type, f.byte_offset, f.byte_len),
                (t, o, l),
                "embedded layout of `{}`",
                f.name
            );
        }
        assert_eq!(
            (data.field_type, data.byte_offset, data.byte_len),
            (11, 64, 0),
            "`chunk_data` must be variable-length STRING at 64"
        );
        assert_eq!(rd(p, sv), 2);
        let len = rd(p, cb) as usize;
        let body = &p[data.byte_offset as usize..data.byte_offset as usize + len];
        assert_eq!(crc32c::crc32c(body) as u64, rd(p, crc), "chunk CRC");
        let shav = p[sha.byte_offset as usize..(sha.byte_offset + sha.byte_len) as usize].to_vec();
        let e = chunks
            .entry(shav)
            .or_insert((rd(p, tot), rd(p, n), Default::default()));
        e.2.insert(rd(p, idx), body.to_vec());
    }
    let mut out = Vec::new();
    for (sha, (tot, n, parts)) in chunks {
        assert_eq!(parts.len() as u64, n, "all continuation chunks present");
        let json: Vec<u8> = parts.into_values().flatten().collect();
        assert_eq!(json.len() as u64, tot);
        assert_eq!(
            sha2::Sha256::digest(&json).to_vec(),
            sha,
            "reassembled SHA-256"
        );
        out.push(serde_json::from_slice(&json).expect("chunk_data is UTF-8 JSON"));
    }
    out
}

#[test]
fn v2_paper_decodes_from_sealed_archive_using_only_embedded_schema() {
    let paper = sample_paper();
    let sealed = seal_buffer_with_manifest(&paper.put_to_bytes(&[]).unwrap(), 4, 2).unwrap();
    let papers = decode_papers_with_embedded_schema_only(&sealed);
    assert_eq!(papers.len(), 1);
    let mut n = paper.clone();
    n.normalize();
    assert_eq!(
        papers[0],
        serde_json::to_value(&n).unwrap(),
        "decoded paper equals what was written"
    );
}

#[test]
fn read_embedded_registry_reports_not_embedded_without_fallback() {
    // Unsealed archive: data tiles only, no Schema tile.
    let bytes = sample_paper().put_to_bytes(&[]).unwrap();
    assert!(
        tiles(&bytes)
            .iter()
            .all(|(h, _)| h.tile_type != TileType::SCHEMA),
        "precondition: no schema tile"
    );
    match s3a_engine::read_embedded_registry_from_bytes(&bytes) {
        Ok(info) => panic!(
            "no Schema tile in the file, so the reader must not return a registry; got {} invented types with spec_version {:?}",
            info.types.len(),
            info.spec_version
        ),
        Err(e) => {
            assert_eq!(e.kind(), std::io::ErrorKind::NotFound, "kind must be NotFound, got {e}");
            assert!(e.to_string().contains("not embedded"), "message must say 'not embedded', got {e}");
        }
    }
}

fn docs_spec_version() -> String {
    include_str!("../../../docs/spec/S3A-TILE-SPEC.md")
        .lines()
        .find_map(|l| l.strip_prefix("**Document Version:**"))
        .map(|s| s.trim().to_string())
        .expect("docs/spec/S3A-TILE-SPEC.md has a Document Version line")
}

#[test]
fn embedded_spec_version_comes_from_docs_spec_and_from_the_file() {
    let sealed =
        seal_buffer_with_manifest(&sample_paper().put_to_bytes(&[]).unwrap(), 4, 2).unwrap();
    let info = s3a_engine::read_embedded_registry_from_bytes(&sealed)
        .expect("sealed archive has a registry");
    let (_, sp) = *tiles(&sealed)
        .iter()
        .find(|(h, _)| h.tile_type == TileType::SCHEMA)
        .unwrap();
    let about = s3a_core::schema::decode_schema_about(sp)
        .unwrap()
        .expect("sealed schema tile carries an about entry");
    let v = serde_json::to_value(&info).unwrap();
    assert_eq!(
        v["spec_version"],
        serde_json::json!(docs_spec_version()),
        "spec version = docs/spec Document Version"
    );
    assert_eq!(
        v["spec_version"],
        serde_json::json!(about.spec_version),
        "spec version must be read from the file's about entry"
    );
    assert_ne!(
        docs_spec_version(),
        env!("CARGO_PKG_VERSION"),
        "spec version and engine/crate version are different things"
    );
    assert!(
        about.writer.contains(env!("CARGO_PKG_VERSION")),
        "engine version belongs in `writer`, got {}",
        about.writer
    );
}

#[test]
fn schema_tile_without_about_entry_has_no_spec_version() {
    // A Schema tile with type definitions but no about entry: the reader must not substitute
    // its own compiled-in spec version.
    let path = std::env::temp_dir().join(format!("acc009_noabout_{}.s3a", std::process::id()));
    std::fs::write(&path, sample_paper().put_to_bytes(&[]).unwrap()).unwrap();
    assert!(
        s3a_engine::append_schema_tile_if_missing(&path).unwrap(),
        "precondition: schema tile appended"
    );
    let bytes = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    let (_, sp) = *tiles(&bytes)
        .iter()
        .find(|(h, _)| h.tile_type == TileType::SCHEMA)
        .unwrap();
    assert!(
        s3a_core::schema::decode_schema_about(sp).unwrap().is_none(),
        "precondition: no about entry"
    );
    let info = s3a_engine::read_embedded_registry_from_bytes(&bytes).expect("schema tile present");
    let v = serde_json::to_value(&info).unwrap();
    assert!(
        v.get("spec_version").map_or(true, |x| x.is_null()),
        "no about entry in the file, so spec_version must be absent/null, got {}",
        v["spec_version"]
    );
}
