//! Prompt 011 fail-first tests: multi-slab (chunked) documents — one logical
//! file stored across multiple tiles/slabs.
//!
//! NOTE — deviations from the prompt, flagged for the report:
//! 1. The prompt expects these tests to be COPIED from `/workspace/slab-prompts/`
//!    (or a copy next to the prompt file). Those sources did not exist anywhere
//!    on this machine (not in the workspace, not in the repo, and the live wiki
//!    is not on this VM), so this file was WRITTEN from the prompt's
//!    specifications instead. The wiki-shape fixtures are REUSED from prompt
//!    010 (`tests/fixtures/wiki_shape.jsonl`, `wiki_shape_measured.json`).
//! 2. The wiki "read first" pages do not exist on this VM (`/home/box/wiki`
//!    absent). Proceeded from the prompt plus repo docs.
//! 3. The wiki+large test's two bounds are arithmetically incompatible as
//!    written: the wiki packs to exactly the measured ceiling (12,100,096
//!    bytes, 20.27x smaller than the 245,236,224-byte prototype), and the
//!    200 KiB file's minimum packed footprint is 233,472 bytes (two chunk
//!    slots, 131,072 + 98,304, plus one 4,096-byte manifest slot), so the
//!    combined vault is 12,333,568 bytes = 19.88x smaller — the literal
//!    `packed * 20 < 245,236,224` cannot hold together with the prompt's own
//!    ceiling clause. This test therefore asserts the ceiling clause verbatim
//!    and a 19x floor (`packed * 19 < 245,236,224`) instead of the literal
//!    20x. No implementation can beat that arithmetic; see the PR report.

use s3a_cli::first_run::{read_document_record, write_document_record};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
const PROTOTYPE_BYTES: u64 = 245_236_224;
const HYPER_TILE_SIZE: u64 = 131_072;
const FILE_HEADER_SIZE: u64 = 512;
const WIKI_CEILING: u64 = 12_100_096;

/// The s3a CLI binary built for this test run.
fn cli_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_s3a") {
        return PathBuf::from(p);
    }
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_s3a-cli") {
        return PathBuf::from(p);
    }
    panic!("Neither CARGO_BIN_EXE_s3a nor CARGO_BIN_EXE_s3a-cli is set");
}

fn unique_tmp(name: &str) -> PathBuf {
    // NOTE: std::env::temp_dir() is a 512 MB tmpfs here, too small for the
    // pre-fix 245 MB wiki vaults; use the workspace target dir (same disk as
    // the repo, and gitignored) instead.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ws_target = manifest.join("../../target");
    let root = if ws_target.is_dir() {
        ws_target
    } else {
        std::env::temp_dir()
    };
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = root.join(format!(
        "test-tmp-011-{}_{}_{}",
        name,
        std::process::id(),
        nanos
    ));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn run_cli<'a>(dir: &Path, args: impl AsRef<[&'a str]>) -> Output {
    let args = args.as_ref();
    let out = Command::new(cli_bin())
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("failed to run s3a {:?}: {}", args, e));
    assert!(
        out.status.success(),
        "s3a {:?} failed (status {:?}):\nstdout: {}\nstderr: {}",
        args,
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    out
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Deterministic fixture content for `key`: SHA-256(key || counter) blocks.
fn content_for(key: &str, size: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(size);
    let mut counter: u64 = 0;
    while out.len() < size {
        let mut h = Sha256::new();
        h.update(key.as_bytes());
        h.update(counter.to_le_bytes());
        out.extend_from_slice(&h.finalize());
        counter += 1;
    }
    out.truncate(size);
    out
}

struct ShapeEntry {
    key: String,
    size: usize,
}

fn load_shape() -> Vec<ShapeEntry> {
    let text = fs::read_to_string(format!("{}/wiki_shape.jsonl", FIXTURES))
        .expect("read wiki_shape.jsonl");
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let v: serde_json::Value =
                serde_json::from_str(line).expect("parse wiki_shape.jsonl line");
            ShapeEntry {
                key: v["key"].as_str().expect("key").to_string(),
                size: v["size"].as_u64().expect("size") as usize,
            }
        })
        .collect()
}

