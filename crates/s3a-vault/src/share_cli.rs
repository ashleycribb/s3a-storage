//! `share` / `open` / `contact` commands, shared by the `s3a-vault` binary and
//! `s3a-cli vault`. Exit codes: 0 ok/VERIFIED, 1 usage/error, 3 REJECTED.

use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

use crate::address::GateAddress;
use crate::keystore::FileKeyStore;
use crate::share::{self, Contacts, SeenStore, ShareToken};
use crate::Vault;

pub const USAGE: &str = "\
    share <vault_file> <item_hash_hex|item_file> [--ttl <secs>] [--aud <gate_address>] [--out-dir <dir>] [--passphrase <pass>]
    open <gate://...share link|qr_image.png> [--item <file>|--item-hash <hex>] [--as <vault_file>] [--seen <file>] [--contacts <file>]
    contact add <petname> <pubkey_hex|gate://...share link> [--contacts <file>]";

fn arg(args: &[String], k: &str) -> Option<String> {
    args.iter()
        .position(|a| a == k)
        .and_then(|i| args.get(i + 1).cloned())
}

fn state_dir() -> PathBuf {
    if let Ok(d) = std::env::var("S3A_VAULT_DIR") {
        return PathBuf::from(d);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    Path::new(&home).join(".s3a").join("vault")
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn parse_hash(s: &str) -> Option<[u8; 32]> {
    let v = hex::decode(s).ok()?;
    v.try_into().ok()
}

fn hash_file(p: &str) -> Result<[u8; 32], String> {
    let d = std::fs::read(p).map_err(|e| format!("cannot read {p}: {e}"))?;
    Ok(Sha256::digest(d).into())
}

/// Runs a subcommand; `args[0]` is `share`, `open` or `contact`.
pub fn run(args: &[String]) -> i32 {
    let r = match args.first().map(|s| s.as_str()) {
        Some("share") => cmd_share(&args[1..]),
        Some("open") => return cmd_open(&args[1..]),
        Some("contact") => cmd_contact(&args[1..]),
        _ => Err(format!("usage:\n{USAGE}")),
    };
    match r {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn cmd_share(args: &[String]) -> Result<(), String> {
    if args.len() < 2 {
        return Err(format!("usage:\n{USAGE}"));
    }
    let vault_path = &args[0];
    let vault = Vault::open(vault_path).map_err(|e| e.to_string())?;
    let id = vault
        .primary_identity()
        .ok_or("vault has no primary identity")?;
    let item_hash = match parse_hash(&args[1]) {
        Some(h) if vault.items().contains_key(&h) => h,
        Some(_) if !Path::new(&args[1]).exists() => {
            return Err("item hash not found in this vault".into())
        }
        _ => hash_file(&args[1])?,
    };
    let pass = arg(args, "--passphrase")
        .or_else(|| std::env::var("S3A_VAULT_PASSPHRASE").ok())
        .unwrap_or_else(|| "default-dev-passphrase".into());
    let ks =
        FileKeyStore::open(format!("{vault_path}.keystore"), &pass).map_err(|e| e.to_string())?;
    let ttl = arg(args, "--ttl")
        .and_then(|s| s.parse().ok())
        .unwrap_or(24 * 3600);
    let mut nonce = [0u8; 16];
    OsRng.fill_bytes(&mut nonce);
    let t = share::issue(
        &ks,
        &id.identity_id,
        item_hash,
        now(),
        ttl,
        nonce,
        arg(args, "--aud"),
    )
    .map_err(|e| e.to_string())?;
    let link = t.to_link();
    let dir = PathBuf::from(arg(args, "--out-dir").unwrap_or_else(|| ".".into()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stem = format!("share-{}", &hex::encode(nonce)[..8]);
    let svg = dir.join(format!("{stem}.svg"));
    let png = dir.join(format!("{stem}.png"));
    std::fs::write(&svg, share::qr_svg(&link).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    std::fs::write(&png, share::qr_png(&link, 8).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    println!("{link}");
    println!("issuer: {}", t.address().to_uri());
    println!("item:   {}", hex::encode(item_hash));
    println!("expires: {} (unix)", t.expires_at);
    println!("qr svg: {}", svg.display());
    println!("qr png: {}", png.display());
    Ok(())
}

fn cmd_open(args: &[String]) -> i32 {
    let Some(input) = args.first() else {
        eprintln!("usage:\n{USAGE}");
        return 1;
    };
    let item = match (arg(args, "--item"), arg(args, "--item-hash")) {
        (Some(f), _) => match hash_file(&f) {
            Ok(h) => Some(h),
            Err(e) => {
                eprintln!("error: {e}");
                return 1;
            }
        },
        (None, Some(h)) => match parse_hash(&h) {
            Some(h) => Some(h),
            None => {
                eprintln!("error: --item-hash must be 64 hex chars");
                return 1;
            }
        },
        _ => None,
    };
    let me = match arg(args, "--as") {
        Some(v) => match Vault::open(&v) {
            Ok(v) => v
                .primary_identity()
                .map(|i| GateAddress::from_public_key(&i.public_key).to_address_string()),
            Err(e) => {
                eprintln!("error: {e}");
                return 1;
            }
        },
        None => None,
    };
    let seen_p = arg(args, "--seen")
        .map(PathBuf::from)
        .unwrap_or_else(|| state_dir().join("share-seen.json"));
    let contacts_p = arg(args, "--contacts")
        .map(PathBuf::from)
        .unwrap_or_else(|| state_dir().join("contacts.json"));
    let contacts = Contacts::load(&contacts_p).unwrap_or_default();
    let t = match share::token_from_input(input) {
        Ok(t) => t,
        Err(r) => {
            println!("REJECTED: {r}\nDo not trust or act on this code.");
            return 3;
        }
    };
    let mut seen = match SeenStore::load(&seen_p) {
        Ok(s) => s,
        Err(e) => {
            println!("REJECTED: replay store unreadable ({e}); refusing to verify");
            return 3;
        }
    };
    let res = share::open(&t, now(), me.as_deref(), item.as_ref(), &mut seen);
    if res.is_ok() {
        if let Err(e) = seen.save(&seen_p) {
            println!("REJECTED: cannot record nonce ({e}); refusing to verify");
            return 3;
        }
    }
    print!("{}", share::describe(&t, &res, &contacts, item.is_some()));
    if res.is_ok() {
        0
    } else {
        println!();
        3
    }
}

fn cmd_contact(args: &[String]) -> Result<(), String> {
    if args.len() < 3 || args[0] != "add" {
        return Err(format!("usage:\n{USAGE}"));
    }
    let pk: [u8; 32] = if args[2].starts_with("gate://") {
        ShareToken::from_link(&args[2])
            .map_err(|e| e.to_string())?
            .issuer_pubkey
    } else {
        hex::decode(&args[2])
            .ok()
            .and_then(|v| v.try_into().ok())
            .ok_or("pubkey must be 64 hex chars")?
    };
    let p = arg(args, "--contacts")
        .map(PathBuf::from)
        .unwrap_or_else(|| state_dir().join("contacts.json"));
    let mut c = Contacts::load(&p).map_err(|e| e.to_string())?;
    c.by_key.insert(hex::encode(pk), args[1].clone());
    c.save(&p).map_err(|e| e.to_string())?;
    println!(
        "pinned {} -> {}",
        args[1],
        GateAddress::from_public_key(&pk).to_uri()
    );
    Ok(())
}
