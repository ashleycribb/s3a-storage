//! Gate Directory Client Operations (§11).
//!
//! Provides CLI and programmatic client capabilities for registering sovereign gate
//! identities via challenge-response cryptographic proofs and resolving permanent gate:// URIs.

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};

use s3a_vault::address::GateAddress;
use s3a_vault::keystore::{FileKeyStore, KeyStore};
use s3a_vault::records::VaultOwnerKind;
use s3a_vault::Vault;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub name: String,
    pub address: String,
    pub public_key: String, // hex
    pub endpoints: Vec<String>,
    pub sequence: u32,
    pub valid_from: u64,
    pub valid_until: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterGateRequest {
    pub name: String,
    pub address: String,
    pub public_key: String, // hex
    pub endpoints: Vec<String>,
    pub challenge: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChallengeResponse {
    pub challenge: String,
}

/// Normalizes directory base URL so requests consistently route to `/gate/...`.
pub fn normalize_directory_url(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    if trimmed.ends_with("/gate") {
        trimmed.to_string()
    } else {
        format!("{}/gate", trimmed)
    }
}

/// Default storage directory for sovereign vaults outside the repository.
pub fn default_vault_dir() -> PathBuf {
    if let Ok(profile) = std::env::var("USERPROFILE") {
        PathBuf::from(profile).join(".s3a").join("vault")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".s3a").join("vault")
    } else {
        PathBuf::from(".s3a").join("vault")
    }
}

/// Default FileKeyStore path.
pub fn default_keystore_path() -> PathBuf {
    default_vault_dir().join("person.keystore")
}

/// Default Vault archive path.
pub fn default_vault_path() -> PathBuf {
    default_vault_dir().join("person.vault.s3a")
}

/// Resolves a gate name against a Gate Directory server.
pub fn resolve_gate(directory_url: &str, name: &str) -> Result<DirectoryEntry, String> {
    let base_url = normalize_directory_url(directory_url);
    let url = format!("{}/resolve/{}", base_url, name.trim());
    let resp = ureq::get(&url)
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(404, _) => format!("Gate name '{}' not found in directory ({})", name, url),
            ureq::Error::Status(code, r) => {
                let msg = r.into_string().unwrap_or_default();
                format!("Failed to resolve gate name '{}' (HTTP {}): {}", name, code, msg)
            }
            ureq::Error::Transport(t) => format!("Transport error connecting to {}: {}", url, t),
        })?;

    let text = resp
        .into_string()
        .map_err(|e| format!("Failed to read directory response: {}", e))?;
    serde_json::from_str::<DirectoryEntry>(&text)
        .map_err(|e| format!("Failed to parse directory response JSON: {}", e))
}

/// Registers a gate name on a Gate Directory server using cryptographic challenge-response.
pub fn register_gate(
    directory_url: &str,
    name: &str,
    keystore: &impl KeyStore,
    identity_id: &[u8; 16],
    endpoints: Vec<String>,
) -> Result<DirectoryEntry, String> {
    let base_url = normalize_directory_url(directory_url);
    let pubkey = keystore
        .get_public_key(identity_id)
        .map_err(|e| format!("Failed to get public key for identity: {}", e))?;
    let address = GateAddress::from_public_key(&pubkey).to_address_string();

    // 1. Fetch challenge nonce from server
    let challenge_url = format!("{}/challenge", base_url);
    let chal_resp = ureq::post(&challenge_url)
        .call()
        .map_err(|e| format!("Failed to obtain challenge from {}: {}", challenge_url, e))?;
    let chal_text = chal_resp
        .into_string()
        .map_err(|e| format!("Failed to read challenge response: {}", e))?;
    let chal_data: ChallengeResponse = serde_json::from_str(&chal_text)
        .map_err(|e| format!("Failed to parse challenge JSON: {}", e))?;

    let challenge_bytes = hex::decode(&chal_data.challenge)
        .map_err(|e| format!("Corrupted challenge hex from directory: {}", e))?;

    // 2. Sign challenge nonce with private key custody
    let sig_bytes = keystore
        .sign(identity_id, &challenge_bytes)
        .map_err(|e| format!("Failed to sign challenge with key: {}", e))?;

    // 3. Register gate name with proof
    let req = RegisterGateRequest {
        name: name.trim().to_string(),
        address: address.clone(),
        public_key: hex::encode(pubkey),
        endpoints,
        challenge: chal_data.challenge,
        signature: hex::encode(sig_bytes),
    };

    let req_body = serde_json::to_string(&req)
        .map_err(|e| format!("Failed to serialize registration payload: {}", e))?;

    let register_url = format!("{}/register", base_url);
    let reg_resp = ureq::post(&register_url)
        .set("Content-Type", "application/json")
        .send_string(&req_body)
        .map_err(|e| match e {
            ureq::Error::Status(409, _) => {
                format!("Registration rejected: Gate name '{}' is already taken by a different authority key.", name)
            }
            ureq::Error::Status(429, _) => {
                "Registration rejected: Rate limit exceeded (too many registrations).".to_string()
            }
            ureq::Error::Status(code, r) => {
                let msg = r.into_string().unwrap_or_default();
                format!("Registration failed (HTTP {}): {}", code, msg)
            }
            ureq::Error::Transport(t) => format!("Transport error submitting registration to {}: {}", register_url, t),
        })?;

    let reg_text = reg_resp
        .into_string()
        .map_err(|e| format!("Failed to read registration response: {}", e))?;
    serde_json::from_str::<DirectoryEntry>(&reg_text)
        .map_err(|e| format!("Failed to parse registration response JSON: {}", e))
}

