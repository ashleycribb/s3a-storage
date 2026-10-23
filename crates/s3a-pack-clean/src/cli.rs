//! `s3a-cli clean ...` subcommands. Returns a process exit code.
use crate::{clean, raw, rulebook::Rulebook, rulebook::RulebookUpdate};
use ed25519_dalek::SigningKey;
use std::path::{Path, PathBuf};

pub const USAGE: &str = "\
s3a-cli clean ingest <file>... --rulebook <rb.yaml> --raw-out <raw.s3a> --clean-out <clean.s3a> --key <key.hex>
                     [--connector <name>] [--org <o>] [--user <u>] [--device <d>] [--location <l>] [--captured-at <unix>]
s3a-cli clean intake <file>... --state <dir> --rulebook <rb.yaml> --key <key.hex> [--label-scope <s> --label <l>]
                     [--retries <n>] [--connector <name>] [--org ..] [--user ..] [--device ..] [--location ..] [--captured-at <unix>]
                     (dedup + labels + dead letters + signed provenance log at <dir>/intake.log)
s3a-cli clean log-verify <dir>                                       (read-only: chain + signatures; exit 4 if torn tail)
s3a-cli clean log-repair <dir>                                       (truncate a torn final line; takes the lock)
s3a-cli clean gc <dir> [--apply]                                     (list archives not referenced by intake.log; delete with --apply)
s3a-cli clean dlq <dir>                                              (list dead-letter entries)
s3a-cli clean verify <raw.s3a> <clean.s3a> [--rulebook <rb.yaml>]   (exit 0 ok, 2 failed)
s3a-cli clean queue <clean.s3a>                                      (review queue)
s3a-cli clean rulebook-sign <old.yaml> <new.yaml> --key <human.hex> --reason <text> --out <update.json>
s3a-cli clean rulebook-verify <old.yaml> <new.yaml> <update.json>";

fn opt(a: &[String], k: &str) -> Option<String> {
    a.iter()
        .position(|x| x == k)
        .and_then(|i| a.get(i + 1))
        .cloned()
}
fn positional(a: &[String]) -> Vec<String> {
    let mut out = vec![];
    let mut i = 0;
    while i < a.len() {
        if a[i].starts_with("--") {
            i += 2;
        } else {
            out.push(a[i].clone());
            i += 1;
        }
    }
    out
}

/// Load a 32-byte hex Ed25519 seed. Never printed.
pub fn load_key(p: &Path) -> Result<SigningKey, String> {
    let s = std::fs::read_to_string(p).map_err(|e| format!("key {}: {e}", p.display()))?;
    let b: [u8; 32] = hex::decode(s.trim())
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or("key must be 64 hex chars")?;
    Ok(SigningKey::from_bytes(&b))
}

pub fn run(args: &[String]) -> i32 {
    match go(args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            2
        }
    }
}

