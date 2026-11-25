//! Folder pack / extract (prompt 012): one whole directory tree packed into
//! a vault and extracted back with fidelity — permissions, empty folders,
//! symlinks.
//!
//! Design (core / engine, not a pack):
//! * One document record per file, written through
//!   [`crate::document::write_document_record`], so prompt 010 slab sizing
//!   and prompt 011 multi-slab spanning apply automatically. A 200 KiB file
//!   inside a packed tree spans exactly as a directly-put file does. File
//!   records are keyed by their relative path (posix separators).
//! * One manifest record under [`MANIFEST_KEY`], carrying a layout version,
//!   the source root name, and the entry list — relative path, kind
//!   (`file` / `dir` / `symlink`), unix mode, size and SHA-256 for files,
//!   target string for symlinks — plus a whole-tree SHA-256 over the sorted
//!   entry list and file hashes. Empty dirs and symlinks live in the
//!   manifest only. A reader that sees an unknown layout version fails
//!   closed.
//! * The manifest and file records are ordinary `DOCUMENT_STORE` tiles, so
//!   `seal` (with `--record-roots`), `verify`, `prove`, `flatten`, and
//!   `unflatten` cover them with no special casing, and `ls --json` shows
//!   the packed tree (the manifest entry plus the file entries).
//!
//! Tree-hash recipe (canonical; the fail-first tests reimplement it from
//! this definition): entries sorted by path (byte order); one line per
//! entry, `{kind}|{path}|{mode:o}|{size}|{hash_or_target}\n`, where `kind`
//! is `file` / `dir` / `symlink`, `mode` is the stored mode formatted as
//! octal, `size` is file bytes (0 for dir/symlink), and `hash_or_target`
//! is the file SHA-256 hex (files), the symlink target string (symlinks),
//! or empty (dirs); SHA-256 over the concatenation.
//!
//! Fidelity and safety rules:
//! * Traversal is deterministic (sorted paths), so the same tree packs to
//!   the same manifest bytes.
//! * Symlinks are never followed — on pack the target string is stored via
//!   `read_link`; on extract the link is recreated as-is, never written
//!   through. A symlink to a file outside the tree stores the target only;
//!   the outside content never enters the vault.
//! * Extract validates the whole manifest (version, tree hash, entry paths,
//!   no entry below a symlink entry, no duplicates) and the overwrite
//!   preflight before writing anything, so a refused extraction leaves the
//!   destination untouched.
//! * Modes are unix mode bits, restored with `set_permissions` on unix; on
//!   non-unix they are stored and restored best-effort (stored as recorded,
//!   restore is a no-op). mtimes are NOT preserved (known limitation).
//! * One tree per vault: packing into a vault that already holds a tree
//!   manifest overwrites the manifest (latest record wins); the previous
//!   tree's file records remain as orphans. Pack refuses a tree that
//!   contains a file with the manifest key itself as its relative path.
//! * The vault file itself is skipped if it lives inside the source tree.

use crate::document::{read_document_record, write_document_record};
use crate::TileWriter;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

/// Document key of the tree manifest record.
pub const MANIFEST_KEY: &str = "__s3a_tree_manifest__";

/// Layout version of the tree manifest. A reader that sees any other
/// version fails closed.
pub const TREE_MANIFEST_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TreeEntryKind {
    File,
    Dir,
    Symlink,
}

impl TreeEntryKind {
    fn as_str(&self) -> &'static str {
        match self {
            TreeEntryKind::File => "file",
            TreeEntryKind::Dir => "dir",
            TreeEntryKind::Symlink => "symlink",
        }
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct TreeEntry {
    /// Posix-style relative path inside the tree (`/` separators).
    pub path: String,
    pub kind: TreeEntryKind,
    /// Full unix mode bits as reported by the filesystem (e.g. `0o100644`).
    pub mode: u32,
    /// File size in bytes; 0 for dirs and symlinks.
    pub size: u64,
    /// Hex SHA-256 of the file bytes; files only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// Symlink target string, verbatim; symlinks only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct TreeManifest {
    pub v: u32,
    /// File name of the packed source directory (informational).
    pub root: String,
    pub entries: Vec<TreeEntry>,
    /// Whole-tree SHA-256 over the sorted entry list (see module docs).
    pub tree_sha256: String,
}

pub struct PackSummary {
    pub files: usize,
    pub dirs: usize,
    pub symlinks: usize,
    pub total_bytes: u64,
}

pub struct ExtractSummary {
    pub files: usize,
    pub dirs: usize,
    pub symlinks: usize,
    pub total_bytes: u64,
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// The canonical tree hash (see the module docs for the recipe).
pub fn compute_tree_sha256(entries: &[TreeEntry]) -> String {
    let mut sorted: Vec<&TreeEntry> = entries.iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));
    let mut h = Sha256::new();
    for e in sorted {
        let hash_or_target = match e.kind {
            TreeEntryKind::File => e.sha256.clone().unwrap_or_default(),
            TreeEntryKind::Symlink => e.target.clone().unwrap_or_default(),
            TreeEntryKind::Dir => String::new(),
        };
        h.update(
            format!(
                "{}|{}|{:o}|{}|{}\n",
                e.kind.as_str(),
                e.path,
                e.mode,
                e.size,
                hash_or_target
            )
            .as_bytes(),
        );
    }
    format!("{:x}", h.finalize())
}

