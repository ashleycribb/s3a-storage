//! CLI command handlers for `s3a-cli vcs ...`

use std::path::PathBuf;
use ed25519_dalek::SigningKey;
use rand_core::{OsRng, RngCore};

use s3a_vcs::{export_git, sync_push, VcsRepo};

pub fn handle_vcs_command(args: &[String]) {
    if args.len() < 3 {
        print_vcs_usage();
        return;
    }

    match args[2].as_str() {
        "init" => {
            if args.len() < 4 {
                println!("Usage: s3a-cli vcs init <archive_file>");
                return;
            }
            let file = &args[3];
            match VcsRepo::init(file) {
                Ok(_) => println!("Initialized empty S3A VCS repository in {}", file),
                Err(e) => eprintln!("Error initializing repository: {}", e),
            }
        }
        "commit" => {
            let mut file_opt = None;
            let mut msg_opt = None;
            let mut branch = "main".to_string();

            let mut i = 3;
            while i < args.len() {
                if (args[i] == "-m" || args[i] == "--message") && i + 1 < args.len() {
                    msg_opt = Some(args[i + 1].clone());
                    i += 2;
                } else if args[i] == "--branch" && i + 1 < args.len() {
                    branch = args[i + 1].clone();
                    i += 2;
                } else if !args[i].starts_with('-') && file_opt.is_none() {
                    file_opt = Some(args[i].clone());
                    i += 1;
                } else {
                    i += 1;
                }
            }

            let Some(file) = file_opt else {
                println!("Usage: s3a-cli vcs commit <archive_file> -m \"<message>\" [--branch <branch>]");
                return;
            };
            let msg = msg_opt.unwrap_or_else(|| "Snapshot commit".to_string());

            let signing_key = load_or_generate_key(&file);
            match VcsRepo::open(&file) {
                Ok(repo) => match repo.commit(&msg, &branch, &signing_key, None) {
                    Ok(commit) => {
                        println!("Commit created on branch '{}':", branch);
                        println!("  Commit ID: {}", hex::encode(commit.commit_id));
                        println!("  Seal Hash: {}", hex::encode(commit.seal_hash));
                        println!("  Message:   {}", commit.message());
                    }
                    Err(e) => eprintln!("Commit failed: {}", e),
                },
                Err(e) => eprintln!("Failed to open repository: {}", e),
            }
        }
        "log" => {
            let json_mode = args.iter().any(|a| a == "--json");
            let file_opt = args.iter().skip(3).find(|a| !a.starts_with('-'));
            let Some(file) = file_opt else {
                println!("Usage: s3a-cli vcs log <archive_file> [--json] [ref_or_commit]");
                return;
            };

            let target = args.iter().skip(4).find(|a| !a.starts_with('-')).map(|s| s.as_str());

            match VcsRepo::open(file) {
                Ok(repo) => match repo.log(target) {
                    Ok(commits) => {
                        if json_mode {
                            let json_list: Vec<serde_json::Value> = commits
                                .iter()
                                .map(|c| {
                                    serde_json::json!({
                                        "commit_id": hex::encode(c.commit_id),
                                        "seal_hash": hex::encode(c.seal_hash),
                                        "author_address": format!("s3a-{:02x}", c.author_checksum),
                                        "timestamp": c.committed_timestamp,
                                        "parent_count": c.parent_count,
                                        "message": c.message(),
                                    })
                                })
                                .collect();
                            println!("{}", serde_json::to_string_pretty(&json_list).unwrap());
                        } else {
                            println!("=== S3A VCS COMMIT LOG ({}) ===", file);
                            for c in commits {
                                println!("commit {}", hex::encode(c.commit_id));
                                println!("seal:   {}", hex::encode(c.seal_hash));
                                println!("date:   {}", c.committed_timestamp);
                                println!("parents: {}", c.parent_count);
                                println!("\n    {}\n", c.message());
                            }
                        }
                    }
                    Err(e) => eprintln!("Log failed: {}", e),
                },
                Err(e) => eprintln!("Failed to open repository: {}", e),
            }
        }
        "branch" => {
            if args.len() < 4 {
                println!("Usage: s3a-cli vcs branch <archive_file> [new_branch_name]");
                return;
            }
            let file = &args[3];
            let new_branch = if args.len() >= 5 { Some(&args[4]) } else { None };

            match VcsRepo::open(file) {
                Ok(repo) => {
                    if let Some(branch_name) = new_branch {
                        let signing_key = load_or_generate_key(file);
                        // Point to current main HEAD
                        let target_seal = repo
                            .get_ref("main")
                            .ok()
                            .flatten()
                            .map(|r| r.target_seal_hash)
                            .unwrap_or([0u8; 32]);
                        match repo.set_branch(branch_name, target_seal, &signing_key) {
                            Ok(r) => println!("Created branch '{}' -> {}", r.ref_name(), hex::encode(r.target_seal_hash)),
                            Err(e) => eprintln!("Failed to create branch: {}", e),
                        }
                    } else {
                        // List branches
                        match repo.list_refs() {
                            Ok(refs) => {
                                println!("Branches in {}:", file);
                                for r in refs {
                                    if r.ref_kind == s3a_vcs::REF_KIND_BRANCH {
                                        println!("  * {} ({})", r.ref_name(), hex::encode(r.target_seal_hash));
                                    }
                                }
                            }
                            Err(e) => eprintln!("Failed to list branches: {}", e),
                        }
                    }
                }
                Err(e) => eprintln!("Failed to open repository: {}", e),
            }
        }
        "tag" => {
            if args.len() < 4 {
                println!("Usage: s3a-cli vcs tag <archive_file> [new_tag_name] [commit_seal]");
                return;
            }
            let file = &args[3];
            let new_tag = if args.len() >= 5 { Some(&args[4]) } else { None };

            match VcsRepo::open(file) {
                Ok(repo) => {
                    if let Some(tag_name) = new_tag {
                        let signing_key = load_or_generate_key(file);
                        let target_seal = if args.len() >= 6 {
                            let mut arr = [0u8; 32];
                            if let Ok(h) = hex::decode(&args[5]) {
                                if h.len() == 32 {
                                    arr.copy_from_slice(&h);
                                }
                            }
                            arr
                        } else {
                            repo.get_ref("main")
                                .ok()
                                .flatten()
                                .map(|r| r.target_seal_hash)
                                .unwrap_or([0u8; 32])
                        };
                        match repo.set_tag(tag_name, target_seal, &signing_key) {
                            Ok(r) => println!("Created tag '{}' -> {}", r.ref_name(), hex::encode(r.target_seal_hash)),
                            Err(e) => eprintln!("Failed to create tag: {}", e),
                        }
                    } else {
                        match repo.list_refs() {
                            Ok(refs) => {
                                println!("Tags in {}:", file);
                                for r in refs {
                                    if r.ref_kind == s3a_vcs::REF_KIND_TAG {
                                        println!("  tag {} ({})", r.ref_name(), hex::encode(r.target_seal_hash));
                                    }
                                }
                            }
                            Err(e) => eprintln!("Failed to list tags: {}", e),
                        }
                    }
                }
                Err(e) => eprintln!("Failed to open repository: {}", e),
            }
        }
        "checkout" => {
            if args.len() < 5 {
                println!("Usage: s3a-cli vcs checkout <archive_file> <ref_or_commit>");
                return;
            }
            let file = &args[3];
            let target = &args[4];
            match VcsRepo::open(file) {
                Ok(repo) => {
                    let seal = if let Some(r) = repo.get_ref(target).ok().flatten() {
                        Some(r.target_seal_hash)
                    } else if let Ok(h) = hex::decode(target) {
                        if h.len() == 32 {
                            let mut a = [0u8; 32];
                            a.copy_from_slice(&h);
                            Some(a)
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    match seal {
                        Some(s) => println!("Checked out view at seal {}", hex::encode(s)),
                        None => eprintln!("Could not resolve '{}' to a known ref or commit", target),
                    }
                }
                Err(e) => eprintln!("Failed to open repository: {}", e),
            }
        }
        "diff" => {
            let json_mode = args.iter().any(|a| a == "--json");
            let non_flags: Vec<&str> = args.iter().skip(3).filter(|a| !a.starts_with('-')).map(|s| s.as_str()).collect();
            if non_flags.len() < 1 {
                println!("Usage: s3a-cli vcs diff <archive_file> [commit_a] [commit_b] [--json]");
                return;
            }
            let file = non_flags[0];
            match VcsRepo::open(file) {
                Ok(repo) => {
                    let from_seal = [0u8; 32];
                    let to_seal = repo.get_ref("main").ok().flatten().map(|r| r.target_seal_hash).unwrap_or([0u8; 32]);
                    match repo.diff(&from_seal, &to_seal) {
                        Ok(diff_report) => {
                            if json_mode {
                                println!("{}", serde_json::to_string_pretty(&diff_report).unwrap());
                            } else {
                                println!("=== S3A VCS DIFF: {}..{} ===", &diff_report.from_seal[..8], &diff_report.to_seal[..8]);
                                println!("Records Added:     {}", diff_report.total_records_added);
                                println!("Records Withdrawn: {}", diff_report.total_records_withdrawn);
                                println!("Records Erased:    {}", diff_report.total_records_erased);
                                println!();
                                for td in diff_report.tile_diffs {
                                    println!("  - {}: +{} added, -{} withdrawn", td.type_name, td.records_added, td.records_withdrawn);
                                }
                            }
                        }
                        Err(e) => eprintln!("Diff failed: {}", e),
                    }
                }
                Err(e) => eprintln!("Failed to open repository: {}", e),
            }
        }
        "merge" => {
            if args.len() < 6 {
                println!("Usage: s3a-cli vcs merge <archive_file> <source_branch> <target_branch>");
                return;
            }
            let file = &args[3];
            let source_branch = &args[4];
            let target_branch = &args[5];
            let signing_key = load_or_generate_key(file);

            match VcsRepo::open(file) {
                Ok(repo) => match repo.merge(source_branch, target_branch, &signing_key, true) {
                    Ok(s3a_vcs::VcsMergeResult::Merged(commit)) => {
                        println!("Merged branch '{}' into '{}':", source_branch, target_branch);
                        println!("  Merge Commit: {}", hex::encode(commit.commit_id));
                        println!("  Seal Hash:    {}", hex::encode(commit.seal_hash));
                    }
                    Ok(s3a_vcs::VcsMergeResult::ConflictNeedsHuman { conflicting_records, message }) => {
                        println!("MERGE BLOCKED (Human Supervisor Judgment Required): {}", message);
                        for c in conflicting_records {
                            println!("  Conflict Record: {} ({})", c.record_id, c.type_name);
                            println!("    Version A ({}): {}", target_branch, c.target_version);
                            println!("    Version B ({}): {}", source_branch, c.source_version);
                        }
                    }
                    Ok(s3a_vcs::VcsMergeResult::Conflict { conflicting_record_ids, message }) => {
                        eprintln!("Merge conflict: {}. Conflicting IDs: {:?}", message, conflicting_record_ids);
                    }
                    Err(e) => eprintln!("Merge failed: {}", e),
                },
                Err(e) => eprintln!("Failed to open repository: {}", e),
            }
        }
        "export-git" => {
            if args.len() < 5 {
                println!("Usage: s3a-cli vcs export-git <archive_file> <out_dir>");
                return;
            }
            let file = &args[3];
            let out_dir = &args[4];
            match VcsRepo::open(file) {
                Ok(repo) => match export_git(&repo, out_dir) {
                    Ok(_) => println!("Successfully exported repository to Git in {}", out_dir),
                    Err(e) => eprintln!("Git export failed: {}", e),
                },
                Err(e) => eprintln!("Failed to open repository: {}", e),
            }
        }
        "push" => {
            if args.len() < 5 {
                println!("Usage: s3a-cli vcs push <source_archive> <target_archive> [--branch <branch>] [--merge]");
                return;
            }
            let src = &args[3];
            let tgt = &args[4];
            let branch = args.iter().position(|a| a == "--branch").and_then(|p| args.get(p + 1)).map(|s| s.as_str()).unwrap_or("main");
            let allow_merge = args.iter().any(|a| a == "--merge");
            let signing_key = load_or_generate_key(src);

            let s_repo = match VcsRepo::open(src) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("Failed to open source: {}", e);
                    return;
                }
            };
            let t_repo = match VcsRepo::open(tgt) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("Failed to open target: {}", e);
                    return;
                }
            };

            match sync_push(&s_repo, &t_repo, branch, allow_merge, None, &signing_key) {
                Ok(transferred) => println!("Push completed: transferred {} missing tiles to target", transferred),
                Err(e) => eprintln!("Push failed: {}", e),
            }
        }
        _ => print_vcs_usage(),
    }
}

