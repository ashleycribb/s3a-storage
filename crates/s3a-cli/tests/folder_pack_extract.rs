//! Prompt 012 fail-first tests: folder pack / extract — one whole directory
//! tree packed into a vault and extracted back with fidelity (permissions,
//! empty folders, symlinks).
//!
//! NOTE — deviations from the prompt, flagged for the report:
//! 1. The prompt expects these tests to be COPIED from `/workspace/slab-prompts/`
//!    (or a copy next to the prompt file). Those sources did not exist anywhere
//!    on this machine (not in the workspace, not in the repo, and the live wiki
//!    is not on this VM), so this file was WRITTEN from the prompt's
//!    specifications instead. The fixture tree is built at runtime in a temp
//!    dir, as the prompt requires — no binary fixtures are needed.
//! 2. The wiki "read first" pages do not exist on this VM (`/home/box/wiki`
//!    absent). Proceeded from the prompt plus repo docs.
//! 3. This file is `#![cfg(unix)]`: modes (`0600`, `0755`) and symlinks are
//!    unix concepts, and the prompt's test list is written in those terms.
//!    The `pack` / `extract` implementation itself still builds on every
//!    platform (unix modes are stored and restored best-effort elsewhere,
//!    as the prompt requires); only these tests are unix-gated.
//! 4. The tree-hash recipe the tampered-manifest test uses to recompute a
//!    valid hash is the canonical recipe defined in
//!    `s3a_engine::tree::compute_tree_sha256`: entries sorted by path; one
//!    line per entry, `{kind}|{path}|{mode:o}|{size}|{hash_or_target}\n`,
//!    where `kind` is `file`/`dir`/`symlink`, `mode` is the stored mode
//!    formatted as octal, `size` is file bytes (0 for dir/symlink), and
//!    `hash_or_target` is the file SHA-256 hex (files), the symlink target
//!    (symlinks), or empty (dirs); SHA-256 over the concatenation. The test
//!    reimplements that recipe from this definition so the fail-first tests
//!    compile against current main (which has no `tree` module yet) and fail
//!    at runtime on the unknown subcommand, as the prompt requires. The
//!    mutation check proves the two implementations agree: if they drift,
//!    disabling path validation no longer makes the traversal test fail.

#![cfg(unix)]

use s3a_cli::first_run::write_document_record;
use sha2::{Digest, Sha256};
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

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
        "test-tmp-012-{}_{}_{}",
        name,
        std::process::id(),
        nanos
    ));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn run_cli<'a>(dir: &Path, args: impl AsRef<[&'a str]>) -> Output {
    let out = run_cli_raw(dir, args.as_ref());
    assert!(
        out.status.success(),
        "s3a {:?} failed (status {:?}):\nstdout: {}\nstderr: {}",
        args.as_ref(),
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    out
}

fn run_cli_raw<'a>(dir: &Path, args: impl AsRef<[&'a str]>) -> Output {
    Command::new(cli_bin())
        .args(args.as_ref())
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("failed to run s3a {:?}: {}", args.as_ref(), e))
}

fn run_cli_fails<'a>(dir: &Path, args: impl AsRef<[&'a str]>) -> Output {
    let out = run_cli_raw(dir, args.as_ref());
    assert!(
        !out.status.success(),
        "s3a {:?} unexpectedly succeeded:\nstdout: {}\nstderr: {}",
        args.as_ref(),
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

/// The tree-hash recipe (see the header note): entries sorted by path, one
/// `{kind}|{path}|{mode:o}|{size}|{hash_or_target}` line each, SHA-256 over
/// the concatenation.
fn tree_sha256(entries: &serde_json::Value) -> String {
    let mut rows: Vec<&serde_json::Value> =
        entries.as_array().expect("entries array").iter().collect();
    rows.sort_by(|a, b| {
        a["path"]
            .as_str()
            .unwrap_or("")
            .cmp(b["path"].as_str().unwrap_or(""))
    });
    let mut h = Sha256::new();
    for e in rows {
        let kind = e["kind"].as_str().expect("entry kind");
        let path = e["path"].as_str().expect("entry path");
        let mode = e["mode"].as_u64().expect("entry mode");
        let size = e.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
        let hash_or_target = if kind == "file" {
            e["sha256"].as_str().unwrap_or("").to_string()
        } else if kind == "symlink" {
            e["target"].as_str().unwrap_or("").to_string()
        } else {
            String::new()
        };
        h.update(
            format!("{}|{}|{:o}|{}|{}\n", kind, path, mode, size, hash_or_target)
                .as_bytes(),
        );
    }
    format!("{:x}", h.finalize())
}

/// `s3a ls --json --vault <vault>` parsed as JSON.
fn ls_json(dir: &Path, vault: &str) -> serde_json::Value {
    let out = run_cli(dir, ["ls", "--json", "--vault", vault]);
    serde_json::from_slice(&out.stdout).expect("parse ls --json")
}

fn doc_keys(ls: &serde_json::Value) -> Vec<String> {
    ls["documents"]
        .as_array()
        .expect("ls documents array")
        .iter()
        .map(|d| d["key"].as_str().expect("doc key").to_string())
        .collect()
}

fn doc_entry<'a>(ls: &'a serde_json::Value, key: &str) -> &'a serde_json::Value {
    ls["documents"]
        .as_array()
        .expect("ls documents array")
        .iter()
        .find(|d| d["key"].as_str() == Some(key))
        .unwrap_or_else(|| panic!("key {} missing from ls --json: {}", key, ls))
}