/// Unix mode bits for a metadata record; on non-unix, a read-only file
/// records `0o444`, anything else `0o644`.
#[cfg(unix)]
fn mode_of(md: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    md.permissions().mode()
}

#[cfg(not(unix))]
fn mode_of(md: &fs::Metadata) -> u32 {
    if md.permissions().readonly() {
        0o444
    } else {
        0o644
    }
}

/// Restore permission bits on unix; a no-op elsewhere (best-effort).
#[cfg(unix)]
fn restore_mode(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o7777))
}

#[cfg(not(unix))]
fn restore_mode(_path: &Path, _mode: u32) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn create_symlink(target: &Path, path: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, path)
}

#[cfg(not(unix))]
fn create_symlink(_target: &Path, _path: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "symlink restore is not supported on this platform",
    ))
}

/// Stream a file's SHA-256 without holding it in memory.
fn hash_file(path: &Path) -> Result<String, String> {
    let mut file =
        fs::File::open(path).map_err(|e| format!("cannot open '{}': {}", path.display(), e))?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        use io::Read;
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("cannot read '{}': {}", path.display(), e))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}

struct WalkState {
    entries: Vec<TreeEntry>,
    files: usize,
    dirs: usize,
    symlinks: usize,
    total_bytes: u64,
}

/// Recursive sorted walk. Never follows symlinks (`symlink_metadata`).
/// `vault_canonical` is skipped when it lies inside the tree, so a vault
/// never packs itself.
fn walk(
    root: &Path,
    current: &Path,
    vault_canonical: Option<&Path>,
    state: &mut WalkState,
) -> Result<(), String> {
    let mut items: Vec<(String, PathBuf, fs::Metadata)> = Vec::new();
    let rd = fs::read_dir(current)
        .map_err(|e| format!("cannot list directory '{}': {}", current.display(), e))?;
    for entry in rd {
        let entry =
            entry.map_err(|e| format!("cannot list directory '{}': {}", current.display(), e))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let md = fs::symlink_metadata(entry.path())
            .map_err(|e| format!("cannot stat '{}': {}", entry.path().display(), e))?;
        items.push((name, entry.path(), md));
    }
    items.sort_by(|a, b| a.0.cmp(&b.0));
    for (_name, path, md) in items {
        let rel = path
            .strip_prefix(root)
            .map_err(|_| format!("path '{}' is outside the tree", path.display()))?;
        let rel_posix = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        if rel_posix == MANIFEST_KEY {
            return Err(format!(
                "tree contains a file named '{}', which collides with the tree manifest key; rename it and pack again",
                MANIFEST_KEY
            ));
        }
        if let Some(vault) = vault_canonical {
            if let Ok(canonical) = path.canonicalize() {
                if canonical == vault {
                    continue; // the vault itself: never pack it into itself
                }
            }
        }
        let ft = md.file_type();
        if ft.is_symlink() {
            let target = fs::read_link(&path)
                .map_err(|e| format!("cannot read link '{}': {}", path.display(), e))?;
            state.entries.push(TreeEntry {
                path: rel_posix,
                kind: TreeEntryKind::Symlink,
                mode: mode_of(&md),
                size: 0,
                sha256: None,
                target: Some(target.to_string_lossy().into_owned()),
            });
            state.symlinks += 1;
        } else if ft.is_dir() {
            state.entries.push(TreeEntry {
                path: rel_posix.clone(),
                kind: TreeEntryKind::Dir,
                mode: mode_of(&md),
                size: 0,
                sha256: None,
                target: None,
            });
            state.dirs += 1;
            walk(root, &path, vault_canonical, state)?;
        } else if ft.is_file() {
            let sha256 = hash_file(&path)?;
            state.entries.push(TreeEntry {
                path: rel_posix,
                kind: TreeEntryKind::File,
                mode: mode_of(&md),
                size: md.len(),
                sha256: Some(sha256),
                target: None,
            });
            state.files += 1;
            state.total_bytes += md.len();
        } else {
            return Err(format!(
                "unsupported entry '{}' (not a file, directory, or symlink); only files, directories, and symlinks can be packed",
                path.display()
            ));
        }
    }
    Ok(())
}

