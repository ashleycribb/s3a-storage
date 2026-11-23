//! Encode and decode CAN bridge records. Fail closed on bad input.

use crate::error::CanError;
use crate::pack::CanTileType;
use crate::records::{
    CanDtcRecord, CanTriggerRecord, DTC_KIND_J1939_DM1, DTC_KIND_OBD_MODE03, TRIGGER_KIND_J1939,
    TRIGGER_KIND_OBD, TRIGGER_KIND_PARAM,
};
use bytemuck::{pod_read_unaligned, Pod};
use sha2::{Digest, Sha256};

/// Successfully decoded payload of one CAN bridge tile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanDecoded {
    Dtc(Vec<CanDtcRecord>),
    Trigger(Vec<CanTriggerRecord>),
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn require_source_id(source_id: [u8; 16]) -> Result<(), CanError> {
    if source_id == [0u8; 16] {
        Err(CanError::MissingSourceId)
    } else {
        Ok(())
    }
}

fn require_origin_ptr(origin_ptr: [u8; 16]) -> Result<(), CanError> {
    if origin_ptr == [0u8; 16] {
        Err(CanError::MissingOriginPtr)
    } else {
        Ok(())
    }
}

fn require_dtc_kind(kind: u8) -> Result<(), CanError> {
    match kind {
        DTC_KIND_J1939_DM1 | DTC_KIND_OBD_MODE03 => Ok(()),
        other => Err(CanError::UnknownDtcKind { kind: other }),
    }
}

fn require_trigger_kind(kind: u8) -> Result<(), CanError> {
    match kind {
        TRIGGER_KIND_J1939 | TRIGGER_KIND_OBD | TRIGGER_KIND_PARAM => Ok(()),
        other => Err(CanError::UnknownTriggerKind { kind: other }),
    }
}

fn require_zero_u8(pad: u8) -> Result<(), CanError> {
    if pad == 0 {
        Ok(())
    } else {
        Err(CanError::NonZeroPad)
    }
}

fn require_zero(pad: &[u8]) -> Result<(), CanError> {
    if pad.iter().all(|b| *b == 0) {
        Ok(())
    } else {
        Err(CanError::NonZeroPad)
    }
}

/// Encode a decoded DTC. `content` is the 4-byte J1939-73 slot or the
/// 2-byte OBD DTC; `content_hash` is SHA-256 of those bytes.
#[allow(clippy::too_many_arguments)]
pub fn encode_can_dtc(
    content: &[u8],
    source_id: [u8; 16],
    origin_ptr: [u8; 16],
    log_hash: [u8; 32],
    frame_offset: u64,
    can_id: u32,
    pgn: u32,
    spn: u32,
    fmi: u8,
    occurrence_count: u8,
    conversion_method: u8,
    kind: u8,
    source_address: u8,
    obd_raw: u16,
) -> Result<CanDtcRecord, CanError> {
    require_source_id(source_id)?;
    require_origin_ptr(origin_ptr)?;
    require_dtc_kind(kind)?;
    if content.is_empty() {
        return Err(CanError::ShortPayload);
    }
    Ok(CanDtcRecord {
        source_id,
        content_hash: sha256(content),
        origin_ptr,
        log_hash,
        frame_offset,
        can_id,
        pgn,
        spn,
        fmi,
        occurrence_count,
        conversion_method,
        kind,
        source_address,
        _pad0: 0,
        obd_raw,
        _pad: [0; 4],
    })
}

/// Encode a trigger record. `content` is the canonical key
/// (`j1939:{spn}:{fmi}`, `obd:{code}`, or `param:{spn}`).
#[allow(clippy::too_many_arguments)]
pub fn encode_can_trigger(
    content: &[u8],
    source_id: [u8; 16],
    origin_ptr: [u8; 16],
    log_hash: [u8; 32],
    frame_offset: u64,
    fault_node_id: u64,
    spn: u32,
    observed_raw: u32,
    fmi: u16,
    obd_raw: u16,
    kind: u8,
) -> Result<CanTriggerRecord, CanError> {
    require_source_id(source_id)?;
    require_origin_ptr(origin_ptr)?;
    require_trigger_kind(kind)?;
    if fault_node_id == 0 {
        return Err(CanError::ZeroFaultNodeId);
    }
    if content.is_empty() {
        return Err(CanError::ShortPayload);
    }
    Ok(CanTriggerRecord {
        source_id,
        content_hash: sha256(content),
        origin_ptr,
        log_hash,
        frame_offset,
        fault_node_id,
        spn,
        observed_raw,
        fmi,
        obd_raw,
        kind,
        _pad: [0; 3],
    })
}

fn decode_pod_slice<T: Pod + Copy>(bytes: &[u8], size: usize) -> Result<Vec<T>, CanError> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    if !bytes.len().is_multiple_of(size) {
        return Err(CanError::TruncatedRecord {
            expected: size,
            actual: bytes.len() % size,
        });
    }
    // When the payload is shorter than one record, actual is the full length
    // (the tests pass a 127-byte slice of a 128-byte record).
    if bytes.len() < size {
        return Err(CanError::TruncatedRecord {
            expected: size,
            actual: bytes.len(),
        });
    }
    let mut out = Vec::with_capacity(bytes.len() / size);
    for chunk in bytes.chunks_exact(size) {
        out.push(pod_read_unaligned(chunk));
    }
    Ok(out)
}

fn layout_ok(tile_type: u32, version: u32) -> Result<(), CanError> {
    match version {
        0 | 1 => Ok(()),
        other => Err(CanError::UnsupportedLayoutVersion {
            tile_type,
            version: other,
        }),
    }
}

/// Decode a CAN bridge tile payload. Layout version 0 aliases to 1.
pub fn decode_can_records(
    tile_type: u32,
    version: u32,
    bytes: &[u8],
) -> Result<CanDecoded, CanError> {
    layout_ok(tile_type, version)?;
    match tile_type {
        t if t == CanTileType::CAN_DTC => {
            let recs = decode_pod_slice::<CanDtcRecord>(bytes, CanDtcRecord::SIZE)?;
            for rec in &recs {
                require_source_id(rec.source_id)?;
                require_origin_ptr(rec.origin_ptr)?;
                require_dtc_kind(rec.kind)?;
                require_zero_u8(rec._pad0)?;
                require_zero(&rec._pad)?;
            }
            Ok(CanDecoded::Dtc(recs))
        }
        t if t == CanTileType::CAN_TRIGGER => {
            let recs = decode_pod_slice::<CanTriggerRecord>(bytes, CanTriggerRecord::SIZE)?;
            for rec in &recs {
                require_source_id(rec.source_id)?;
                require_origin_ptr(rec.origin_ptr)?;
                require_trigger_kind(rec.kind)?;
                if rec.fault_node_id == 0 {
                    return Err(CanError::ZeroFaultNodeId);
                }
                require_zero(&rec._pad)?;
            }
            Ok(CanDecoded::Trigger(recs))
        }
        other => Err(CanError::UnknownTileType(other)),
    }
}
