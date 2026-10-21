//! Transfers and cryptographic ownership chains (§5).

use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

use crate::errors::VaultError;
use crate::keystore::KeyStore;
use crate::records::{ItemTransferRecord, TRANSFER_MODE_UNIQUE};

/// Compute transfer hash (§5.3):
/// transfer_hash = SHA-256(prev_transfer_hash || item_hash || from_pubkey || to_pubkey || timestamp)
pub fn compute_transfer_hash(
    prev_transfer_hash: &[u8; 32],
    item_hash: &[u8; 32],
    from_pubkey: &[u8; 32],
    to_pubkey: &[u8; 32],
    timestamp: u64,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(prev_transfer_hash);
    hasher.update(item_hash);
    hasher.update(from_pubkey);
    hasher.update(to_pubkey);
    hasher.update(&timestamp.to_le_bytes());
    hasher.finalize().into()
}

/// Create and sign an ItemTransferRecord.
pub fn create_transfer_record(
    transfer_id: [u8; 16],
    item_hash: [u8; 32],
    from_identity_id: [u8; 16],
    to_pubkey: [u8; 32],
    timestamp: u64,
    prev_transfer_hash: [u8; 32],
    transfer_mode: u16,
    keystore: &impl KeyStore,
) -> Result<ItemTransferRecord, VaultError> {
    let from_pubkey = keystore.get_public_key(&from_identity_id)?;

    let mut record = ItemTransferRecord {
        transfer_id,
        item_hash,
        from_pubkey,
        to_pubkey,
        timestamp: timestamp.to_le_bytes(),
        prev_transfer_hash,
        transfer_mode,
        _pad: [0u8; 6],
        _reserved: [0u8; 12],
        record_crc32: 0,
        sender_signature: [0u8; 64],
    };

    record.record_crc32 = record.compute_crc();
    let sig_bytes = keystore.sign(&from_identity_id, record.signed_bytes())?;
    record.sender_signature = sig_bytes;
    Ok(record)
}

/// Verify an individual transfer record's CRC and Ed25519 signature.
pub fn verify_transfer_record(record: &ItemTransferRecord) -> Result<(), VaultError> {
    if !record.is_crc_valid() {
        return Err(VaultError::InvalidCrc("ItemTransferRecord CRC32C mismatch"));
    }

    let verifying_key = VerifyingKey::from_bytes(&record.from_pubkey)
        .map_err(|_| VaultError::InvalidSignature("Invalid sender public key"))?;
    let sig = Signature::from_bytes(&record.sender_signature);

    verifying_key
        .verify_strict(record.signed_bytes(), &sig)
        .map_err(|_| VaultError::InvalidSignature("Transfer record sender signature mismatch"))?;

    Ok(())
}

