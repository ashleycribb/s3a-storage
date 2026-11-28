//! Prompt 015 fail-first tests: per-vault Ed25519 keypair, signing at seal
//! time, and verification of that signature at verify time.

use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

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
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ws_target = manifest.join("../../target");
    let root = if ws_target.is_dir() {
        ws_target
    } else {
        std::env::temp_dir()
    };
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = root.join(format!("s3a_015_{}_{}_{}", name, std::process::id(), nanos));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn run_s3a(home_dir: &Path, args: &[&str]) -> Output {
    Command::new(cli_bin())
        .args(args)
        .env("HOME", home_dir)
        .env("USERPROFILE", home_dir)
        .output()
        .expect("failed to execute s3a binary")
}

fn parse_verify_json(out: &Output) -> Value {
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout
        .lines()
        .rev()
        .find(|l| l.trim_start().starts_with('{'))
        .unwrap_or_else(|| {
            panic!(
                "expected JSON line in stdout, got:\nstdout: {}\nstderr: {}",
                stdout,
                String::from_utf8_lossy(&out.stderr)
            )
        });
    serde_json::from_str(line).unwrap_or_else(|e| panic!("invalid JSON '{line}': {e}"))
}

fn parse_key_hex(path: &Path) -> [u8; 32] {
    let raw = fs::read_to_string(path).unwrap();
    let bytes = hex::decode(raw.trim()).expect("key file must contain hex");
    bytes.try_into().expect("key must be 32 bytes")
}

#[test]
fn test_keygen_writes_private_key_0600() {
    let tmp = unique_tmp("keygen");
    let home = tmp.join("home");
    fs::create_dir_all(&home).unwrap();

    let out = run_s3a(&home, &["keygen", "--name", "test"]);
    assert!(
        out.status.success(),
        "keygen failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let keys_dir = home.join(".s3a").join("keys");
    let priv_path = keys_dir.join("test.key");
    let pub_path = keys_dir.join("test.pub");
    assert!(priv_path.exists(), "missing private key at {:?}", priv_path);
    assert!(pub_path.exists(), "missing public key at {:?}", pub_path);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let dir_mode = fs::metadata(&keys_dir).unwrap().permissions().mode() & 0o777;
        let priv_mode = fs::metadata(&priv_path).unwrap().permissions().mode() & 0o777;
        let pub_mode = fs::metadata(&pub_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700, "keys dir mode must be 0700");
        assert_eq!(priv_mode, 0o600, "private key mode must be 0600");
        assert_eq!(pub_mode, 0o644, "public key mode must be 0644");
    }

    let priv_bytes = parse_key_hex(&priv_path);
    let pub_bytes = parse_key_hex(&pub_path);
    let signing_key = SigningKey::from_bytes(&priv_bytes);
    let verifying_key = VerifyingKey::from_bytes(&pub_bytes).unwrap();
    let msg = b"s3a-015-keygen-check";
    let sig = signing_key.sign(msg);
    verifying_key
        .verify(msg, &sig)
        .expect("pub file must verify signature made with private key");
}

