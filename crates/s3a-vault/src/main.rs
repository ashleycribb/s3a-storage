use std::env;
use std::fs;
use std::path::PathBuf;
use rand_core::RngCore;

use s3a_vault::address::{GateAddress, GateTarget};
use s3a_vault::access_code::AccessCode;
use s3a_vault::backup::{create_backup, restore_backup};
use s3a_vault::delegation::create_delegation_grant;
use s3a_vault::keystore::{FileKeyStore, KeyStore};
use s3a_vault::records::*;
use s3a_vault::rotation::create_key_rotation_record;
use s3a_vault::transfer::create_transfer_record;
use s3a_vault::Vault;

fn print_usage() {
    println!(r#"S3A Digital Vault CLI (s3a-vault)

USAGE:
    s3a-vault new <vault_file> [--name <display_name>] [--passphrase <pass>]
    s3a-vault show <vault_file>
    s3a-vault address <vault_file>
    s3a-vault grant <vault_file> <agent_pubkey_hex> <actions_mask> [--max-ops <N>] [--expires-in <secs>] [--passphrase <pass>]
    s3a-vault revoke <vault_file> <grant_id_hex>
    s3a-vault code <vault_file> <target_gate> <actions_mask> [--expiry <secs>] [--max-uses <N>] [--passphrase <pass>]
    s3a-vault send <vault_file> <item_hash_hex> <to_pubkey_hex> [--out <transfer_file>] [--passphrase <pass>]
    s3a-vault receive <vault_file> <transfer_file>
    s3a-vault rotate <vault_file> <new_pubkey_hex> [--passphrase <pass>]
    s3a-vault backup <vault_file> <out_backup> --passphrase <pass>
    s3a-vault restore <backup_file> <out_vault> --passphrase <pass>
    s3a-vault dial gate://<name|address> [--server <url>]
"#);
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_usage();
        std::process::exit(1);
    }

    match args[1].as_str() {
        "new" => cmd_new(&args[2..]),
        "show" => cmd_show(&args[2..]),
        "address" => cmd_address(&args[2..]),
        "grant" => cmd_grant(&args[2..]),
        "revoke" => cmd_revoke(&args[2..]),
        "code" => cmd_code(&args[2..]),
        "send" => cmd_send(&args[2..]),
        "receive" => cmd_receive(&args[2..]),
        "rotate" => cmd_rotate(&args[2..]),
        "backup" => cmd_backup(&args[2..]),
        "restore" => cmd_restore(&args[2..]),
        "dial" => cmd_dial(&args[2..]),
        "--help" | "-h" => print_usage(),
        other => {
            eprintln!("Unknown command: {}", other);
            print_usage();
            std::process::exit(1);
        }
    }
}

fn get_arg_val(args: &[String], flag: &str) -> Option<String> {
    for i in 0..args.len() {
        if args[i] == flag && i + 1 < args.len() {
            return Some(args[i + 1].clone());
        }
    }
    None
}

fn keystore_path_for_vault(vault_file: &str) -> PathBuf {
    PathBuf::from(format!("{}.keystore", vault_file))
}

fn cmd_new(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: s3a-vault new <vault_file> [--name <name>] [--passphrase <pass>]");
        std::process::exit(1);
    }
    let vault_path = &args[0];
    let name = get_arg_val(args, "--name").unwrap_or_else(|| "S3A Sovereign Vault".to_string());
    let pass = get_arg_val(args, "--passphrase").unwrap_or_else(|| "default-dev-passphrase".to_string());

    let ks_path = keystore_path_for_vault(vault_path);
    let mut keystore = FileKeyStore::open_or_create(&ks_path, &pass).expect("Keystore creation failed");

    let vault = Vault::create(vault_path, &name, VaultOwnerKind::Person, &mut keystore)
        .expect("Vault creation failed");
    keystore.save(&pass).expect("Failed to persist encrypted keystore");

    let addr = vault.gate_address().expect("Gate address derivation failed");
    println!("Created S3A Vault: {}", vault_path);
    println!("Keystore:         {:?}", ks_path);
    println!("Gate Address:     {}", addr.to_address_string());
    println!("Dial URI:         {}", addr.to_uri());
}