fn load_measured() -> serde_json::Value {
    let text = fs::read_to_string(format!("{}/wiki_shape_measured.json", FIXTURES))
        .expect("read wiki_shape_measured.json");
    serde_json::from_str(&text).expect("parse wiki_shape_measured.json")
}

fn create_vault(path: &Path) {
    s3a_engine::TileWriter::create(path).expect("create vault");
}

fn pack_shape(vault: &Path, shape: &[ShapeEntry]) {
    for entry in shape {
        let content = content_for(&entry.key, entry.size);
        write_document_record(vault, &entry.key, &content)
            .unwrap_or_else(|e| panic!("put {} ({} bytes): {}", entry.key, entry.size, e));
    }
}

/// Count `.slab` extent files anywhere under `dir` (the prompt's ceiling allows
/// 4,096 bytes per extent file actually created; the packed single-file layout
/// creates none).
fn count_slab_extent_files(dir: &Path) -> u64 {
    fn walk(p: &Path, n: &mut u64) {
        if let Ok(rd) = fs::read_dir(p) {
            for e in rd.flatten() {
                let path = e.path();
                if path.is_dir() {
                    walk(&path, n);
                } else if path.extension().map(|x| x == "slab").unwrap_or(false) {
                    *n += 1;
                }
            }
        }
    }
    let mut n = 0;
    walk(dir, &mut n);
    n
}

/// `s3a ls --json --vault <vault>` parsed as JSON.
fn ls_json(dir: &Path, vault: &str) -> serde_json::Value {
    let out = run_cli(dir, ["ls", "--json", "--vault", vault]);
    serde_json::from_slice(&out.stdout).expect("parse ls --json")
}

fn doc_entry<'a>(ls: &'a serde_json::Value, key: &str) -> &'a serde_json::Value {
    ls["documents"]
        .as_array()
        .expect("ls documents array")
        .iter()
        .find(|d| d["key"].as_str() == Some(key))
        .unwrap_or_else(|| panic!("key {} missing from ls --json: {}", key, ls))
}

#[test]
fn test_file_larger_than_128kib_round_trips() {
    let dir = unique_tmp("roundtrip");
    let vault = dir.join("big.s3a");
    create_vault(&vault);
    let content = content_for("large-200kib", 200 * 1024);
    write_document_record(&vault, "big", &content).expect("put 200 KiB document");
    let got = read_document_record(&vault, "big")
        .expect("get 200 KiB document")
        .expect("big key must exist");
    assert_eq!(got, content, "200 KiB document must round-trip byte-identical");

    // The CLI must round-trip it the same way.
    let big_file = dir.join("big.bin");
    fs::write(&big_file, &content).expect("write big.bin");
    run_cli(&dir, ["put", "big.s3a", "big", "@big.bin"]);
    let out = run_cli(&dir, ["get", "big.s3a", "big"]);
    assert_eq!(
        out.stdout, content,
        "CLI get of the 200 KiB document must return the exact bytes"
    );
}

