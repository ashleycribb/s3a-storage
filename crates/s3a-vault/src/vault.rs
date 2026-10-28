//! High-level S3A Digital Vault storage engine (§1.2, §2, §7, §13).

use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
    /// `None` for a vault opened from bytes: such a vault is read-only.
    path: Option<PathBuf>,
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
        let copy_len = display_name.len().min(31);
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
            path: Some(path_buf),
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
        Self::from_reader(&reader, Some(path_buf))
    }

    /// Opens a vault from an in-memory copy of its file (no file path, no scratch copy). Reads
    /// the same records as [`Vault::open`] with the same filters. The result is read-only: every
    /// method that would append to the file (`add_*`, `log_event`) returns an
    /// `IoError(PermissionDenied)` instead.
    pub fn open_bytes(bytes: &[u8]) -> Result<Self, VaultError> {
        let reader = MmapReader::from_bytes(bytes)?;
        Self::from_reader(&reader, None)
    }

    fn writable_path(&self) -> Result<&Path, VaultError> {
        self.path.as_deref().ok_or_else(|| {
            VaultError::IoError(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "vault was opened from bytes and is read-only",
            ))
        })
    }

    fn from_reader(reader: &MmapReader, path: Option<PathBuf>) -> Result<Self, VaultError> {
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
                        if let Ok(records) =
                            bytemuck::try_cast_slice::<u8, VaultIdentityRecord>(payload)
                        {
                            for r in records {
                                if r.is_crc_valid()
                                    && r.key_algorithm == 1
                                    && r.owner_kind >= 1
                                    && r.owner_kind <= 4
                                {
                                    identities.push(*r);
                                }
                            }
                        }
                    }

                    // 2. DelegationGrantRecord (192 bytes)
                    if payload_len >= 192 && payload_len % 192 == 0 {
                        if let Ok(records) =
                            bytemuck::try_cast_slice::<u8, DelegationGrantRecord>(payload)
                        {
                            for r in records {
                                if r.get_expires_at() >= r.get_issued_at() {
                                    grants.insert(r.grant_id, *r);
                                }
                            }
                        }
                    }

                    // 3. VaultItemRecord (160 bytes)
                    if payload_len >= 160 && payload_len % 160 == 0 {
                        if let Ok(records) =
                            bytemuck::try_cast_slice::<u8, VaultItemRecord>(payload)
                        {
                            for r in records {
                                if r.is_crc_valid() {
                                    items.insert(r.item_hash, *r);
                                }
                            }
                        }
                    }

                    // 4. ItemTransferRecord (240 bytes)
                    if payload_len >= 240 && payload_len % 240 == 0 {
                        if let Ok(records) =
                            bytemuck::try_cast_slice::<u8, ItemTransferRecord>(payload)
                        {
                            for r in records {
                                if r.is_crc_valid() {
                                    transfers.entry(r.item_hash).or_default().push(*r);
                                }
                            }
                        }
                    }

                    // 5. KeyRotationRecord (220 bytes)
                    if payload_len >= 220 && payload_len % 220 == 0 {
                        if let Ok(records) =
                            bytemuck::try_cast_slice::<u8, KeyRotationRecord>(payload)
                        {
                            for r in records {
                                if r.is_crc_valid() {
                                    let _ = revocation_registry.register_rotation(r);
                                }
                            }
                        }
                    }

                    // 6. RevokedKeyRecord (64 bytes)
                    if payload_len >= 64 && payload_len % 64 == 0 {
                        if let Ok(records) =
                            bytemuck::try_cast_slice::<u8, RevokedKeyRecord>(payload)
                        {
                            for r in records {
                                if r.is_crc_valid() {
                                    let _ = revocation_registry.register_revocation(r);
                                }
                            }
                        }
                    }

                    // 7. VaultEventRecord (80 bytes)
                    if payload_len >= 80 && payload_len % 80 == 0 {
                        if let Ok(records) =
                            bytemuck::try_cast_slice::<u8, VaultEventRecord>(payload)
                        {
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
            path,
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
            .ok_or(VaultError::CorruptedData("Vault has no primary identity"))?;
        Ok(GateAddress::from_public_key(&primary.public_key))
    }

    /// Canonical URI `gate://<address>` (§11.1).
    pub fn gate_uri(&self) -> Result<String, VaultError> {
        Ok(self.gate_address()?.to_uri())
    }

    /// Record a new item in the vault.
    pub fn add_item(&mut self, mut item: VaultItemRecord) -> Result<(), VaultError> {
        self.writable_path()?; // read-only vaults must fail before touching any state
        item.record_crc32 = item.compute_crc();
        let mut writer = TileWriter::open_append(self.writable_path()?)?;
        writer.write_hyper_tile(TileType::RECORD, &[item], None, None)?;
        self.items.insert(item.item_hash, item);
        self.log_event(EVENT_ITEM_RECEIVED, 1, item.item_id, 0)?;
        Ok(())
    }

    /// Record a delegation grant.
    pub fn add_grant(&mut self, grant: DelegationGrantRecord) -> Result<(), VaultError> {
        self.writable_path()?; // read-only vaults must fail before touching any state
        crate::delegation::verify_delegation_grant(&grant)?;
        let mut writer = TileWriter::open_append(self.writable_path()?)?;
        writer.write_hyper_tile(TileType::RECORD, &[grant], None, None)?;
        self.grants.insert(grant.grant_id, grant);
        self.log_event(EVENT_GRANT_ISSUED, 1, grant.grant_id, 0)?;
        Ok(())
    }

    /// Record an item transfer link.
    pub fn add_transfer(&mut self, mut transfer: ItemTransferRecord) -> Result<(), VaultError> {
        self.writable_path()?; // read-only vaults must fail before touching any state
        transfer.record_crc32 = transfer.compute_crc();
        crate::transfer::verify_transfer_record(&transfer)?;

        // Append to chain and verify no double send
        let mut chain = self
            .transfers
            .get(&transfer.item_hash)
            .cloned()
            .unwrap_or_default();
        chain.push(transfer);

        let issuer_pk = if let Some(item) = self.items.get(&transfer.item_hash) {
            item.issuer_pubkey
        } else {
            chain[0].from_pubkey
        };

        resolve_chain_ownership(&issuer_pk, &chain)?;

        let mut writer = TileWriter::open_append(self.writable_path()?)?;
        writer.write_hyper_tile(TileType::RECORD, &[transfer], None, None)?;

        self.transfers.insert(transfer.item_hash, chain);
        self.log_event(EVENT_ITEM_SENT, 1, transfer.transfer_id, 0)?;
        Ok(())
    }

    /// Append a KeyRotationRecord.
    pub fn add_key_rotation(&mut self, rotation: KeyRotationRecord) -> Result<(), VaultError> {
        self.writable_path()?; // read-only vaults must fail before touching any state
        self.revocation_registry.register_rotation(&rotation)?;
        let mut writer = TileWriter::open_append(self.writable_path()?)?;
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
        self.writable_path()?; // read-only vaults must fail before touching any state
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

        let mut writer = TileWriter::open_append(self.writable_path()?)?;
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

    /// Every identity record in the vault, in file order. The first is the primary identity.
    /// Records passed their CRC, key-algorithm and owner-kind checks on load. Identity records
    /// carry no signature, so nothing here is proof the holder controls the key.
    pub fn identities(&self) -> &[VaultIdentityRecord] {
        &self.identities
    }

    /// Transfer records by item hash, each chain in file order (genesis link first). Records
    /// passed their CRC on load; signatures are NOT checked by `open`. Use
    /// [`Vault::verify_records`] for that.
    pub fn transfers(&self) -> &HashMap<[u8; 32], Vec<ItemTransferRecord>> {
        &self.transfers
    }

    /// Checks the Ed25519 signatures on every loaded delegation grant and transfer record and
    /// resolves every ownership chain. Read-only; `Vault::open` does not do this, and keeps
    /// accepting such records, so an existing vault opens exactly as before.
    pub fn verify_records(&self) -> crate::verify::VaultVerifyReport {
        crate::verify::verify_all(&self.grants, &self.items, &self.transfers)
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
    fn open_bytes_matches_open_and_is_read_only() {
        let temp_dir = tempfile::tempdir().unwrap();
        let vault_path = temp_dir.path().join("bob.vault.s3a");
        let mut ks = MemoryKeyStore::new();
        let vault =
            Vault::create(&vault_path, "Bob Vault", VaultOwnerKind::Person, &mut ks).unwrap();
        let bytes = std::fs::read(&vault_path).unwrap();

        let mut ro = Vault::open_bytes(&bytes).unwrap();
        let from_file = Vault::open(&vault_path).unwrap();
        assert_eq!(ro.gate_address().unwrap(), vault.gate_address().unwrap());
        assert_eq!(ro.events().len(), from_file.events().len());
        assert_eq!(ro.items().len(), from_file.items().len());
        assert_eq!(ro.grants().len(), from_file.grants().len());
        assert_eq!(
            ro.primary_identity().unwrap().public_key,
            from_file.primary_identity().unwrap().public_key
        );

        // Mutators fail without touching in-memory state or any file.
        let before = ro.events().len();
        assert!(matches!(
            ro.log_event(EVENT_ITEM_RECEIVED, 1, [0u8; 16], 0),
            Err(VaultError::IoError(e)) if e.kind() == std::io::ErrorKind::PermissionDenied
        ));
        assert_eq!(ro.events().len(), before);
        assert_eq!(std::fs::read(&vault_path).unwrap(), bytes);
    }

    #[test]
    fn open_bytes_matches_open_on_conformance_vault_vectors() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance");
        let mut seen = 0;
        for v in std::fs::read_dir(&dir).unwrap() {
            let files = v.unwrap().path().join("files");
            let Ok(rd) = std::fs::read_dir(&files) else {
                continue;
            };
            for f in rd {
                let path = f.unwrap().path();
                let name = path.file_name().unwrap().to_string_lossy().to_string();
                if !name.starts_with("vault_") || !name.ends_with(".s3a") {
                    continue;
                }
                let bytes = std::fs::read(&path).unwrap();
                let (a, b) = (Vault::open(&path), Vault::open_bytes(&bytes));
                match (a, b) {
                    (Ok(a), Ok(b)) => {
                        assert_eq!(a.events().len(), b.events().len(), "{name}");
                        assert_eq!(a.items().len(), b.items().len(), "{name}");
                        assert_eq!(a.grants().len(), b.grants().len(), "{name}");
                        assert_eq!(
                            a.primary_identity().map(|i| i.public_key),
                            b.primary_identity().map(|i| i.public_key),
                            "{name}"
                        );
                    }
                    (Err(a), Err(b)) => assert_eq!(a.to_string(), b.to_string(), "{name}"),
                    _ => panic!("{name}: open and open_bytes disagree"),
                }
                seen += 1;
            }
        }
        assert!(
            seen >= 4,
            "expected the vault conformance vectors, saw {seen}"
        );
    }

    #[test]
    fn test_vault_create_open_and_gate_address() {
        let temp_dir = tempfile::tempdir().unwrap();
        let vault_path = temp_dir.path().join("alice.vault.s3a");
        let mut ks = MemoryKeyStore::new();

        let vault =
            Vault::create(&vault_path, "Alice Vault", VaultOwnerKind::Person, &mut ks).unwrap();
        let addr = vault.gate_address().unwrap();
        let uri = vault.gate_uri().unwrap();

        assert!(uri.starts_with("gate://s3a2-"));
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

    fn vector_copy(name: &str, dir: &std::path::Path) -> PathBuf {
        let src = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../conformance/vectors-1.5/files")
            .join(name);
        let dst = dir.join(name);
        std::fs::copy(&src, &dst).unwrap();
        dst
    }

    #[test]
    fn identities_transfers_and_verify_records_on_transfer_chain_vector() {
        let dir = tempfile::tempdir().unwrap();
        let path = vector_copy("vault_transfer_chain.s3a", dir.path());
        let v = Vault::open(&path).unwrap();
        assert!(!v.identities().is_empty());
        assert_eq!(
            v.identities()[0].public_key,
            v.primary_identity().unwrap().public_key
        );
        assert_eq!(v.transfers().len(), 1);
        let chain = v.transfers().values().next().unwrap();
        assert_eq!(chain.len(), 2, "issuer -> alice -> bob");
        assert_eq!(
            chain[0].from_pubkey,
            v.items().values().next().unwrap().issuer_pubkey
        );

        let rep = v.verify_records();
        assert!(rep.all_ok(), "{rep:?}");
        assert_eq!(rep.transfers.len(), 2);
        assert_eq!(
            rep.transfers.iter().map(|t| t.index).collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert_eq!(rep.chains.len(), 1);
        assert_eq!(rep.chains[0].owner_pubkey, Some(chain[1].to_pubkey));
        assert_eq!(
            rep.chains[0].owner_pubkey,
            Some(v.get_item_owner(&chain[0].item_hash).unwrap())
        );
    }

    #[test]
    fn forged_transfer_signature_loads_as_before_but_verify_records_flags_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = vector_copy("vault_transfer_chain.s3a", dir.path());
        let clean = Vault::open(&path).unwrap();
        let mut forged = *clean.transfers().values().next().unwrap().last().unwrap();
        forged.transfer_id = [0xEE; 16];
        forged.record_crc32 = forged.compute_crc();
        forged.sender_signature[0] ^= 0xFF; // the CRC does not cover the signature
        assert!(forged.is_crc_valid());
        let mut w = TileWriter::open_append(&path).unwrap();
        w.write_hyper_tile(TileType::RECORD, &[forged], None, None)
            .unwrap();
        drop(w);

        // open() is unchanged: it still accepts a CRC-valid record, so the chain grows to 3.
        let v = Vault::open(&path).unwrap();
        assert_eq!(v.transfers().values().next().unwrap().len(), 3);
        let rep = v.verify_records();
        assert!(!rep.all_ok());
        let bad: Vec<_> = rep.transfers.iter().filter(|t| !t.ok).collect();
        assert_eq!(bad.len(), 1);
        assert_eq!(bad[0].transfer_id, [0xEE; 16]);
        assert!(bad[0].error.as_ref().unwrap().contains("signature"));
        assert!(!rep.chains[0].ok && rep.chains[0].owner_pubkey.is_none());
    }

    #[test]
    fn forged_grant_signature_is_flagged_and_genuine_grants_pass() {
        let dir = tempfile::tempdir().unwrap();
        let path = vector_copy("vault_delegation_governance.s3a", dir.path());
        let clean = Vault::open(&path).unwrap();
        assert!(!clean.grants().is_empty());
        assert!(clean.verify_records().all_ok());

        let mut forged = *clean.grants().values().next().unwrap();
        forged.grant_id = [0xDD; 16];
        forged.granter_signature[3] ^= 0x55;
        let mut w = TileWriter::open_append(&path).unwrap();
        w.write_hyper_tile(TileType::RECORD, &[forged], None, None)
            .unwrap();
        drop(w);

        let v = Vault::open(&path).unwrap();
        assert_eq!(
            v.grants().len(),
            clean.grants().len() + 1,
            "open() still loads it"
        );
        let rep = v.verify_records();
        let bad: Vec<_> = rep.grants.iter().filter(|g| !g.ok).collect();
        assert_eq!(bad.len(), 1);
        assert_eq!(bad[0].grant_id, [0xDD; 16]);
        assert_eq!(
            rep.grants.iter().filter(|g| g.ok).count(),
            clean.grants().len()
        );
    }
}