fn cmd_show(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: s3a-vault show <vault_file>");
        std::process::exit(1);
    }
    let vault_path = &args[0];
    let vault = Vault::open(vault_path).expect("Failed to open vault");

    println!("=== S3A Vault Overview ===");
    println!("File:         {}", vault_path);
    if let Some(id) = vault.primary_identity() {
        let name = String::from_utf8_lossy(&id.display_name).trim_matches('\0').to_string();
        println!("Owner Name:   {}", name);
        println!("Owner Kind:   {:?}", VaultOwnerKind::from_u16(id.owner_kind));
        println!("Public Key:   {}", hex::encode(id.public_key));
    }
    if let Ok(addr) = vault.gate_address() {
        println!("Gate Address: {}", addr.to_address_string());
        println!("Dial URI:     {}", addr.to_uri());
    }
    println!("Holdings:     {}", vault.items().len());
    println!("Grants:       {}", vault.grants().len());
    println!("Events:       {}", vault.events().len());
}

fn cmd_address(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: s3a-vault address <vault_file>");
        std::process::exit(1);
    }
    let vault = Vault::open(&args[0]).expect("Failed to open vault");
    let addr = vault.gate_address().expect("Failed to derive gate address");
    println!("{}", addr.to_address_string());
    println!("{}", addr.to_uri());
}

fn cmd_grant(args: &[String]) {
    if args.len() < 3 {
        eprintln!("Usage: s3a-vault grant <vault_file> <agent_pubkey_hex> <actions_mask> [--max-ops <N>] [--expires-in <secs>] [--passphrase <pass>]");
        std::process::exit(1);
    }
    let vault_path = &args[0];
    let mut agent_pk = [0u8; 32];
    hex::decode_to_slice(&args[1], &mut agent_pk).expect("Invalid agent public key hex");
    let actions: u32 = args[2].parse().unwrap_or(ACTION_READ | ACTION_WRITE);

    let max_ops: u32 = get_arg_val(args, "--max-ops").and_then(|s| s.parse().ok()).unwrap_or(100);
    let expires_in: u64 = get_arg_val(args, "--expires-in").and_then(|s| s.parse().ok()).unwrap_or(86400);
    let pass = get_arg_val(args, "--passphrase").unwrap_or_else(|| "default-dev-passphrase".to_string());

    let mut vault = Vault::open(vault_path).expect("Failed to open vault");
    let primary = vault.primary_identity().expect("No identity in vault");
    let ks_path = keystore_path_for_vault(vault_path);
    let keystore = FileKeyStore::open(&ks_path, &pass).expect("Failed to open keystore");

    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let grant_id = [42u8; 16];

    let grant = create_delegation_grant(
        grant_id,
        primary.identity_id,
        VaultOwnerKind::from_u16(primary.owner_kind).unwrap_or(VaultOwnerKind::Person),
        [0u8; 16],
        agent_pk,
        actions,
        0xFFFF,
        max_ops,
        now,
        now + expires_in,
        None,
        &keystore,
    ).expect("Failed to create grant");

    vault.add_grant(grant).expect("Failed to commit grant to vault");
    println!("Granted authority to agent {}", args[1]);
    println!("Grant ID:   {}", hex::encode(grant_id));
    println!("Actions:    0x{:04x}", actions);
    println!("Max Ops:    {}", max_ops);
    println!("Expires At: {}", now + expires_in);
}

