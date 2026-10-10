use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

use bytemuck::bytes_of;
use s3a_core::{
    S3AError, TelemetryRecord, TileType, HYPER_TILE_HEADER_SIZE, HYPER_TILE_SIZE,
    SEALED_ENDCAP_SIZE,
};
use s3a_engine::{
    attach_anchor_to_file, compute_seal_hash_v2, compute_sealed_bytes_fingerprint,
    read_sealed_index_v2, seal_archive_v1, seal_archive_v2, verify_seal_chain,
    verify_sealed_archive, AnchorProof, SealOptionsV2, TileWriter,
};

struct TempTestFile {
    path: PathBuf,
}

impl TempTestFile {
    fn new(suffix: &str) -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("s3a_sealed_v2_{}_{}", unique, suffix));
        Self { path }
    }
}

impl Drop for TempTestFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

struct TrackingReader<R> {
    inner: R,
    pub read_ranges: Vec<(u64, usize)>,
}

impl<R: Seek> TrackingReader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            read_ranges: Vec::new(),
        }
    }
}

impl<R: Read + Seek> Read for TrackingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let pos = self.inner.stream_position()?;
        let n = self.inner.read(buf)?;
        if n > 0 {
            self.read_ranges.push((pos, n));
        }
        Ok(n)
    }
}

impl<R: Seek> Seek for TrackingReader<R> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

#[test]
fn test_index_lookup_reads_only_footer_bytes() {
    let raw = TempTestFile::new("raw.s3a");
    let sealed = TempTestFile::new("sealed.s3a");

    let mut writer = TileWriter::create(&raw.path).unwrap();
    for i in 0..3 {
        let recs = vec![
            TelemetryRecord::new(100 + i, 1, 1, 10.0 * (i as f64)),
            TelemetryRecord::new(200 + i, 1, 2, 20.0 * (i as f64)),
        ];
        writer.write_hyper_tile(TileType::TELEMETRY, &recs, None, None).unwrap();
    }
    drop(writer);

    let opts = SealOptionsV2 {
        enable_parity: false,
        k: 16,
        m: 2,
        prev_seal_hash: [0u8; 32],
        seal_generation: 1,
    };
    seal_archive_v2(&raw.path, &sealed.path, &opts).unwrap();

    let file = std::fs::File::open(&sealed.path).unwrap();
    let file_len = file.metadata().unwrap().len();
    let mut tracker = TrackingReader::new(file);

    let (endcap, header, entries) = read_sealed_index_v2(&mut tracker, file_len).unwrap();
    assert_eq!(endcap.footer_version, 2);
    assert_eq!(header.tile_count, 3);
    assert_eq!(entries.len(), 3);

    // Verify all reads occurred strictly within the footer range (>= index_offset)
    assert!(!tracker.read_ranges.is_empty());
    for &(pos, len) in &tracker.read_ranges {
        assert!(
            pos >= endcap.index_offset || pos >= file_len - (SEALED_ENDCAP_SIZE as u64),
            "Read at offset {} (len {}) touched tile body data! Expected >= index_offset {}",
            pos,
            len,
            endcap.index_offset
        );
    }
}

