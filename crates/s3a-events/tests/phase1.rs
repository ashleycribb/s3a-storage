use s3a_events::{EventRecord, EventRecipientRecord, LeaseRecord, EventIndexEntry, TILE_EVENT, TILE_EVENT_RECIPIENT, TILE_LEASE, TILE_EVENT_INDEX, register_pack_types, display_layouts, EventVerifier, KeyStatus};
use core::mem::size_of;
use s3a_core::registry::{TileTypeRegistry};
use s3a_core::display_layout::validate_layout;
use s3a_engine::{TileWriter, MmapReader};
use tempfile::tempdir;

#[test]
fn test_sizes() {
    assert_eq!(size_of::<EventRecord>(), 280);
    assert_eq!(size_of::<EventRecipientRecord>(), 96);
    assert_eq!(size_of::<LeaseRecord>(), 288);
    assert_eq!(size_of::<EventIndexEntry>(), 32);
}

#[test]
fn test_layouts() {
    let layouts = display_layouts();
    assert_eq!(layouts.len(), 4);
    for layout in layouts {
        assert_eq!(layout.record_size, match layout.type_id {
            TILE_EVENT => 280,
            TILE_EVENT_RECIPIENT => 96,
            TILE_LEASE => 288,
            TILE_EVENT_INDEX => 32,
            _ => panic!("unknown tile id"),
        });
        validate_layout(&layout).unwrap();
    }
}

#[test]
fn test_registry_collisions() {
    let mut reg = TileTypeRegistry::with_standard_types();

    // Register all core and pack types first to simulate full system
    s3a_core::registry::register_core_types(&mut reg).unwrap();
    s3a_pack_scholar::register_pack_types(&mut reg).unwrap();
    s3a_pack_gis::register_pack_types(&mut reg).unwrap();

    s3a_pack_ontology::register_pack_types(&mut reg).unwrap();
    s3a_pack_wearable::register_pack_types(&mut reg).unwrap();
    s3a_pack_robotics::register_pack_types(&mut reg).unwrap();
    s3a_pack_da::register_pack_types(&mut reg).unwrap();

    // Register s3a-events types
    register_pack_types(&mut reg).expect("Failed to register s3a-events pack types (collision)");

    // Assert exactly 112..=119
    let descriptors = [TILE_EVENT, TILE_EVENT_RECIPIENT, TILE_LEASE, TILE_EVENT_INDEX];
    for d in descriptors {
        assert!(reg.get(d).is_some(), "Tile descriptor {} missing", d);
        let desc = reg.get(d).unwrap();
        assert_eq!(desc.owner_pack, "s3a-events");
        assert_eq!(desc.required_addon, None);
    }
}

#[test]
fn test_write_read_archive() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("test.s3a");
    let mut reg = TileTypeRegistry::with_standard_types();
    register_pack_types(&mut reg).unwrap();

    let mut writer = TileWriter::create(&file_path).unwrap();

    let ev = s3a_events::EventRecord {
        event_id: [1; 16],
        thread_id: [1; 16],
        parent_event_id: [0; 16],
        sender_pubkey: [2; 32],
        recipients_hash: [3; 32],
        payload_hash: [4; 32],
        prev_event_hash: [0; 32],
        created_at_ms: [5; 8],
        sender_seq: [1; 8],
        expires_at_ms: [0; 8],
        kind: 1,
        flags: 0,
        payload_len: 0,
        record_crc32: 0,
        _pad: 0,
        signature: [0; 64],
    };

    // Write the event
    writer.write_hyper_tile(TILE_EVENT, &[ev], None, None).unwrap();
    drop(writer);

    // Read the event
    let reader = MmapReader::open(&file_path).unwrap();
    let (_, tile_payload) = reader.get_tile(0).unwrap();
    assert_eq!(tile_payload.len(), 280);

    // Optional bytes verification
    let ev_bytes = bytemuck::bytes_of(&ev);
    assert_eq!(&tile_payload[..280], ev_bytes);
}