/// The packed tree's manifest: the one document key that is not one of the
/// tree's file keys, fetched with `s3a get` and parsed as JSON.
fn find_manifest(dir: &Path, vault: &str, file_keys: &[&str]) -> (String, serde_json::Value) {
    let ls = ls_json(dir, vault);
    let keys = doc_keys(&ls);
    let manifest_key = keys
        .iter()
        .find(|k| !file_keys.contains(&k.as_str()))
        .unwrap_or_else(|| {
            panic!(
                "no manifest key in ls (keys {:?}, file keys {:?})",
                keys, file_keys
            )
        })
        .clone();
    let out = run_cli(dir, ["get", vault, &manifest_key]);
    let manifest: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("manifest parses as JSON");
    assert!(
        manifest.get("entries").is_some(),
        "manifest must carry an entries list: {}",
        manifest
    );
    assert_eq!(
        manifest.get("v").and_then(|v| v.as_u64()),
        Some(1),
        "manifest must carry layout version 1: {}",
        manifest
    );
    (manifest_key, manifest)
}

fn mode_of(path: &Path) -> u32 {
    fs::metadata(path)
        .unwrap_or_else(|e| panic!("stat {}: {}", path.display(), e))
        .permissions()
        .mode()
        & 0o777
}

fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .unwrap_or_else(|e| panic!("lstat {}: {}", path.display(), e))
        .file_type()
        .is_symlink()
}

fn link_target(path: &Path) -> String {
    fs::read_link(path)
        .unwrap_or_else(|e| panic!("read_link {}: {}", path.display(), e))
        .to_string_lossy()
        .into_owned()
}

fn set_mode(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .unwrap_or_else(|e| panic!("chmod {} {:o}: {}", path.display(), mode, e));
}

/// The fixture tree used by the round-trip, seal, and overwrite tests:
/// nested dirs, small files, one 200 KiB file (must span via 011), one empty
/// dir, a symlink to a file and a symlink to a dir inside the tree, one
/// `0600` file and one `0755` executable.
struct FixtureTree {
    /// (relative path, expected bytes) for every regular file.
    files: Vec<(String, Vec<u8>)>,
}

fn build_tree(root: &Path) -> FixtureTree {
    fs::create_dir_all(root.join("sub/nested")).expect("create nested dirs");
    fs::create_dir_all(root.join("empty")).expect("create empty dir");
    let mut files = Vec::new();
    let mut write = |rel: &str, bytes: &[u8]| {
        let p = root.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).expect("create parent dirs");
        }
        fs::write(&p, bytes).unwrap_or_else(|e| panic!("write {}: {}", p.display(), e));
        files.push((rel.to_string(), bytes.to_vec()));
    };
    write("a.txt", b"hello a\n");
    write("sub/b.txt", b"hello b\n");
    write("sub/nested/c.txt", b"hello c\n");
    write("big.bin", &content_for("big-200kib", 200 * 1024));
    write("secret.txt", b"top secret\n");
    write("run.sh", b"#!/bin/sh\necho hi\n");
    set_mode(&root.join("secret.txt"), 0o600);
    set_mode(&root.join("run.sh"), 0o755);
    symlink("a.txt", root.join("link-file")).expect("symlink link-file -> a.txt");
    symlink("sub", root.join("link-dir")).expect("symlink link-dir -> sub");
    FixtureTree { files }
}

