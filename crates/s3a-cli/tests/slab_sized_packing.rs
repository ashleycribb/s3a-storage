//! Prompt 010 fail-first tests: right-sized slab selection for document packing.
//!
//! NOTE — deviation from the prompt, flagged for the report: the prompt expects
//! these tests to be COPIED from `/workspace/slab-prompts/` (or a copy next to
//! the prompt file). Those sources did not exist anywhere on this machine (not
//! in the workspace, not in the repo, and the live wiki is not on this VM), so
//! this file and the fixtures under `tests/fixtures/` were WRITTEN from the
//! prompt's specifications instead. The wiki-shape fixtures are reconstructions
//! from the measured numbers in the prompt (1,871 files, 5,961,273 content
//! bytes, largest 117,998 bytes, class distribution 1356/283/122/65/29/6/6/1/0/3,
//! ceiling 12,100,096 bytes) — see `tests/fixtures/wiki_shape_measured.json`.
//! Fixture content is a SHA-256 stream of each key, not live wiki content.
//! `legacy_one_doc.s3a` was generated with current main's `s3a put`
//! (key `legacy-doc`, 17-byte content `hello-slab-legacy`, 131,584 bytes).

use s3a_cli::first_run::{list_all_documents, read_document_record, write_document_record};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
const PROTOTYPE_BYTES: u64 = 245_236_224;
const HYPER_TILE_SIZE: u64 = 131_072;
const FILE_HEADER_SIZE: u64 = 512;

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
        "test-tmp-010-{}_{}_{}",
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
fn wiki_content(key: &str, size: usize) -> Vec<u8> {
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
        let content = wiki_content(&entry.key, entry.size);
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

#[test]
fn test_small_document_is_not_padded_to_128kib() {
    let dir = unique_tmp("small");
    let vault = dir.join("small.s3a");
    run_cli(&dir, ["init", "small.s3a"]);
    let content = b"0123456789abcdefg"; // 17 bytes
    run_cli(
        &dir,
        ["put", "small.s3a", "hello", "0123456789abcdefg"],
    );
    let out = run_cli(&dir, ["get", "small.s3a", "hello"]);
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "0123456789abcdefg",
        "17-byte document must round-trip exactly"
    );
    assert_eq!(content.len(), 17);
    let len = fs::metadata(&vault).expect("stat vault").len();
    assert!(
        len < FILE_HEADER_SIZE + HYPER_TILE_SIZE,
        "17-byte put produced a {}-byte vault, still 128 KiB-padded",
        len
    );
    // Class-0 budget: file header + one 4 KiB slot + optional 4 KiB extent
    // header + at most two extra class-0 metadata slots.
    let class0_budget = 512 + 4096 + 4096 + 2 * 4096;
    assert!(
        len <= class0_budget,
        "17-byte put produced a {}-byte vault, over the class-0 budget {}",
        len,
        class0_budget
    );
}

#[test]
fn test_different_payloads_select_different_size_classes() {
    let dir = unique_tmp("classes");
    let vault0 = dir.join("class0.s3a");
    let vault1 = dir.join("class1.s3a");
    create_vault(&vault0);
    create_vault(&vault1);
    // key "a" (1 byte): stored 512 + 64 + 1 + content. 100 bytes -> 677 (class 0);
    // 5,000 bytes -> 5,577 (class 1).
    write_document_record(&vault0, "a", &vec![b'x'; 100]).expect("put class-0 payload");
    write_document_record(&vault1, "a", &vec![b'y'; 5000]).expect("put class-1 payload");
    let bytes0 = fs::read(&vault0).expect("read vault0");
    let bytes1 = fs::read(&vault1).expect("read vault1");
    assert_ne!(bytes0, bytes1, "different payloads must produce different vault bytes");
    assert_ne!(
        sha256_hex(&bytes0),
        sha256_hex(&bytes1),
        "different payloads must produce different vault SHA-256"
    );
    assert!(
        bytes1.len() > bytes0.len(),
        "class-1 vault ({} bytes) must be larger than class-0 vault ({} bytes)",
        bytes1.len(),
        bytes0.len()
    );
    for (name, len) in [("class-0", bytes0.len()), ("class-1", bytes1.len())] {
        assert_ne!(
            len as u64,
            FILE_HEADER_SIZE + HYPER_TILE_SIZE,
            "{} vault is still 128 KiB-padded (131,584 bytes)",
            name
        );
    }
    assert_eq!(
        read_document_record(&vault0, "a").expect("get class-0").unwrap(),
        vec![b'x'; 100]
    );
    assert_eq!(
        read_document_record(&vault1, "a").expect("get class-1").unwrap(),
        vec![b'y'; 5000]
    );
}