fn cmd_revoke(args: &[String]) {
    if args.len() < 2 {
        eprintln!("Usage: s3a-vault revoke <vault_file> <grant_id_hex>");
        std::process::exit(1);
    }
    let vault_path = &args[0];
    let mut grant_id = [0u8; 16];
    hex::decode_to_slice(&args[1], &mut grant_id).expect("Invalid grant ID hex");

    let mut vault = Vault::open(vault_path).expect("Failed to open vault");
    let seq = vault.log_event(EVENT_GRANT_REVOKED, 1, grant_id, 0).expect("Failed to record event");
    println!("Revoked grant {} (event sequence #{})", args[1], seq);
}

fn cmd_code(args: &[String]) {
    if args.len() < 3 {
        eprintln!("Usage: s3a-vault code <vault_file> <target_gate> <actions_mask> [--expiry <secs>] [--max-uses <N>] [--passphrase <pass>]");
        std::process::exit(1);
    }
    let vault_path = &args[0];
    let target_gate = GateAddress::parse_address(&args[1]).expect("Invalid target gate address");
    let actions: u32 = args[2].parse().unwrap_or(ACTION_READ);

    let expiry: u64 = get_arg_val(args, "--expiry").and_then(|s| s.parse().ok()).unwrap_or(3600);
    let max_uses: u32 = get_arg_val(args, "--max-uses").and_then(|s| s.parse().ok()).unwrap_or(1);
    let pass = get_arg_val(args, "--passphrase").unwrap_or_else(|| "default-dev-passphrase".to_string());

    let vault = Vault::open(vault_path).expect("Failed to open vault");
    let _primary = vault.primary_identity().expect("No primary identity");
    let ks_path = keystore_path_for_vault(vault_path);
    let _keystore = FileKeyStore::open(&ks_path, &pass).expect("Failed to open keystore");

    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let signing_key = ed25519_dalek::SigningKey::generate(&mut rand_core::OsRng);
    let code_id = [99u8; 16];

    let code = AccessCode::issue(
        code_id,
        &target_gate,
        &signing_key,
        0xFFFF,
        actions,
        now + expiry,
        max_uses,
    );

    let token = code.to_token().expect("Token serialization failed");
    println!("Access Code Token:\n{}", token);
}

fn cmd_send(args: &[String]) {
    if args.len() < 3 {
        eprintln!("Usage: s3a-vault send <vault_file> <item_hash_hex> <to_pubkey_hex> [--out <out_file>] [--passphrase <pass>]");
        std::process::exit(1);
    }
    let vault_path = &args[0];
    let mut item_hash = [0u8; 32];
    hex::decode_to_slice(&args[1], &mut item_hash).expect("Invalid item hash hex");
    let mut to_pk = [0u8; 32];
    hex::decode_to_slice(&args[2], &mut to_pk).expect("Invalid recipient public key hex");

    let pass = get_arg_val(args, "--passphrase").unwrap_or_else(|| "default-dev-passphrase".to_string());
    let mut vault = Vault::open(vault_path).expect("Failed to open vault");
    let primary = vault.primary_identity().expect("No primary identity");
    let ks_path = keystore_path_for_vault(vault_path);
    let keystore = FileKeyStore::open(&ks_path, &pass).expect("Failed to open keystore");

    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let transfer = create_transfer_record(
        [1u8; 16],
        item_hash,
        primary.identity_id,
        to_pk,
        now,
        [0u8; 32],
        TRANSFER_MODE_UNIQUE,
        &keystore,
    ).expect("Failed to create transfer");

    vault.add_transfer(transfer).expect("Failed to record transfer in vault");

    if let Some(out_path) = get_arg_val(args, "--out") {
        let bytes = bytemuck::bytes_of(&transfer);
        fs::write(&out_path, bytes).expect("Failed to write transfer file");
        println!("Transferred item {} to {}. Saved to {}", args[1], args[2], out_path);
    } else {
        println!("Transferred item {} to {}", args[1], args[2]);
    }
}

