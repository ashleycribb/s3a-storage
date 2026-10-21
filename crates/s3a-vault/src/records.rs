//! Binary record definitions for S3A Digital Vault (S3A-VAULT)

use bytemuck::{Pod, Zeroable};

/// Owner kind for vault identities (§2.1).
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultOwnerKind {
    Person = 1,
    Agent = 2,
    Organization = 3,
    Thing = 4,
}

impl VaultOwnerKind {
    pub fn from_u16(val: u16) -> Option<Self> {
        match val {
            1 => Some(Self::Person),
            2 => Some(Self::Agent),
            3 => Some(Self::Organization),
            4 => Some(Self::Thing),
            _ => None,
        }
    }
}

/// Allowed actions bitmask for delegation grants (§3.1.1).
pub const ACTION_READ: u32 = 0x0001;
pub const ACTION_WRITE: u32 = 0x0002;
pub const ACTION_SEND_ITEM: u32 = 0x0004;
pub const ACTION_ISSUE_CREDENTIAL: u32 = 0x0008;
pub const ACTION_ASK_HUMAN: u32 = 0x0010;

/// Item categories (§4.2).
pub const ITEM_VERIFIABLE_CREDENTIAL: u16 = 0x0001;
pub const ITEM_LEARNER_RECORD: u16 = 0x0002;
pub const ITEM_RESEARCH_DATASET: u16 = 0x0003;
pub const ITEM_ALGORITHMIC_MODEL: u16 = 0x0004;
pub const ITEM_LICENSE_GRANT: u16 = 0x0005;
pub const ITEM_BINARY_ATTACHMENT: u16 = 0x0006;

/// Item flags (§4.1).
pub const ITEM_FLAG_UNIQUE: u16 = 0x0001;
pub const ITEM_FLAG_VERIFIABLE_CREDENTIAL: u16 = 0x0002;
pub const ITEM_FLAG_ENCRYPTED: u16 = 0x0004;

/// Consent scope bitmask (§8.1).
pub const SCOPE_RESEARCH_ONLY: u32 = 0x0001;
pub const SCOPE_INTERNAL_ENTERPRISE: u32 = 0x0002;
pub const SCOPE_NO_REEXPORT: u32 = 0x0004;
pub const SCOPE_HUMAN_APPROVAL_ON_EXPORT: u32 = 0x0008;
pub const SCOPE_MODEL_TRAINING_FORBIDDEN: u32 = 0x0010;

/// Transfer modes (§5.2).
pub const TRANSFER_MODE_UNIQUE: u16 = 0;
pub const TRANSFER_MODE_COPY_WITH_PROVENANCE: u16 = 1;

/// Event categories (§7.2).
pub const EVENT_ITEM_RECEIVED: u16 = 0x0001;
pub const EVENT_ITEM_SENT: u16 = 0x0002;
pub const EVENT_GRANT_ISSUED: u16 = 0x0003;
pub const EVENT_GRANT_REVOKED: u16 = 0x0004;
pub const EVENT_GRANT_USED: u16 = 0x0005;
pub const EVENT_KEY_ROTATED: u16 = 0x0006;
pub const EVENT_CONSENT_MODIFIED: u16 = 0x0007;
pub const EVENT_CHECKPOINT_EVALUATED: u16 = 0x0008;
pub const EVENT_THING_PROVISIONED: u16 = 0x0009;

/// Checkpoint types (§12.2).
pub const CHECKPOINT_PREFLIGHT_HALLUCINATION: u16 = 1;
pub const CHECKPOINT_HUMAN_REVIEW: u16 = 2;
pub const CHECKPOINT_CONSENT: u16 = 3;
pub const CHECKPOINT_TOOL_EXECUTION: u16 = 4;

