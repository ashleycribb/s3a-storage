//! High-level S3A Digital Vault storage engine (§1.2, §2, §7, §13).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};

use s3a_core::TileType;
use s3a_engine::{MmapReader, TileWriter};

use crate::address::GateAddress;
use crate::errors::VaultError;
use crate::keystore::KeyStore;
use crate::records::*;
use crate::rotation::RevocationRegistry;
use crate::transfer::resolve_chain_ownership;

/// An S3A Digital Vault archive handle (§1.2).
pub struct Vault {
    path: PathBuf,
    identities: Vec<VaultIdentityRecord>,
    grants: HashMap<[u8; 16], DelegationGrantRecord>,
    items: HashMap<[u8; 32], VaultItemRecord>,
    transfers: HashMap<[u8; 32], Vec<ItemTransferRecord>>,
    events: Vec<VaultEventRecord>,
    revocation_registry: RevocationRegistry,
    latest_event_hash: [u8; 32],
}

impl Vault {
    /// Create a new S3A Digital Vault archive at `path`.
    pub fn create(
        path: impl AsRef<Path>,
        display_name: &str,
        owner_kind: VaultOwnerKind,
        keystore: &mut impl KeyStore,
    ) -> Result<Self, VaultError> {
        let path_buf = path.as_ref().to_path_buf();

        let mut identity_id = [0u8; 16];
        OsRng.fill_bytes(&mut identity_id);

        let pubkey = keystore.generate_keypair(&identity_id)?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut name_bytes = [0u8; 32];
        let copy_len = display_name.as_bytes().len().min(31);
        name_bytes[0..copy_len].copy_from_slice(&display_name.as_bytes()[0..copy_len]);

        let mut identity = VaultIdentityRecord {
            identity_id,
            owner_kind: owner_kind as u16,
            key_algorithm: 1, // ALG_ED25519
            public_key: pubkey,
            display_name: name_bytes,
            created_at: now.to_le_bytes(),
            record_crc32: 0,
        };
        identity.record_crc32 = identity.compute_crc();

        let mut initial_event = VaultEventRecord {
            event_sequence: 1u64.to_le_bytes(),
            timestamp: now.to_le_bytes(),
            event_type: match owner_kind {
                VaultOwnerKind::Thing => EVENT_THING_PROVISIONED,
                _ => EVENT_ITEM_RECEIVED,
            },
            actor_kind: owner_kind as u16,
            target_id: identity_id,
            prev_event_hash: [0u8; 32],
            grant_id_suffix: 0u64.to_le_bytes(),
            record_crc32: 0,
        };
        initial_event.record_crc32 = initial_event.compute_crc();

        let mut hasher = Sha256::new();
        hasher.update(bytemuck::bytes_of(&initial_event));
        let event_hash: [u8; 32] = hasher.finalize().into();

        // Write initial tiles to archive
        {
            let mut writer = TileWriter::create(&path_buf)?;
            let id_records = vec![identity];
            writer.write_hyper_tile(TileType::RECORD, &id_records, None, None)?;

            let event_records = vec![initial_event];
            writer.write_hyper_tile(TileType::RECORD, &event_records, None, None)?;
        }

        let mut identities = Vec::new();
        identities.push(identity);

        let mut events = Vec::new();
        events.push(initial_event);

        Ok(Self {
            path: path_buf,
            identities,
            grants: HashMap::new(),
            items: HashMap::new(),
            transfers: HashMap::new(),
            events,
            revocation_registry: RevocationRegistry::new(),
            latest_event_hash: event_hash,
        })
    }

    /// Open an existing S3A Vault archive and verify integrity.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, VaultError> {
        let path_buf = path.as_ref().to_path_buf();
        let reader = MmapReader::open(&path_buf)?;

        let mut identities = Vec::new();
        let mut grants = HashMap::new();
        let mut items = HashMap::new();
        let mut transfers: HashMap<[u8; 32], Vec<ItemTransferRecord>> = HashMap::new();
        let mut events = Vec::new();
        let mut revocation_registry = RevocationRegistry::new();
        let mut latest_event_hash = [0u8; 32];