#[test]
fn test_corrupted_index_hash_rejected() {
    let raw = TempTestFile::new("raw.s3a");
    let sealed = TempTestFile::new("sealed.s3a");

    let mut writer = TileWriter::create(&raw.path).unwrap();
    let recs = vec![TelemetryRecord::new(100, 1, 1, 42.0)];
    writer.write_hyper_tile(TileType::TELEMETRY, &recs, None, None).unwrap();
    drop(writer);

    let opts = SealOptionsV2::default();
    seal_archive_v2(&raw.path, &sealed.path, &opts).unwrap();

    // Corrupt index_hash in the 64-byte end cap
    let mut bytes = std::fs::read(&sealed.path).unwrap();
    let len = bytes.len();
    let mut endcap: s3a_core::SealedEndCapV2 =
        bytemuck::pod_read_unaligned(&bytes[len - SEALED_ENDCAP_SIZE..]);
    endcap.index_hash[0] ^= 0xFF; // corrupt index_hash
    endcap.endcap_crc32 = s3a_engine::compute_endcap_crc32_v2(&endcap);
    bytes[len - SEALED_ENDCAP_SIZE..].copy_from_slice(bytes_of(&endcap));
    std::fs::write(&sealed.path, &bytes).unwrap();

    let mut file = std::fs::File::open(&sealed.path).unwrap();
    let err = read_sealed_index_v2(&mut file, len as u64).unwrap_err();
    assert_eq!(err, S3AError::SealedIndexHashMismatch);

    let v_err = verify_sealed_archive(&sealed.path, None).unwrap_err();
    assert_eq!(v_err, S3AError::SealedIndexHashMismatch);
}

#[test]
fn test_seal_hash_chain_verification_and_break_detection() {
    let file_uuid = [12345u64, 67890u64];
    let entries = vec![s3a_core::SealedTileIndexEntry {
        offset: 512,
        length: HYPER_TILE_SIZE as u64,
        type_id: 1,
        type_version: 1,
        _pad: [0; 2],
        record_count: 5,
        tile_index: 0,
        min_key: 100,
        max_key: 200,
        payload_hash: [1u8; 32],
    }];

    // Generate seal 1 (gen 1)
    let prev_0 = [0u8; 32];
    let seal_hash_1 = compute_seal_hash_v2(&prev_0, 1, 1, file_uuid, &entries);
    let seal1 = s3a_core::SealedIndexHeaderV2 {
        magic: s3a_core::SEALED_INDEX_MAGIC_V2,
        tile_count: 1,
        flags: 0,
        k: 16,
        m: 2,
        _padding: [0; 6],
        file_uuid,
        prev_seal_hash: prev_0,
        seal_generation: 1,
        sealed_timestamp: 1000,
        seal_hash: seal_hash_1,
        _reserved: [0; 8],
    };

    // Generate seal 2 (gen 2)
    let seal_hash_2 = compute_seal_hash_v2(&seal_hash_1, 2, 1, file_uuid, &entries);
    let seal2 = s3a_core::SealedIndexHeaderV2 {
        magic: s3a_core::SEALED_INDEX_MAGIC_V2,
        tile_count: 1,
        flags: 0,
        k: 16,
        m: 2,
        _padding: [0; 6],
        file_uuid,
        prev_seal_hash: seal_hash_1,
        seal_generation: 2,
        sealed_timestamp: 2000,
        seal_hash: seal_hash_2,
        _reserved: [0; 8],
    };

    // Generate seal 3 (gen 3)
    let seal_hash_3 = compute_seal_hash_v2(&seal_hash_2, 3, 1, file_uuid, &entries);
    let seal3 = s3a_core::SealedIndexHeaderV2 {
        magic: s3a_core::SEALED_INDEX_MAGIC_V2,
        tile_count: 1,
        flags: 0,
        k: 16,
        m: 2,
        _padding: [0; 6],
        file_uuid,
        prev_seal_hash: seal_hash_2,
        seal_generation: 3,
        sealed_timestamp: 3000,
        seal_hash: seal_hash_3,
        _reserved: [0; 8],
    };

    // 1. Valid chain passes
    assert!(verify_seal_chain(&[seal1, seal2, seal3]).is_ok());

    // 2. Chain break (tampered prev_seal_hash in seal 2)
    let mut broken_seal2 = seal2;
    broken_seal2.prev_seal_hash[0] ^= 0xAA;
    assert_eq!(
        verify_seal_chain(&[seal1, broken_seal2, seal3]).unwrap_err(),
        S3AError::SealedChainBreak
    );

    // 3. Root seal with non-zero prev_seal_hash
    let mut bad_root = seal1;
    bad_root.prev_seal_hash[0] = 1;
    assert_eq!(
        verify_seal_chain(&[bad_root, seal2]).unwrap_err(),
        S3AError::SealedChainBreak
    );

    // 4. Swapped / reordered seals
    assert_eq!(
        verify_seal_chain(&[seal1, seal3, seal2]).unwrap_err(),
        S3AError::SealedSwappedGeneration
    );

    // 5. Skipped generation
    let mut skipped_gen_seal2 = seal2;
    skipped_gen_seal2.seal_generation = 4;
    assert_eq!(
        verify_seal_chain(&[seal1, skipped_gen_seal2]).unwrap_err(),
        S3AError::SealedSwappedGeneration
    );
}