fn file_keys(tree: &FixtureTree) -> Vec<&str> {
    tree.files.iter().map(|(k, _)| k.as_str()).collect()
}

/// Assert the extracted tree matches the fixture: every file byte-identical,
/// the empty dir present, both symlinks recreated as links with the same
/// targets, modes preserved.
fn assert_tree_restored(root: &Path, tree: &FixtureTree) {
    for (rel, expected) in &tree.files {
        let p = root.join(rel);
        let got = fs::read(&p).unwrap_or_else(|e| panic!("read {}: {}", p.display(), e));
        assert_eq!(got, *expected, "file {} did not restore byte-identical", rel);
    }
    assert!(
        root.join("empty").is_dir(),
        "empty dir must be restored as a directory"
    );
    assert!(is_symlink(&root.join("link-file")), "link-file must be a symlink");
    assert_eq!(
        link_target(&root.join("link-file")),
        "a.txt",
        "link-file target must be preserved"
    );
    assert!(is_symlink(&root.join("link-dir")), "link-dir must be a symlink");
    assert_eq!(
        link_target(&root.join("link-dir")),
        "sub",
        "link-dir target must be preserved"
    );
    assert_eq!(
        mode_of(&root.join("secret.txt")),
        0o600,
        "secret.txt mode must stay 0600"
    );
    assert_eq!(
        mode_of(&root.join("run.sh")),
        0o755,
        "run.sh mode must stay 0755"
    );
}

#[test]
fn test_pack_extract_round_trips_tree() {
    let dir = unique_tmp("roundtrip");
    let tree_root = dir.join("tree");
    let tree = build_tree(&tree_root);

    run_cli(&dir, ["pack", "tree", "tree.s3a"]);

    // ls --json must show the packed tree: the manifest entry plus the file
    // entries with sha256 and chunk_count (the 200 KiB file spans via 011).
    let ls = ls_json(&dir, "tree.s3a");
    let keys = doc_keys(&ls);
    for (rel, expected) in &tree.files {
        let entry = doc_entry(&ls, rel);
        assert_eq!(
            entry["sha256"].as_str().unwrap(),
            sha256_hex(expected),
            "ls sha256 for {} must match its content",
            rel
        );
        assert!(
            entry["chunk_count"].as_u64().unwrap() >= 1,
            "ls chunk_count for {} must be at least 1",
            rel
        );
    }
    let big = doc_entry(&ls, "big.bin");
    assert!(
        big["chunk_count"].as_u64().unwrap() >= 2,
        "200 KiB file must span at least 2 chunks, got {}",
        big["chunk_count"]
    );
    assert_eq!(
        big["size_bytes"].as_u64().unwrap(),
        200 * 1024,
        "big.bin size must be 200 KiB"
    );
    // The manifest entry itself must be listed.
    let (_manifest_key, manifest) = find_manifest(&dir, "tree.s3a", &file_keys(&tree));
    assert!(
        keys.len() > tree.files.len(),
        "ls must list the manifest in addition to the {} files",
        tree.files.len()
    );
    let _ = manifest;

    run_cli(&dir, ["extract", "tree.s3a", "restored"]);
    assert_tree_restored(&dir.join("restored"), &tree);
}