/// Checkpoint outcomes (§12.1).
pub const CHECKPOINT_OUTCOME_PENDING: u16 = 0;
pub const CHECKPOINT_OUTCOME_APPROVED: u16 = 1;
pub const CHECKPOINT_OUTCOME_REJECTED: u16 = 2;
pub const CHECKPOINT_OUTCOME_ESCALATED: u16 = 3;
pub const CHECKPOINT_OUTCOME_TIMED_OUT: u16 = 4;

/// Tile wire types for S3A Vault (§13.2).
pub const TILE_VAULT_IDENTITY: u16 = 36;
pub const TILE_VAULT_DELEGATION: u16 = 37;
pub const TILE_VAULT_HOLDING: u16 = 38;
pub const TILE_VAULT_TRANSFER: u16 = 39;
pub const TILE_VAULT_EVENT_LOG: u16 = 40;

/// VaultIdentityRecord (96 Bytes, 8-Byte Aligned, Little-Endian) (§2.3).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct VaultIdentityRecord {
    pub identity_id: [u8; 16],
    pub owner_kind: u16,
    pub key_algorithm: u16, // MUST be 1 = ALG_ED25519
    pub public_key: [u8; 32],
    pub display_name: [u8; 32],
    pub created_at: [u8; 8],
    pub record_crc32: u32,
}

impl VaultIdentityRecord {
    pub fn get_created_at(&self) -> u64 {
        u64::from_le_bytes(self.created_at)
    }

    pub fn set_created_at(&mut self, val: u64) {
        self.created_at = val.to_le_bytes();
    }

    pub fn compute_crc(&self) -> u32 {
        let bytes = bytemuck::bytes_of(self);
        crc32c::crc32c(&bytes[0..92])
    }

    pub fn is_crc_valid(&self) -> bool {
        self.record_crc32 == self.compute_crc()
    }
}

/// DelegationGrantRecord (192 Bytes, 8-Byte Aligned) (§3.1).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct DelegationGrantRecord {
    pub grant_id: [u8; 16],
    pub granter_pubkey: [u8; 32],
    pub agent_id: [u8; 16],
    pub agent_pubkey: [u8; 32],
    pub allowed_actions: u32,
    pub allowed_scopes: u32,
    pub max_operations: u32,
    pub _pad: [u8; 4],
    pub issued_at: [u8; 8],
    pub expires_at: [u8; 8],
    pub granter_signature: [u8; 64],
}

impl DelegationGrantRecord {
    pub fn get_issued_at(&self) -> u64 {
        u64::from_le_bytes(self.issued_at)
    }

    pub fn set_issued_at(&mut self, val: u64) {
        self.issued_at = val.to_le_bytes();
    }

    pub fn get_expires_at(&self) -> u64 {
        u64::from_le_bytes(self.expires_at)
    }

    pub fn set_expires_at(&mut self, val: u64) {
        self.expires_at = val.to_le_bytes();
    }

    pub fn signed_bytes(&self) -> &[u8] {
        let bytes = bytemuck::bytes_of(self);
        &bytes[0..128]
    }
}

/// VaultItemRecord (160 Bytes, 8-Byte Aligned, Little-Endian) (§4.1).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct VaultItemRecord {
    pub item_id: [u8; 16],
    pub item_type: u16,
    pub item_flags: u16,
    pub item_hash: [u8; 32],
    pub issuer_pubkey: [u8; 32],
    pub current_owner_pubkey: [u8; 32],
    pub acquired_at: [u8; 8],
    pub consent_scope: u32,
    pub payload_ref: [u8; 16],
    pub _reserved: [u8; 12],
    pub record_crc32: u32,
}