#[test]
fn test_two_large_files_have_different_content_and_hashes() {
    let dir = unique_tmp("two-large");
    let vault = dir.join("two.s3a");
    create_vault(&vault);
    let content_a = content_for("a-large", 200 * 1024);
    let content_b = content_for("b-large", 300 * 1024);
    assert_ne!(content_a, content_b, "fixtures must differ");
    write_document_record(&vault, "a-large", &content_a).expect("put 200 KiB file");
    write_document_record(&vault, "b-large", &content_b).expect("put 300 KiB file");
    assert_eq!(
        read_document_record(&vault, "a-large")
            .expect("get a-large")
            .unwrap(),
        content_a
    );
    assert_eq!(
        read_document_record(&vault, "b-large")
            .expect("get b-large")
            .unwrap(),
        content_b
    );

    let ls = ls_json(&dir, "two.s3a");
    let a = doc_entry(&ls, "a-large");
    let b = doc_entry(&ls, "b-large");
    assert_eq!(a["size_bytes"].as_u64().unwrap(), 200 * 1024);
    assert_eq!(b["size_bytes"].as_u64().unwrap(), 300 * 1024);
    let hash_a = a["sha256"].as_str().expect("a-large sha256 in ls");
    let hash_b = b["sha256"].as_str().expect("b-large sha256 in ls");
    assert_eq!(hash_a, sha256_hex(&content_a), "a-large sha256 must match content");
    assert_eq!(hash_b, sha256_hex(&content_b), "b-large sha256 must match content");
    assert_ne!(hash_a, hash_b, "different files must have different whole-file hashes");
    let chunks_a = a["chunk_count"].as_u64().expect("a-large chunk_count in ls");
    let chunks_b = b["chunk_count"].as_u64().expect("b-large chunk_count in ls");
    assert!(chunks_a >= 2, "200 KiB file must span at least 2 chunks, got {}", chunks_a);
    assert!(chunks_b >= 3, "300 KiB file must span at least 3 chunks, got {}", chunks_b);
    assert_ne!(
        chunks_a, chunks_b,
        "200 KiB and 300 KiB files must have different chunk counts"
    );
}

#[test]
fn test_chunk_list_is_ordered_complete_and_hashed() {
    let dir = unique_tmp("chunks");
    let vault = dir.join("chunks.s3a");
    create_vault(&vault);
    let content = content_for("large-200kib", 200 * 1024);
    write_document_record(&vault, "big", &content).expect("put 200 KiB document");

    let ls = ls_json(&dir, "chunks.s3a");
    let entry = doc_entry(&ls, "big");
    let chunk_count = entry["chunk_count"].as_u64().expect("chunk_count in ls") as usize;
    assert!(chunk_count >= 2, "200 KiB file must span chunks, got {}", chunk_count);
    let chunks = entry["chunks"].as_array().expect("chunks array in ls");
    assert_eq!(
        chunks.len(),
        chunk_count,
        "chunks array length must equal chunk_count"
    );
    let mut total_bytes: u64 = 0;
    let mut seen_tiles = Vec::new();
    for (i, chunk) in chunks.iter().enumerate() {
        assert_eq!(
            chunk["index"].as_u64().unwrap() as usize,
            i,
            "chunk {} must have index {} (ordered 0..n-1)",
            i,
            i
        );
        let total = chunk
            .get("total_chunks")
            .or_else(|| chunk.get("total"))
            .and_then(|v| v.as_u64())
            .expect("chunk total/total_chunks");
        assert_eq!(total as usize, chunk_count, "chunk total must equal chunk_count");
        let bytes = chunk["chunk_bytes"].as_u64().expect("chunk_bytes");
        assert!(
            bytes > 0 && bytes <= HYPER_TILE_SIZE,
            "chunk {} has {} bytes, must be in (0, 128 KiB]",
            i,
            bytes
        );
        total_bytes += bytes;
        // Each chunk must name its tile.
        assert!(
            chunk.get("tile_index").is_some() || chunk.get("tile_id").is_some(),
            "chunk {} must name its tile (tile_index or tile_id): {}",
            i,
            chunk
        );
        if let Some(t) = chunk.get("tile_index").and_then(|v| v.as_u64()) {
            assert!(
                !seen_tiles.contains(&t),
                "chunk tiles must be distinct, tile {} repeats",
                t
            );
            seen_tiles.push(t);
        }
    }
    assert_eq!(
        total_bytes,
        200 * 1024,
        "sum of chunk lengths must equal the logical file size"
    );
    assert_eq!(
        entry["sha256"].as_str().unwrap(),
        sha256_hex(&content),
        "sha256 must be the SHA-256 of the concatenated file"
    );
}

