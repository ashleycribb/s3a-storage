//! Trust Architecture, Device Identity & Cryptographic Sealing (§6).

use sha2::{Digest, Sha256};

/// Magic prefix for the on-device S3A Edge seal hash chain.
pub const SEAL_CHAIN_DOMAIN_TAG: &[u8; 8] = b"S3AEDGE1";

/// Computes the deterministic on-device seal hash for tile `tile_index`:
/// `seal_hash_n = SHA-256(b"S3AEDGE1" || prev_seal_hash || generation_n || tile_index_n || payload_hash_n || device_id)`.
pub fn compute_seal_hash(
    prev_seal_hash: &[u8; 32],
    generation: u64,
    tile_index: u32,
    payload_hash: &[u8; 32],
    device_id: &[u8; 16],
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(SEAL_CHAIN_DOMAIN_TAG);
    hasher.update(prev_seal_hash);
    hasher.update(generation.to_le_bytes());
    hasher.update(tile_index.to_le_bytes());
    hasher.update(payload_hash);
    hasher.update(device_id);
    hasher.finalize().into()
}

/// Verifies whether `candidate_seal_hash` matches the expected link in the chain.
pub fn verify_seal_chain_link(
    candidate_seal_hash: &[u8; 32],
    prev_seal_hash: &[u8; 32],
    generation: u64,
    tile_index: u32,
    payload_hash: &[u8; 32],
    device_id: &[u8; 16],
) -> bool {
    let expected = compute_seal_hash(
        prev_seal_hash,
        generation,
        tile_index,
        payload_hash,
        device_id,
    );
    &expected == candidate_seal_hash
}

#[cfg(feature = "sign")]
pub use ed25519_support::*;

#[cfg(feature = "sign")]
mod ed25519_support {
    use ed25519_dalek::{Signature, Signer, Verifier};
    pub use ed25519_dalek::{SigningKey, VerifyingKey};

    /// Signs the 32-byte `seal_hash` with an Ed25519 signing key.
    pub fn sign_seal_hash(signing_key: &SigningKey, seal_hash: &[u8; 32]) -> [u8; 64] {
        let sig = signing_key.sign(seal_hash);
        sig.to_bytes()
    }

    /// Verifies an Ed25519 signature over a 32-byte `seal_hash` against a public verifying key.
    pub fn verify_signature(
        verifying_key: &VerifyingKey,
        seal_hash: &[u8; 32],
        signature_bytes: &[u8; 64],
    ) -> bool {
        let sig = Signature::from_bytes(signature_bytes);
        verifying_key.verify(seal_hash, &sig).is_ok()
    }

    /// Verifies an Ed25519 signature using raw 32-byte public key bytes.
    pub fn verify_signature_raw(
        public_key_bytes: &[u8; 32],
        seal_hash: &[u8; 32],
        signature_bytes: &[u8; 64],
    ) -> bool {
        if let Ok(verifying_key) = VerifyingKey::from_bytes(public_key_bytes) {
            verify_signature(&verifying_key, seal_hash, signature_bytes)
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_seal_hash_chain_progression() {
        let device_id = [0xAA; 16];
        let genesis_prev = [0u8; 32];
        let payload0 = [0x11; 32];
        let seal0 = compute_seal_hash(&genesis_prev, 1, 0, &payload0, &device_id);

        let payload1 = [0x22; 32];
        let seal1 = compute_seal_hash(&seal0, 2, 1, &payload1, &device_id);

        assert!(verify_seal_chain_link(
            &seal0,
            &genesis_prev,
            1,
            0,
            &payload0,
            &device_id
        ));
        assert!(verify_seal_chain_link(
            &seal1, &seal0, 2, 1, &payload1, &device_id
        ));

        // Mismatched prev_seal fails
        assert!(!verify_seal_chain_link(
            &seal1,
            &genesis_prev,
            2,
            1,
            &payload1,
            &device_id
        ));
        // Tampered payload fails
        assert!(!verify_seal_chain_link(
            &seal1, &seal0, 2, 1, &payload0, &device_id
        ));
    }

    #[cfg(feature = "sign")]
    #[test]
    fn test_ed25519_sign_and_verify() {
        use ed25519_dalek::SigningKey;

        let seed = [42u8; 32];
        let signing_key = SigningKey::from_bytes(&seed);
        let verifying_key = signing_key.verifying_key();

        let seal_hash = [0x77u8; 32];
        let sig = sign_seal_hash(&signing_key, &seal_hash);

        assert!(verify_signature(&verifying_key, &seal_hash, &sig));
        assert!(verify_signature_raw(
            &verifying_key.to_bytes(),
            &seal_hash,
            &sig
        ));

        // Forgery or altered hash rejected
        let forged_hash = [0x88u8; 32];
        assert!(!verify_signature(&verifying_key, &forged_hash, &sig));
    }
}