#[test]
fn test_anchors_verify_against_seal_hash() {
    let raw = TempTestFile::new("raw_anchors.s3a");
    let sealed = TempTestFile::new("sealed_anchors.s3a");

    let mut writer = TileWriter::create(&raw.path).unwrap();
    let recs = vec![TelemetryRecord::new(100, 1, 1, 42.0)];
    writer.write_hyper_tile(TileType::TELEMETRY, &recs, None, None).unwrap();
    drop(writer);

    let opts = SealOptionsV2::default();
    seal_archive_v2(&raw.path, &sealed.path, &opts).unwrap();

    let fp = compute_sealed_bytes_fingerprint(&std::fs::read(&sealed.path).unwrap()).unwrap();
    let fp_hex = hex::encode(fp);

    let proof = AnchorProof {
        method: "opentimestamps".to_string(),
        fingerprint: fp_hex.clone(),
        proof_bytes: vec![0x00, 0x01, 0x02],
        submitted_at: 1234567,
        status: "pending".to_string(),
        uri: Some("https://a.pool.opentimestamps.org".to_string()),
        block_height: None,
        block_hash: None,
    };
    attach_anchor_to_file(&sealed.path, proof.clone()).unwrap();

    let report = verify_sealed_archive(&sealed.path, None).unwrap();
    assert_eq!(report.footer_version, 2);
    assert_eq!(report.seal_hash, fp);
    assert_eq!(report.anchors.len(), 1);
    assert_eq!(report.anchors[0].fingerprint, fp_hex);
    assert_eq!(report.anchors[0].method, "opentimestamps");
}