/// Authoritative ownership resolution for an item given its transfer chain (§5.3).
/// Returns the current owner public key.
pub fn resolve_chain_ownership(
    issuer_pubkey: &[u8; 32],
    chain: &[ItemTransferRecord],
) -> Result<[u8; 32], VaultError> {
    if chain.is_empty() {
        return Ok(*issuer_pubkey);
    }

    // Genesis link check
    let genesis = &chain[0];
    verify_transfer_record(genesis)?;

    if genesis.prev_transfer_hash != [0u8; 32] {
        return Err(VaultError::CorruptedData(
            "Genesis transfer link must have zero prev_transfer_hash",
        ));
    }
    if &genesis.from_pubkey != issuer_pubkey {
        return Err(VaultError::CorruptedData(
            "Genesis transfer must be signed by the issuer",
        ));
    }

    let mut prev_hash = compute_transfer_hash(
        &genesis.prev_transfer_hash,
        &genesis.item_hash,
        &genesis.from_pubkey,
        &genesis.to_pubkey,
        genesis.get_timestamp(),
    );
    let mut current_owner = genesis.to_pubkey;

    // Set of senders that have already transferred away this unique item
    let mut spent_senders: HashSet<[u8; 32]> = HashSet::new();
    if genesis.transfer_mode == TRANSFER_MODE_UNIQUE {
        spent_senders.insert(genesis.from_pubkey);
    }

    for (_idx, link) in chain.iter().enumerate().skip(1) {
        verify_transfer_record(link)?;

        // Must link to exact prev_transfer_hash
        if link.prev_transfer_hash != prev_hash {
            return Err(VaultError::CorruptedData(
                "Transfer chain prev_transfer_hash broken",
            ));
        }

        // Must be signed by the current owner
        if link.from_pubkey != current_owner {
            return Err(VaultError::CorruptedData(
                "Transfer link sender is not current owner",
            ));
        }

        // Double-send detection for unique transfers
        if link.transfer_mode == TRANSFER_MODE_UNIQUE {
            if spent_senders.contains(&link.from_pubkey) {
                return Err(VaultError::DoubleSendDetected(link.item_hash));
            }
            spent_senders.insert(link.from_pubkey);
        }

        prev_hash = compute_transfer_hash(
            &link.prev_transfer_hash,
            &link.item_hash,
            &link.from_pubkey,
            &link.to_pubkey,
            link.get_timestamp(),
        );
        current_owner = link.to_pubkey;
    }

    Ok(current_owner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keystore::MemoryKeyStore;

    #[test]
    fn test_transfer_chain_ownership_and_double_send() {
        let mut ks = MemoryKeyStore::new();

        let issuer_id = [1u8; 16];
        let issuer_pk = ks.generate_keypair(&issuer_id).unwrap();

        let alice_id = [2u8; 16];
        let alice_pk = ks.generate_keypair(&alice_id).unwrap();

        let bob_id = [3u8; 16];
        let bob_pk = ks.generate_keypair(&bob_id).unwrap();

        let charlie_id = [4u8; 16];
        let charlie_pk = ks.generate_keypair(&charlie_id).unwrap();

        let item_hash = [77u8; 32];

        // 1. Genesis: Issuer -> Alice
        let t1 = create_transfer_record(
            [10u8; 16],
            item_hash,
            issuer_id,
            alice_pk,
            1000,
            [0u8; 32],
            TRANSFER_MODE_UNIQUE,
            &ks,
        )
        .unwrap();

        let h1 = compute_transfer_hash(&[0u8; 32], &item_hash, &issuer_pk, &alice_pk, 1000);

        // 2. Alice -> Bob
        let t2 = create_transfer_record(
            [11u8; 16],
            item_hash,
            alice_id,
            bob_pk,
            2000,
            h1,
            TRANSFER_MODE_UNIQUE,
            &ks,
        )
        .unwrap();

        let chain = vec![t1, t2];
        let resolved = resolve_chain_ownership(&issuer_pk, &chain).expect("Chain must resolve");
        assert_eq!(resolved, bob_pk);

        // Corrupted hash in chain must fail
        let mut bad_chain = chain.clone();
        bad_chain[1].prev_transfer_hash[0] ^= 0xFF;
        assert!(resolve_chain_ownership(&issuer_pk, &bad_chain).is_err());

        // Corrupted signature must fail
        let mut bad_sig_chain = chain.clone();
        bad_sig_chain[1].sender_signature[0] ^= 0xFF;
        assert!(resolve_chain_ownership(&issuer_pk, &bad_sig_chain).is_err());

        // Double-send: Alice tries to transfer the same item to Charlie after already transferring to Bob
        let t2_double = create_transfer_record(
            [12u8; 16],
            item_hash,
            alice_id,
            charlie_pk,
            2500,
            h1,
            TRANSFER_MODE_UNIQUE,
            &ks,
        )
        .unwrap();

        let double_send_chain = vec![chain[0], chain[1], t2_double];
        let err = resolve_chain_ownership(&issuer_pk, &double_send_chain);
        assert!(err.is_err());
    }
}