/// Pack the directory tree at `dir` into the vault at `vault`, creating the
/// vault if it does not exist. One tree per vault: packing into a vault that
/// already holds a tree manifest overwrites the manifest (latest record
/// wins); the previous tree's file records remain as orphans.
pub fn pack_tree(vault: &Path, dir: &Path) -> Result<PackSummary, String> {
    if !dir.is_dir() {
        return Err(format!(
            "pack source '{}' is not a directory",
            dir.display()
        ));
    }
    if let Some(parent) = vault.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create vault parent '{}': {}", parent.display(), e))?;
        }
    }
    if !vault.exists() {
        TileWriter::create(vault)
            .map_err(|e| format!("failed to create vault '{}': {}", vault.display(), e))?;
    }
    let root = dir
        .canonicalize()
        .map_err(|e| format!("cannot resolve pack source '{}': {}", dir.display(), e))?;
    let vault_canonical = vault.canonicalize().ok();
    let root_name = dir
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| dir.to_string_lossy().into_owned());

    // Phase 1: walk and hash. Nothing is written yet, so an unsupported
    // entry or an unreadable file fails before the vault changes.
    let mut state = WalkState {
        entries: Vec::new(),
        files: 0,
        dirs: 0,
        symlinks: 0,
        total_bytes: 0,
    };
    walk(&root, &root, vault_canonical.as_deref(), &mut state)?;

    // Phase 2: write one document record per file. Each file is re-read
    // and re-hashed; a file that changed between the walk and the write
    // aborts the pack instead of sealing a manifest that lies about it.
    for entry in &state.entries {
        if entry.kind != TreeEntryKind::File {
            continue;
        }
        let file_path = root.join(&entry.path);
        let bytes = fs::read(&file_path)
            .map_err(|e| format!("cannot read '{}': {}", file_path.display(), e))?;
        if sha256_hex(&bytes) != entry.sha256.clone().unwrap_or_default() {
            return Err(format!(
                "file '{}' changed while packing; aborting",
                file_path.display()
            ));
        }
        if bytes.len() as u64 != entry.size {
            return Err(format!(
                "file '{}' changed size while packing; aborting",
                file_path.display()
            ));
        }
        write_document_record(vault, &entry.path, &bytes)?;
    }

    let mut entries = state.entries;
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    let tree_sha256 = compute_tree_sha256(&entries);
    let manifest = TreeManifest {
        v: TREE_MANIFEST_VERSION,
        root: root_name,
        entries,
        tree_sha256,
    };
    let manifest_json = serde_json::to_vec(&manifest)
        .map_err(|e| format!("cannot serialize tree manifest: {}", e))?;
    write_document_record(vault, MANIFEST_KEY, &manifest_json)?;

    Ok(PackSummary {
        files: state.files,
        dirs: state.dirs,
        symlinks: state.symlinks,
        total_bytes: state.total_bytes,
    })
}

/// Lexical path validation for one manifest entry: relative, canonical
/// (no `.`, no empty components), and never escaping the destination.
/// Absolute paths and `..` that would climb out are rejected.
fn check_entry_path(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("manifest entry has an empty path".to_string());
    }
    if path.as_bytes().contains(&0) {
        return Err(format!("manifest entry '{}' contains a NUL byte", path));
    }
    let p = Path::new(path);
    if p.is_absolute() {
        return Err(format!("manifest entry '{}' has an absolute path", path));
    }
    let mut depth: i64 = 0;
    for comp in p.components() {
        match comp {
            Component::CurDir => {
                return Err(format!(
                    "manifest entry '{}' has a non-canonical '.' component",
                    path
                ));
            }
            Component::ParentDir => {
                depth -= 1;
                if depth < 0 {
                    return Err(format!("manifest entry '{}' escapes the destination", path));
                }
            }
            Component::Normal(_) => depth += 1,
            _ => {
                return Err(format!(
                    "manifest entry '{}' has an unsupported path component",
                    path
                ));
            }
        }
    }
    Ok(())
}