impl VaultItemRecord {
    pub fn new(
        item_id: [u8; 16],
        item_type: u16,
        item_hash: [u8; 32],
        issuer_pubkey: [u8; 32],
        current_owner_pubkey: [u8; 32],
        acquired_at: u64,
        payload_tile_index: u32,
        payload_byte_length: u32,
    ) -> Self {
        let mut payload_ref = [0u8; 16];
        payload_ref[0..4].copy_from_slice(&payload_tile_index.to_le_bytes());
        payload_ref[4..8].copy_from_slice(&payload_byte_length.to_le_bytes());
        let mut rec = Self {
            item_id,
            item_type,
            item_flags: 0,
            item_hash,
            issuer_pubkey,
            current_owner_pubkey,
            acquired_at: acquired_at.to_le_bytes(),
            consent_scope: 0,
            payload_ref,
            _reserved: [0u8; 12],
            record_crc32: 0,
        };
        rec.record_crc32 = rec.compute_crc();
        rec
    }

    pub fn payload_tile_index(&self) -> u32 {
        u32::from_le_bytes(self.payload_ref[0..4].try_into().unwrap())
    }

    pub fn payload_byte_length(&self) -> u32 {
        u32::from_le_bytes(self.payload_ref[4..8].try_into().unwrap())
    }

    pub fn payload_byte_offset(&self) -> u32 {
        0
    }

    pub fn get_acquired_at(&self) -> u64 {
        u64::from_le_bytes(self.acquired_at)
    }

    pub fn set_acquired_at(&mut self, val: u64) {
        self.acquired_at = val.to_le_bytes();
    }

    pub fn compute_crc(&self) -> u32 {
        let bytes = bytemuck::bytes_of(self);
        crc32c::crc32c(&bytes[0..156])
    }

    pub fn is_crc_valid(&self) -> bool {
        self.record_crc32 == self.compute_crc()
    }
}

/// ItemTransferRecord (240 Bytes, 8-Byte Aligned, Little-Endian) (§5.1).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct ItemTransferRecord {
    pub transfer_id: [u8; 16],
    pub item_hash: [u8; 32],
    pub from_pubkey: [u8; 32],
    pub to_pubkey: [u8; 32],
    pub timestamp: [u8; 8],
    pub prev_transfer_hash: [u8; 32],
    pub transfer_mode: u16,
    pub _pad: [u8; 6],
    pub _reserved: [u8; 12],
    pub record_crc32: u32,
    pub sender_signature: [u8; 64],
}

impl ItemTransferRecord {
    pub fn compute_transfer_hash(&self) -> [u8; 32] {
        let mut hasher = sha2::Sha256::new();
        use sha2::Digest;
        hasher.update(self.prev_transfer_hash);
        hasher.update(self.item_hash);
        hasher.update(self.from_pubkey);
        hasher.update(self.to_pubkey);
        hasher.update(self.timestamp);
        hasher.finalize().into()
    }

    pub fn get_timestamp(&self) -> u64 {
        u64::from_le_bytes(self.timestamp)
    }

    pub fn set_timestamp(&mut self, val: u64) {
        self.timestamp = val.to_le_bytes();
    }

    pub fn compute_crc(&self) -> u32 {
        let bytes = bytemuck::bytes_of(self);
        crc32c::crc32c(&bytes[0..172])
    }

    pub fn is_crc_valid(&self) -> bool {
        self.record_crc32 == self.compute_crc()
    }

    pub fn signed_bytes(&self) -> &[u8] {
        let bytes = bytemuck::bytes_of(self);
        &bytes[0..176]
    }
}

/// KeyRotationRecord (220 Bytes, 8-Byte Aligned, Little-Endian) (§6.1).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct KeyRotationRecord {
    pub rotation_id: [u8; 16],
    pub old_public_key: [u8; 32],
    pub new_public_key: [u8; 32],
    pub rotated_at: [u8; 8],
    pub superseded_sig: [u8; 64],
    pub confirm_sig: [u8; 64],
    pub record_crc32: u32,
}

impl KeyRotationRecord {
    pub fn get_rotated_at(&self) -> u64 {
        u64::from_le_bytes(self.rotated_at)
    }

    pub fn set_rotated_at(&mut self, val: u64) {
        self.rotated_at = val.to_le_bytes();
    }

    pub fn compute_crc(&self) -> u32 {
        let bytes = bytemuck::bytes_of(self);
        crc32c::crc32c(&bytes[0..216])
    }