#[test]
fn test_seal_verify_prove_flatten_unflatten_and_tamper() {
    let dir = unique_tmp("seal");
    let vault = dir.join("seal.s3a");
    create_vault(&vault);
    let content = content_for("large-200kib", 200 * 1024);
    write_document_record(&vault, "big", &content).expect("put 200 KiB document");

    run_cli(
        &dir,
        [
            "seal",
            "seal.s3a",
            "seal.sealed.s3a",
            "--about",
            "--record-roots",
        ],
    );
    let verify_out = run_cli(&dir, ["verify", "--json", "seal.sealed.s3a"]);
    let report: serde_json::Value =
        serde_json::from_slice(&verify_out.stdout).expect("parse verify --json");
    assert_eq!(report["ok"], true, "verify --json must be ok: {}", report);
    assert_eq!(
        report["is_sealed"], true,
        "verify --json must report is_sealed: {}",
        report
    );
    let chunk_count = doc_entry(&ls_json(&dir, "seal.sealed.s3a"), "big")["chunk_count"]
        .as_u64()
        .expect("chunk_count in sealed ls");
    assert_eq!(
        report["record_roots"]["state"], "ok",
        "record roots must be ok: {}",
        report
    );
    assert!(
        report["record_roots"]["checked"].as_u64().unwrap() >= chunk_count,
        "record roots must check at least the chunk count ({}): {}",
        chunk_count,
        report
    );

    // prove + verify-proof must succeed for every chunk tile (PROOF OK).
    let sealed_ls = ls_json(&dir, "seal.sealed.s3a");
    let chunks = doc_entry(&sealed_ls, "big")["chunks"]
        .as_array()
        .expect("chunks array in sealed ls");
    assert!(!chunks.is_empty(), "sealed ls must expose the chunk list");
    for chunk in chunks {
        let tile = chunk["tile_index"].as_u64().expect("chunk tile_index") as usize;
        let proof_file = dir.join(format!("proof-{}.json", tile));
        let prove_out = run_cli(
            &dir,
            [
                "prove",
                "seal.sealed.s3a",
                &tile.to_string(),
                "0",
            ],
        );
        fs::write(&proof_file, &prove_out.stdout).expect("write proof json");
        let proof_name = proof_file
            .file_name()
            .expect("proof file name")
            .to_str()
            .expect("proof file name utf8");
        let verify_proof_out = run_cli(
            &dir,
            ["verify-proof", proof_name, "seal.sealed.s3a"],
        );
        assert!(
            String::from_utf8_lossy(&verify_proof_out.stdout).contains("PROOF OK"),
            "verify-proof for chunk tile {} must print PROOF OK: {}",
            tile,
            String::from_utf8_lossy(&verify_proof_out.stdout)
        );
    }

    // flatten --zstd / unflatten restores the sealed archive byte for byte,
    // and the 200 KiB file still matches.
    run_cli(
        &dir,
        [
            "flatten",
            "--zstd",
            "seal.sealed.s3a",
            "seal.sealed.s3af",
        ],
    );
    run_cli(
        &dir,
        ["unflatten", "seal.sealed.s3af", "seal.restored.s3a"],
    );
    assert_eq!(
        fs::read(dir.join("seal.restored.s3a")).expect("read restored"),
        fs::read(dir.join("seal.sealed.s3a")).expect("read sealed"),
        "flatten/unflatten of the sealed archive must round-trip byte for byte"
    );
    assert_eq!(
        read_document_record(&dir.join("seal.restored.s3a"), "big")
            .expect("read big from restored")
            .unwrap(),
        content,
        "200 KiB file must still match after flatten/unflatten"
    );

    // Flipping one byte of the sealed file must make verify fail.
    let tampered = dir.join("seal.tampered.s3a");
    let mut bytes = fs::read(dir.join("seal.sealed.s3a")).expect("read sealed");
    assert!(bytes.len() > 2048, "sealed file unexpectedly small");
    bytes[1024] ^= 0x01; // inside the first tile's payload
    fs::write(&tampered, &bytes).expect("write tampered");
    let tampered_out = Command::new(cli_bin())
        .args(["verify", "--json", "seal.tampered.s3a"])
        .current_dir(&dir)
        .output()
        .expect("run verify on tampered file");
    let tampered_stdout = String::from_utf8_lossy(&tampered_out.stdout);
    let tampered_failed =
        !tampered_out.status.success() || tampered_stdout.contains("\"ok\":false");
    assert!(
        tampered_failed,
        "flipping one byte must fail verify (status {:?}, stdout {})",
        tampered_out.status, tampered_stdout
    );

    // Flipping one byte inside a chunk's content in the UNSEALED vault must
    // make the read fail closed (chunk CRC / whole-file hash mismatch).
    // Offset 2048 is inside the first chunk's content for any chunk-key
    // scheme: the first tile starts at 512, its 512-byte header and the
    // 64-byte record header plus a short key end well before 2048, and the
    // first chunk of a 200 KiB file extends past 130,000.
    let tampered_unsealed = dir.join("seal.unsealed-tampered.s3a");
    let mut raw = fs::read(&vault).expect("read unsealed vault");
    assert!(raw.len() > 2048, "unsealed vault unexpectedly small");
    raw[2048] ^= 0x01;
    fs::write(&tampered_unsealed, &raw).expect("write tampered unsealed");
    assert!(
        read_document_record(&tampered_unsealed, "big").is_err(),
        "flipping one byte inside a chunk must fail the read"
    );
}

