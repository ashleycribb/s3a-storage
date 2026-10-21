//! Edge Profile Record Structs and Encodings (§5, §6, §7).

use crate::crc::{crc16_ccitt, crc32c};
use bytemuck::{bytes_of, Pod, Zeroable};

// --- Quality Flags (§5.1.1) ---
pub const QUALITY_OK: u16 = 0;
pub const QUALITY_CLIPPED: u16 = 1;
pub const QUALITY_DEGRADED: u16 = 2;

// --- Canonical Equipment Event Categories (§5.2.2) ---
pub const EVENT_MACHINE_START: u16 = 0x0001;
pub const EVENT_MACHINE_STOP: u16 = 0x0002;
pub const EVENT_EMERGENCY_STOP: u16 = 0x0003;
pub const EVENT_FAULT_CODE: u16 = 0x0004;
pub const EVENT_MAINTENANCE: u16 = 0x0005;
pub const EVENT_CALIBRATION: u16 = 0x0006;
pub const EVENT_OPERATOR_CHANGE: u16 = 0x0007;

// --- Severity Levels (§5.2.1) ---
pub const SEVERITY_INFO: u8 = 0;
pub const SEVERITY_WARN: u8 = 1;
pub const SEVERITY_ALARM: u8 = 2;
pub const SEVERITY_EMERGENCY: u8 = 3;

// --- Config Value Types (§5.3.1) ---
pub const CONFIG_VAL_I32: u16 = 1;
pub const CONFIG_VAL_U32: u16 = 2;
pub const CONFIG_VAL_F32: u16 = 3;
pub const CONFIG_VAL_BOOL: u16 = 4;
pub const CONFIG_VAL_RAW_BYTES8: u16 = 5;

// --- Allowed Purposes Bitmask (§7.1.2) ---
pub const PURPOSE_OPERATIONAL_MAINTENANCE: u32 = 0x0001;
pub const PURPOSE_SAFETY_AUDIT: u32 = 0x0002;
pub const PURPOSE_RESEARCH_ANALYTICS: u32 = 0x0004;
pub const PURPOSE_MODEL_TRAINING: u32 = 0x0008;
pub const PURPOSE_THIRD_PARTY_SHARING: u32 = 0x0010;

// --- Sharing Scope Bitmask (§7.1.3) ---
pub const SCOPE_TELEMETRY: u32 = 0x0001;
pub const SCOPE_EQUIPMENT_FAULTS: u32 = 0x0002;
pub const SCOPE_CALIBRATION_LOGS: u32 = 0x0004;
pub const SCOPE_OPERATOR_IDENTITY: u32 = 0x0008;

/// Compact Sensor Record (16 Bytes, 4-Byte Aligned) (§5.1.1).
#[repr(C, align(4))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct CompactEdgeSensorRecord {
    pub delta_ts_ms: u16, // Delta milliseconds from tile min_timestamp (offset 0..2)
    pub sensor_id: u8,    // Physical sensor channel (0..255) (offset 2..3)
    pub metric_id: u8,    // Metric type (e.g. 1=Temp, 2=Vibration, 3=Current) (offset 3..4)
    pub quantized_val: i32, // Fixed-point value (offset 4..8)
    pub quality_flags: u16, // Hardware status: 0=OK, 1=Clipped, 2=Degraded (offset 8..10)
    pub _reserved: [u8; 4], // Alignment padding (offset 10..14)
    pub record_crc16: u16, // CCITT CRC16 over preceding 14 bytes (offset 14..16)
}

impl CompactEdgeSensorRecord {
    pub fn new(
        delta_ts_ms: u16,
        sensor_id: u8,
        metric_id: u8,
        quantized_val: i32,
        quality_flags: u16,
    ) -> Self {
        let mut rec = Self {
            delta_ts_ms,
            sensor_id,
            metric_id,
            quantized_val,
            quality_flags,
            _reserved: [0u8; 4],
            record_crc16: 0,
        };
        let bytes = bytes_of(&rec);
        rec.record_crc16 = crc16_ccitt(&bytes[..14]);
        rec
    }