fn get_arg_val(args: &[String], flag: &str) -> Option<String> {
    for i in 0..args.len() {
        if args[i] == flag && i + 1 < args.len() {
            return Some(args[i + 1].clone());
        }
    }
    None
}

fn get_or_prompt_passphrase(provided: Option<String>) -> Result<String, String> {
    if let Some(p) = provided {
        if !p.is_empty() {
            return Ok(p);
        }
    }
    if let Ok(p) = std::env::var("S3A_VAULT_PASSPHRASE") {
        if !p.is_empty() {
            return Ok(p);
        }
    }
    print!("Enter vault passphrase: ");
    let _ = io::stdout().flush();
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map_err(|e| format!("Failed to read passphrase: {}", e))?;
    let pass = input.trim_end_matches(&['\r', '\n'][..]).to_string();
    if pass.is_empty() {
        return Err("Passphrase cannot be empty".to_string());
    }
    Ok(pass)
}

pub fn handle_gate_command(args: &[String]) {
    if args.len() < 3 {
        print_gate_usage();
        return;
    }

    match args[2].as_str() {
        "resolve" => {
            if args.len() < 4 {
                println!("Usage: s3a-cli gate resolve <name> --directory <url>");
                return;
            }
            let name = &args[3];
            let Some(directory) = get_arg_val(args, "--directory") else {
                eprintln!("Error: --directory <url> is required");
                return;
            };

            match resolve_gate(&directory, name) {
                Ok(entry) => {
                    println!("Resolved gate name from directory:");
                    println!("  Directory:    {}", normalize_directory_url(&directory));
                    println!("  Gate Name:    gate://{}", entry.name);
                    println!("  Gate Address: {}", entry.address);
                    println!("  Public Key:   {}", entry.public_key);
                    if !entry.endpoints.is_empty() {
                        println!("  Endpoints:    {}", entry.endpoints.join(", "));
                    }
                    println!("  Sequence:     {}", entry.sequence);
                    println!("  Valid From:   {}", entry.valid_from);
                    println!("  Valid Until:  {}", entry.valid_until);
                }
                Err(err) => {
                    eprintln!("{}", err);
                }
            }
        }
        "register" => {
            if args.len() < 4 {
                println!("Usage: s3a-cli gate register <name> --directory <url> [--keystore <path>] [--vault <path>] [--passphrase <pass>] [--endpoints <csv>]");
                return;
            }
            let name = &args[3];
            let Some(directory) = get_arg_val(args, "--directory") else {
                eprintln!("Error: --directory <url> is required");
                return;
            };

            let vault_arg = get_arg_val(args, "--vault").map(PathBuf::from);
            let keystore_arg = get_arg_val(args, "--keystore").map(PathBuf::from);
            let pass_arg = get_arg_val(args, "--passphrase");
            let endpoints_csv = get_arg_val(args, "--endpoints");
            let endpoints: Vec<String> = endpoints_csv
                .map(|csv| csv.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
                .unwrap_or_default();

            let ks_path = keystore_arg.unwrap_or_else(|| {
                if let Some(ref v_path) = vault_arg {
                    PathBuf::from(format!("{}.keystore", v_path.display()))
                } else {
                    default_keystore_path()
                }
            });

            let vault_path = vault_arg.unwrap_or_else(default_vault_path);

            let passphrase = match get_or_prompt_passphrase(pass_arg) {
                Ok(p) => p,
                Err(err) => {
                    eprintln!("Error: {}", err);
                    return;
                }
            };

            let (keystore, identity_id) = if ks_path.exists() {
                let mut ks = match FileKeyStore::open(&ks_path, &passphrase) {
                    Ok(k) => k,
                    Err(e) => {
                        eprintln!("Failed to open keystore {:?}: {}", ks_path, e);
                        return;
                    }
                };

                let id = if vault_path.exists() {
                    match Vault::open(&vault_path) {
                        Ok(v) => v.primary_identity().map(|i| i.identity_id),
                        Err(_) => ks.first_identity(),
                    }
                } else {
                    ks.first_identity()
                };

                let id = match id {
                    Some(i) => i,
                    None => {
                        let mut new_id = [0u8; 16];
                        OsRng.fill_bytes(&mut new_id);
                        if let Err(e) = ks.generate_keypair(&new_id) {
                            eprintln!("Failed to generate identity keypair: {}", e);
                            return;
                        }
                        if let Err(e) = ks.save(&passphrase) {
                            eprintln!("Failed to save keystore: {}", e);
                            return;
                        }
                        new_id
                    }
                };
                (ks, id)
            } else {
                if let Some(parent) = ks_path.parent() {
                    if let Err(e) = fs::create_dir_all(parent) {
                        eprintln!("Failed to create keystore directory {:?}: {}", parent, e);
                        return;
                    }
                }

                let mut ks = match FileKeyStore::open_or_create(&ks_path, &passphrase) {
                    Ok(k) => k,
                    Err(e) => {
                        eprintln!("Failed to create keystore {:?}: {}", ks_path, e);
                        return;
                    }
                };

                let mut new_id = [0u8; 16];
                OsRng.fill_bytes(&mut new_id);

                if let Some(v_parent) = vault_path.parent() {
                    let _ = fs::create_dir_all(v_parent);
                }

                if !vault_path.exists() {
                    let vault_name = format!("{} Sovereign Vault", name);
                    match Vault::create(&vault_path, &vault_name, VaultOwnerKind::Person, &mut ks) {
                        Ok(v) => {
                            if let Some(primary) = v.primary_identity() {
                                new_id = primary.identity_id;
                            }
                        }
                        Err(e) => {
                            eprintln!("Warning: could not create vault archive {:?}: {}", vault_path, e);
                            if let Err(gen_err) = ks.generate_keypair(&new_id) {
                                eprintln!("Failed to generate identity keypair: {}", gen_err);
                                return;
                            }
                        }
                    }
                } else {
                    if let Err(gen_err) = ks.generate_keypair(&new_id) {
                        eprintln!("Failed to generate identity keypair: {}", gen_err);
                        return;
                    }
                }

                if let Err(e) = ks.save(&passphrase) {
                    eprintln!("Failed to persist encrypted keystore: {}", e);
                    return;
                }

                println!("Created new sovereign Person identity in keystore: {:?}", ks_path);
                (ks, new_id)
            };

            match register_gate(&directory, name, &keystore, &identity_id, endpoints) {
                Ok(entry) => {
                    println!("Successfully registered gate name on directory:");
                    println!("  Directory:    {}", normalize_directory_url(&directory));
                    println!("  Gate Name:    gate://{}", entry.name);
                    println!("  Gate Address: {}", entry.address);
                    println!("  Public Key:   {}", entry.public_key);
                    if !entry.endpoints.is_empty() {
                        println!("  Endpoints:    {}", entry.endpoints.join(", "));
                    }
                    println!("  Sequence:     {}", entry.sequence);
                    println!("  Keystore:     {}", ks_path.display());
                    if vault_path.exists() {
                        println!("  Vault:        {}", vault_path.display());
                    }
                }
                Err(err) => {
                    eprintln!("{}", err);
                }
            }
        }
        _ => {
            print_gate_usage();
        }
    }
}

pub fn print_gate_usage() {
    println!("S3A GATE DIRECTORY & RELAY CLIENT");
    println!();
    println!("USAGE:");
    println!("  s3a-cli gate register <name> --directory <url> [OPTIONS]");
    println!("  s3a-cli gate resolve <name> --directory <url>");
    println!();
    println!("OPTIONS:");
    println!("  --directory <url>     Gate directory URL (e.g. https://34.74.163.88.sslip.io/gate/)");
    println!("  --keystore <path>     Path to encrypted FileKeyStore (default: ~/.s3a/vault/person.keystore)");
    println!("  --vault <path>        Path to .vault.s3a archive (default: ~/.s3a/vault/person.vault.s3a)");
    println!("  --passphrase <pass>   Passphrase for Argon2id key decryption (or via prompt/env)");
    println!("  --endpoints <csv>     Comma-separated reachable endpoint URIs");
}