fn go(args: &[String]) -> Result<i32, String> {
    let Some(cmd) = args.first() else {
        println!("{USAGE}");
        return Ok(1);
    };
    let rest = &args[1..];
    let need = |k: &str| opt(rest, k).ok_or(format!("missing {k}"));
    match cmd.as_str() {
        "intake" => {
            let files: Vec<PathBuf> = positional(rest).into_iter().map(PathBuf::from).collect();
            if files.is_empty() {
                return Err("no input files".into());
            }
            let state = PathBuf::from(need("--state")?);
            let rb_bytes = std::fs::read(need("--rulebook")?).map_err(|e| e.to_string())?;
            let rb = Rulebook::parse(&rb_bytes)?;
            let key = load_key(Path::new(&need("--key")?))?;
            let label = match (opt(rest, "--label-scope"), opt(rest, "--label")) {
                (Some(s), Some(l)) => Some((s, l)),
                (None, None) => None,
                _ => return Err("--label-scope and --label go together".into()),
            };
            let max_retries = opt(rest, "--retries")
                .map(|s| s.parse())
                .transpose()
                .map_err(|_| "bad --retries")?
                .unwrap_or(0);
            let req = crate::intake::Request {
                state_dir: &state,
                files,
                meta: raw::SourceMeta {
                    connector: opt(rest, "--connector").unwrap_or_else(|| "file".into()),
                    org: opt(rest, "--org"),
                    user: opt(rest, "--user"),
                    device: opt(rest, "--device"),
                    location: opt(rest, "--location"),
                },
                label,
                captured_at: match opt(rest, "--captured-at") {
                    Some(s) => s.parse().map_err(|_| "bad --captured-at")?,
                    None => std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0),
                },
                redelivery: crate::intake::Redelivery { max_retries },
            };
            use crate::intake::Outcome::*;
            match crate::intake::ingest(&req, &rb, &rb_bytes, &key, &mut crate::intake::FsIo)
                .map_err(|e| e.to_string())?
            {
                Committed {
                    raw_snapshot_hash,
                    raw_archive,
                    clean_archive,
                    dead_letters,
                    duplicates,
                    ..
                } => {
                    println!("committed raw={raw_snapshot_hash} raw_archive={} clean_archive={} duplicates_skipped={} dead_letters={}", raw_archive.display(), clean_archive.display(), duplicates.len(), dead_letters.len());
                    Ok(if dead_letters.is_empty() { 0 } else { 3 })
                }
                Duplicate {
                    existing,
                    dead_letters,
                } => {
                    for d in &existing {
                        println!(
                            "duplicate {} (connector {}) already in raw {}",
                            d.item_sha256, d.connector_id, d.raw_snapshot_hash
                        );
                    }
                    Ok(if dead_letters.is_empty() { 0 } else { 3 })
                }
                AllDeadLettered(d) => {
                    println!("all inputs dead-lettered: {}", d.len());
                    Ok(3)
                }
            }
        }
        "log-verify" | "dlq" => {
            let dir = positional(rest).into_iter().next().ok_or("missing <dir>")?;
            let log =
                crate::intake_log::IntakeLog::inspect(&crate::intake::log_path(Path::new(&dir)))
                    .map_err(|e| e.to_string())?;
            if cmd == "log-verify" {
                println!(
                    "ok: {} records, {} signed events, head {}",
                    log.records.len(),
                    log.events().count(),
                    log.last_hash()
                );
                if log.recovered_torn_tail {
                    println!("torn tail: {} bytes after the last good line (crash mid-write); run `clean log-repair {dir}`", log.torn_tail_bytes);
                    return Ok(4);
                }
            } else {
                for d in log.dead_letters() {
                    println!("{}", serde_json::to_string(d).map_err(|e| e.to_string())?);
                }
            }
            Ok(0)
        }
        "log-repair" => {
            let dir = PathBuf::from(positional(rest).into_iter().next().ok_or("missing <dir>")?);
            let _lock = crate::intake::StateLock::acquire(&dir).map_err(|e| e.to_string())?;
            let n = crate::intake_log::IntakeLog::repair(&crate::intake::log_path(&dir))
                .map_err(|e| e.to_string())?;
            println!("removed {n} torn bytes");
            Ok(0)
        }
        "gc" => {
            let dir = PathBuf::from(
                rest.iter()
                    .find(|a| !a.starts_with("--"))
                    .ok_or("missing <dir>")?,
            );
            let apply = rest.iter().any(|a| a == "--apply");
            let orphans = crate::intake::gc(&dir, apply).map_err(|e| e.to_string())?;
            for o in &orphans {
                println!(
                    "{} {} ({} bytes)",
                    if apply { "deleted" } else { "would delete" },
                    o.path.display(),
                    o.bytes
                );
            }
            println!(
                "{} unreferenced archive(s){}",
                orphans.len(),
                if apply {
                    ""
                } else {
                    "; dry run, pass --apply to delete"
                }
            );
            Ok(0)
        }
        "ingest" => {
            let files: Vec<PathBuf> = positional(rest).into_iter().map(PathBuf::from).collect();
            if files.is_empty() {
                return Err("no input files".into());
            }
            let rb_bytes = std::fs::read(need("--rulebook")?).map_err(|e| e.to_string())?;
            let rb = Rulebook::parse(&rb_bytes)?;
            let key = load_key(Path::new(&need("--key")?))?;
            let meta = raw::SourceMeta {
                connector: opt(rest, "--connector").unwrap_or_else(|| "file".into()),
                org: opt(rest, "--org"),
                user: opt(rest, "--user"),
                device: opt(rest, "--device"),
                location: opt(rest, "--location"),
            };
            let at = match opt(rest, "--captured-at") {
                Some(s) => s.parse().map_err(|_| "bad --captured-at")?,
                None => std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
            };
            let raw_out = PathBuf::from(need("--raw-out")?);
            let clean_out = PathBuf::from(need("--clean-out")?);
            if clean_out.exists() {
                return Err(format!(
                    "{} exists; sealed archives are never overwritten",
                    clean_out.display()
                ));
            }
            let refs: Vec<&Path> = files.iter().map(PathBuf::as_path).collect();
            raw::intake(&refs, &meta, at, &raw_out).map_err(|e| e.to_string())?;
            // Re-read from the sealed archive so cleaning only ever sees what was sealed.
            let (snap, blobs) = raw::load(&raw_out).map_err(|e| e.to_string())?;
            let cv = clean::clean(&snap, &blobs, &rb, &rb_bytes, &key);
            clean::write(&clean_out, &cv).map_err(|e| e.to_string())?;
            println!(
                "raw snapshot:   {} ({} items) -> {}",
                snap.snapshot_hash,
                snap.items.len(),
                raw_out.display()
            );
            println!(
                "cleaned hash:   {} -> {}",
                cv.cleaned_hash,
                clean_out.display()
            );
            println!(
                "rulebook:       {} (sha256 {})",
                cv.body.rulebook_version, cv.body.rulebook_sha256
            );
            println!(
                "records: {}  changes: {}  review queue: {}",
                cv.body.records.len(),
                cv.body.changes.len(),
                cv.body.review_queue.len()
            );
            Ok(0)
        }
        "verify" => {
            let p = positional(rest);
            if p.len() != 2 {
                return Err("usage: clean verify <raw.s3a> <clean.s3a>".into());
            }
            let (snap, blobs) = raw::load(Path::new(&p[0])).map_err(|e| format!("raw: {e}"))?;
            let cv = clean::read(Path::new(&p[1])).map_err(|e| format!("clean: {e}"))?;
            clean::verify(&cv, &snap)?;
            if let Some(rbp) = opt(rest, "--rulebook") {
                let b = std::fs::read(rbp).map_err(|e| e.to_string())?;
                let rb = Rulebook::parse(&b)?;
                let h = clean::body_hash(&clean::clean_body(&snap, &blobs, &rb, &b));
                if h != cv.cleaned_hash {
                    return Err(format!(
                        "replay mismatch: got {h}, archive says {}",
                        cv.cleaned_hash
                    ));
                }
                println!("replay: OK");
            }
            println!(
                "OK: {} records cite raw snapshot {}",
                cv.body.records.len(),
                snap.snapshot_hash
            );
            Ok(0)
        }
        "queue" => {
            let p = positional(rest);
            let cv = clean::read(Path::new(p.first().ok_or("missing <clean.s3a>")?))
                .map_err(|e| e.to_string())?;
            for q in &cv.body.review_queue {
                println!(
                    "{:?}\t{}\trow={}\tfield={}\t{}",
                    q.tag,
                    q.source_path,
                    q.row.map(|r| r.to_string()).unwrap_or("-".into()),
                    q.field.as_deref().unwrap_or("-"),
                    q.reason
                );
            }
            Ok(0)
        }
        "rulebook-sign" => {
            let p = positional(rest);
            if p.len() != 2 {
                return Err("usage: clean rulebook-sign <old> <new> ...".into());
            }
            let old = std::fs::read(&p[0]).map_err(|e| e.to_string())?;
            let new = std::fs::read(&p[1]).map_err(|e| e.to_string())?;
            let key = load_key(Path::new(&need("--key")?))?;
            let u = RulebookUpdate::sign(&old, &new, &need("--reason")?, &key)?;
            u.verify(&old, &new)?;
            let out = need("--out")?;
            std::fs::write(&out, serde_json::to_vec_pretty(&u).unwrap())
                .map_err(|e| e.to_string())?;
            println!("signed rulebook update -> {out}");
            Ok(0)
        }
        "rulebook-verify" => {
            let p = positional(rest);
            if p.len() != 3 {
                return Err("usage: clean rulebook-verify <old> <new> <update.json>".into());
            }
            let r = |i: usize| std::fs::read(&p[i]).map_err(|e| e.to_string());
            let u: RulebookUpdate = serde_json::from_slice(&r(2)?).map_err(|e| e.to_string())?;
            u.verify(&r(0)?, &r(1)?)?;
            println!(
                "OK: rulebook update to {} signed by approver {}",
                u.to_version, u.signer_pubkey
            );
            Ok(0)
        }
        _ => {
            println!("{USAGE}");
            Ok(1)
        }
    }
}