#[test]
fn test_signed_seal_round_trips() {
    let tmp = unique_tmp("signed_rt");
    let home = tmp.join("home");
    fs::create_dir_all(&home).unwrap();

    let key_out = tmp.join("vault.key");
    let pub_out = tmp.join("vault.pub");
    let kg = run_s3a(
        &home,
        &[
            "keygen",
            "--name",
            "vault",
            "--out",
            key_out.to_str().unwrap(),
        ],
    );
    assert!(kg.status.success(), "keygen failed");
    let pub_hex = fs::read_to_string(&pub_out).unwrap().trim().to_string();

    let vault = tmp.join("my.s3a");
    let init = run_s3a(&home, &["init", vault.to_str().unwrap(), "--key", &pub_hex]);
    assert!(
        init.status.success(),
        "init --key failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );

    let put = run_s3a(
        &home,
        &["put", vault.to_str().unwrap(), "greeting", "hello world"],
    );
    assert!(put.status.success(), "put failed");

    let sealed = tmp.join("my.sealed.s3a");
    let seal = run_s3a(
        &home,
        &[
            "seal",
            vault.to_str().unwrap(),
            sealed.to_str().unwrap(),
            "--key",
            key_out.to_str().unwrap(),
        ],
    );
    assert!(
        seal.status.success(),
        "seal --key failed: {}",
        String::from_utf8_lossy(&seal.stderr)
    );

    let verify = run_s3a(&home, &["verify", sealed.to_str().unwrap(), "--json"]);
    assert!(verify.status.success(), "verify failed");
    let v = parse_verify_json(&verify);
    assert_eq!(v["ok"], true);
    assert_eq!(v["signed"], true);
    assert_eq!(v["signature_valid"], true);
    assert_eq!(v["signer"], pub_hex);

    let info = run_s3a(&home, &["info", "--vault", sealed.to_str().unwrap()]);
    assert!(info.status.success(), "info failed");
    let info_stdout = String::from_utf8_lossy(&info.stdout);
    assert!(
        info_stdout.contains(&pub_hex),
        "info output must show signer pubkey '{}', got:\n{}",
        pub_hex,
        info_stdout
    );
}

#[test]
fn test_tampered_sealed_file_fails_signature() {
    let tmp = unique_tmp("tamper_payload");
    let home = tmp.join("home");
    fs::create_dir_all(&home).unwrap();

    let key_out = tmp.join("k.key");
    let pub_out = tmp.join("k.pub");
    assert!(
        run_s3a(&home, &["keygen", "--out", key_out.to_str().unwrap()])
            .status
            .success()
    );
    let pub_hex = fs::read_to_string(&pub_out).unwrap().trim().to_string();

    let vault = tmp.join("v.s3a");
    assert!(
        run_s3a(&home, &["init", vault.to_str().unwrap(), "--key", &pub_hex])
            .status
            .success()
    );
    assert!(run_s3a(
        &home,
        &["put", vault.to_str().unwrap(), "k", "payload-bytes"]
    )
    .status
    .success());

    let sealed = tmp.join("v.sealed.s3a");
    assert!(run_s3a(
        &home,
        &[
            "seal",
            vault.to_str().unwrap(),
            sealed.to_str().unwrap(),
            "--key",
            key_out.to_str().unwrap(),
        ],
    )
    .status
    .success());

    let mut bytes = fs::read(&sealed).unwrap();
    // Flip one byte inside the first tile payload (past 128B file header + 512B tile header).
    let flip_idx = 128 + 512 + 4;
    bytes[flip_idx] ^= 0xFF;
    fs::write(&sealed, &bytes).unwrap();

    let verify = run_s3a(&home, &["verify", sealed.to_str().unwrap(), "--json"]);
    let v = parse_verify_json(&verify);
    assert_eq!(v["signed"], true);
    assert_eq!(v["signature_valid"], false);
}

#[test]
fn test_tampered_signer_pubkey_fails_signature() {
    let tmp = unique_tmp("tamper_signer");
    let home = tmp.join("home");
    fs::create_dir_all(&home).unwrap();

    let key1 = tmp.join("k1.key");
    let pub1 = tmp.join("k1.pub");
    let key2 = tmp.join("k2.key");
    let pub2 = tmp.join("k2.pub");
    assert!(run_s3a(&home, &["keygen", "--out", key1.to_str().unwrap()])
        .status
        .success());
    assert!(run_s3a(&home, &["keygen", "--out", key2.to_str().unwrap()])
        .status
        .success());
    let pub1_bytes = parse_key_hex(&pub1);
    let pub2_bytes = parse_key_hex(&pub2);
    let pub1_hex = hex::encode(pub1_bytes);

    let vault = tmp.join("v.s3a");
    assert!(run_s3a(
        &home,
        &["init", vault.to_str().unwrap(), "--key", &pub1_hex]
    )
    .status
    .success());
    assert!(
        run_s3a(&home, &["put", vault.to_str().unwrap(), "k", "value"])
            .status
            .success()
    );

    let sealed = tmp.join("v.sealed.s3a");
    assert!(run_s3a(
        &home,
        &[
            "seal",
            vault.to_str().unwrap(),
            sealed.to_str().unwrap(),
            "--key",
            key1.to_str().unwrap(),
        ],
    )
    .status
    .success());

    let mut bytes = fs::read(&sealed).unwrap();
    // Replace the embedded signer public key in the seal footer (near the end of the file).
    let tail_start = bytes.len().saturating_sub(512);
    let pos = bytes[tail_start..]
        .windows(32)
        .rposition(|w| w == pub1_bytes)
        .expect("signer pubkey must appear in seal footer");
    bytes[tail_start + pos..tail_start + pos + 32].copy_from_slice(&pub2_bytes);
    fs::write(&sealed, &bytes).unwrap();

    let verify = run_s3a(&home, &["verify", sealed.to_str().unwrap(), "--json"]);
    let v = parse_verify_json(&verify);
    assert_eq!(v["signed"], true);
    assert_eq!(v["signature_valid"], false);
}

#[test]
fn test_unsigned_seal_still_verifies() {
    let tmp = unique_tmp("unsigned");
    let home = tmp.join("home");
    fs::create_dir_all(&home).unwrap();

    let vault = tmp.join("v.s3a");
    assert!(run_s3a(&home, &["init", vault.to_str().unwrap()])
        .status
        .success());
    assert!(run_s3a(&home, &["put", vault.to_str().unwrap(), "a", "b"])
        .status
        .success());

    let sealed = tmp.join("v.sealed.s3a");
    assert!(run_s3a(
        &home,
        &["seal", vault.to_str().unwrap(), sealed.to_str().unwrap()],
    )
    .status
    .success());

    let verify = run_s3a(&home, &["verify", sealed.to_str().unwrap(), "--json"]);
    assert!(verify.status.success());
    let v = parse_verify_json(&verify);
    assert_eq!(v["ok"], true);
    assert_eq!(v["status"], "verified");
    assert_eq!(v["signed"], false);

    // Pre-015 sealed vector must also verify with signed: false.
    let pre015 = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance/vectors-1.6/files/sealed_ok.s3a");
    let verify_old = run_s3a(&home, &["verify", pre015.to_str().unwrap(), "--json"]);
    assert!(verify_old.status.success());
    let v_old = parse_verify_json(&verify_old);
    assert_eq!(v_old["ok"], true);
    assert_eq!(v_old["signed"], false);
}

#[test]
fn test_seal_without_key_on_keyed_vault_fails_closed() {
    let tmp = unique_tmp("missing_key_on_keyed");
    let home = tmp.join("home");
    fs::create_dir_all(&home).unwrap();

    let key_out = tmp.join("k.key");
    let pub_out = tmp.join("k.pub");
    assert!(
        run_s3a(&home, &["keygen", "--out", key_out.to_str().unwrap()])
            .status
            .success()
    );
    let pub_hex = fs::read_to_string(&pub_out).unwrap().trim().to_string();

    let vault = tmp.join("v.s3a");
    assert!(
        run_s3a(&home, &["init", vault.to_str().unwrap(), "--key", &pub_hex])
            .status
            .success()
    );

    let sealed_fail = tmp.join("fail.sealed.s3a");
    let out_fail = run_s3a(
        &home,
        &[
            "seal",
            vault.to_str().unwrap(),
            sealed_fail.to_str().unwrap(),
        ],
    );
    assert!(!out_fail.status.success(), "seal without --key must fail");
    assert!(
        !sealed_fail.exists(),
        "failed seal must not write output file"
    );
    let err_text = format!(
        "{}{}",
        String::from_utf8_lossy(&out_fail.stdout),
        String::from_utf8_lossy(&out_fail.stderr)
    );
    assert!(
        err_text.contains(&pub_hex),
        "error must name the pinned key '{}', got: {}",
        pub_hex,
        err_text
    );

    let sealed_nosign = tmp.join("nosign.sealed.s3a");
    let out_nosign = run_s3a(
        &home,
        &[
            "seal",
            vault.to_str().unwrap(),
            sealed_nosign.to_str().unwrap(),
            "--no-sign",
        ],
    );
    assert!(
        out_nosign.status.success(),
        "seal --no-sign must succeed: {}",
        String::from_utf8_lossy(&out_nosign.stderr)
    );
    let verify = run_s3a(
        &home,
        &["verify", sealed_nosign.to_str().unwrap(), "--json"],
    );
    assert!(verify.status.success());
    let v = parse_verify_json(&verify);
    assert_eq!(v["signed"], false);
}

#[test]
fn test_seal_with_missing_or_wrong_key_fails_closed() {
    let tmp = unique_tmp("wrong_key");
    let home = tmp.join("home");
    fs::create_dir_all(&home).unwrap();

    let key1 = tmp.join("k1.key");
    let pub1 = tmp.join("k1.pub");
    let key2 = tmp.join("k2.key");
    assert!(run_s3a(&home, &["keygen", "--out", key1.to_str().unwrap()])
        .status
        .success());
    assert!(run_s3a(&home, &["keygen", "--out", key2.to_str().unwrap()])
        .status
        .success());
    let pub1_hex = fs::read_to_string(&pub1).unwrap().trim().to_string();

    let vault = tmp.join("v.s3a");
    assert!(run_s3a(
        &home,
        &["init", vault.to_str().unwrap(), "--key", &pub1_hex]
    )
    .status
    .success());

    // 1. Nonexistent key file
    let out_missing = tmp.join("missing.sealed.s3a");
    let r1 = run_s3a(
        &home,
        &[
            "seal",
            vault.to_str().unwrap(),
            out_missing.to_str().unwrap(),
            "--key",
            tmp.join("does_not_exist.key").to_str().unwrap(),
        ],
    );
    assert!(!r1.status.success());
    assert!(!out_missing.exists());

    // 2. Wrong key for pinned vault
    let out_wrong = tmp.join("wrong.sealed.s3a");
    let r2 = run_s3a(
        &home,
        &[
            "seal",
            vault.to_str().unwrap(),
            out_wrong.to_str().unwrap(),
            "--key",
            key2.to_str().unwrap(),
        ],
    );
    assert!(!r2.status.success());
    assert!(!out_wrong.exists());

    // 3. Group/other-readable private key (0644 on unix)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&key1, fs::Permissions::from_mode(0o644)).unwrap();
        let out_perm = tmp.join("perm.sealed.s3a");
        let r3 = run_s3a(
            &home,
            &[
                "seal",
                vault.to_str().unwrap(),
                out_perm.to_str().unwrap(),
                "--key",
                key1.to_str().unwrap(),
            ],
        );
        assert!(!r3.status.success(), "must reject 0644 private key");
        assert!(!out_perm.exists());
    }
}

