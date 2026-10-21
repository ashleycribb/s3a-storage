use s3a_edge::*;

#[test]
fn test_append_and_seal_roundtrip() {
    let flash = RamFlash::<{ 64 * 1024 }>::new(256, 512);
    let device_id = [0x42u8; 16];
    let file_uuid = [0x11223344, 0x55667788];
    let now = 1700000000u64;

    let mut writer = EdgeWriter::<_, 4096>::open_or_format(flash, device_id, file_uuid, now)
        .expect("format should succeed");

    // Tile 0: Provisioning (Consent + Identity)
    let consent = EdgeConsentRecord::new(
        [0x99; 16],
        PURPOSE_OPERATIONAL_MAINTENANCE | PURPOSE_SAFETY_AUDIT,
        SCOPE_TELEMETRY | SCOPE_EQUIPMENT_FAULTS,
        now,
        0,
        1001,
    );
    let identity =
        DeviceIdentityRecord::new(device_id, "SENS-NODE-V1", [1, 0, 0, 1], [0xAA; 32], now, 1);
    let seal0 = writer
        .provision(&consent, &identity, now)
        .expect("provisioning tile should seal");
    assert_eq!(writer.tile_count(), 2);
    assert_ne!(seal0, [0u8; 32]);

    // Tile 2: Telemetry Sensor Records
    for i in 0..50 {
        let rec =
            CompactEdgeSensorRecord::new((i * 100) as u16, 1, 1, 2000 + (i as i32) * 5, QUALITY_OK);
        writer
            .append_sensor_record(&rec)
            .expect("sensor append should succeed");
    }
    let seal1 = writer
        .seal_tile(now + 5)
        .expect("telemetry tile should seal");
    assert_eq!(writer.tile_count(), 3);
    assert_ne!(seal1, seal0);

    // Tile 3: Industrial Equipment Event Logs
    let ev1 = EquipmentEventRecord::new(
        now + 10,
        EVENT_MACHINE_START,
        SEVERITY_INFO,
        1,
        101,
        [0xEE; 8],
        1500, // 1500 RPM
    );
    let ev2 = EquipmentEventRecord::new(
        now + 15,
        EVENT_FAULT_CODE,
        SEVERITY_ALARM,
        1,
        404,
        [0xEE; 8],
        85, // 85 degC
    );
    writer
        .append_equipment_event(&ev1)
        .expect("event append should succeed");
    writer
        .append_equipment_event(&ev2)
        .expect("event append should succeed");
    let seal2 = writer.seal_tile(now + 20).expect("event tile should seal");
    assert_eq!(writer.tile_count(), 4);
    assert_ne!(seal2, seal1);

    // Recover from the flash state and verify everything persists cleanly
    let flash_state = writer.storage().clone();
    let recovered =
        EdgeWriter::<_, 4096>::recover(flash_state, device_id).expect("recover should succeed");

    assert_eq!(recovered.tile_count(), 4);
    assert_eq!(recovered.generation(), 5); // Genesis(1) + 4 seals = 5
    assert_eq!(recovered.prev_seal_hash(), &seal2);
    assert_eq!(recovered.file_uuid(), file_uuid);
}

