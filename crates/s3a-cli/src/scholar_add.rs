//! `scholar add-paper` CLI command (usability fix B2).
//!
//! The command function lives here so the `scholar` dispatch in
//! `main.rs` — owned by another agent — only needs the hook described at
//! the bottom of this file. This module is declared in the `s3a-cli`
//! library crate (`src/lib.rs`), so the binary can call it as
//! `s3a_cli::scholar_add::run_scholar_add_paper(&args)`.
//!
//! Usage:
//! ```text
//! s3a-cli scholar add-paper <archive.s3a> --doi <doi> [--file <paper.pdf>]
//!     [--dossier <dossier.json>] [--title "..."] [--year 2024] [--venue 1]
//!     [--citations 25] [--open-access] [--topic x,y,z]
//! ```
//! * `<archive.s3a>` is created if missing (with `ADDON_EDU` enabled).
//! * `--doi` accepts `10.1000/182`, `doi:10.1000/182`, or a full
//!   `https://doi.org/…` URL; it is normalised before hashing.
//! * `--file` is a content file (usually the paper PDF). Its SHA-256 is
//!   stored as a citation fingerprint; the file bytes are never copied
//!   into the archive.
//! * `--dossier` is a JSON file with the same fields (`doi`, `title`,
//!   `year`, `venue_id`, `citation_count`, `open_access`, `topic`).
//!   Explicit flags override dossier fields.
//! * `--topic x,y,z` places the paper explicitly on the 3D topic space
//!   (each axis `[0, 1]`); without it, coordinates are the deterministic
//!   hash spread from `s3a_pack_scholar::derive_topic_coords`.
//!
//! All hashing and Hilbert-coordinate computation happens inside
//! `s3a_pack_scholar::add_paper`; this module only parses arguments,
//! reads the optional file, and prints the receipt.
//!
//! ## Dispatch hook for the coordinator (add to `crates/s3a-cli/src/main.rs`)
//!
//! Inside the `"scholar"` match arm (next to `"ingest"`), add:
//! ```ignore
//! "add-paper" => {
//!     s3a_cli::scholar_add::run_scholar_add_paper(&args[3..]);
//! }
//! ```
//! and add the usage line to the bare-`scholar` usage print:
//! ```ignore
//! println!("  s3a-cli scholar add-paper <archive.s3a> --doi <doi> [--file <paper.pdf>] [--dossier <dossier.json>] [--title \"...\"] [--year <n>] [--venue <id>] [--citations <n>] [--open-access] [--topic x,y,z]");
//! ```
//! No `mod scholar_add;` line is needed in `main.rs`: this file is a
//! module of the `s3a_cli` library crate, which the binary already links.
//! (If the coordinator prefers a binary-local module instead, add
//! `mod scholar_add;` to `main.rs` and call
//! `scholar_add::run_scholar_add_paper(&args[3..])` — the function
//! signature is the same either way.)
//!
//! Failure behaviour follows the neighbouring scholar commands in
//! `main.rs`: errors print `[ERROR] …` to stderr and exit 1 (the
//! `proof`/`export-bib` precedent in the same dispatch).

use std::path::PathBuf;

use s3a_pack_scholar::{add_paper, content_kind_for_file_name, AddPaperInput};

/// Parsed `scholar add-paper` invocation: target archive, paper metadata,
/// and an optional content file.
#[derive(Debug, Clone)]
pub struct AddPaperCli {
    pub archive: PathBuf,
    pub input: AddPaperInput,
    pub file: Option<PathBuf>,
}

pub const ADD_PAPER_USAGE: &str = "Usage: s3a-cli scholar add-paper <archive.s3a> --doi <doi> [--file <paper.pdf>] [--dossier <dossier.json>] [--title \"...\"] [--year <n>] [--venue <id>] [--citations <n>] [--open-access] [--topic x,y,z]";

fn parse_topic(value: &str) -> Result<[f32; 3], String> {
    let parts: Vec<&str> = value.split(',').collect();
    if parts.len() != 3 {
        return Err(format!(
            "--topic needs exactly 3 comma-separated numbers (x,y,z), got {value:?}"
        ));
    }
    let mut coords = [0.0f32; 3];
    for (i, p) in parts.iter().enumerate() {
        let n: f32 = p
            .trim()
            .parse()
            .map_err(|_| format!("--topic value {p:?} is not a number"))?;
        if !n.is_finite() {
            return Err(format!("--topic value {p:?} is not finite"));
        }
        coords[i] = n;
    }
    Ok(coords)
}

