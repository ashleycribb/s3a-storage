//! Edge Profile Writer Implementation (§3, §4, §6).
//!
//! Features:
//! - Pure `#![no_std]`, zero heap allocation (`no_alloc`).
//! - Fixed RAM staging buffer via const generics (`TILE_SIZE` = 4096 or 8192).
//! - Append + seal only; no query/compaction.
//! - Power-loss safety: Two-phase sector commit protocol (header written last).
//! - Atomic recovery on boot: Scans tiles, truncates incomplete/torn writes, recovers append pointer.
//! - Cryptographic SHA-256 seal hash chain linking every micro-tile to genesis.

use crate::crc::crc32c;
use crate::flash::FlashStorage;
use crate::header::*;
use crate::records::*;
use crate::trust::compute_seal_hash;
use bytemuck::bytes_of;
use sha2::{Digest, Sha256};

/// Errors returned by the S3A Edge Writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeError<E> {
    Storage(E),
    TileFull,
    TileTypeMismatch { expected: u16, actual: u16 },
    CorruptedArchive,
    InvalidMagic,
    UnsupportedVersion(u16),
    HeaderCrcMismatch,
    TileCrcMismatch,
    PayloadOverflow,
    SealChainBreak,
}

impl<E> From<E> for EdgeError<E> {
    fn from(err: E) -> Self {
        EdgeError::Storage(err)
    }
}

/// Zero-allocation, crash-resilient S3A Micro-Tile Flash Writer.
pub struct EdgeWriter<S: FlashStorage, const TILE_SIZE: usize = 4096> {
    storage: S,
    device_id: [u8; 16],
    active_slot: usize,
    generation: u64,
    tile_count: u32,
    file_uuid: [u64; 2],
    created_timestamp: u64,
    addon_mask: u32,
    prev_seal_hash: [u8; 32],
    staging_buffer: [u8; TILE_SIZE],
    staging_type_id: u16,
    staging_record_count: u32,
    staging_len: usize,
}

impl<S: FlashStorage, const TILE_SIZE: usize> EdgeWriter<S, TILE_SIZE> {
    /// Opens an existing S3A edge archive via atomic recovery, or formats a new genesis archive
    /// if the flash is completely blank / erased.
    pub fn open_or_format(
        mut storage: S,
        device_id: [u8; 16],
        file_uuid: [u64; 2],
        created_timestamp: u64,
    ) -> Result<Self, EdgeError<S::Error>> {
        let mut slot_buf = [0u8; FILE_HEADER_SLOT_SIZE];
        let has_valid_slot = if storage
            .read(FILE_HEADER_SLOT_A_OFFSET, &mut slot_buf)
            .is_ok()
        {
            let slot_a: FileHeader = bytemuck::pod_read_unaligned(&slot_buf);
            if slot_a.verify() {
                true
            } else if storage
                .read(FILE_HEADER_SLOT_B_OFFSET, &mut slot_buf)
                .is_ok()
            {
                let slot_b: FileHeader = bytemuck::pod_read_unaligned(&slot_buf);
                slot_b.verify()
            } else {
                false
            }
        } else {
            false
        };

        if has_valid_slot {
            Self::recover(storage, device_id)
        } else {
            Self::format_genesis(storage, device_id, file_uuid, created_timestamp)
        }
    }

    /// Recovers an existing archive from flash.
    /// Fails closed if neither header slot is valid.
    pub fn recover(storage: S, device_id: [u8; 16]) -> Result<Self, EdgeError<S::Error>> {
        Self::recover_internal(storage, device_id)
    }

    fn format_genesis(
        mut storage: S,
        device_id: [u8; 16],
        file_uuid: [u64; 2],
        created_timestamp: u64,
    ) -> Result<Self, EdgeError<S::Error>> {
        // Erase header sectors (512 bytes, at least one sector)
        let sector_size = TILE_SIZE;
        storage.erase(0, sector_size)?;

        let mut slot_a = FileHeader::new(0, TILE_SIZE as u32, created_timestamp);
        slot_a.file_uuid = file_uuid;
        slot_a.header_crc32 = compute_file_header_crc32(&slot_a);

        let dual_header = DualFileHeader::new(slot_a);
        storage.write(0, bytes_of(&dual_header))?;

        let mut staging_buffer = [0xFFu8; TILE_SIZE];
        staging_buffer[..COMMON_TILE_HEADER_SIZE].fill(0xFF);

        Ok(Self {
            storage,
            device_id,
            active_slot: 0,
            generation: 1,
            tile_count: 0,
            file_uuid,
            created_timestamp,
            addon_mask: ADDON_WEARABLE,
            prev_seal_hash: [0u8; 32],
            staging_buffer,
            staging_type_id: 0,
            staging_record_count: 0,
            staging_len: COMMON_TILE_HEADER_SIZE,
        })
    }

