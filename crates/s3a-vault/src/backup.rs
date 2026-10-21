//! Encrypted vault disaster recovery backup and restore containers (§6.3).

use argon2::Argon2;
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};
use rand_core::{OsRng, RngCore};
use std::fs;
use std::path::Path;
use zeroize::Zeroize;

use crate::errors::VaultError;

const BACKUP_MAGIC: &[u8; 8] = b"S3ABAK\x01\x00";

/// Export a full vault archive to an encrypted backup container (§6.3).
pub fn create_backup(
    vault_path: impl AsRef<Path>,
    backup_path: impl AsRef<Path>,
    passphrase: &str,
) -> Result<(), VaultError> {
    let raw_vault = fs::read(vault_path)?;

    let mut salt = [0u8; 16];
    OsRng.fill_bytes(&mut salt);

    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);

    let mut derived_key = [0u8; 32];
    let argon2 = Argon2::default();
    argon2
        .hash_password_into(passphrase.as_bytes(), &salt, &mut derived_key)
        .map_err(|e| VaultError::KeyStoreError(format!("Argon2 KDF failed: {}", e)))?;

    let cipher = ChaCha20Poly1305::new_from_slice(&derived_key)
        .map_err(|e| VaultError::KeyStoreError(format!("Cipher init failed: {}", e)))?;
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, raw_vault.as_ref())
        .map_err(|e| VaultError::KeyStoreError(format!("Encryption failed: {}", e)))?;

    derived_key.zeroize();

    let mut out_bytes = Vec::with_capacity(8 + 16 + 12 + 8 + ciphertext.len());
    out_bytes.extend_from_slice(BACKUP_MAGIC);
    out_bytes.extend_from_slice(&salt);
    out_bytes.extend_from_slice(&nonce_bytes);
    out_bytes.extend_from_slice(&(ciphertext.len() as u64).to_le_bytes());
    out_bytes.extend_from_slice(&ciphertext);

    if let Some(parent) = backup_path.as_ref().parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(backup_path, out_bytes)?;

    Ok(())
}

/// Restore an encrypted backup container into an S3A vault archive (§6.3).
pub fn restore_backup(
    backup_path: impl AsRef<Path>,
    restore_path: impl AsRef<Path>,
    passphrase: &str,
) -> Result<(), VaultError> {
    let backup_data = fs::read(backup_path)?;

    if backup_data.len() < 8 + 16 + 12 + 8 || &backup_data[0..8] != BACKUP_MAGIC {
        return Err(VaultError::CorruptedData("Invalid backup container format"));
    }

    let salt = &backup_data[8..24];
    let nonce_bytes = &backup_data[24..36];
    let len_bytes: [u8; 8] = backup_data[36..44].try_into().unwrap();
    let ciphertext_len = u64::from_le_bytes(len_bytes) as usize;

    let ciphertext = &backup_data[44..44 + ciphertext_len];

    let mut derived_key = [0u8; 32];
    let argon2 = Argon2::default();
    argon2
        .hash_password_into(passphrase.as_bytes(), salt, &mut derived_key)
        .map_err(|e| VaultError::KeyStoreError(format!("Argon2 KDF failed: {}", e)))?;

    let cipher = ChaCha20Poly1305::new_from_slice(&derived_key)
        .map_err(|e| VaultError::KeyStoreError(format!("Cipher init failed: {}", e)))?;
    let nonce = Nonce::from_slice(nonce_bytes);

    let decrypted = cipher.decrypt(nonce, ciphertext).map_err(|_| {
        VaultError::KeyStoreError(
            "Backup decryption failed (invalid passphrase or corrupted container)".into(),
        )
    })?;

    derived_key.zeroize();

    if let Some(parent) = restore_path.as_ref().parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(restore_path, decrypted)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backup_restore_roundtrip() {
        let temp_dir = tempfile::tempdir().unwrap();
        let src_vault = temp_dir.path().join("source.vault.s3a");
        let backup_file = temp_dir.path().join("vault.backup");
        let restored_vault = temp_dir.path().join("restored.vault.s3a");

        let original_data =
            b"S3A2\x01\x00Mock Vault Archive Content For Testing Roundtrip 1234567890";
        fs::write(&src_vault, original_data).unwrap();

        let pass = "vault-backup-master-phrase";

        // Create backup
        create_backup(&src_vault, &backup_file, pass).unwrap();

        // Backup file must not match plaintext
        let backup_bytes = fs::read(&backup_file).unwrap();
        assert!(backup_bytes.starts_with(BACKUP_MAGIC));
        assert_ne!(backup_bytes[44..], original_data[..]);

        // Wrong passphrase fails restore
        assert!(restore_backup(&backup_file, &restored_vault, "wrong-pass").is_err());

        // Correct passphrase restores identically
        restore_backup(&backup_file, &restored_vault, pass).unwrap();
        let restored_data = fs::read(&restored_vault).unwrap();
        assert_eq!(restored_data, original_data);
    }
}