    pub fn verify(&self) -> bool {
        let bytes = bytes_of(self);
        crc16_ccitt(&bytes[..14]) == self.record_crc16
    }
}

/// EquipmentEventRecord (32 Bytes, 8-Byte Aligned) (§5.2.1).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct EquipmentEventRecord {
    pub timestamp_sec: u64,
    pub event_type: u16,
    pub severity: u8,
    pub subsystem_id: u8,
    pub event_code: u32,
    pub operator_id: [u8; 8],
    pub param1: i32,
    pub record_crc32: u32, // Castagnoli CRC32C over bytes 0..28
}

impl EquipmentEventRecord {
    pub fn new(
        timestamp_sec: u64,
        event_type: u16,
        severity: u8,
        subsystem_id: u8,
        event_code: u32,
        operator_id: [u8; 8],
        param1: i32,
    ) -> Self {
        let mut rec = Self {
            timestamp_sec,
            event_type,
            severity,
            subsystem_id,
            event_code,
            operator_id,
            param1,
            record_crc32: 0,
        };
        let bytes = bytes_of(&rec);
        rec.record_crc32 = crc32c(&bytes[..28]);
        rec
    }

    pub fn verify(&self) -> bool {
        let bytes = bytes_of(self);
        crc32c(&bytes[..28]) == self.record_crc32
    }
}

/// ConfigEntryRecord (32 Bytes, 4-Byte Aligned) (§5.3.1).
#[repr(C, align(4))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct ConfigEntryRecord {
    pub key: [u8; 16],      // Null-terminated ASCII key (e.g. "sample_hz\0")
    pub val_type: u16,      // 1=i32, 2=u32, 3=f32, 4=bool, 5=raw_bytes8
    pub _pad: u16,          // Alignment padding
    pub val_bytes: [u8; 8], // Little-endian representation of value
    pub record_crc32: u32,  // Castagnoli CRC32C over bytes 0..28
}

impl ConfigEntryRecord {
    pub fn new(key_str: &str, val_type: u16, val_bytes: [u8; 8]) -> Self {
        let mut key = [0u8; 16];
        let bytes = key_str.as_bytes();
        let copy_len = bytes.len().min(15);
        key[..copy_len].copy_from_slice(&bytes[..copy_len]);

        let mut rec = Self {
            key,
            val_type,
            _pad: 0,
            val_bytes,
            record_crc32: 0,
        };
        let rec_bytes = bytes_of(&rec);
        rec.record_crc32 = crc32c(&rec_bytes[..28]);
        rec
    }

    pub fn new_i32(key_str: &str, val: i32) -> Self {
        let mut b = [0u8; 8];
        b[..4].copy_from_slice(&val.to_le_bytes());
        Self::new(key_str, CONFIG_VAL_I32, b)
    }

    pub fn new_u32(key_str: &str, val: u32) -> Self {
        let mut b = [0u8; 8];
        b[..4].copy_from_slice(&val.to_le_bytes());
        Self::new(key_str, CONFIG_VAL_U32, b)
    }

    pub fn new_f32(key_str: &str, val: f32) -> Self {
        let mut b = [0u8; 8];
        b[..4].copy_from_slice(&val.to_le_bytes());
        Self::new(key_str, CONFIG_VAL_F32, b)
    }

    pub fn new_bool(key_str: &str, val: bool) -> Self {
        let mut b = [0u8; 8];
        b[0] = if val { 1 } else { 0 };
        Self::new(key_str, CONFIG_VAL_BOOL, b)
    }

    pub fn verify(&self) -> bool {
        let bytes = bytes_of(self);
        crc32c(&bytes[..28]) == self.record_crc32
    }
}