    fn recover_internal(mut storage: S, device_id: [u8; 16]) -> Result<Self, EdgeError<S::Error>> {
        let mut slot_buf = [0u8; FILE_HEADER_SLOT_SIZE];

        // Read Slot A
        storage.read(FILE_HEADER_SLOT_A_OFFSET, &mut slot_buf)?;
        let slot_a: FileHeader = bytemuck::pod_read_unaligned(&slot_buf);
        let valid_a = slot_a.verify();

        // Read Slot B
        storage.read(FILE_HEADER_SLOT_B_OFFSET, &mut slot_buf)?;
        let slot_b: FileHeader = bytemuck::pod_read_unaligned(&slot_buf);
        let valid_b = slot_b.verify();

        let (active_slot, active_header) = match (valid_a, valid_b) {
            (true, true) => {
                if slot_b.generation > slot_a.generation {
                    (1, slot_b)
                } else {
                    (0, slot_a)
                }
            }
            (true, false) => (0, slot_a),
            (false, true) => (1, slot_b),
            (false, false) => return Err(EdgeError::CorruptedArchive),
        };

        let mut valid_tile_count = 0u32;
        let mut prev_seal_hash = [0u8; 32];
        let mut header_buf = [0u8; COMMON_TILE_HEADER_SIZE];

        // Scan tile sectors sequentially from offset 512
        for i in 0..active_header.tile_count {
            let tile_offset = FILE_HEADER_SIZE + (i as usize) * TILE_SIZE;
            if tile_offset + TILE_SIZE > storage.total_size() {
                break;
            }

            storage.read(tile_offset, &mut header_buf)?;

            // If header is completely erased (0xFF), this was an uncommitted tile
            if header_buf.iter().all(|&b| b == 0xFF) {
                break;
            }

            let tile_header: CommonTileHeader = bytemuck::pod_read_unaligned(&header_buf);
            if !tile_header.verify() {
                // Corrupted / torn tile header
                break;
            }

            let payload_len = tile_header.payload_len as usize;
            if payload_len > TILE_SIZE - COMMON_TILE_HEADER_SIZE {
                break;
            }

            // Verify payload CRC32C and SHA-256
            let mut payload_buf = [0u8; TILE_SIZE];
            let payload_slice = &mut payload_buf[..payload_len];
            storage.read(tile_offset + COMMON_TILE_HEADER_SIZE, payload_slice)?;

            let actual_crc = crc32c(payload_slice);
            if actual_crc != tile_header.data_crc32 {
                break;
            }

            let mut hasher = Sha256::new();
            hasher.update(payload_slice);
            let actual_hash: [u8; 32] = hasher.finalize().into();
            if actual_hash != tile_header.payload_hash {
                break;
            }

            // Advance seal hash chain
            let tile_gen = if tile_header.generation != 0 {
                tile_header.generation as u64
            } else {
                (active_header
                    .generation
                    .saturating_sub(active_header.tile_count as u64))
                    + (i as u64)
                    + 1
            };
            let seal_hash = compute_seal_hash(
                &prev_seal_hash,
                tile_gen,
                i,
                &tile_header.payload_hash,
                &device_id,
            );
            prev_seal_hash = seal_hash;
            valid_tile_count += 1;
        }

        let mut current_slot = active_slot;
        let mut current_generation = active_header.generation;

        // Crash recovery: if valid tiles < active_header.tile_count, clean torn sector
        // and commit truncated state
        if valid_tile_count < active_header.tile_count {
            let torn_offset = FILE_HEADER_SIZE + (valid_tile_count as usize) * TILE_SIZE;
            if torn_offset + TILE_SIZE <= storage.total_size() {
                let _ = storage.erase(torn_offset, TILE_SIZE);
            }

            // Switch to alternate slot with updated valid tile count
            let next_slot = 1 - current_slot;
            let next_gen = current_generation + 1;
            let mut new_slot = FileHeader::with_commit(
                valid_tile_count,
                TILE_SIZE as u32,
                next_gen,
                active_header.created_timestamp,
                active_header.commit_timestamp,
            );
            new_slot.file_uuid = active_header.file_uuid;
            new_slot.addon_mask = active_header.addon_mask;
            new_slot.header_crc32 = compute_file_header_crc32(&new_slot);

            let slot_offset = if next_slot == 0 {
                FILE_HEADER_SLOT_A_OFFSET
            } else {
                FILE_HEADER_SLOT_B_OFFSET
            };
            storage.write(slot_offset, bytes_of(&new_slot))?;
            current_slot = next_slot;
            current_generation = next_gen;
        }

        let mut staging_buffer = [0xFFu8; TILE_SIZE];
        staging_buffer[..COMMON_TILE_HEADER_SIZE].fill(0xFF);

        Ok(Self {
            storage,
            device_id,
            active_slot: current_slot,
            generation: current_generation,
            tile_count: valid_tile_count,
            file_uuid: active_header.file_uuid,
            created_timestamp: active_header.created_timestamp,
            addon_mask: active_header.addon_mask,
            prev_seal_hash,
            staging_buffer,
            staging_type_id: 0,
            staging_record_count: 0,
            staging_len: COMMON_TILE_HEADER_SIZE,
        })
    }