/// Full manifest validation: entry paths, no duplicates, no entry below a
/// symlink entry (extract must never write through a link).
fn validate_entries(entries: &[TreeEntry]) -> Result<(), String> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut symlink_paths: HashSet<&str> = HashSet::new();
    for e in entries {
        check_entry_path(&e.path)?;
        if !seen.insert(e.path.as_str()) {
            return Err(format!("manifest lists '{}' twice", e.path));
        }
        match e.kind {
            TreeEntryKind::File => match &e.sha256 {
                Some(h) if h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()) => {}
                _ => {
                    return Err(format!(
                        "manifest entry '{}' carries a malformed SHA-256",
                        e.path
                    ));
                }
            },
            TreeEntryKind::Symlink => {
                if e.target.is_none() {
                    return Err(format!(
                        "manifest entry '{}' is a symlink with no target",
                        e.path
                    ));
                }
                symlink_paths.insert(e.path.as_str());
            }
            TreeEntryKind::Dir => {}
        }
    }
    for e in entries {
        let mut ancestor = Path::new(&e.path).parent();
        while let Some(a) = ancestor {
            if let Some(a_str) = a.to_str() {
                if !a_str.is_empty() && symlink_paths.contains(a_str) {
                    return Err(format!(
                        "manifest entry '{}' sits below symlink '{}'",
                        e.path, a_str
                    ));
                }
            }
            ancestor = a.parent();
        }
    }
    Ok(())
}

/// Create each ancestor of `rel` under `dest`, refusing to traverse a
/// symlink component (defense in depth: manifest validation already
/// rejects entries below symlink entries, but the destination may hold
/// pre-existing links when overwriting).
fn ensure_parent_dirs(dest: &Path, rel: &Path) -> Result<(), String> {
    let parent = match rel.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => return Ok(()),
    };
    let mut cur = dest.to_path_buf();
    for comp in parent.components() {
        let Component::Normal(name) = comp else {
            return Err(format!(
                "cannot create parent directory for '{}': non-canonical component",
                rel.display()
            ));
        };
        cur.push(name);
        match fs::symlink_metadata(&cur) {
            Ok(md) if md.file_type().is_symlink() => {
                return Err(format!(
                    "refusing to traverse symlink '{}' while creating parent directories",
                    cur.display()
                ));
            }
            Ok(md) if !md.is_dir() => {
                return Err(format!("'{}' exists and is not a directory", cur.display()));
            }
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&cur)
                    .map_err(|e| format!("cannot create directory '{}': {}", cur.display(), e))?;
            }
            Err(e) => {
                return Err(format!("cannot stat '{}': {}", cur.display(), e));
            }
        }
    }
    Ok(())
}

/// Remove a file or symlink at `path` (never a directory); missing is fine.
fn remove_if_exists(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(md) if md.is_dir() => Err(format!(
            "'{}' is a directory and cannot be replaced by a file or link",
            path.display()
        )),
        Ok(_) => {
            fs::remove_file(path).map_err(|e| format!("cannot remove '{}': {}", path.display(), e))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("cannot stat '{}': {}", path.display(), e)),
    }
}