#[test]
fn test_wiki_shape_plus_large_file_still_beats_245mb_and_restores() {
    let dir = unique_tmp("wiki-large");
    let shape = load_shape();
    let measured = load_measured();
    assert_eq!(shape.len(), 1871, "fixture must hold 1,871 files");
    assert_eq!(
        measured["files"].as_u64().unwrap(),
        1871,
        "measured fixture must record 1,871 files"
    );

    let vault = dir.join("wiki.s3a");
    create_vault(&vault);
    pack_shape(&vault, &shape);
    let large = content_for("overflow/large.bin", 200 * 1024);
    write_document_record(&vault, "overflow/large.bin", &large)
        .expect("put 200 KiB overflow file");
    let len = fs::metadata(&vault).expect("stat vault").len();
    let extents = count_slab_extent_files(&dir);
    let ceiling = WIKI_CEILING + 2 * HYPER_TILE_SIZE + 4096 * extents;
    assert!(
        len <= ceiling,
        "packed wiki shape + large file is {} bytes, over the ceiling {} ({} .slab extents)",
        len,
        ceiling,
        extents
    );
    // See the header note: the prompt's literal 20x bound cannot hold with
    // its own ceiling (wiki alone is 20.27x; the 200 KiB file's minimum
    // packed footprint brings the combined vault to 19.88x). Assert 19x.
    assert!(
        len * 19 < PROTOTYPE_BYTES,
        "packed wiki shape + large file ({} bytes) is not at least 19x smaller than the 245 MB prototype",
        len
    );

    // Restore the large file and two different wiki keys byte-identically.
    assert_eq!(
        read_document_record(&vault, "overflow/large.bin")
            .expect("read large file")
            .unwrap(),
        large,
        "large file must restore byte-identical"
    );
    let mut first: Option<Vec<u8>> = None;
    let mut second: Option<Vec<u8>> = None;
    for entry in shape.iter().take(2) {
        let expected = content_for(&entry.key, entry.size);
        let got = read_document_record(&vault, &entry.key)
            .unwrap_or_else(|e| panic!("read {}: {}", entry.key, e))
            .unwrap_or_else(|| panic!("key {} missing", entry.key));
        assert_eq!(got, expected, "key {} did not restore byte-identical", entry.key);
        if first.is_none() {
            first = Some(got);
        } else if second.is_none() {
            second = Some(got);
        }
    }
    assert_ne!(
        first.unwrap(),
        second.unwrap(),
        "two different wiki keys must restore different bytes"
    );
}