/// DeviceIdentityRecord Layout (96 Bytes, 8-Byte Aligned) (§6.3.1).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct DeviceIdentityRecord {
    pub device_id: [u8; 16],
    pub hardware_model: [u8; 16],
    pub firmware_version: [u16; 4], // [major, minor, patch, build]
    pub public_key: [u8; 32],       // 32-byte Ed25519 device public key
    pub provisioned_at: u64,
    pub boot_count: u64,
    pub _reserved: u32,
    pub record_crc32: u32, // Castagnoli CRC32C over bytes 0..92
}

impl DeviceIdentityRecord {
    pub fn new(
        device_id: [u8; 16],
        model_str: &str,
        firmware_version: [u16; 4],
        public_key: [u8; 32],
        provisioned_at: u64,
        boot_count: u64,
    ) -> Self {
        let mut hardware_model = [0u8; 16];
        let bytes = model_str.as_bytes();
        let copy_len = bytes.len().min(15);
        hardware_model[..copy_len].copy_from_slice(&bytes[..copy_len]);

        let mut rec = Self {
            device_id,
            hardware_model,
            firmware_version,
            public_key,
            provisioned_at,
            boot_count,
            _reserved: 0,
            record_crc32: 0,
        };
        let rec_bytes = bytes_of(&rec);
        rec.record_crc32 = crc32c(&rec_bytes[..92]);
        rec
    }

    pub fn verify(&self) -> bool {
        let bytes = bytes_of(self);
        crc32c(&bytes[..92]) == self.record_crc32
    }
}

/// EdgeConsentRecord Layout (64 Bytes, 8-Byte Aligned) (§7.1.1).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct EdgeConsentRecord {
    pub owner_uuid: [u8; 16],
    pub allowed_purposes: u32,
    pub sharing_scope: u32,
    pub granted_at: u64,
    pub expires_at: u64,
    pub admin_actor_id: u64,
    pub consent_version: u32,
    pub _reserved: u32,
    pub record_crc32: u32, // Castagnoli CRC32C over bytes 0..56
    pub _pad: u32,
}

impl EdgeConsentRecord {
    pub fn new(
        owner_uuid: [u8; 16],
        allowed_purposes: u32,
        sharing_scope: u32,
        granted_at: u64,
        expires_at: u64,
        admin_actor_id: u64,
    ) -> Self {
        let mut rec = Self {
            owner_uuid,
            allowed_purposes,
            sharing_scope,
            granted_at,
            expires_at,
            admin_actor_id,
            consent_version: 1,
            _reserved: 0,
            record_crc32: 0,
            _pad: 0,
        };
        let bytes = bytes_of(&rec);
        rec.record_crc32 = crc32c(&bytes[..56]);
        rec
    }

    pub fn verify(&self) -> bool {
        let bytes = bytes_of(self);
        crc32c(&bytes[..56]) == self.record_crc32
    }
}

/// Dedicated Cryptographic Seal Record (144 Bytes, 8-Byte Aligned) for Seal Tiles (§6.2).
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct DeviceSealRecord {
    pub tile_index: u32,
    pub _pad1: u32,
    pub generation: u64,
    pub seal_hash: [u8; 32],
    pub prev_seal_hash: [u8; 32],
    pub signature: [u8; 64], // 64-byte Ed25519 hardware signature
    pub record_crc32: u32,   // Castagnoli CRC32C over bytes 0..140
    pub _pad2: u32,
}

impl DeviceSealRecord {
    pub fn new(
        tile_index: u32,
        generation: u64,
        seal_hash: [u8; 32],
        prev_seal_hash: [u8; 32],
        signature: [u8; 64],
    ) -> Self {
        let mut rec = Self {
            tile_index,
            _pad1: 0,
            generation,
            seal_hash,
            prev_seal_hash,
            signature,
            record_crc32: 0,
            _pad2: 0,
        };
        let bytes = bytes_of(&rec);
        rec.record_crc32 = crc32c(&bytes[..140]);
        rec
    }

    pub fn verify(&self) -> bool {
        let bytes = bytes_of(self);
        crc32c(&bytes[..140]) == self.record_crc32
    }
}
