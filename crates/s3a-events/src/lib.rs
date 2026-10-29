#![no_std]

use core::mem::size_of;
use bytemuck::{Pod, Zeroable};
use s3a_core::registry::{TileTypeRegistry, TileTypeDescriptor, TileLifecycle, TileTypeFlags, RegistryError};
use s3a_core::display_layout::{DisplayLayoutRegistry, ft};
use s3a_core::schema::SchemaTypeDefinition;
use s3a_core::display_layout;

extern crate alloc;
use alloc::vec::Vec;

// Constants for tile types
pub const TILE_EVENT: u32 = 112;
pub const TILE_EVENT_RECIPIENT: u32 = 113;
pub const TILE_LEASE: u32 = 114;
pub const TILE_EVENT_INDEX: u32 = 115;

#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Zeroable)]
pub struct EventRecord {
    pub event_id: [u8; 16],
    pub thread_id: [u8; 16],
    pub parent_event_id: [u8; 16],
    pub sender_pubkey: [u8; 32],
    pub recipients_hash: [u8; 32],
    pub payload_hash: [u8; 32],
    pub prev_event_hash: [u8; 32],
    pub created_at_ms: [u8; 8],
    pub sender_seq: [u8; 8],
    pub expires_at_ms: [u8; 8],
    pub kind: u16,
    pub flags: u16,
    pub payload_len: u32,
    pub record_crc32: u32,
    pub _pad: u32,
    pub signature: [u8; 64],
}
unsafe impl Pod for EventRecord {}
const _: () = assert!(size_of::<EventRecord>() == 280);

#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Zeroable)]
pub struct EventRecipientRecord {
    pub event_id: [u8; 16],
    pub recipient_pubkey: [u8; 32],
    pub grant_id: [u8; 16],
    pub delivered_at_ms: [u8; 8],
    pub role: u16,
    pub flags: u16,
    pub record_crc32: u32,
    pub _pad: [u8; 16],
}
unsafe impl Pod for EventRecipientRecord {}
const _: () = assert!(size_of::<EventRecipientRecord>() == 96);

#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Zeroable)]
pub struct LeaseRecord {
    pub lease_id: [u8; 16],
    pub holder_pubkey: [u8; 32],
    pub resource: [u8; 96],
    pub grant_id: [u8; 16],
    pub acquired_at_ms: [u8; 8],
    pub ttl_ms: u32,
    pub action: u16,
    pub flags: u16,
    pub renew_of: [u8; 16],
    pub holder_seq: [u8; 8],
    pub idem_key: [u8; 16],
    pub record_crc32: u32,
    pub _pad: u32,
    pub signature: [u8; 64],
}
unsafe impl Pod for LeaseRecord {}
const _: () = assert!(size_of::<LeaseRecord>() == 288);

#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Zeroable)]
pub struct EventIndexEntry {
    pub key_hash: u64,
    pub event_id: [u8; 16],
    pub created_at_ms: u64,
}
unsafe impl Pod for EventIndexEntry {}
const _: () = assert!(size_of::<EventIndexEntry>() == 32);