#[test]
fn test_signed_pack_extract_round_trips() {
    let tmp = unique_tmp("pack_extract");
    let home = tmp.join("home");
    fs::create_dir_all(&home).unwrap();

    let key_out = tmp.join("p.key");
    let pub_out = tmp.join("p.pub");
    assert!(
        run_s3a(&home, &["keygen", "--out", key_out.to_str().unwrap()])
            .status
            .success()
    );
    let pub_hex = fs::read_to_string(&pub_out).unwrap().trim().to_string();

    let src_dir = tmp.join("src_tree");
    fs::create_dir_all(src_dir.join("sub")).unwrap();
    fs::write(src_dir.join("hello.txt"), b"hello s3a 015\n").unwrap();
    fs::write(src_dir.join("sub/data.bin"), &[1u8, 2, 3, 4, 5]).unwrap();

    let vault = tmp.join("tree.s3a");
    assert!(
        run_s3a(&home, &["init", vault.to_str().unwrap(), "--key", &pub_hex])
            .status
            .success()
    );

    let pack = run_s3a(
        &home,
        &["pack", src_dir.to_str().unwrap(), vault.to_str().unwrap()],
    );
    assert!(
        pack.status.success(),
        "pack failed: {}",
        String::from_utf8_lossy(&pack.stderr)
    );

    let sealed = tmp.join("tree.sealed.s3a");
    let seal = run_s3a(
        &home,
        &[
            "seal",
            vault.to_str().unwrap(),
            sealed.to_str().unwrap(),
            "--key",
            key_out.to_str().unwrap(),
        ],
    );
    assert!(seal.status.success(), "seal failed");

    let verify = run_s3a(&home, &["verify", sealed.to_str().unwrap(), "--json"]);
    assert!(verify.status.success());
    let v = parse_verify_json(&verify);
    assert_eq!(v["ok"], true);
    assert_eq!(v["signed"], true);
    assert_eq!(v["signature_valid"], true);
    assert_eq!(v["signer"], pub_hex);

    let out_dir = tmp.join("extracted");
    let extract = run_s3a(
        &home,
        &[
            "extract",
            sealed.to_str().unwrap(),
            out_dir.to_str().unwrap(),
        ],
    );
    assert!(
        extract.status.success(),
        "extract failed: {}",
        String::from_utf8_lossy(&extract.stderr)
    );
    assert_eq!(
        fs::read(out_dir.join("hello.txt")).unwrap(),
        b"hello s3a 015\n"
    );
    assert_eq!(
        fs::read(out_dir.join("sub/data.bin")).unwrap(),
        &[1u8, 2, 3, 4, 5]
    );
}