/// Extract the packed tree from `vault` into `dest`. Refuses the whole
/// extraction (writing nothing) when the manifest is missing, has an
/// unknown version, fails its tree hash, contains an escaping path, or the
/// destination is non-empty without `overwrite`.
pub fn extract_tree(vault: &Path, dest: &Path, overwrite: bool) -> Result<ExtractSummary, String> {
    let raw = read_document_record(vault, MANIFEST_KEY)?
        .ok_or_else(|| "vault holds no packed tree (missing manifest)".to_string())?;
    let manifest: TreeManifest = serde_json::from_slice(&raw)
        .map_err(|e| format!("packed-tree manifest is not valid JSON: {}", e))?;
    if manifest.v != TREE_MANIFEST_VERSION {
        return Err(format!(
            "packed-tree manifest has unsupported layout version {}",
            manifest.v
        ));
    }
    if compute_tree_sha256(&manifest.entries) != manifest.tree_sha256 {
        return Err("packed-tree manifest fails its whole-tree SHA-256 check".to_string());
    }
    validate_entries(&manifest.entries)?;

    // Overwrite preflight: refuse a non-empty destination without
    // --overwrite, before writing anything.
    if dest.exists() {
        if !dest.is_dir() {
            return Err(format!(
                "destination '{}' exists and is not a directory",
                dest.display()
            ));
        }
        if !overwrite {
            let mut rd = fs::read_dir(dest)
                .map_err(|e| format!("cannot list destination '{}': {}", dest.display(), e))?;
            if rd.next().is_some() {
                return Err(format!(
                    "destination '{}' is not empty; pass --overwrite to replace",
                    dest.display()
                ));
            }
        }
    }
    // Per-path conflict preflight: no packed entry may overwrite an
    // existing path unless --overwrite was given.
    if !overwrite {
        for e in &manifest.entries {
            let target = dest.join(&e.path);
            if fs::symlink_metadata(&target).is_ok() {
                return Err(format!(
                    "refusing to overwrite existing '{}'; pass --overwrite",
                    target.display()
                ));
            }
        }
    }

    fs::create_dir_all(dest)
        .map_err(|e| format!("cannot create destination '{}': {}", dest.display(), e))?;

    let mut summary = ExtractSummary {
        files: 0,
        dirs: 0,
        symlinks: 0,
        total_bytes: 0,
    };
    let mut dir_modes: Vec<(&TreeEntry, PathBuf)> = Vec::new();
    for e in &manifest.entries {
        let target = dest.join(&e.path);
        match e.kind {
            TreeEntryKind::File => {
                ensure_parent_dirs(dest, Path::new(&e.path))?;
                if overwrite {
                    remove_if_exists(&target)?;
                }
                let content = read_document_record(vault, &e.path)?
                    .ok_or_else(|| format!("packed file '{}' is missing from the vault", e.path))?;
                if content.len() as u64 != e.size {
                    return Err(format!(
                        "packed file '{}' has {} bytes, manifest says {}",
                        e.path,
                        content.len(),
                        e.size
                    ));
                }
                if sha256_hex(&content) != e.sha256.clone().unwrap_or_default() {
                    return Err(format!("packed file '{}' fails its SHA-256 check", e.path));
                }
                fs::write(&target, &content)
                    .map_err(|e| format!("cannot write '{}': {}", target.display(), e))?;
                restore_mode(&target, e.mode)
                    .map_err(|e| format!("cannot restore mode on '{}': {}", target.display(), e))?;
                summary.files += 1;
                summary.total_bytes += e.size;
            }
            TreeEntryKind::Dir => {
                ensure_parent_dirs(dest, Path::new(&e.path))?;
                match fs::symlink_metadata(&target) {
                    Ok(md) if md.is_dir() => {}
                    Ok(_) => {
                        fs::remove_file(&target)
                            .map_err(|e| format!("cannot remove '{}': {}", target.display(), e))?;
                        fs::create_dir(&target).map_err(|e| {
                            format!("cannot create directory '{}': {}", target.display(), e)
                        })?;
                    }
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {
                        fs::create_dir(&target).map_err(|e| {
                            format!("cannot create directory '{}': {}", target.display(), e)
                        })?;
                    }
                    Err(e) => {
                        return Err(format!("cannot stat '{}': {}", target.display(), e));
                    }
                }
                dir_modes.push((e, target));
                summary.dirs += 1;
            }
            TreeEntryKind::Symlink => {
                ensure_parent_dirs(dest, Path::new(&e.path))?;
                if overwrite {
                    remove_if_exists(&target)?;
                }
                let link_target = e.target.clone().unwrap_or_default();
                create_symlink(Path::new(&link_target), &target).map_err(|e| {
                    format!(
                        "cannot create symlink '{}' -> '{}': {}",
                        target.display(),
                        link_target,
                        e
                    )
                })?;
                summary.symlinks += 1;
            }
        }
    }

    // Directory modes last, deepest first, so a read-only directory never
    // blocks writing its children.
    dir_modes.sort_by_key(|(e, _)| std::cmp::Reverse(e.path.matches('/').count()));
    for (e, target) in dir_modes {
        restore_mode(&target, e.mode)
            .map_err(|e| format!("cannot restore mode on '{}': {}", target.display(), e))?;
    }

    Ok(summary)
}
