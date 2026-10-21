//! Key rotation and revoked-key enforcement (§6).

use ed25519_dalek::{Signature, VerifyingKey};
use std::collections::HashMap;

use crate::errors::VaultError;
use crate::keystore::KeyStore;
use crate::records::{KeyRotationRecord, RevokedKeyRecord};

/// Create and dual-sign a KeyRotationRecord (§6.1).
pub fn create_key_rotation_record(
    rotation_id: [u8; 16],
    old_identity_id: [u8; 16],
    new_identity_id: [u8; 16],
    rotated_at: u64,
    keystore: &impl KeyStore,
) -> Result<KeyRotationRecord, VaultError> {
    let old_public_key = keystore.get_public_key(&old_identity_id)?;
    let new_public_key = keystore.get_public_key(&new_identity_id)?;

    let mut record = KeyRotationRecord {
        rotation_id,
        old_public_key,
        new_public_key,
        rotated_at: rotated_at.to_le_bytes(),
        superseded_sig: [0u8; 64],
        confirm_sig: [0u8; 64],
        record_crc32: 0,
    };

    let signable = record.signed_bytes().to_vec();
    let old_sig = keystore.sign(&old_identity_id, &signable)?;
    let new_sig = keystore.sign(&new_identity_id, &signable)?;

    record.superseded_sig = old_sig;
    record.confirm_sig = new_sig;
    record.record_crc32 = record.compute_crc();

    Ok(record)
}

/// Verify a KeyRotationRecord.
pub fn verify_key_rotation_record(record: &KeyRotationRecord) -> Result<(), VaultError> {
    if !record.is_crc_valid() {
        return Err(VaultError::InvalidCrc("KeyRotationRecord CRC mismatch"));
    }

    let old_vk = VerifyingKey::from_bytes(&record.old_public_key)
        .map_err(|_| VaultError::InvalidSignature("Invalid old public key"))?;
    let new_vk = VerifyingKey::from_bytes(&record.new_public_key)
        .map_err(|_| VaultError::InvalidSignature("Invalid new public key"))?;

    let old_sig = Signature::from_bytes(&record.superseded_sig);
    let new_sig = Signature::from_bytes(&record.confirm_sig);

    old_vk
        .verify_strict(record.signed_bytes(), &old_sig)
        .map_err(|_| VaultError::InvalidSignature("Key rotation superseded_sig mismatch"))?;

    new_vk
        .verify_strict(record.signed_bytes(), &new_sig)
        .map_err(|_| VaultError::InvalidSignature("Key rotation confirm_sig mismatch"))?;

    Ok(())
}

/// Append-only Revocation Registry enforcing revoked key invariants (§6.2).
#[derive(Default)]
pub struct RevocationRegistry {
    /// Maps revoked public key -> revoked_at timestamp
    revocations: HashMap<[u8; 32], u64>,
}

impl RevocationRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a key rotation. The old key is marked revoked starting at `record.rotated_at`.
    pub fn register_rotation(&mut self, record: &KeyRotationRecord) -> Result<(), VaultError> {
        verify_key_rotation_record(record)?;
        self.revocations
            .insert(record.old_public_key, record.get_rotated_at());
        Ok(())
    }

    /// Register an explicit revocation record.
    pub fn register_revocation(&mut self, record: &RevokedKeyRecord) -> Result<(), VaultError> {
        if !record.is_crc_valid() {
            return Err(VaultError::InvalidCrc("RevokedKeyRecord CRC mismatch"));
        }
        self.revocations
            .insert(record.revoked_public_key, record.get_revoked_at());
        Ok(())
    }

    /// Check if a key is authorized to perform an action at `timestamp`.
    /// Invariant: Actions at or after `revoked_at` fail; historical actions prior to `revoked_at` remain valid.
    pub fn check_key_valid(&self, public_key: &[u8; 32], timestamp: u64) -> Result<(), VaultError> {
        if let Some(&revoked_at) = self.revocations.get(public_key) {
            if timestamp >= revoked_at {
                return Err(VaultError::KeyRevokedError(*public_key));
            }
        }
        Ok(())
    }

    pub fn is_revoked(&self, public_key: &[u8; 32]) -> bool {
        self.revocations.contains_key(public_key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keystore::MemoryKeyStore;

    #[test]
    fn test_key_rotation_and_revocation_enforcement() {
        let mut ks = MemoryKeyStore::new();

        let k1_id = [1u8; 16];
        let k1_pk = ks.generate_keypair(&k1_id).unwrap();

        let k2_id = [2u8; 16];
        let k2_pk = ks.generate_keypair(&k2_id).unwrap();

        let rotated_at = 2000;
        let rot = create_key_rotation_record([10u8; 16], k1_id, k2_id, rotated_at, &ks).unwrap();
        assert!(verify_key_rotation_record(&rot).is_ok());

        let mut reg = RevocationRegistry::new();
        reg.register_rotation(&rot).unwrap();

        // 1. Historical action signed by K1 before rotation (timestamp 1500 < 2000) remains valid
        assert!(reg.check_key_valid(&k1_pk, 1500).is_ok());

        // 2. Action attempted by K1 at or after rotation is rejected
        assert!(matches!(
            reg.check_key_valid(&k1_pk, rotated_at),
            Err(VaultError::KeyRevokedError(pk)) if pk == k1_pk
        ));
        assert!(matches!(
            reg.check_key_valid(&k1_pk, 2500),
            Err(VaultError::KeyRevokedError(pk)) if pk == k1_pk
        ));

        // 3. New key K2 is valid
        assert!(reg.check_key_valid(&k2_pk, 2500).is_ok());
    }
}