/// Parses the arguments after `scholar add-paper` (i.e. `args[3..]` from
/// `main.rs`: archive path first, then flags). Returns a descriptive
/// error string on any malformed input; the caller prints it and exits.
pub fn parse_add_paper_args(args: &[String]) -> Result<AddPaperCli, String> {
    if args.is_empty() {
        return Err(ADD_PAPER_USAGE.to_string());
    }
    let archive = PathBuf::from(&args[0]);
    let mut dossier_path: Option<String> = None;
    let mut file: Option<PathBuf> = None;
    // Raw flag values, overlaid on the dossier (if any) afterwards so
    // explicit flags win regardless of argument order.
    let mut doi: Option<String> = None;
    let mut title: Option<String> = None;
    let mut year: Option<u16> = None;
    let mut venue_id: Option<u16> = None;
    let mut citation_count: Option<u32> = None;
    let mut open_access = false;
    let mut open_access_given = false;
    let mut topic: Option<[f32; 3]> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--doi" => {
                if i + 1 >= args.len() {
                    return Err("--doi needs a value".to_string());
                }
                doi = Some(args[i + 1].clone());
                i += 2;
            }
            "--file" => {
                if i + 1 >= args.len() {
                    return Err("--file needs a path".to_string());
                }
                file = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--dossier" => {
                if i + 1 >= args.len() {
                    return Err("--dossier needs a path".to_string());
                }
                dossier_path = Some(args[i + 1].clone());
                i += 2;
            }
            "--title" => {
                if i + 1 >= args.len() {
                    return Err("--title needs a value".to_string());
                }
                title = Some(args[i + 1].clone());
                i += 2;
            }
            "--year" => {
                if i + 1 >= args.len() {
                    return Err("--year needs a value".to_string());
                }
                year = Some(
                    args[i + 1]
                        .parse::<u16>()
                        .map_err(|_| format!("--year value {:?} is not a year", args[i + 1]))?,
                );
                i += 2;
            }
            "--venue" => {
                if i + 1 >= args.len() {
                    return Err("--venue needs a numeric venue id".to_string());
                }
                venue_id =
                    Some(args[i + 1].parse::<u16>().map_err(|_| {
                        format!("--venue value {:?} is not a numeric id", args[i + 1])
                    })?);
                i += 2;
            }
            "--citations" => {
                if i + 1 >= args.len() {
                    return Err("--citations needs a value".to_string());
                }
                citation_count =
                    Some(args[i + 1].parse::<u32>().map_err(|_| {
                        format!("--citations value {:?} is not a number", args[i + 1])
                    })?);
                i += 2;
            }
            "--open-access" => {
                open_access = true;
                open_access_given = true;
                i += 1;
            }
            "--topic" => {
                if i + 1 >= args.len() {
                    return Err("--topic needs x,y,z".to_string());
                }
                topic = Some(parse_topic(&args[i + 1])?);
                i += 2;
            }
            other => {
                return Err(format!("Unknown argument for scholar add-paper: {other}"));
            }
        }
    }

    let mut input = if let Some(dossier) = &dossier_path {
        let json = std::fs::read_to_string(dossier)
            .map_err(|e| format!("cannot read dossier {dossier}: {e}"))?;
        AddPaperInput::from_dossier_json(&json)?
    } else {
        AddPaperInput::default()
    };
    if let Some(d) = doi {
        input.doi = d;
    }
    if let Some(t) = title {
        input.title = Some(t);
    }
    if let Some(y) = year {
        input.year = Some(y);
    }
    if let Some(v) = venue_id {
        input.venue_id = Some(v);
    }
    if let Some(c) = citation_count {
        input.citation_count = c;
    }
    if open_access_given {
        input.open_access = open_access;
    }
    if let Some(t) = topic {
        input.topic = Some(t);
    }
    if input.doi.trim().is_empty() {
        return Err("--doi <doi> is required (or a dossier with a \"doi\" field)".to_string());
    }
    Ok(AddPaperCli {
        archive,
        input,
        file,
    })
}