    /// Appends a raw record into the in-memory staging buffer.
    pub fn append_record(
        &mut self,
        type_id: u16,
        record_bytes: &[u8],
    ) -> Result<(), EdgeError<S::Error>> {
        if self.staging_record_count == 0 {
            self.staging_type_id = type_id;
        } else if self.staging_type_id != type_id {
            return Err(EdgeError::TileTypeMismatch {
                expected: self.staging_type_id,
                actual: type_id,
            });
        }

        if self.staging_len + record_bytes.len() > TILE_SIZE {
            return Err(EdgeError::TileFull);
        }

        self.staging_buffer[self.staging_len..self.staging_len + record_bytes.len()]
            .copy_from_slice(record_bytes);
        self.staging_len += record_bytes.len();
        self.staging_record_count += 1;

        Ok(())
    }

    /// Appends a typed sensor record (§5.1.1).
    pub fn append_sensor_record(
        &mut self,
        record: &CompactEdgeSensorRecord,
    ) -> Result<(), EdgeError<S::Error>> {
        self.append_record(TILE_TYPE_TIME_SERIES, bytes_of(record))
    }

    /// Appends a typed equipment event record (§5.2.1).
    pub fn append_equipment_event(
        &mut self,
        record: &EquipmentEventRecord,
    ) -> Result<(), EdgeError<S::Error>> {
        self.append_record(TILE_TYPE_RECORD, bytes_of(record))
    }

    /// Appends a typed key-value configuration snapshot record (§5.3.1).
    pub fn append_config_entry(
        &mut self,
        record: &ConfigEntryRecord,
    ) -> Result<(), EdgeError<S::Error>> {
        self.append_record(TILE_TYPE_CONFIG, bytes_of(record))
    }

    /// Appends a typed device identity record (§6.3.1).
    pub fn append_device_identity(
        &mut self,
        record: &DeviceIdentityRecord,
    ) -> Result<(), EdgeError<S::Error>> {
        self.append_record(TILE_TYPE_RECORD, bytes_of(record))
    }

    /// Appends a typed participant consent & ownership record (§7.1.1).
    pub fn append_consent(
        &mut self,
        record: &EdgeConsentRecord,
    ) -> Result<(), EdgeError<S::Error>> {
        self.append_record(TILE_TYPE_CONSENT, bytes_of(record))
    }