#[test]
fn test_class_boundaries_match_select_size_class() {
    use s3a_core::{select_size_class, SLAB_SIZE_CLASSES};
    for c in 0..=8usize {
        let just_fits = SLAB_SIZE_CLASSES[c] as usize;
        assert_eq!(
            select_size_class(just_fits),
            Some(c as u8),
            "payload of exactly class {} ({} bytes) must select class {}",
            c,
            just_fits,
            c
        );
        assert_eq!(
            select_size_class(just_fits + 1),
            Some((c + 1) as u8),
            "payload one byte over class {} must select class {}",
            c,
            c + 1
        );
    }
    assert_eq!(select_size_class(131_072), Some(9));
    assert_eq!(select_size_class(131_073), None);
}

#[test]
fn test_wiki_shape_is_at_least_20x_smaller_than_245mb_prototype() {
    let dir = unique_tmp("wiki");
    let shape = load_shape();
    let measured = load_measured();
    assert_eq!(shape.len(), 1871, "fixture must hold 1,871 files");
    assert_eq!(
        measured["files"].as_u64().unwrap(),
        1871,
        "measured fixture must record 1,871 files"
    );
    // Fixture self-consistency: the distribution the prompt states must follow
    // from select_size_class on the stored size (512 tile header + 64 record
    // header + key + content).
    let mut counts = [0u64; 10];
    let mut total_content: u64 = 0;
    let mut largest: usize = 0;
    for entry in &shape {
        let stored = 512 + 64 + entry.key.len() + entry.size;
        let class = s3a_core::select_size_class(stored).expect("fits class 9");
        counts[class as usize] += 1;
        total_content += entry.size as u64;
        largest = largest.max(entry.size);
    }
    let expected_counts: Vec<u64> = measured["class_counts"]
        .as_array()
        .expect("class_counts")
        .iter()
        .map(|v| v.as_u64().unwrap())
        .collect();
    assert_eq!(
        counts.to_vec(),
        expected_counts,
        "fixture class distribution must match the measured one"
    );
    assert_eq!(total_content, 5_961_273);
    assert_eq!(largest, 117_998);

    let vault = dir.join("wiki.s3a");
    create_vault(&vault);
    pack_shape(&vault, &shape);
    let len = fs::metadata(&vault).expect("stat vault").len();
    let extents = count_slab_extent_files(&dir);
    let ceiling = measured["ceiling_bytes"].as_u64().unwrap() + 4096 * extents;
    assert!(
        len <= ceiling,
        "packed wiki shape is {} bytes, over the measured ceiling {} ({} .slab extents)",
        len,
        ceiling,
        extents
    );
    assert!(
        len * 20 < PROTOTYPE_BYTES,
        "packed wiki shape ({} bytes) is not at least 20x smaller than the 245 MB prototype",
        len
    );
}