#[test]
fn test_power_cut_at_every_byte_offset_of_tile_write() {
    let device_id = [0x55u8; 16];
    let file_uuid = [0x1234, 0x5678];
    let now = 1700000000u64;

    // 1. Setup clean baseline archive with Tile 0 sealed
    let flash = RamFlash::<{ 32 * 1024 }>::new(256, 512);
    let mut writer =
        EdgeWriter::<_, 4096>::open_or_format(flash, device_id, file_uuid, now).unwrap();

    let consent = EdgeConsentRecord::new(
        [1; 16],
        PURPOSE_OPERATIONAL_MAINTENANCE,
        SCOPE_TELEMETRY,
        now,
        0,
        1,
    );
    let identity = DeviceIdentityRecord::new(device_id, "TEST-DEV", [1, 0, 0, 0], [0; 32], now, 1);
    let seal0 = writer.provision(&consent, &identity, now).unwrap();
    assert_eq!(writer.tile_count(), 2);

    let baseline_flash = writer.storage().clone();

    // Measure total write operations when writing and sealing Tile 2
    let mut test_writer =
        EdgeWriter::<_, 4096>::recover(baseline_flash.clone(), device_id).unwrap();
    let rec = CompactEdgeSensorRecord::new(100, 1, 1, 2500, QUALITY_OK);
    test_writer.append_sensor_record(&rec).unwrap();
    test_writer.storage_mut().reset_write_counter();
    test_writer.seal_tile(now + 10).unwrap();
    let total_bytes_written = test_writer.storage().total_bytes_written;
    assert!(
        total_bytes_written > 0,
        "seal_tile must perform write operations"
    );

    // 2. Cut power at every single byte offset: from 0 up to total_bytes_written - 1
    for cut_byte in 0..total_bytes_written {
        let mut sim_flash = baseline_flash.clone();
        sim_flash.set_write_limit(Some(cut_byte));

        let mut sim_writer = EdgeWriter::<_, 4096>::recover(sim_flash, device_id).unwrap();
        let rec = CompactEdgeSensorRecord::new(100, 1, 1, 2500, QUALITY_OK);
        sim_writer.append_sensor_record(&rec).unwrap();

        let seal_res = sim_writer.seal_tile(now + 10);
        assert!(
            seal_res.is_err(),
            "Power cut at byte {} must cause write failure",
            cut_byte
        );

        // Power failure simulated! Extract memory image after the cut:
        let cut_memory = sim_writer.storage().clone();

        // 3. Perform boot-time recovery on the power-cut flash:
        // Core invariant: Boot recovery MUST succeed, keep all prior sealed tiles (Tile 0 & 1),
        // never corrupt flash, and be ready to resume appending.
        let mut recovered =
            EdgeWriter::<_, 4096>::recover(cut_memory, device_id).unwrap_or_else(|e| {
                panic!(
                    "Recovery failed after power cut at byte {}: {:?}",
                    cut_byte, e
                )
            });
        recovered.storage_mut().set_write_limit(None);

        let count = recovered.tile_count();
        assert!(
            count == 2 || count == 3,
            "After power cut at byte {}, recovered count must be 2 (rolled back) or 3 (committed), got {}",
            cut_byte,
            count
        );

        if count == 2 {
            assert_eq!(
                recovered.prev_seal_hash(),
                &seal0,
                "Seal chain must preserve Tile 1's seal hash after cut at byte {}",
                cut_byte
            );
        }

        // Resume writing: Must cleanly append and seal the next tile without error or corruption
        let fresh_rec = CompactEdgeSensorRecord::new(200, 1, 1, 2600, QUALITY_OK);
        recovered.append_sensor_record(&fresh_rec).unwrap();
        let resumed_seal = recovered.seal_tile(now + 20).unwrap();
        assert_eq!(recovered.tile_count(), count + 1);
        assert_ne!(resumed_seal, seal0);
    }
}

#[test]
fn test_crc_mismatch_rejected() {
    // 1. Record CRC mismatch
    let rec = CompactEdgeSensorRecord::new(100, 1, 1, 200, QUALITY_OK);
    assert!(rec.verify());

    let mut bad_rec = rec;
    bad_rec.quantized_val = 9999;
    assert!(
        !bad_rec.verify(),
        "Modified record must fail CRC verification"
    );

    let ev = EquipmentEventRecord::new(
        1700000000,
        EVENT_MACHINE_START,
        SEVERITY_INFO,
        1,
        10,
        [0; 8],
        100,
    );
    assert!(ev.verify());
    let mut bad_ev = ev;
    bad_ev.param1 = 999;
    assert!(
        !bad_ev.verify(),
        "Modified event record must fail CRC verification"
    );

    // 2. Tile header CRC mismatch
    let mut tile_hdr = CommonTileHeader::new(TILE_TYPE_RECORD, 5, 160);
    tile_hdr.data_crc32 = 0x12345678;
    tile_hdr.header_crc32 = compute_common_tile_header_crc32(&tile_hdr);
    assert!(tile_hdr.verify());

    tile_hdr.record_count = 6;
    assert!(
        !tile_hdr.verify(),
        "Tampered tile header must fail CRC verification"
    );

    // 3. File header CRC mismatch
    let mut file_hdr = FileHeader::new(2, 4096, 1700000000);
    file_hdr.header_crc32 = compute_file_header_crc32(&file_hdr);
    assert!(file_hdr.verify());

    file_hdr.tile_count = 3;
    assert!(
        !file_hdr.verify(),
        "Tampered file header must fail CRC verification"
    );
}

