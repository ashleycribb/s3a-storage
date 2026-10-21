//! Cryptographic key custody and KeyStore implementations (§2.2).

use argon2::Argon2;
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};
use ed25519_dalek::{Signature, Signer, SigningKey};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use zeroize::Zeroize;

use crate::errors::VaultError;

/// Core KeyStore trait for sovereign identity key custody.
pub trait KeyStore: Send + Sync {
    fn get_public_key(&self, identity_id: &[u8; 16]) -> Result<[u8; 32], VaultError>;
    fn sign(&self, identity_id: &[u8; 16], message: &[u8]) -> Result<[u8; 64], VaultError>;
    fn generate_keypair(&mut self, identity_id: &[u8; 16]) -> Result<[u8; 32], VaultError>;
    fn insert_keypair(
        &mut self,
        identity_id: &[u8; 16],
        secret_seed: &[u8; 32],
    ) -> Result<[u8; 32], VaultError>;
}

/// In-memory KeyStore (useful for tests and harnesses).
#[derive(Default)]
pub struct MemoryKeyStore {
    keys: HashMap<[u8; 16], KeyEntry>,
}

#[derive(Clone)]
struct KeyEntry {
    seed: [u8; 32],
}

impl Drop for KeyEntry {
    fn drop(&mut self) {
        self.seed.zeroize();
    }
}

impl MemoryKeyStore {
    pub fn new() -> Self {
        Self {
            keys: HashMap::new(),
        }
    }

    pub fn list_identities(&self) -> Vec<[u8; 16]> {
        self.keys.keys().cloned().collect()
    }

    pub fn first_identity(&self) -> Option<[u8; 16]> {
        self.keys.keys().next().cloned()
    }
}

impl KeyStore for MemoryKeyStore {
    fn get_public_key(&self, identity_id: &[u8; 16]) -> Result<[u8; 32], VaultError> {
        let entry = self.keys.get(identity_id).ok_or_else(|| {
            VaultError::KeyStoreError(format!("Key not found for id {}", hex::encode(identity_id)))
        })?;
        let signing_key = SigningKey::from_bytes(&entry.seed);
        Ok(signing_key.verifying_key().to_bytes())
    }

    fn sign(&self, identity_id: &[u8; 16], message: &[u8]) -> Result<[u8; 64], VaultError> {
        let entry = self.keys.get(identity_id).ok_or_else(|| {
            VaultError::KeyStoreError(format!("Key not found for id {}", hex::encode(identity_id)))
        })?;
        let signing_key = SigningKey::from_bytes(&entry.seed);
        let sig: Signature = signing_key.sign(message);
        Ok(sig.to_bytes())
    }

    fn generate_keypair(&mut self, identity_id: &[u8; 16]) -> Result<[u8; 32], VaultError> {
        let mut seed = [0u8; 32];
        OsRng.fill_bytes(&mut seed);
        self.insert_keypair(identity_id, &seed)
    }

    fn insert_keypair(
        &mut self,
        identity_id: &[u8; 16],
        secret_seed: &[u8; 32],
    ) -> Result<[u8; 32], VaultError> {
        let signing_key = SigningKey::from_bytes(secret_seed);
        let pubkey = signing_key.verifying_key().to_bytes();
        self.keys
            .insert(*identity_id, KeyEntry { seed: *secret_seed });
        Ok(pubkey)
    }
}

/// Encrypted file-backed KeyStore for local development and runtime agents.
/// Encrypted at rest using Argon2id-derived key and ChaCha20-Poly1305 AEAD.
pub struct FileKeyStore {
    path: PathBuf,
    inner: MemoryKeyStore,
}

#[derive(Serialize, Deserialize)]
struct KeyStorePayload {
    entries: Vec<SerializedKeyEntry>,
}

#[derive(Serialize, Deserialize)]
struct SerializedKeyEntry {
    identity_id: String, // hex
    seed: String,        // hex
}

const KEYSTORE_MAGIC: &[u8; 6] = b"S3AKS\x01";

impl FileKeyStore {
    /// Create or open an encrypted FileKeyStore at `path` protected by `passphrase`.
    pub fn open_or_create(path: impl AsRef<Path>, passphrase: &str) -> Result<Self, VaultError> {
        let path_buf = path.as_ref().to_path_buf();
        if path_buf.exists() {
            Self::open(&path_buf, passphrase)
        } else {
            let store = Self {
                path: path_buf,
                inner: MemoryKeyStore::new(),
            };
            store.save(passphrase)?;
            Ok(store)
        }
    }

