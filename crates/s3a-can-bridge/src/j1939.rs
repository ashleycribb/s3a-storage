use crate::error::CanError;
use crate::frame::CanFrame;

/// Parsed 29-bit J1939 CAN identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct J1939Id {
    pub priority: u8,
    pub reserved: u8,
    pub data_page: u8,
    pub pdu_format: u8,
    pub pdu_specific: u8,
    pub source_address: u8,
    pub pgn: u32,
}

/// One diagnostic trouble code extracted from a DM1 message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct J1939Dtc {
    pub spn: u32,
    pub fmi: u8,
    pub occurrence_count: u8,
    pub conversion_method: u8,
    pub slot: [u8; 4],
}

/// A DM1 DTC annotated with the frame that produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodedJ1939Dtc {
    pub frame_offset: u64,
    pub can_id: u32,
    pub source_address: u8,
    pub pgn: u32,
    pub spn: u32,
    pub fmi: u8,
    pub occurrence_count: u8,
    pub conversion_method: u8,
    pub slot: [u8; 4],
}

/// PGN 65226: Active Diagnostic Trouble Codes (DM1).
pub const PGN_DM1: u32 = 65226;
/// PGN 60416: Transport Protocol — Connection Management (TP.CM).
pub const PGN_TP_CM: u32 = 60416;
/// PGN 60160: Transport Protocol — Data Transfer (TP.DT).
pub const PGN_TP_DT: u32 = 60160;
/// BAM control byte in TP.CM.
const TP_CM_BAM: u8 = 0x20;

/// Parse a 29-bit J1939 identifier. 11-bit IDs and values with bits above
/// 28 set are rejected.
pub fn parse_j1939_id(can_id: u32) -> Result<J1939Id, CanError> {
    if can_id > 0x1FFF_FFFF {
        return Err(CanError::NotJ1939Id { can_id });
    }
    // An 11-bit identifier (standard CAN) is not a 29-bit J1939 frame.
    if can_id <= 0x7FF {
        return Err(CanError::NotJ1939Id { can_id });
    }
    let priority = ((can_id >> 26) & 0x7) as u8;
    let reserved = ((can_id >> 25) & 0x1) as u8;
    let data_page = ((can_id >> 24) & 0x1) as u8;
    let pdu_format = ((can_id >> 16) & 0xFF) as u8;
    let pdu_specific = ((can_id >> 8) & 0xFF) as u8;
    let source_address = (can_id & 0xFF) as u8;
    let pgn = if pdu_format < 240 {
        ((reserved as u32) << 17) | ((data_page as u32) << 16) | ((pdu_format as u32) << 8)
    } else {
        ((reserved as u32) << 17)
            | ((data_page as u32) << 16)
            | ((pdu_format as u32) << 8)
            | (pdu_specific as u32)
    };
    Ok(J1939Id {
        priority,
        reserved,
        data_page,
        pdu_format,
        pdu_specific,
        source_address,
        pgn,
    })
}

/// Decode the DTC slots of a DM1 payload (lamp bytes followed by 4-byte slots).
///
/// A slot of all `0x00` or all `0xFF` is padding, not a DTC. A trailing
/// partial slot (fewer than 4 bytes after the last complete slot) is
/// ignored when it is all `0xFF` padding; any other leftover is
/// `ShortPayload`.
pub fn decode_dm1(payload: &[u8]) -> Result<Vec<J1939Dtc>, CanError> {
    if payload.len() < 2 {
        return Err(CanError::ShortPayload);
    }
    let mut dtcs = Vec::new();
    let mut rest = &payload[2..];
    while rest.len() >= 4 {
        let mut slot = [0u8; 4];
        slot.copy_from_slice(&rest[..4]);
        rest = &rest[4..];
        if slot == [0, 0, 0, 0] || slot == [0xFF, 0xFF, 0xFF, 0xFF] {
            continue;
        }
        let spn = (slot[0] as u32) | ((slot[1] as u32) << 8) | (((slot[2] as u32) >> 5) << 16);
        let fmi = slot[2] & 0x1F;
        let occurrence_count = slot[3] & 0x7F;
        let conversion_method = slot[3] >> 7;
        dtcs.push(J1939Dtc {
            spn,
            fmi,
            occurrence_count,
            conversion_method,
            slot,
        });
    }
    if !rest.is_empty() {
        if rest.iter().all(|b| *b == 0xFF) {
            // leftover two bytes of a truncated all-0xFF slot are padding
        } else if rest.iter().all(|b| *b == 0) {
            // leftover zeros are also padding
        } else {
            return Err(CanError::ShortPayload);
        }
    }
    Ok(dtcs)
}

