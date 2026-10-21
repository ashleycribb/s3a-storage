use s3a_edge::*;
use s3a_engine::*;
use std::fs;

#[test]
fn test_edge_to_full_engine_import_and_deduplication() {
    let dir = std::env::temp_dir().join(format!("s3a_edge_interop_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();

    let edge_file_path = dir.join("sensor_device.edge.s3a");
    let master_file_path = dir.join("master_database.s3a");

    let device_id = [0x99u8; 16];
    let file_uuid = [0x55AA55AA, 0x12345678];
    let now = 1700000000u64;

    // 1. Generate an S3A Edge archive using EdgeWriter
    let flash = RamFlash::<{ 64 * 1024 }>::new(256, 512);
    let mut writer =
        EdgeWriter::<_, 4096>::open_or_format(flash, device_id, file_uuid, now).unwrap();

    // Tile 0 & 1: Genesis Provisioning (Consent + Device Identity)
    let consent = EdgeConsentRecord::new(
        [0x77; 16],
        PURPOSE_OPERATIONAL_MAINTENANCE | PURPOSE_SAFETY_AUDIT,
        SCOPE_TELEMETRY | SCOPE_EQUIPMENT_FAULTS,
        now,
        0,
        999,
    );
    let identity = DeviceIdentityRecord::new(
        device_id,
        "TURBINE-EDGE-1",
        [2, 1, 0, 0],
        [0xEE; 32],
        now,
        1,
    );
    let genesis_seal = writer.provision(&consent, &identity, now).unwrap();
    assert_ne!(genesis_seal, [0u8; 32]);

    // Tile 2: Sensor telemetry records
    for i in 0..20 {
        let rec = CompactEdgeSensorRecord::new(
            (i * 100) as u16,
            1,
            1,
            3000 + (i as i32) * 10,
            QUALITY_OK,
        );
        writer.append_sensor_record(&rec).unwrap();
    }
    let seal2 = writer.seal_tile(now + 10).unwrap();
    assert_ne!(seal2, genesis_seal);

    // Tile 3: Industrial equipment events
    let ev = EquipmentEventRecord::new(
        now + 15,
        EVENT_MACHINE_START,
        SEVERITY_INFO,
        1,
        10,
        [0x11; 8],
        1800,
    );
    writer.append_equipment_event(&ev).unwrap();
    let seal3 = writer.seal_tile(now + 20).unwrap();
    assert_ne!(seal3, seal2);

    assert_eq!(writer.tile_count(), 4);

    // Save edge memory to disk file
    let edge_bytes = writer.storage().as_slice();
    let file_extent = 512 + 4 * 4096;
    fs::write(&edge_file_path, &edge_bytes[..file_extent]).unwrap();

    // 2. Full S3A Engine verification: Open edge archive directly with MmapReader
    let edge_reader = MmapReader::open(&edge_file_path).expect("Full engine must open edge file");
    assert!(
        edge_reader.is_edge_archive(),
        "Reader must detect edge profile"
    );
    assert_eq!(edge_reader.tile_stride(), 4096);
    assert_eq!(edge_reader.tile_count(), 4);

    // Invariant: Full engine verifies checksums of edge micro-tiles natively
    edge_reader
        .verify_checksums()
        .expect("Full engine checksum verification must pass");

    // Invariant: Full engine losslessly promotes micro-tiles to HyperTileHeader
    for i in 0..4 {
        let (promoted, payload) = edge_reader
            .get_promoted_tile(i)
            .expect("Tile promotion must succeed");
        assert_eq!(promoted.data_crc32, crc32c::crc32c(payload));
        assert_ne!(promoted.provenance.merkle_root, [0u8; 32]);
    }

    // 3. First Import: Merge edge archive into full engine master archive
    let report1 = import_edge_archive(&edge_file_path, &master_file_path)
        .expect("Importing edge archive into master must succeed");

    assert_eq!(report1.imported_tiles, 4);
    assert_eq!(report1.skipped_duplicates, 0);
    assert_eq!(report1.device_id, device_id);
    assert!(
        report1.seal_chain_verified,
        "Seal chain must be verified back to genesis"
    );
    assert_eq!(report1.final_seal_hash, seal3);

    // Verify master archive now contains 4 Hyper-Tiles
    let master_reader = MmapReader::open(&master_file_path).unwrap();
    assert_eq!(master_reader.tile_count(), 4);
    assert!(
        !master_reader.is_edge_archive(),
        "Master database is standard Hyper-Tile format"
    );
    master_reader
        .verify_checksums()
        .expect("Master database checksums must pass");

    // Verify provenance retention on master tiles
    for i in 0..4 {
        let (hdr, _) = master_reader.get_tile(i).unwrap();
        let expected_device_low = u64::from_le_bytes(device_id[..8].try_into().unwrap());
        assert_eq!(
            hdr.security.tenant_id, expected_device_low,
            "Originating device_id provenance must be retained"
        );
    }

    // 4. Duplicate Import: Ingesting the same edge archive again must skip all duplicates
    let report2 = import_edge_archive(&edge_file_path, &master_file_path)
        .expect("Second import of identical edge archive must succeed");

    assert_eq!(
        report2.imported_tiles, 0,
        "No duplicate tiles should be imported"
    );
    assert_eq!(
        report2.skipped_duplicates, 4,
        "All 4 duplicate tiles must be skipped by payload hash"
    );
    assert_eq!(report2.device_id, device_id);

    // Master database still has exactly 4 tiles
    let master_reader_after = MmapReader::open(&master_file_path).unwrap();
    assert_eq!(master_reader_after.tile_count(), 4);

    let _ = fs::remove_dir_all(dir);
}