pub fn register_pack_types(reg: &mut TileTypeRegistry) -> Result<(), RegistryError> {
    reg.register(TileTypeDescriptor {
        tile_type: TILE_EVENT,
        name: "event_record",
        owner_pack: "s3a-events",
        lifecycle: TileLifecycle::Active,
        required_addon: None,
        record_size: 280,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::FIXED_POD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    reg.register(TileTypeDescriptor {
        tile_type: TILE_EVENT_RECIPIENT,
        name: "event_recipient_record",
        owner_pack: "s3a-events",
        lifecycle: TileLifecycle::Active,
        required_addon: None,
        record_size: 96,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::FIXED_POD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    reg.register(TileTypeDescriptor {
        tile_type: TILE_LEASE,
        name: "lease_record",
        owner_pack: "s3a-events",
        lifecycle: TileLifecycle::Active,
        required_addon: None,
        record_size: 288,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::FIXED_POD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    reg.register(TileTypeDescriptor {
        tile_type: TILE_EVENT_INDEX,
        name: "event_index_derived",
        owner_pack: "s3a-events",
        lifecycle: TileLifecycle::Active,
        required_addon: None,
        record_size: 32,
        current_layout_version: 1,
        min_supported_layout_version: 1,
        flags: TileTypeFlags::FIXED_POD | TileTypeFlags::COPY_THROUGH_COMPACT,
    })?;

    Ok(())
}

pub fn display_layouts() -> Vec<SchemaTypeDefinition> {
    let mut layouts = Vec::new();
    layouts.push(display_layout!(EventRecord, TILE_EVENT, 1, "EventRecord", "s3a-events", [
        (event_id, ft::UUID),
        (thread_id, ft::UUID),
        (parent_event_id, ft::UUID),
        (sender_pubkey, ft::BYTES),
        (recipients_hash, ft::BYTES),
        (payload_hash, ft::BYTES),
        (prev_event_hash, ft::BYTES),
        (created_at_ms, ft::U64),
        (sender_seq, ft::U64),
        (expires_at_ms, ft::U64),
        (kind, ft::U16),
        (flags, ft::U16),
        (payload_len, ft::U32),
        (record_crc32, ft::U32),
        (signature, ft::BYTES),
    ]));
    layouts.push(display_layout!(EventRecipientRecord, TILE_EVENT_RECIPIENT, 1, "EventRecipientRecord", "s3a-events", [
        (event_id, ft::UUID),
        (recipient_pubkey, ft::BYTES),
        (grant_id, ft::UUID),
        (delivered_at_ms, ft::U64),
        (role, ft::U16),
        (flags, ft::U16),
        (record_crc32, ft::U32),
    ]));
    layouts.push(display_layout!(LeaseRecord, TILE_LEASE, 1, "LeaseRecord", "s3a-events", [
        (lease_id, ft::UUID),
        (holder_pubkey, ft::BYTES),
        (resource, ft::STRING),
        (grant_id, ft::UUID),
        (acquired_at_ms, ft::U64),
        (ttl_ms, ft::U32),
        (action, ft::U16),
        (flags, ft::U16),
        (renew_of, ft::UUID),
        (holder_seq, ft::U64),
        (idem_key, ft::UUID),
        (record_crc32, ft::U32),
        (signature, ft::BYTES),
    ]));
    layouts.push(display_layout!(EventIndexEntry, TILE_EVENT_INDEX, 1, "EventIndexEntry", "s3a-events", [
        (key_hash, ft::U64),
        (event_id, ft::UUID),
        (created_at_ms, ft::U64),
    ]));
    layouts
}

pub fn display_registry() -> DisplayLayoutRegistry {
    DisplayLayoutRegistry::from_layouts(display_layouts()).expect("failed to create display registry")
}

pub struct EventVerifier<'a> {
    pub vault: &'a s3a_vault::Vault,
    pub now_ms: u64,
}

#[derive(Debug)]
pub enum EventError {
    BadSignature,
    UnknownSender,
    RevokedSender,
    LegacyAddress,
    PayloadTooLarge,
    BadPayload,
}

impl EventError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::BadSignature => "bad_signature",
            Self::UnknownSender => "unknown_sender",
            Self::RevokedSender => "revoked_sender",
            Self::LegacyAddress => "legacy_address",
            Self::PayloadTooLarge => "payload_too_large",
            Self::BadPayload => "bad_payload",
        }
    }
}


#[derive(Debug, PartialEq, Eq)]
pub enum KeyStatus {
    Valid,
    SinceRevoked { revoked_at: u64 },
}

impl<'a> EventVerifier<'a> {
    pub fn new(vault: &'a s3a_vault::Vault, now_ms: u64) -> Self {
        Self { vault, now_ms }
    }

    pub fn verify_event(&self, event: &EventRecord, _recipients: &[EventRecipientRecord], payload: Option<&[u8]>) -> Result<(), EventError> {
        use ed25519_dalek::{VerifyingKey, Signature};

        // 1. Rebuild payload to verify signature
        let mut msg = Vec::new();
        msg.extend_from_slice(b"s3a-event-v1\0");
        let event_bytes = bytemuck::bytes_of(event);
        msg.extend_from_slice(&event_bytes[..216]); // everything before signature

        let sig = match Signature::from_slice(&event.signature) {
            Ok(s) => s,
            Err(_) => return Err(EventError::BadSignature),
        };

        let vk = match VerifyingKey::from_bytes(&event.sender_pubkey) {
            Ok(k) => k,
            Err(_) => return Err(EventError::BadSignature),
        };

        if vk.verify_strict(&msg, &sig).is_err() {
            return Err(EventError::BadSignature);
        }

        // 2. Check vault identity
        let ids = self.vault.identities();
        let mut found = false;
        for id in ids {
            if id.public_key == event.sender_pubkey {
                found = true;
                break;
            }
        }
        if !found {
            return Err(EventError::UnknownSender);
        }

        // 3. Check revocation
        let rr = self.vault.revocation_registry();
        let event_time_secs = u64::from_le_bytes(event.created_at_ms) / 1000;
        if rr.is_revoked(&event.sender_pubkey) {
            if let Err(_) = rr.check_key_valid(&event.sender_pubkey, event_time_secs) {
                return Err(EventError::RevokedSender);
            }
        }

        // 4. Legacy address check
        let addr = s3a_vault::GateAddress::from_public_key(&event.sender_pubkey);
        if addr.is_legacy() {
            return Err(EventError::LegacyAddress);
        }



        // Payload validation (kinds 1-5 validated, event-definition payload schema validated only)
        if event.kind >= 1 && event.kind <= 5 {
            if let Some(p) = payload {
                if p.len() > 65536 {
                    return Err(EventError::PayloadTooLarge);
                }

                let s = core::str::from_utf8(p).map_err(|_| EventError::BadPayload)?;
                let json: serde_json::Value = serde_json::from_str(s).map_err(|_| EventError::BadPayload)?;

                if !json.is_object() {
                    return Err(EventError::BadPayload);
                }

                if event.kind == 5 {
                    let obj = json.as_object().unwrap();
                    if !obj.contains_key("title") || !obj.contains_key("starts_at_ms") || !obj.contains_key("ends_at_ms") || !obj.contains_key("capacity") || !obj.contains_key("sessions") || !obj.contains_key("recurrence") {
                        // capacity and recurrence are accepted as data and never acted on
                        // we're validating existence
                        return Err(EventError::BadPayload);
                    }
                }
            }
        }

        Ok(())
    }

    pub fn get_key_status(&self, event: &EventRecord) -> KeyStatus {
        let rr = self.vault.revocation_registry();
        if rr.is_revoked(&event.sender_pubkey) {
            panic!("STOP: RevocationRegistry is missing a public API to get the exact `revoked_at` timestamp. Need to report this to Ashley.");
        }
        KeyStatus::Valid
    }


}