    pub fn is_crc_valid(&self) -> bool {
        self.record_crc32 == self.compute_crc()
    }

    pub fn signed_bytes(&self) -> &[u8] {
        let bytes = bytemuck::bytes_of(self);
        &bytes[0..88]
    }
}

/// RevokedKeyRecord (64 Bytes, 8-Byte Aligned, Little-Endian) (§6.2).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct RevokedKeyRecord {
    pub revocation_id: [u8; 16],
    pub revoked_public_key: [u8; 32],
    pub revoked_at: [u8; 8],
    pub reason_code: u32,
    pub record_crc32: u32,
}

impl RevokedKeyRecord {
    pub fn get_revoked_at(&self) -> u64 {
        u64::from_le_bytes(self.revoked_at)
    }

    pub fn set_revoked_at(&mut self, val: u64) {
        self.revoked_at = val.to_le_bytes();
    }

    pub fn compute_crc(&self) -> u32 {
        let bytes = bytemuck::bytes_of(self);
        crc32c::crc32c(&bytes[0..60])
    }

    pub fn is_crc_valid(&self) -> bool {
        self.record_crc32 == self.compute_crc()
    }
}

/// ThingIdentityRecord (128 Bytes, 8-Byte Aligned, Little-Endian) (§10.1).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct ThingIdentityRecord {
    pub thing_id: [u8; 16],
    pub model_id: [u8; 16],
    pub serial_number: [u8; 16],
    pub firmware_version: [u8; 8],
    pub public_key: [u8; 32],
    pub provisioned_at: [u8; 8],
    pub operational_flags: u32,
    pub _reserved: [u8; 24],
    pub record_crc32: u32,
}

impl ThingIdentityRecord {
    pub fn get_firmware_version(&self) -> u64 {
        u64::from_le_bytes(self.firmware_version)
    }

    pub fn set_firmware_version(&mut self, val: u64) {
        self.firmware_version = val.to_le_bytes();
    }

    pub fn get_provisioned_at(&self) -> u64 {
        u64::from_le_bytes(self.provisioned_at)
    }

    pub fn set_provisioned_at(&mut self, val: u64) {
        self.provisioned_at = val.to_le_bytes();
    }

    pub fn compute_crc(&self) -> u32 {
        let bytes = bytemuck::bytes_of(self);
        crc32c::crc32c(&bytes[0..124])
    }

    pub fn is_crc_valid(&self) -> bool {
        self.record_crc32 == self.compute_crc()
    }
}

/// GateDirectoryRecord (160 Bytes, 8-Byte Aligned, Little-Endian) (§11.2).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct GateDirectoryRecord {
    pub record_id: [u8; 16],
    pub gate_name: [u8; 32],
    pub gate_address: [u8; 32],
    pub endpoint_ref: [u8; 32],
    pub valid_from: [u8; 8],
    pub valid_until: [u8; 8],
    pub directory_seq: u32,
    pub _reserved: [u8; 24],
    pub record_crc32: u32,
}

impl GateDirectoryRecord {
    pub fn get_valid_from(&self) -> u64 {
        u64::from_le_bytes(self.valid_from)
    }

    pub fn set_valid_from(&mut self, val: u64) {
        self.valid_from = val.to_le_bytes();
    }

    pub fn get_valid_until(&self) -> u64 {
        u64::from_le_bytes(self.valid_until)
    }

    pub fn set_valid_until(&mut self, val: u64) {
        self.valid_until = val.to_le_bytes();
    }

    pub fn compute_crc(&self) -> u32 {
        let bytes = bytemuck::bytes_of(self);
        crc32c::crc32c(&bytes[0..156])
    }

    pub fn is_crc_valid(&self) -> bool {
        self.record_crc32 == self.compute_crc()
    }
}