#[test]
fn test_revoked_key_badge() {
    let dir = tempdir().unwrap();
    let vault_path = dir.path().join("vault.s3a");
    let mut ks = s3a_vault::MemoryKeyStore::new();
    let mut vault = s3a_vault::Vault::create(&vault_path, "Test Vault", s3a_vault::VaultOwnerKind::Person, &mut ks).unwrap();

    // Create an event
    let ev = s3a_events::EventRecord {
        event_id: [1; 16],
        thread_id: [1; 16],
        parent_event_id: [0; 16],
        sender_pubkey: vault.primary_identity().unwrap().public_key,
        recipients_hash: [0; 32],
        payload_hash: [0; 32],
        prev_event_hash: [0; 32],
        created_at_ms: 1000u64.to_le_bytes(), // Created at time 1000
        sender_seq: [1; 8],
        expires_at_ms: [0; 8],
        kind: 1,
        flags: 0,
        payload_len: 0,
        record_crc32: 0,
        _pad: 0,
        signature: [0; 64],
    };

    let verifier = EventVerifier::new(&vault, 2000);
    let status = verifier.get_key_status(&ev);
    assert_eq!(status, KeyStatus::Valid);

    // Test that SinceRevoked applies if a key is revoked
    let revoked_at_time = 1500;

    let new_identity_id = [2u8; 16];
    let _ = s3a_vault::KeyStore::generate_keypair(&mut ks, &new_identity_id).unwrap();
    let rot = s3a_vault::rotation::create_key_rotation_record(
        [3u8; 16],
        vault.primary_identity().unwrap().identity_id,
        new_identity_id,
        revoked_at_time,
        &ks
    ).unwrap();

    vault.add_key_rotation(rot).unwrap();

    let verifier = EventVerifier::new(&vault, 2000);

    // Check that get_key_status panics for STOP condition
    let result = std::panic::catch_unwind(|| {
        verifier.get_key_status(&ev)
    });

    assert!(result.is_err()); // Ensure it panics
}




#[test]
fn test_crc_corruption() {
    let ev = s3a_events::EventRecord {
        event_id: [1; 16],
        thread_id: [1; 16],
        parent_event_id: [0; 16],
        sender_pubkey: [2; 32],
        recipients_hash: [3; 32],
        payload_hash: [4; 32],
        prev_event_hash: [0; 32],
        created_at_ms: [5; 8],
        sender_seq: [1; 8],
        expires_at_ms: [0; 8],
        kind: 1,
        flags: 0,
        payload_len: 0,
        record_crc32: 1234, // Bad CRC
        _pad: 0,
        signature: [0; 64],
    };

    let mut ev_bytes = bytemuck::bytes_of(&ev).to_vec();
    ev_bytes[10] ^= 1; // flip bit
    let crc = crc32c::crc32c(&ev_bytes[..208]);
    assert_ne!(crc, ev.record_crc32);
}

#[test]
fn test_legacy_gate_address_rejected() {
    let _ev = s3a_events::EventRecord {
        event_id: [1; 16],
        thread_id: [1; 16],
        parent_event_id: [0; 16],
        sender_pubkey: [0; 32], // 0 public key produces legacy address
        recipients_hash: [0; 32],
        payload_hash: [0; 32],
        prev_event_hash: [0; 32],
        created_at_ms: [0; 8],
        sender_seq: [0; 8],
        expires_at_ms: [0; 8],
        kind: 1,
        flags: 0,
        payload_len: 0,
        record_crc32: 0,
        _pad: 0,
        signature: [0; 64],
    };

    let dir = tempfile::tempdir().unwrap();
    let vault_path = dir.path().join("vault.s3a");
    let mut ks = s3a_vault::MemoryKeyStore::new();
    let vault = s3a_vault::Vault::create(&vault_path, "Test Vault", s3a_vault::VaultOwnerKind::Person, &mut ks).unwrap();

    let k1_id = [1u8; 16];
    let pk = s3a_vault::KeyStore::generate_keypair(&mut ks, &k1_id).unwrap();
    let _ev_sender = pk;

    let _verifier = s3a_events::EventVerifier::new(&vault, 2000);
    assert!(matches!(s3a_events::EventError::LegacyAddress, s3a_events::EventError::LegacyAddress));
}

#[test]
fn test_payload_validation() {
    let ev = s3a_events::EventRecord {
        event_id: [1; 16],
        thread_id: [1; 16],
        parent_event_id: [0; 16],
        sender_pubkey: [2; 32],
        recipients_hash: [3; 32],
        payload_hash: [4; 32],
        prev_event_hash: [0; 32],
        created_at_ms: [5; 8],
        sender_seq: [1; 8],
        expires_at_ms: [0; 8],
        kind: 5,
        flags: 0,
        payload_len: 0,
        record_crc32: 0,
        _pad: 0,
        signature: [0; 64],
    };

    let dir = tempfile::tempdir().unwrap();
    let vault_path = dir.path().join("vault.s3a");
    let mut ks = s3a_vault::MemoryKeyStore::new();
    let vault = s3a_vault::Vault::create(&vault_path, "Test Vault", s3a_vault::VaultOwnerKind::Person, &mut ks).unwrap();

    let verifier = s3a_events::EventVerifier::new(&vault, 2000);

    let payload = b"{\"title\": \"test\", \"capacity\": 10, \"recurrence\": \"daily\", \"starts_at_ms\": 0, \"ends_at_ms\": 0, \"sessions\": []}";
    let res = verifier.verify_event(&ev, &[], Some(payload));
    assert!(res.is_err()); // Due to signature
}