#[test]
fn test_symlink_is_stored_as_link_not_followed() {
    let dir = unique_tmp("symlink");
    // A file OUTSIDE the tree, with content that must never enter the vault.
    let outside = dir.join("outside.txt");
    let outside_content = b"OUTSIDE SECRET -- must never be stored\n";
    fs::write(&outside, outside_content).expect("write outside file");
    let outside_sha = sha256_hex(outside_content);

    let tree_root = dir.join("tree");
    fs::create_dir_all(&tree_root).expect("create tree");
    fs::write(tree_root.join("a.txt"), b"hello a\n").expect("write a.txt");
    symlink(&outside, tree_root.join("link-out")).expect("symlink link-out -> outside");

    run_cli(&dir, ["pack", "tree", "tree.s3a"]);

    // The outside file's content must not be in the vault: no document
    // carries its hash, and the link itself is not a document (links live
    // in the manifest only).
    let ls = ls_json(&dir, "tree.s3a");
    for doc in ls["documents"].as_array().expect("documents") {
        assert_ne!(
            doc["sha256"].as_str().unwrap(),
            outside_sha,
            "outside content leaked into the vault under key {}",
            doc["key"]
        );
        assert_ne!(
            doc["key"].as_str().unwrap(),
            "link-out",
            "link-out must not be a document record"
        );
    }
    // The manifest records the link as a link, with the target string.
    let (_key, manifest) = find_manifest(&dir, "tree.s3a", &["a.txt"]);
    let link_entry = manifest["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .find(|e| e["path"] == "link-out")
        .expect("link-out entry in manifest");
    assert_eq!(link_entry["kind"], "symlink", "link-out must be a symlink entry");
    assert_eq!(
        link_entry["target"].as_str().unwrap(),
        outside.to_str().unwrap(),
        "link-out target must be the outside path verbatim"
    );

    run_cli(&dir, ["extract", "tree.s3a", "restored"]);
    let restored_link = dir.join("restored/link-out");
    assert!(is_symlink(&restored_link), "link-out must be restored as a link");
    assert_eq!(
        link_target(&restored_link),
        outside.to_str().unwrap(),
        "link-out target must be preserved verbatim"
    );
    // Reading through the restored link reaches the outside file — proving
    // it is a real link, not a copy of the content.
    assert_eq!(
        fs::read(&restored_link).expect("read through link-out"),
        outside_content,
        "reading through the restored link must reach the outside file"
    );
    assert_eq!(
        fs::read(dir.join("restored/a.txt")).expect("read restored a.txt"),
        b"hello a\n"
    );
}

#[test]
fn test_path_traversal_fails_closed() {
    let dir = unique_tmp("traversal");
    let tree_root = dir.join("tree");
    let tree = build_tree(&tree_root);
    run_cli(&dir, ["pack", "tree", "tree.s3a"]);
    let vault = dir.join("tree.s3a");

    // Part A: a crafted manifest — take the real manifest, move one file
    // entry to `../evil.txt`, recompute the tree hash so the manifest is
    // otherwise valid. Extract must refuse the whole extraction and write
    // nothing outside the destination.
    let (manifest_key, mut manifest) = find_manifest(&dir, "tree.s3a", &file_keys(&tree));
    let entries = manifest["entries"].as_array_mut().expect("entries");
    let first_file = entries
        .iter_mut()
        .find(|e| e["kind"] == "file")
        .expect("a file entry to tamper");
    first_file["path"] = serde_json::json!("../evil.txt");
    manifest["tree_sha256"] = serde_json::json!(tree_sha256(&manifest["entries"]));
    write_document_record(
        &vault,
        &manifest_key,
        &serde_json::to_vec(&manifest).expect("serialize tampered manifest"),
    )
    .expect("overwrite manifest with tampered copy");
    run_cli_fails(&dir, ["extract", "tree.s3a", "out"]);
    assert!(
        !dir.join("evil.txt").exists(),
        "traversal must not write evil.txt outside the destination"
    );
    assert!(
        !dir.join("out").exists(),
        "a refused extraction must not create the destination"
    );

    // Part B: a symlink inside the tree whose target escapes the tree must
    // be recreated as-is, never written through.
    let tree2_root = dir.join("tree2");
    fs::create_dir_all(&tree2_root).expect("create tree2");
    fs::write(tree2_root.join("a.txt"), b"hello a\n").expect("write a.txt");
    symlink("../../etc", tree2_root.join("escape")).expect("symlink escape -> ../../etc");
    run_cli(&dir, ["pack", "tree2", "tree2.s3a"]);
    run_cli(&dir, ["extract", "tree2.s3a", "out2"]);
    let restored_escape = dir.join("out2/escape");
    assert!(is_symlink(&restored_escape), "escape must be restored as a link");
    assert_eq!(
        link_target(&restored_escape),
        "../../etc",
        "escape target must be preserved verbatim, not resolved"
    );
    assert_eq!(
        fs::read(dir.join("out2/a.txt")).expect("read out2 a.txt"),
        b"hello a\n"
    );
}

#[test]
fn test_extract_refuses_to_overwrite() {
    let dir = unique_tmp("overwrite");
    let tree_root = dir.join("tree");
    let tree = build_tree(&tree_root);
    run_cli(&dir, ["pack", "tree", "tree.s3a"]);
    run_cli(&dir, ["extract", "tree.s3a", "restored"]);

    // A sentinel file the second extraction must not touch.
    fs::write(dir.join("restored/keep.txt"), b"keep me\n").expect("write keep.txt");

    run_cli_fails(&dir, ["extract", "tree.s3a", "restored"]);
    assert_eq!(
        fs::read(dir.join("restored/keep.txt")).expect("read keep.txt"),
        b"keep me\n",
        "refused extraction must leave existing files untouched"
    );
    assert_eq!(
        fs::read(dir.join("restored/a.txt")).expect("read a.txt"),
        b"hello a\n",
        "refused extraction must leave packed files untouched"
    );

    // With --overwrite the same extraction replaces the packed entries.
    run_cli(&dir, ["extract", "tree.s3a", "restored", "--overwrite"]);
    assert_tree_restored(&dir.join("restored"), &tree);
    assert_eq!(
        fs::read(dir.join("restored/keep.txt")).expect("read keep.txt after overwrite"),
        b"keep me\n",
        "overwrite replaces packed entries; unrelated files stay"
    );
}

#[test]
fn test_pack_extract_seal_verify() {
    let dir = unique_tmp("seal");
    let tree_root = dir.join("tree");
    let tree = build_tree(&tree_root);
    run_cli(&dir, ["pack", "tree", "tree.s3a"]);

    run_cli(
        &dir,
        [
            "seal",
            "tree.s3a",
            "tree.sealed.s3a",
            "--about",
            "--record-roots",
        ],
    );
    let verify_out = run_cli(&dir, ["verify", "--json", "tree.sealed.s3a"]);
    let report: serde_json::Value =
        serde_json::from_slice(&verify_out.stdout).expect("parse verify --json");
    assert_eq!(report["ok"], true, "verify --json must be ok: {}", report);
    assert_eq!(
        report["is_sealed"], true,
        "verify --json must report is_sealed: {}",
        report
    );
    assert_eq!(
        report["record_roots"]["state"], "ok",
        "record roots must be ok: {}",
        report
    );
    let doc_count = ls_json(&dir, "tree.sealed.s3a")["documents"]
        .as_array()
        .expect("documents")
        .len() as u64;
    assert!(
        report["record_roots"]["checked"].as_u64().unwrap() >= doc_count,
        "record roots must cover at least the {} documents: {}",
        doc_count,
        report
    );

    // Flipping one byte of the sealed file must make verify fail.
    let tampered = dir.join("tree.tampered.s3a");
    let mut bytes = fs::read(dir.join("tree.sealed.s3a")).expect("read sealed");
    assert!(bytes.len() > 2048, "sealed file unexpectedly small");
    bytes[1024] ^= 0x01;
    fs::write(&tampered, &bytes).expect("write tampered");
    let tampered_out = run_cli_raw(&dir, ["verify", "--json", "tree.tampered.s3a"]);
    let tampered_stdout = String::from_utf8_lossy(&tampered_out.stdout);
    let tampered_failed =
        !tampered_out.status.success() || tampered_stdout.contains("\"ok\":false");
    assert!(
        tampered_failed,
        "flipping one byte must fail verify (status {:?}, stdout {})",
        tampered_out.status, tampered_stdout
    );

    // Extract from the sealed vault still restores byte-identical files.
    run_cli(&dir, ["extract", "tree.sealed.s3a", "restored-sealed"]);
    assert_tree_restored(&dir.join("restored-sealed"), &tree);
}

#[test]
fn test_empty_tree_and_empty_dir_only() {
    let dir = unique_tmp("empty");

    // A tree containing only an empty dir.
    let only_root = dir.join("only");
    fs::create_dir_all(only_root.join("only-empty")).expect("create only-empty");
    run_cli(&dir, ["pack", "only", "only.s3a"]);
    run_cli(&dir, ["extract", "only.s3a", "only-restored"]);
    assert!(
        dir.join("only-restored/only-empty").is_dir(),
        "an empty dir must survive pack/extract"
    );

    // A tree with zero files at all.
    let bare_root = dir.join("bare");
    fs::create_dir_all(&bare_root).expect("create bare tree");
    run_cli(&dir, ["pack", "bare", "bare.s3a"]);
    run_cli(&dir, ["extract", "bare.s3a", "bare-restored"]);
    assert!(
        dir.join("bare-restored").is_dir(),
        "extracting an empty tree must still create the destination"
    );
    assert_eq!(
        fs::read_dir(dir.join("bare-restored"))
            .expect("read bare-restored")
            .count(),
        0,
        "an empty tree must extract to an empty destination"
    );
}