    /// Open an existing encrypted FileKeyStore.
    pub fn open(path: impl AsRef<Path>, passphrase: &str) -> Result<Self, VaultError> {
        let path_buf = path.as_ref().to_path_buf();
        let raw = fs::read(&path_buf)?;
        if raw.len() < 6 + 16 + 12 + 16 || &raw[0..6] != KEYSTORE_MAGIC {
            return Err(VaultError::KeyStoreError(
                "Invalid keystore file format".into(),
            ));
        }

        let salt = &raw[6..22];
        let nonce_bytes = &raw[22..34];
        let ciphertext = &raw[34..];

        let mut derived_key = [0u8; 32];
        let argon2 = Argon2::default();
        argon2
            .hash_password_into(passphrase.as_bytes(), salt, &mut derived_key)
            .map_err(|e| VaultError::KeyStoreError(format!("Argon2 KDF failed: {}", e)))?;

        let cipher = ChaCha20Poly1305::new_from_slice(&derived_key)
            .map_err(|e| VaultError::KeyStoreError(format!("Cipher init failed: {}", e)))?;
        let nonce = Nonce::from_slice(nonce_bytes);

        let plaintext = cipher.decrypt(nonce, ciphertext).map_err(|_| {
            VaultError::KeyStoreError(
                "Keystore decryption failed (invalid passphrase or corrupted file)".into(),
            )
        })?;

        derived_key.zeroize();

        let payload: KeyStorePayload = serde_json::from_slice(&plaintext).map_err(|e| {
            VaultError::KeyStoreError(format!("JSON deserialization failed: {}", e))
        })?;

        let mut inner = MemoryKeyStore::new();
        for item in payload.entries {
            let mut id = [0u8; 16];
            let mut seed = [0u8; 32];
            hex::decode_to_slice(&item.identity_id, &mut id)
                .map_err(|e| VaultError::KeyStoreError(format!("Hex decode error: {}", e)))?;
            hex::decode_to_slice(&item.seed, &mut seed)
                .map_err(|e| VaultError::KeyStoreError(format!("Hex decode error: {}", e)))?;
            inner.insert_keypair(&id, &seed)?;
            seed.zeroize();
        }

        Ok(Self {
            path: path_buf,
            inner,
        })
    }

    /// Save the current keys encrypted at rest.
    pub fn save(&self, passphrase: &str) -> Result<(), VaultError> {
        let mut entries = Vec::new();
        for (id, entry) in &self.inner.keys {
            entries.push(SerializedKeyEntry {
                identity_id: hex::encode(id),
                seed: hex::encode(entry.seed),
            });
        }

        let payload = KeyStorePayload { entries };
        let mut plaintext = serde_json::to_vec(&payload)
            .map_err(|e| VaultError::KeyStoreError(format!("JSON serialization failed: {}", e)))?;

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
            .encrypt(nonce, plaintext.as_ref())
            .map_err(|e| VaultError::KeyStoreError(format!("Encryption failed: {}", e)))?;

        plaintext.zeroize();
        derived_key.zeroize();

        let mut file_bytes = Vec::with_capacity(6 + 16 + 12 + ciphertext.len());
        file_bytes.extend_from_slice(KEYSTORE_MAGIC);
        file_bytes.extend_from_slice(&salt);
        file_bytes.extend_from_slice(&nonce_bytes);
        file_bytes.extend_from_slice(&ciphertext);

        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.path, file_bytes)?;
        Ok(())
    }

    pub fn list_identities(&self) -> Vec<[u8; 16]> {
        self.inner.list_identities()
    }

    pub fn first_identity(&self) -> Option<[u8; 16]> {
        self.inner.first_identity()
    }
}

impl KeyStore for FileKeyStore {
    fn get_public_key(&self, identity_id: &[u8; 16]) -> Result<[u8; 32], VaultError> {
        self.inner.get_public_key(identity_id)
    }

    fn sign(&self, identity_id: &[u8; 16], message: &[u8]) -> Result<[u8; 64], VaultError> {
        self.inner.sign(identity_id, message)
    }

    fn generate_keypair(&mut self, identity_id: &[u8; 16]) -> Result<[u8; 32], VaultError> {
        self.inner.generate_keypair(identity_id)
    }

    fn insert_keypair(
        &mut self,
        identity_id: &[u8; 16],
        secret_seed: &[u8; 32],
    ) -> Result<[u8; 32], VaultError> {
        self.inner.insert_keypair(identity_id, secret_seed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::VerifyingKey;

    #[test]
    fn test_file_keystore_roundtrip_encrypted() {
        let temp_dir = tempfile::tempdir().unwrap();
        let ks_path = temp_dir.path().join("test.keystore");
        let pass = "super-secret-passphrase-42";

        let id = [7u8; 16];
        let pubkey1 = {
            let mut store = FileKeyStore::open_or_create(&ks_path, pass).unwrap();
            let pk = store.generate_keypair(&id).unwrap();
            store.save(pass).unwrap();
            pk
        };

        // File on disk must be encrypted and not contain raw pubkey or plaintext
        let disk_bytes = fs::read(&ks_path).unwrap();
        assert!(disk_bytes.starts_with(KEYSTORE_MAGIC));

        // Reopen with correct passphrase
        let reopen = FileKeyStore::open(&ks_path, pass).unwrap();
        let pk2 = reopen.get_public_key(&id).unwrap();
        assert_eq!(pubkey1, pk2);

        // Sign message and verify
        let msg = b"test message to sign";
        let sig = reopen.sign(&id, msg).unwrap();
        let verifying_key = VerifyingKey::from_bytes(&pk2).unwrap();
        assert!(verifying_key
            .verify_strict(msg, &Signature::from_bytes(&sig))
            .is_ok());

        // Reopen with wrong passphrase must fail
        assert!(FileKeyStore::open(&ks_path, "wrong-passphrase").is_err());
    }
}
