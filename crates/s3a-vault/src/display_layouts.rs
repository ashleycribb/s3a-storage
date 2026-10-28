//! Read-only display layouts for the vault's records (all stored in tile type 64).
//! Metadata only; see [`s3a_core::display_layout`]. A vault tile holds one record kind, so pick the
//! layout by record size (`payload_len / record_count`), the same rule `Vault::open` uses.

use s3a_core::display_layout;
use s3a_core::display_layout::{ft, DisplayLayoutRegistry};
use s3a_core::schema::SchemaTypeDefinition;
use s3a_core::TileType;

use crate::records::{
    DelegationGrantRecord, ItemTransferRecord, KeyRotationRecord, RevokedKeyRecord,
    VaultEventRecord, VaultIdentityRecord, VaultItemRecord,
};

/// Layouts for the seven vault record kinds in tile type 64.
pub fn display_layouts() -> Vec<SchemaTypeDefinition> {
    let t = TileType::RECORD;
    vec![
        display_layout!(
            VaultIdentityRecord,
            t,
            1,
            "vault_identity",
            "vault",
            [
                (identity_id, ft::UUID),
                (owner_kind, ft::U16),
                (key_algorithm, ft::U16),
                (public_key, ft::BYTES),
                (display_name, ft::STRING),
                (created_at, ft::U64),
                (record_crc32, ft::U32),
            ]
        ),
        display_layout!(
            DelegationGrantRecord,
            t,
            1,
            "delegation_grant",
            "vault",
            [
                (grant_id, ft::UUID),
                (granter_pubkey, ft::BYTES),
                (agent_id, ft::UUID),
                (agent_pubkey, ft::BYTES),
                (allowed_actions, ft::U32),
                (allowed_scopes, ft::U32),
                (max_operations, ft::U32),
                (issued_at, ft::U64),
                (expires_at, ft::U64),
                (granter_signature, ft::BYTES),
            ]
        ),
        display_layout!(
            VaultItemRecord,
            t,
            1,
            "vault_item",
            "vault",
            [
                (item_id, ft::UUID),
                (item_type, ft::U16),
                (item_flags, ft::U16),
                (item_hash, ft::BYTES),
                (issuer_pubkey, ft::BYTES),
                (current_owner_pubkey, ft::BYTES),
                (acquired_at, ft::U64),
                (consent_scope, ft::U32),
                (payload_ref, ft::BYTES),
                (record_crc32, ft::U32),
            ]
        ),
        display_layout!(
            ItemTransferRecord,
            t,
            1,
            "item_transfer",
            "vault",
            [
                (transfer_id, ft::UUID),
                (item_hash, ft::BYTES),
                (from_pubkey, ft::BYTES),
                (to_pubkey, ft::BYTES),
                (timestamp, ft::U64),
                (prev_transfer_hash, ft::BYTES),
                (transfer_mode, ft::U16),
                (record_crc32, ft::U32),
                (sender_signature, ft::BYTES),
            ]
        ),
        display_layout!(
            KeyRotationRecord,
            t,
            1,
            "key_rotation",
            "vault",
            [
                (rotation_id, ft::UUID),
                (old_public_key, ft::BYTES),
                (new_public_key, ft::BYTES),
                (rotated_at, ft::U64),
                (superseded_sig, ft::BYTES),
                (confirm_sig, ft::BYTES),
                (record_crc32, ft::U32),
            ]
        ),
        display_layout!(
            RevokedKeyRecord,
            t,
            1,
            "revoked_key",
            "vault",
            [
                (revocation_id, ft::UUID),
                (revoked_public_key, ft::BYTES),
                (revoked_at, ft::U64),
                (reason_code, ft::U32),
                (record_crc32, ft::U32),
            ]
        ),
        display_layout!(
            VaultEventRecord,
            t,
            1,
            "vault_event",
            "vault",
            [
                (event_sequence, ft::U64),
                (timestamp, ft::U64),
                (event_type, ft::U16),
                (actor_kind, ft::U16),
                (target_id, ft::UUID),
                (prev_event_hash, ft::BYTES),
                (grant_id_suffix, ft::U64),
                (record_crc32, ft::U32),
            ]
        ),
    ]
}

/// A validated registry of [`display_layouts`].
pub fn display_registry() -> DisplayLayoutRegistry {
    DisplayLayoutRegistry::from_layouts(display_layouts())
        .expect("built-in vault display layouts are valid")
}