#[test]
fn test_optional_parity_repairs_up_to_m_and_fails_beyond() {
    let raw = TempTestFile::new("raw_parity.s3a");
    let sealed = TempTestFile::new("sealed_parity.s3a");
    let repaired = TempTestFile::new("repaired_parity.s3a");

    let mut writer = TileWriter::create(&raw.path).unwrap();
    for i in 0..4 {
        let recs = vec![TelemetryRecord::new(100 + i, i as u32, 1, 10.0)];
        writer.write_hyper_tile(TileType::TELEMETRY, &recs, None, None).unwrap();
    }
    drop(writer);

    // Seal with RS Parity enabled (k=4, m=2)
    let opts = SealOptionsV2 {
        enable_parity: true,
        k: 4,
        m: 2,
        prev_seal_hash: [0u8; 32],
        seal_generation: 1,
    };
    seal_archive_v2(&raw.path, &sealed.path, &opts).unwrap();
    let pristine_bytes = std::fs::read(&sealed.path).unwrap();

    // 1. Corrupt 1 tile (tile 0) -> RS repairs 1 tile
    let mut bytes_corrupt_1 = pristine_bytes.clone();
    let offset_t0 = s3a_core::FILE_HEADER_SIZE + HYPER_TILE_HEADER_SIZE;
    bytes_corrupt_1[offset_t0] ^= 0xFF;
    std::fs::write(&sealed.path, &bytes_corrupt_1).unwrap();

    let report1 = verify_sealed_archive(&sealed.path, Some(&repaired.path)).unwrap();
    assert_eq!(report1.repaired_tiles, vec![0]);
    let rep1_report = verify_sealed_archive(&repaired.path, None).unwrap();
    assert!(rep1_report.repaired_tiles.is_empty());

    // 2. Corrupt 2 tiles (tile 0 and tile 2) -> RS repairs both tiles
    let mut bytes_corrupt_2 = pristine_bytes.clone();
    let offset_t2 = s3a_core::FILE_HEADER_SIZE + (2 * HYPER_TILE_SIZE) + HYPER_TILE_HEADER_SIZE;
    bytes_corrupt_2[offset_t0] ^= 0xFF;
    bytes_corrupt_2[offset_t2] ^= 0x55;
    std::fs::write(&sealed.path, &bytes_corrupt_2).unwrap();

    let report2 = verify_sealed_archive(&sealed.path, Some(&repaired.path)).unwrap();
    assert_eq!(report2.repaired_tiles.len(), 2);
    assert!(report2.repaired_tiles.contains(&0));
    assert!(report2.repaired_tiles.contains(&2));

    // 3. Corrupt 3 tiles (exceeding M=2) -> UnrepairableCorruption cleanly returned
    let mut bytes_corrupt_3 = pristine_bytes.clone();
    let offset_t1 = s3a_core::FILE_HEADER_SIZE + HYPER_TILE_SIZE + HYPER_TILE_HEADER_SIZE;
    bytes_corrupt_3[offset_t0] ^= 0xFF;
    bytes_corrupt_3[offset_t1] ^= 0xAA;
    bytes_corrupt_3[offset_t2] ^= 0x55;
    std::fs::write(&sealed.path, &bytes_corrupt_3).unwrap();

    let err3 = verify_sealed_archive(&sealed.path, None).unwrap_err();
    match err3 {
        S3AError::UnrepairableCorruption { group_idx, corrupted_count, max_repairable } => {
            assert_eq!(group_idx, 0);
            assert_eq!(corrupted_count, 3);
            assert_eq!(max_repairable, 2);
        }
        other => panic!("Expected UnrepairableCorruption, got {:?}", other),
    }

    // 4. Parity off by default -> corruption returns ChecksumMismatch without repair
    let unparity_sealed = TempTestFile::new("unparity.s3a");
    let unparity_opts = SealOptionsV2 {
        enable_parity: false,
        k: 4,
        m: 2,
        prev_seal_hash: [0u8; 32],
        seal_generation: 1,
    };
    seal_archive_v2(&raw.path, &unparity_sealed.path, &unparity_opts).unwrap();
    let mut unparity_bytes = std::fs::read(&unparity_sealed.path).unwrap();
    unparity_bytes[offset_t0] ^= 0xFF;
    std::fs::write(&unparity_sealed.path, &unparity_bytes).unwrap();

    let unparity_err = verify_sealed_archive(&unparity_sealed.path, None).unwrap_err();
    assert!(matches!(unparity_err, S3AError::ChecksumMismatch { .. }));
}

#[test]
fn test_v1_file_still_opens_and_verifies_backcompat() {
    let raw = TempTestFile::new("raw_v1.s3a");
    let sealed = TempTestFile::new("sealed_v1.s3a");

    let mut writer = TileWriter::create(&raw.path).unwrap();
    let recs = vec![TelemetryRecord::new(100, 1, 1, 42.0)];
    writer.write_hyper_tile(TileType::TELEMETRY, &recs, None, None).unwrap();
    drop(writer);

    // Write with explicit v1 format
    seal_archive_v1(&raw.path, &sealed.path, 16, 2).unwrap();

    // Verify with v2 reader
    let report = verify_sealed_archive(&sealed.path, None).unwrap();
    assert!(report.is_sealed);
    assert_eq!(report.footer_version, 1);
    assert_eq!(report.tile_count, 1);
    assert!(report.warnings.is_empty());
}