/// Runs `scholar add-paper` for the arguments after the subcommand.
/// Prints a receipt on success; on failure prints `[ERROR] …` to stderr
/// and exits 1, matching the neighbouring scholar commands in `main.rs`.
pub fn run_scholar_add_paper(args: &[String]) {
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{ADD_PAPER_USAGE}");
        println!("Adds one paper to an S3A archive, computing all hashes and Hilbert coordinates from the DOI.");
        println!("The title (if given) influences topic placement only — Tile 12 stores hashes, not titles.");
        return;
    }
    let cli = match parse_add_paper_args(args) {
        Ok(cli) => cli,
        Err(e) => {
            eprintln!("[ERROR] scholar add-paper: {e}");
            eprintln!("{ADD_PAPER_USAGE}");
            std::process::exit(1);
        }
    };
    let (file_bytes, file_kind) = match &cli.file {
        Some(path) => match std::fs::read(path) {
            Ok(bytes) => {
                let kind = content_kind_for_file_name(
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .as_ref(),
                );
                (Some(bytes), kind)
            }
            Err(e) => {
                eprintln!(
                    "[ERROR] scholar add-paper: cannot read file {}: {e}",
                    path.display()
                );
                std::process::exit(1);
            }
        },
        None => (None, 0),
    };
    match add_paper(&cli.archive, &cli.input, file_bytes.as_deref(), file_kind) {
        Ok(out) => {
            println!("Added paper to '{}':", cli.archive.display());
            println!(
                "  DOI: {} (paper_id 0x{:016x})",
                out.doi_normalized, out.record.paper_id
            );
            println!("  Coordinate: {}", out.coordinate);
            println!(
                "  {} · {} citations · venue {} · open access: {}",
                if out.record.year == 0 {
                    "year unknown".to_string()
                } else {
                    format!("year {}", out.record.year)
                },
                out.record.citation_count,
                out.record.venue_id,
                if out.record.open_access_flag == 1 {
                    "yes"
                } else {
                    "no"
                },
            );
            println!(
                "  Topic: ({:.3}, {:.3}, {:.3}) · Hilbert index: {}",
                out.record.topic_x,
                out.record.topic_y,
                out.record.topic_z,
                out.record.hilbert_index,
            );
            if let Some(path) = &cli.file {
                println!(
                    "  Content: {} (SHA-256 {}) — citation fingerprint stored",
                    path.display(),
                    out.content_sha256_hex.as_deref().unwrap_or("?"),
                );
            }
            if cli.input.title.is_some() {
                println!("  Note: the title is not stored in the record (Tile 12 holds hashes only); it influenced topic placement and cannot be shown back by SIFT.");
            }
        }
        Err(e) => {
            eprintln!("[ERROR] scholar add-paper: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn s(v: &str) -> String {
        v.to_string()
    }

    fn temp_path(label: &str, suffix: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut p = std::env::temp_dir();
        p.push(format!(
            "s3a_cli_add_paper_{}_{}_{}{}",
            label,
            std::process::id(),
            id,
            suffix
        ));
        p
    }

    #[test]
    fn test_parse_add_paper_args_full() {
        let cli = parse_add_paper_args(&[
            s("/tmp/x.s3a"),
            s("--doi"),
            s("https://doi.org/10.1000/182"),
            s("--file"),
            s("/tmp/p.pdf"),
            s("--title"),
            s("Some Title"),
            s("--year"),
            s("2024"),
            s("--venue"),
            s("3"),
            s("--citations"),
            s("12"),
            s("--open-access"),
            s("--topic"),
            s("0.1,0.2,0.3"),
        ])
        .unwrap();
        assert_eq!(cli.archive, PathBuf::from("/tmp/x.s3a"));
        assert_eq!(cli.input.doi, "https://doi.org/10.1000/182");
        assert_eq!(cli.file, Some(PathBuf::from("/tmp/p.pdf")));
        assert_eq!(cli.input.title.as_deref(), Some("Some Title"));
        assert_eq!(cli.input.year, Some(2024));
        assert_eq!(cli.input.venue_id, Some(3));
        assert_eq!(cli.input.citation_count, 12);
        assert!(cli.input.open_access);
        assert_eq!(cli.input.topic, Some([0.1, 0.2, 0.3]));
    }

    #[test]
    fn test_parse_add_paper_args_rejects_bad_input() {
        assert!(parse_add_paper_args(&[]).is_err());
        assert!(parse_add_paper_args(&[s("/tmp/x.s3a")]).is_err()); // no --doi
        assert!(parse_add_paper_args(&[
            s("/tmp/x.s3a"),
            s("--doi"),
            s("10.1/x"),
            s("--topic"),
            s("0.1,0.2")
        ])
        .is_err());
        assert!(parse_add_paper_args(&[
            s("/tmp/x.s3a"),
            s("--doi"),
            s("10.1/x"),
            s("--year"),
            s("abc")
        ])
        .is_err());
        assert!(
            parse_add_paper_args(&[s("/tmp/x.s3a"), s("--doi"), s("10.1/x"), s("--bogus")])
                .is_err()
        );
    }

    #[test]
    fn test_parse_add_paper_args_dossier_with_flag_override() {
        let dossier = temp_path("dossier", ".json");
        std::fs::write(
            &dossier,
            r#"{"doi":"10.1000/182","title":"Dossier Title","year":2023,"citation_count":5}"#,
        )
        .unwrap();
        let cli = parse_add_paper_args(&[
            s("/tmp/x.s3a"),
            s("--dossier"),
            dossier.to_string_lossy().into_owned(),
            s("--year"),
            s("2024"),
        ])
        .unwrap();
        assert_eq!(cli.input.doi, "10.1000/182");
        assert_eq!(cli.input.title.as_deref(), Some("Dossier Title"));
        assert_eq!(cli.input.year, Some(2024)); // flag overrides dossier
        assert_eq!(cli.input.citation_count, 5);
        let _ = std::fs::remove_file(&dossier);
    }

    #[test]
    fn test_run_add_paper_end_to_end_then_sift_reads_it_back() {
        let archive = temp_path("e2e", ".s3a");
        let pdf = temp_path("e2e", ".pdf");
        std::fs::write(&pdf, b"%PDF-1.4 end-to-end paper bytes").unwrap();
        let _guard = scopeguard(&archive, &pdf);

        run_scholar_add_paper(&[
            archive.to_string_lossy().into_owned(),
            s("--doi"),
            s("10.1000/182"),
            s("--file"),
            pdf.to_string_lossy().into_owned(),
            s("--title"),
            s("End to end Hilbert paper"),
            s("--year"),
            s("2024"),
            s("--citations"),
            s("7"),
            s("--open-access"),
        ]);

        // Full loop: SIFT via the engine (same handlers the CLI registers)
        // must show the decoded paper readably.
        s3a_pack_scholar::register_ql_handlers();
        let query = format!(
            "SIFT ACADEMIC_PAPERS FROM \"{}\"",
            archive.to_string_lossy()
        );
        let result = s3a_engine::execute_query(&query).unwrap();
        match result {
            s3a_engine::QueryResult::PackData {
                count,
                summary,
                json_payload,
                ..
            } => {
                assert_eq!(count, 1);
                assert!(summary.contains("year 2024"), "{summary}");
                assert!(summary.contains("7 citations"), "{summary}");
                assert!(summary.contains("OA yes"), "{summary}");
                assert!(summary.contains("hilbert"), "{summary}");
                assert!(summary.contains("cannot be recovered"), "{summary}");
                let payload: serde_json::Value =
                    serde_json::from_str(&json_payload.unwrap()).unwrap();
                assert_eq!(payload["records"][0]["year"], 2024);
                assert_eq!(payload["records"][0]["title"], serde_json::Value::Null);
            }
            other => panic!("expected PackData, got {other:?}"),
        }
    }

    struct Guard(Vec<PathBuf>);
    impl Drop for Guard {
        fn drop(&mut self) {
            for p in &self.0 {
                let _ = std::fs::remove_file(p);
            }
        }
    }
    fn scopeguard(a: &PathBuf, b: &PathBuf) -> Guard {
        Guard(vec![a.clone(), b.clone()])
    }
}