    /// Seals the current in-progress tile using the Two-Phase Commit Protocol (§4.2).
    ///
    /// Phase 1: Payload and padding are written into flash pages (offset 64..TILE_SIZE).
    ///          The 64-byte header block remains 0xFF.
    /// Phase 2: Complete tile header is computed, checked, and written to offset 0.
    ///          FileHeader slot is toggled atomically to commit the tile.
    ///
    /// Returns the 32-byte cryptographic `seal_hash`.
    pub fn seal_tile(&mut self, timestamp: u64) -> Result<[u8; 32], EdgeError<S::Error>> {
        let payload_len = self.staging_len - COMMON_TILE_HEADER_SIZE;
        let payload = &self.staging_buffer[COMMON_TILE_HEADER_SIZE..self.staging_len];

        let data_crc32 = crc32c(payload);

        let mut hasher = Sha256::new();
        hasher.update(payload);
        let payload_hash: [u8; 32] = hasher.finalize().into();

        let seal_hash = compute_seal_hash(
            &self.prev_seal_hash,
            self.generation + 1,
            self.tile_count,
            &payload_hash,
            &self.device_id,
        );

        let mut tile_header = CommonTileHeader::new(
            self.staging_type_id,
            self.staging_record_count,
            payload_len as u64,
        );
        tile_header.data_crc32 = data_crc32;
        tile_header.payload_hash = payload_hash;
        tile_header.generation = (self.generation + 1) as u32;
        tile_header.header_crc32 = compute_common_tile_header_crc32(&tile_header);

        // Fill trailing unused bytes of staging buffer with zeros
        for b in &mut self.staging_buffer[self.staging_len..TILE_SIZE] {
            *b = 0;
        }

        // Copy computed header into staging buffer
        self.staging_buffer[..COMMON_TILE_HEADER_SIZE].copy_from_slice(bytes_of(&tile_header));

        let tile_offset = FILE_HEADER_SIZE + (self.tile_count as usize) * TILE_SIZE;

        // Erase target sector first
        self.storage.erase(tile_offset, TILE_SIZE)?;

        // CRITICAL POWER-LOSS SAFETY: Write payload first (pages 64..TILE_SIZE)
        self.storage.write(
            tile_offset + COMMON_TILE_HEADER_SIZE,
            &self.staging_buffer[COMMON_TILE_HEADER_SIZE..TILE_SIZE],
        )?;

        // Commit Marker: Write header last (page 0..64)
        self.storage
            .write(tile_offset, &self.staging_buffer[..COMMON_TILE_HEADER_SIZE])?;

        // Atomic FileHeader commit switch (Slot A <-> Slot B)
        let next_slot = 1 - self.active_slot;
        let next_gen = self.generation + 1;
        let next_tile_count = self.tile_count + 1;

        let mut new_slot = FileHeader::with_commit(
            next_tile_count,
            TILE_SIZE as u32,
            next_gen,
            self.created_timestamp,
            timestamp,
        );
        new_slot.file_uuid = self.file_uuid;
        new_slot.addon_mask = self.addon_mask;
        new_slot.header_crc32 = compute_file_header_crc32(&new_slot);

        let slot_offset = if next_slot == 0 {
            FILE_HEADER_SLOT_A_OFFSET
        } else {
            FILE_HEADER_SLOT_B_OFFSET
        };
        self.storage.write(slot_offset, bytes_of(&new_slot))?;

        self.active_slot = next_slot;
        self.generation = next_gen;
        self.tile_count = next_tile_count;
        self.prev_seal_hash = seal_hash;

        // Reset staging buffer for next tile
        self.staging_buffer = [0xFFu8; TILE_SIZE];
        self.staging_len = COMMON_TILE_HEADER_SIZE;
        self.staging_record_count = 0;
        self.staging_type_id = 0;

        Ok(seal_hash)
    }

    /// Factory provisioning helper: writes consent record first, followed by device identity,
    /// and seals genesis micro-tile.
    pub fn provision(
        &mut self,
        consent: &EdgeConsentRecord,
        identity: &DeviceIdentityRecord,
        timestamp: u64,
    ) -> Result<[u8; 32], EdgeError<S::Error>> {
        self.append_consent(consent)?;
        self.seal_tile(timestamp)?;
        self.append_device_identity(identity)?;
        self.seal_tile(timestamp)
    }

    /// Accessors
    pub fn tile_count(&self) -> u32 {
        self.tile_count
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn active_slot(&self) -> usize {
        self.active_slot
    }

    pub fn prev_seal_hash(&self) -> &[u8; 32] {
        &self.prev_seal_hash
    }

    pub fn device_id(&self) -> &[u8; 16] {
        &self.device_id
    }

    pub fn file_uuid(&self) -> [u64; 2] {
        self.file_uuid
    }

    pub fn into_storage(self) -> S {
        self.storage
    }

    pub fn storage(&self) -> &S {
        &self.storage
    }

    pub fn storage_mut(&mut self) -> &mut S {
        &mut self.storage
    }
}