fn print_vcs_usage() {
    println!("S3A Git-Style Version Control (s3a-cli vcs):");
    println!("  s3a-cli vcs init <archive>                            Initialize VCS on an S3A archive");
    println!("  s3a-cli vcs commit <archive> -m \"<msg>\" [--branch B]   Seal archive with signed commit record");
    println!("  s3a-cli vcs log <archive> [--json] [ref]              Display commit DAG log history");
    println!("  s3a-cli vcs branch <archive> [branch_name]            List branches or create new branch pointer");
    println!("  s3a-cli vcs tag <archive> [tag_name] [seal]           List tags or create immutable snapshot tag");
    println!("  s3a-cli vcs checkout <archive> <ref_or_commit>        Checkout read-only view at ref/commit");
    println!("  s3a-cli vcs diff <archive> [commit_a] [commit_b]      Aggregate record changes without privacy leaks");
    println!("  s3a-cli vcs merge <archive> <source> <target>         Append-only union merge branches");
    println!("  s3a-cli vcs push <src> <tgt> [--branch B] [--merge]   Inventory sync push transmitting missing tiles");
    println!("  s3a-cli vcs export-git <archive> <out_dir>            Export repository to real read-only Git repo");
}

fn load_or_generate_key(file: &str) -> SigningKey {
    let key_file = PathBuf::from(format!("{}.vcs.key", file));
    if key_file.exists() {
        if let Ok(bytes) = std::fs::read(&key_file) {
            if bytes.len() == 32 {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&bytes);
                return SigningKey::from_bytes(&arr);
            }
        }
    }
    let mut seed = [0u8; 32];
    OsRng.fill_bytes(&mut seed);
    let key = SigningKey::from_bytes(&seed);
    let _ = std::fs::write(&key_file, seed);
    key
}