struct BamSession {
    sa: u8,
    cm_id: u32,
    cm_offset: u64,
    pgn: u32,
    size: usize,
    packets: u8,
    next_seq: u8,
    buf: Vec<u8>,
}

/// Decode J1939 DM1 DTCs from a frame list, reassembling BAM sessions.
///
/// `frame_offset` is the 0-based index in `frames`. For a BAM-transported
/// DM1, every DTC uses the TP.CM frame index and the TP.CM CAN ID.
pub fn decode_j1939_dtcs(frames: &[CanFrame]) -> Result<Vec<DecodedJ1939Dtc>, CanError> {
    let mut sessions: Vec<BamSession> = Vec::new();
    let mut out = Vec::new();
    for (idx, frame) in frames.iter().enumerate() {
        if !frame.extended {
            continue;
        }
        let id = parse_j1939_id(frame.id)?;
        if id.pgn == PGN_TP_CM {
            ingest_tp_cm(&mut sessions, idx as u64, frame, &id)?;
        } else if id.pgn == PGN_TP_DT {
            if let Some(dtcs) = ingest_tp_dt(&mut sessions, frame, &id)? {
                out.extend(dtcs);
            }
        } else if id.pgn == PGN_DM1 {
            for dtc in decode_dm1(frame.payload())? {
                out.push(to_decoded(idx as u64, frame.id, id.source_address, dtc));
            }
        }
    }
    if !sessions.is_empty() {
        return Err(CanError::IncompleteBam);
    }
    Ok(out)
}

fn to_decoded(offset: u64, can_id: u32, sa: u8, dtc: J1939Dtc) -> DecodedJ1939Dtc {
    DecodedJ1939Dtc {
        frame_offset: offset,
        can_id,
        source_address: sa,
        pgn: PGN_DM1,
        spn: dtc.spn,
        fmi: dtc.fmi,
        occurrence_count: dtc.occurrence_count,
        conversion_method: dtc.conversion_method,
        slot: dtc.slot,
    }
}

fn ingest_tp_cm(
    sessions: &mut Vec<BamSession>,
    offset: u64,
    frame: &CanFrame,
    id: &J1939Id,
) -> Result<(), CanError> {
    let p = frame.payload();
    if p.len() < 8 {
        return Err(CanError::ShortPayload);
    }
    if p[0] != TP_CM_BAM {
        return Err(CanError::UnsupportedTpCm { control: p[0] });
    }
    if sessions.iter().any(|s| s.sa == id.source_address) {
        return Err(CanError::BamSequence);
    }
    let size = u16::from_le_bytes([p[1], p[2]]) as usize;
    let packets = p[3];
    if packets == 0 || size == 0 {
        return Err(CanError::IncompleteBam);
    }
    let pgn = (p[5] as u32) | ((p[6] as u32) << 8) | ((p[7] as u32) << 16);
    let capacity = (packets as usize).saturating_mul(7);
    let min_size = (packets as usize - 1).saturating_mul(7);
    if size > capacity || size <= min_size {
        return Err(CanError::BamSequence);
    }
    sessions.push(BamSession {
        sa: id.source_address,
        cm_id: frame.id,
        cm_offset: offset,
        pgn,
        size,
        packets,
        next_seq: 1,
        buf: Vec::with_capacity(size),
    });
    Ok(())
}

fn ingest_tp_dt(
    sessions: &mut Vec<BamSession>,
    frame: &CanFrame,
    id: &J1939Id,
) -> Result<Option<Vec<DecodedJ1939Dtc>>, CanError> {
    let pos = sessions
        .iter()
        .position(|s| s.sa == id.source_address)
        .ok_or(CanError::UnexpectedTpDt)?;
    let p = frame.payload();
    if p.is_empty() {
        return Err(CanError::ShortPayload);
    }
    let seq = p[0];
    if seq != sessions[pos].next_seq {
        return Err(CanError::BamSequence);
    }
    sessions[pos].buf.extend_from_slice(&p[1..]);
    sessions[pos].next_seq = sessions[pos]
        .next_seq
        .checked_add(1)
        .ok_or(CanError::BamSequence)?;
    if sessions[pos].next_seq <= sessions[pos].packets {
        return Ok(None);
    }
    let session = sessions.remove(pos);
    if session.buf.len() < session.size {
        return Err(CanError::IncompleteBam);
    }
    let payload = &session.buf[..session.size];
    if session.pgn != PGN_DM1 {
        return Ok(Some(Vec::new()));
    }
    let dtcs = decode_dm1(payload)?;
    Ok(Some(
        dtcs.into_iter()
            .map(|d| to_decoded(session.cm_offset, session.cm_id, session.sa, d))
            .collect(),
    ))
}