/// ApprovalCheckpointRecord (192 Bytes, 8-Byte Aligned, Little-Endian) (§12.1).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct ApprovalCheckpointRecord {
    pub checkpoint_id: [u8; 16],
    pub checkpoint_type: u16,
    pub outcome: u16,
    pub target_action: u32,
    pub requested_by: [u8; 16],
    pub target_item_or_task: [u8; 16],
    pub threshold_confidence: u32,
    pub observed_confidence: u32,
    pub evaluated_at: [u8; 8],
    pub expires_at: [u8; 8],
    pub approver_pubkey: [u8; 32],
    pub _reserved: [u8; 12],
    pub record_crc32: u32,
    pub approver_signature: [u8; 64],
}

impl ApprovalCheckpointRecord {
    pub fn get_evaluated_at(&self) -> u64 {
        u64::from_le_bytes(self.evaluated_at)
    }

    pub fn set_evaluated_at(&mut self, val: u64) {
        self.evaluated_at = val.to_le_bytes();
    }

    pub fn get_expires_at(&self) -> u64 {
        u64::from_le_bytes(self.expires_at)
    }

    pub fn set_expires_at(&mut self, val: u64) {
        self.expires_at = val.to_le_bytes();
    }

    pub fn compute_crc(&self) -> u32 {
        let bytes = bytemuck::bytes_of(self);
        crc32c::crc32c(&bytes[0..124])
    }

    pub fn is_crc_valid(&self) -> bool {
        self.record_crc32 == self.compute_crc()
    }

    pub fn signed_bytes(&self) -> &[u8] {
        let bytes = bytemuck::bytes_of(self);
        &bytes[0..128]
    }
}

/// VaultEventRecord (80 Bytes, 8-Byte Aligned) (§7.1).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct VaultEventRecord {
    pub event_sequence: [u8; 8],
    pub timestamp: [u8; 8],
    pub event_type: u16,
    pub actor_kind: u16,
    pub target_id: [u8; 16],
    pub prev_event_hash: [u8; 32],
    pub grant_id_suffix: [u8; 8],
    pub record_crc32: u32,
}

impl VaultEventRecord {
    pub fn get_event_sequence(&self) -> u64 {
        u64::from_le_bytes(self.event_sequence)
    }

    pub fn set_event_sequence(&mut self, val: u64) {
        self.event_sequence = val.to_le_bytes();
    }

    pub fn get_timestamp(&self) -> u64 {
        u64::from_le_bytes(self.timestamp)
    }

    pub fn set_timestamp(&mut self, val: u64) {
        self.timestamp = val.to_le_bytes();
    }

    pub fn get_grant_id_suffix(&self) -> u64 {
        u64::from_le_bytes(self.grant_id_suffix)
    }

    pub fn set_grant_id_suffix(&mut self, val: u64) {
        self.grant_id_suffix = val.to_le_bytes();
    }

    pub fn compute_crc(&self) -> u32 {
        let bytes = bytemuck::bytes_of(self);
        crc32c::crc32c(&bytes[0..76])
    }

    pub fn is_crc_valid(&self) -> bool {
        self.record_crc32 == self.compute_crc()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_sizes() {
        assert_eq!(std::mem::size_of::<VaultIdentityRecord>(), 96);
        assert_eq!(std::mem::size_of::<DelegationGrantRecord>(), 192);
        assert_eq!(std::mem::size_of::<VaultItemRecord>(), 160);
        assert_eq!(std::mem::size_of::<ItemTransferRecord>(), 240);
        assert_eq!(std::mem::size_of::<KeyRotationRecord>(), 220);
        assert_eq!(std::mem::size_of::<RevokedKeyRecord>(), 64);
        assert_eq!(std::mem::size_of::<ThingIdentityRecord>(), 128);
        assert_eq!(std::mem::size_of::<GateDirectoryRecord>(), 160);
        assert_eq!(std::mem::size_of::<ApprovalCheckpointRecord>(), 192);
        assert_eq!(std::mem::size_of::<VaultEventRecord>(), 80);
    }
}