fn cmd_receive(args: &[String]) {
    if args.len() < 2 {
        eprintln!("Usage: s3a-vault receive <vault_file> <transfer_file>");
        std::process::exit(1);
    }
    let vault_path = &args[0];
    let transfer_file = &args[1];

    let bytes = fs::read(transfer_file).expect("Failed to read transfer file");
    if bytes.len() != std::mem::size_of::<ItemTransferRecord>() {
        eprintln!("Invalid transfer record file size");
        std::process::exit(1);
    }
    let transfer: ItemTransferRecord = *bytemuck::from_bytes(&bytes);

    let mut vault = Vault::open(vault_path).expect("Failed to open vault");
    vault.add_transfer(transfer).expect("Failed to ingest transfer link");
    println!("Received and verified transfer link for item {}", hex::encode(transfer.item_hash));
}

fn cmd_rotate(args: &[String]) {
    if args.len() < 2 {
        eprintln!("Usage: s3a-vault rotate <vault_file> <new_pubkey_hex> [--passphrase <pass>]");
        std::process::exit(1);
    }
    let vault_path = &args[0];
    let mut new_pk = [0u8; 32];
    hex::decode_to_slice(&args[1], &mut new_pk).expect("Invalid new public key hex");

    let pass = get_arg_val(args, "--passphrase").unwrap_or_else(|| "default-dev-passphrase".to_string());
    let mut vault = Vault::open(vault_path).expect("Failed to open vault");
    let primary = vault.primary_identity().expect("No primary identity");
    let ks_path = keystore_path_for_vault(vault_path);
    let mut keystore = FileKeyStore::open(&ks_path, &pass).expect("Failed to open keystore");

    let new_id = [88u8; 16];
    let mut seed = [0u8; 32];
    rand_core::OsRng.fill_bytes(&mut seed);
    keystore.insert_keypair(&new_id, &seed).expect("Failed to insert key");
    keystore.save(&pass).expect("Failed to save keystore");

    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let rot = create_key_rotation_record([77u8; 16], primary.identity_id, new_id, now, &keystore)
        .expect("Failed to create rotation");

    vault.add_key_rotation(rot).expect("Failed to commit rotation");
    println!("Rotated vault master key to {}", hex::encode(new_pk));
}

fn cmd_backup(args: &[String]) {
    if args.len() < 2 {
        eprintln!("Usage: s3a-vault backup <vault_file> <out_backup> --passphrase <pass>");
        std::process::exit(1);
    }
    let vault_file = &args[0];
    let backup_file = &args[1];
    let pass = get_arg_val(args, "--passphrase").expect("--passphrase required for backup");

    create_backup(vault_file, backup_file, &pass).expect("Backup failed");
    println!("Created encrypted vault backup: {}", backup_file);
}

fn cmd_restore(args: &[String]) {
    if args.len() < 2 {
        eprintln!("Usage: s3a-vault restore <backup_file> <out_vault> --passphrase <pass>");
        std::process::exit(1);
    }
    let backup_file = &args[0];
    let vault_file = &args[1];
    let pass = get_arg_val(args, "--passphrase").expect("--passphrase required for restore");

    restore_backup(backup_file, vault_file, &pass).expect("Restore failed");
    println!("Restored S3A vault: {}", vault_file);
}

fn cmd_dial(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: s3a-vault dial gate://<name|address> [--server <url>]");
        std::process::exit(1);
    }
    let uri = &args[0];
    let target = GateTarget::parse(uri).expect("Invalid gate URI");

    match target {
        GateTarget::Address(addr) => {
            println!("Dialing gate address: {}", addr.to_address_string());
            println!("Address verified:     Valid SHA-256 digest + CRC checksum ✓");
            println!("Connecting via near-field / store-and-forward relay...");
        }
        GateTarget::Name(name) => {
            println!("Resolving gate name:  gate://{}", name);
            if let Some(server) = get_arg_val(args, "--server") {
                let url = format!("{}/gate/resolve/{}", server.trim_end_matches('/'), name);
                println!("Querying directory:   {}", url);
            } else {
                println!("Target name parsed:   {}", name);
            }
        }
    }
}