#[test]
fn test_seal_chain_break_detected() {
    let device_id = [0x77u8; 16];
    let prev_seal = [0xAAu8; 32];
    let payload = [0x11u8; 32];
    let seal1 = compute_seal_hash(&prev_seal, 2, 1, &payload, &device_id);

    // Tampered payload breaks link
    let tampered_payload = [0x22u8; 32];
    assert!(!trust::verify_seal_chain_link(
        &seal1,
        &prev_seal,
        2,
        1,
        &tampered_payload,
        &device_id
    ));

    // Tampered prev_seal breaks link
    let tampered_prev = [0xBBu8; 32];
    assert!(!trust::verify_seal_chain_link(
        &seal1,
        &tampered_prev,
        2,
        1,
        &payload,
        &device_id
    ));

    // Reordered tile index breaks link
    assert!(!trust::verify_seal_chain_link(
        &seal1, &prev_seal, 2, 2, &payload, &device_id
    ));

    // Swapped device_id breaks link
    let foreign_device = [0x88u8; 16];
    assert!(!trust::verify_seal_chain_link(
        &seal1,
        &prev_seal,
        2,
        1,
        &payload,
        &foreign_device
    ));
}

#[test]
fn test_timeseries_compression_ratio_on_sample() {
    // Generate realistic sensor telemetry (temperature sensor with minor fluctuations)
    let mut records = [CompactEdgeSensorRecord::new(0, 1, 1, 0, QUALITY_OK); 100];
    let mut cur_temp_fixed = 2500i32; // 25.00 degC
    for i in 0..100 {
        cur_temp_fixed += (i as i32 % 5) - 2; // -2 to +2 delta
        records[i] = CompactEdgeSensorRecord::new(
            (i * 50) as u16, // delta 50ms
            1,
            1,
            cur_temp_fixed,
            QUALITY_OK,
        );
    }

    let uncompressed_bytes = records.len() * core::mem::size_of::<CompactEdgeSensorRecord>();
    assert_eq!(uncompressed_bytes, 1600); // 100 * 16 bytes = 1600 bytes

    let mut compressed_buf = [0u8; 1600];
    let compressed_len = compress_sensor_records(&records, &mut compressed_buf);
    assert!(compressed_len > 0);

    let ratio = (uncompressed_bytes as f64) / (compressed_len as f64);
    assert!(
        ratio > 2.0,
        "Compression ratio must be > 2.0x, got {:.2}x ({} -> {} bytes)",
        ratio,
        uncompressed_bytes,
        compressed_len
    );
}

#[cfg(feature = "sign")]
#[test]
fn test_signature_verify_and_forge_rejected() {
    use ed25519_dalek::SigningKey;

    let device_key = SigningKey::from_bytes(&[123u8; 32]);
    let device_pubkey = device_key.verifying_key();

    let seal_hash = [0x55u8; 32];
    let sig = trust::sign_seal_hash(&device_key, &seal_hash);

    // Valid signature verifies
    assert!(trust::verify_signature(&device_pubkey, &seal_hash, &sig));

    // Tampered seal hash rejected
    let tampered_hash = [0x56u8; 32];
    assert!(!trust::verify_signature(
        &device_pubkey,
        &tampered_hash,
        &sig
    ));

    // Forgery with attacker key rejected
    let attacker_key = SigningKey::from_bytes(&[234u8; 32]);
    let forged_sig = trust::sign_seal_hash(&attacker_key, &seal_hash);
    assert!(!trust::verify_signature(
        &device_pubkey,
        &seal_hash,
        &forged_sig
    ));
}