        for i in 0..reader.tile_count() as usize {
            if let Some((header, payload)) = reader.get_tile(i) {
                // If tile is a RECORD tile (64) or domain pack tiles 36..=40
                if header.tile_type == TileType::RECORD
                    || (header.tile_type >= 36 && header.tile_type <= 40)
                {
                    // Scan payload for known Vault Pod structs
                    let payload_len = payload.len();

                    // 1. VaultIdentityRecord (96 bytes)
                    if payload_len >= 96 && payload_len % 96 == 0 {
                        if let Ok(records) = bytemuck::try_cast_slice::<u8, VaultIdentityRecord>(payload) {
                            for r in records {
                                if r.is_crc_valid() && r.key_algorithm == 1 && r.owner_kind >= 1 && r.owner_kind <= 4 {
                                    identities.push(*r);
                                }
                            }
                        }
                    }

                    // 2. DelegationGrantRecord (192 bytes)
                    if payload_len >= 192 && payload_len % 192 == 0 {
                        if let Ok(records) = bytemuck::try_cast_slice::<u8, DelegationGrantRecord>(payload) {
                            for r in records {
                                if r.get_expires_at() >= r.get_issued_at() {
                                    grants.insert(r.grant_id, *r);
                                }
                            }
                        }
                    }

                    // 3. VaultItemRecord (160 bytes)
                    if payload_len >= 160 && payload_len % 160 == 0 {
                        if let Ok(records) = bytemuck::try_cast_slice::<u8, VaultItemRecord>(payload) {
                            for r in records {
                                if r.is_crc_valid() {
                                    items.insert(r.item_hash, *r);
                                }
                            }
                        }
                    }

                    // 4. ItemTransferRecord (240 bytes)
                    if payload_len >= 240 && payload_len % 240 == 0 {
                        if let Ok(records) = bytemuck::try_cast_slice::<u8, ItemTransferRecord>(payload) {
                            for r in records {
                                if r.is_crc_valid() {
                                    transfers.entry(r.item_hash).or_default().push(*r);
                                }
                            }
                        }
                    }

                    // 5. KeyRotationRecord (220 bytes)
                    if payload_len >= 220 && payload_len % 220 == 0 {
                        if let Ok(records) = bytemuck::try_cast_slice::<u8, KeyRotationRecord>(payload) {
                            for r in records {
                                if r.is_crc_valid() {
                                    let _ = revocation_registry.register_rotation(r);
                                }
                            }
                        }
                    }

                    // 6. RevokedKeyRecord (64 bytes)
                    if payload_len >= 64 && payload_len % 64 == 0 {
                        if let Ok(records) = bytemuck::try_cast_slice::<u8, RevokedKeyRecord>(payload) {
                            for r in records {
                                if r.is_crc_valid() {
                                    let _ = revocation_registry.register_revocation(r);
                                }
                            }
                        }
                    }

                    // 7. VaultEventRecord (80 bytes)
                    if payload_len >= 80 && payload_len % 80 == 0 {
                        if let Ok(records) = bytemuck::try_cast_slice::<u8, VaultEventRecord>(payload) {
                            for r in records {
                                if r.is_crc_valid() {
                                    events.push(*r);
                                    let mut hasher = Sha256::new();
                                    hasher.update(bytemuck::bytes_of(r));
                                    latest_event_hash = hasher.finalize().into();
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(Self {
            path: path_buf,
            identities,
            grants,
            items,
            transfers,
            events,
            revocation_registry,
            latest_event_hash,
        })
    }

    /// Primary identity of the vault.
    pub fn primary_identity(&self) -> Option<&VaultIdentityRecord> {
        self.identities.first()
    }

    /// Gate Address for dialing (§11.1).
    pub fn gate_address(&self) -> Result<GateAddress, VaultError> {
        let primary = self
            .primary_identity()
            .ok_or_else(|| VaultError::CorruptedData("Vault has no primary identity"))?;
        Ok(GateAddress::from_public_key(&primary.public_key))
    }

    /// Canonical URI `gate://<address>` (§11.1).
    pub fn gate_uri(&self) -> Result<String, VaultError> {
        Ok(self.gate_address()?.to_uri())
    }

    /// Record a new item in the vault.
    pub fn add_item(&mut self, mut item: VaultItemRecord) -> Result<(), VaultError> {
        item.record_crc32 = item.compute_crc();
        let mut writer = TileWriter::open_append(&self.path)?;
        writer.write_hyper_tile(TileType::RECORD, &[item], None, None)?;
        self.items.insert(item.item_hash, item);
        self.log_event(EVENT_ITEM_RECEIVED, 1, item.item_id, 0)?;
        Ok(())
    }

    /// Record a delegation grant.
    pub fn add_grant(&mut self, grant: DelegationGrantRecord) -> Result<(), VaultError> {
        crate::delegation::verify_delegation_grant(&grant)?;
        let mut writer = TileWriter::open_append(&self.path)?;
        writer.write_hyper_tile(TileType::RECORD, &[grant], None, None)?;
        self.grants.insert(grant.grant_id, grant);
        self.log_event(EVENT_GRANT_ISSUED, 1, grant.grant_id, 0)?;
        Ok(())
    }

    /// Record an item transfer link.
    pub fn add_transfer(&mut self, mut transfer: ItemTransferRecord) -> Result<(), VaultError> {
        transfer.record_crc32 = transfer.compute_crc();
        crate::transfer::verify_transfer_record(&transfer)?;

        // Append to chain and verify no double send
        let mut chain = self.transfers.get(&transfer.item_hash).cloned().unwrap_or_default();
        chain.push(transfer);

        let issuer_pk = if let Some(item) = self.items.get(&transfer.item_hash) {
            item.issuer_pubkey
        } else {
            chain[0].from_pubkey
        };

        resolve_chain_ownership(&issuer_pk, &chain)?;

        let mut writer = TileWriter::open_append(&self.path)?;
        writer.write_hyper_tile(TileType::RECORD, &[transfer], None, None)?;

        self.transfers.insert(transfer.item_hash, chain);
        self.log_event(EVENT_ITEM_SENT, 1, transfer.transfer_id, 0)?;
        Ok(())
    }

    /// Append a KeyRotationRecord.
    pub fn add_key_rotation(&mut self, rotation: KeyRotationRecord) -> Result<(), VaultError> {
        self.revocation_registry.register_rotation(&rotation)?;
        let mut writer = TileWriter::open_append(&self.path)?;
        writer.write_hyper_tile(TileType::RECORD, &[rotation], None, None)?;
        self.log_event(EVENT_KEY_ROTATED, 1, rotation.rotation_id, 0)?;
        Ok(())
    }

    /// Resolve current owner public key of an item.
    pub fn get_item_owner(&self, item_hash: &[u8; 32]) -> Result<[u8; 32], VaultError> {
        let chain = self.transfers.get(item_hash);
        let item = self.items.get(item_hash);

        match (item, chain) {
            (Some(item), Some(chain)) => resolve_chain_ownership(&item.issuer_pubkey, chain),
            (Some(item), None) => Ok(item.current_owner_pubkey),
            (None, Some(chain)) => resolve_chain_ownership(&chain[0].from_pubkey, chain),
            (None, None) => Err(VaultError::ItemNotFound([0u8; 16])),
        }
    }

    /// Append an event to the vault event log (§7.1).
    pub fn log_event(
        &mut self,
        event_type: u16,
        actor_kind: u16,
        target_id: [u8; 16],
        grant_id_suffix: u64,
    ) -> Result<u64, VaultError> {
        let seq = (self.events.len() as u64) + 1;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut event = VaultEventRecord {
            event_sequence: seq.to_le_bytes(),
            timestamp: now.to_le_bytes(),
            event_type,
            actor_kind,
            target_id,
            prev_event_hash: self.latest_event_hash,
            grant_id_suffix: grant_id_suffix.to_le_bytes(),
            record_crc32: 0,
        };
        event.record_crc32 = event.compute_crc();

        let mut hasher = Sha256::new();
        hasher.update(bytemuck::bytes_of(&event));
        self.latest_event_hash = hasher.finalize().into();

        let mut writer = TileWriter::open_append(&self.path)?;
        writer.write_hyper_tile(TileType::RECORD, &[event], None, None)?;
        self.events.push(event);

        Ok(seq)
    }

    pub fn events(&self) -> &[VaultEventRecord] {
        &self.events
    }

    pub fn items(&self) -> &HashMap<[u8; 32], VaultItemRecord> {
        &self.items
    }

    pub fn grants(&self) -> &HashMap<[u8; 16], DelegationGrantRecord> {
        &self.grants
    }

    pub fn revocation_registry(&self) -> &RevocationRegistry {
        &self.revocation_registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keystore::MemoryKeyStore;

    #[test]
    fn test_vault_create_open_and_gate_address() {
        let temp_dir = tempfile::tempdir().unwrap();
        let vault_path = temp_dir.path().join("alice.vault.s3a");
        let mut ks = MemoryKeyStore::new();

        let vault = Vault::create(&vault_path, "Alice Vault", VaultOwnerKind::Person, &mut ks).unwrap();
        let addr = vault.gate_address().unwrap();
        let uri = vault.gate_uri().unwrap();

        assert!(uri.starts_with("gate://s3a-"));
        assert_eq!(vault.events().len(), 1);

        // Reopen vault and verify recovered state
        let reopened = Vault::open(&vault_path).unwrap();
        assert_eq!(reopened.gate_address().unwrap(), addr);
        assert_eq!(reopened.events().len(), 1);
        assert_eq!(
            reopened.primary_identity().unwrap().public_key,
            vault.primary_identity().unwrap().public_key
        );
    }
}