#[test]
fn test_wiki_shape_tamper_fails_verify_and_restore_is_byte_identical() {
    let dir = unique_tmp("tamper");
    let shape = load_shape();
    let vault = dir.join("wiki.s3a");
    create_vault(&vault);
    pack_shape(&vault, &shape);
    let sealed = dir.join("wiki.sealed.s3a");
    run_cli(
        &dir,
        [
            "seal",
            "wiki.s3a",
            "wiki.sealed.s3a",
            "--about",
            "--record-roots",
        ],
    );
    let verify_out = run_cli(&dir, ["verify", "--json", "wiki.sealed.s3a"]);
    let report: serde_json::Value =
        serde_json::from_slice(&verify_out.stdout).expect("parse verify --json");
    assert_eq!(report["ok"], true, "verify --json must be ok: {}", report);
    assert_eq!(
        report["is_sealed"], true,
        "verify --json must report is_sealed: {}",
        report
    );

    // Flip one byte in a COPY of the sealed file; verify must fail on the copy.
    let tampered = dir.join("wiki.tampered.s3a");
    let mut bytes = fs::read(&sealed).expect("read sealed");
    assert!(bytes.len() > 2048, "sealed file unexpectedly small");
    bytes[1024] ^= 0x01; // inside the first tile's payload
    fs::write(&tampered, &bytes).expect("write tampered");
    let tampered_out = Command::new(cli_bin())
        .args(["verify", "--json", "wiki.tampered.s3a"])
        .current_dir(&dir)
        .output()
        .expect("run verify on tampered file");
    let tampered_stdout = String::from_utf8_lossy(&tampered_out.stdout);
    let tampered_failed = !tampered_out.status.success() || tampered_stdout.contains("\"ok\":false");
    assert!(
        tampered_failed,
        "flipping one byte must fail verify (status {:?}, stdout {})",
        tampered_out.status, tampered_stdout
    );

    // Restore every key byte-identical from the intact sealed archive.
    let mut first: Option<Vec<u8>> = None;
    let mut second: Option<Vec<u8>> = None;
    for entry in &shape {
        let expected = wiki_content(&entry.key, entry.size);
        let got = read_document_record(&sealed, &entry.key)
            .unwrap_or_else(|e| panic!("read {} from sealed archive: {}", entry.key, e))
            .unwrap_or_else(|| panic!("key {} missing from sealed archive", entry.key));
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
        "two different keys must restore different bytes"
    );

    // The sealed packed archive itself must round-trip through flatten/unflatten
    // byte for byte.
    run_cli(
        &dir,
        [
            "flatten",
            "--zstd",
            "wiki.sealed.s3a",
            "wiki.sealed.s3af",
        ],
    );
    run_cli(
        &dir,
        ["unflatten", "wiki.sealed.s3af", "wiki.restored.s3a"],
    );
    assert_eq!(
        fs::read(dir.join("wiki.restored.s3a")).expect("read restored"),
        fs::read(&sealed).expect("read sealed"),
        "flatten/unflatten of the sealed packed archive must round-trip byte for byte"
    );
}

#[test]
fn test_legacy_128kib_stride_archive_still_opens() {
    let fixture = PathBuf::from(format!("{}/legacy_one_doc.s3a", FIXTURES));
    assert_eq!(
        fs::metadata(&fixture).expect("stat legacy fixture").len(),
        FILE_HEADER_SIZE + HYPER_TILE_SIZE,
        "legacy fixture must be exactly one 128 KiB tile plus the file header"
    );
    let got = read_document_record(&fixture, "legacy-doc")
        .expect("read legacy fixture")
        .expect("legacy-doc key must exist");
    assert_eq!(got, b"hello-slab-legacy".to_vec());
    let listed = list_all_documents(&fixture).expect("list legacy fixture");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].key, "legacy-doc");
}

#[test]
fn test_oversize_document_is_rejected_until_prompt_011() {
    let dir = unique_tmp("oversize");
    let vault = dir.join("big.s3a");
    create_vault(&vault);
    let content = vec![0u8; 200 * 1024];
    let err = write_document_record(&vault, "big", &content);
    assert!(
        err.is_err(),
        "a 200 KiB put must still error until prompt 011 implements spanning"
    );
    // The CLI must fail the same way.
    let big_file = dir.join("big.bin");
    fs::write(&big_file, &content).expect("write big.bin");
    let out = Command::new(cli_bin())
        .args(["put", "big.s3a", "big", "@big.bin"])
        .current_dir(&dir)
        .output()
        .expect("run put on 200 KiB file");
    assert!(
        !out.status.success(),
        "CLI put of a 200 KiB file must fail until prompt 011"
    );
}
