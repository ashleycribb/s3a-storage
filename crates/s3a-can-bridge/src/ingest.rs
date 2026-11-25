use crate::codec::{encode_can_dtc, encode_can_trigger};
use crate::error::{CanError, TriggerResult};
use crate::frame::CanFrame;
use crate::j1939::{decode_j1939_dtcs, parse_j1939_id, PGN_TP_CM, PGN_TP_DT};
use crate::log::parse_candump;
use crate::obd::{format_obd_dtc, parse_obd_mode03};
use crate::records::{
    CanDtcRecord, CanTriggerRecord, DTC_KIND_J1939_DM1, DTC_KIND_OBD_MODE03, TRIGGER_KIND_J1939,
    TRIGGER_KIND_OBD, TRIGGER_KIND_PARAM,
};
use crate::trigger::{MappingTable, ThresholdTable};
use sha2::{Digest, Sha256};

/// Result of ingesting a replayable CAN log.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IngestReport {
    pub dtcs: Vec<CanDtcRecord>,
    pub triggers: Vec<CanTriggerRecord>,
    pub no_match_count: usize,
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn j1939_key(spn: u32, fmi: u8) -> String {
    format!("j1939:{spn}:{fmi}")
}

fn obd_key(raw: u16) -> String {
    let code = format_obd_dtc(raw);
    format!(
        "obd:{}",
        std::str::from_utf8(&code).expect("ASCII OBD code")
    )
}

fn param_key(spn: u32) -> String {
    format!("param:{spn}")
}

fn ingest_frames(
    frames: &[CanFrame],
    log_hash: [u8; 32],
    mapping: &MappingTable,
    thresholds: &ThresholdTable,
    source_id: [u8; 16],
    origin_ptr: [u8; 16],
) -> Result<IngestReport, CanError> {
    let mut report = IngestReport::default();
    let mut pending_dtcs: Vec<(u64, CanDtcRecord)> = Vec::new();
    let mut pending_trigs: Vec<(u64, u8, CanTriggerRecord)> = Vec::new();

    for dtc in decode_j1939_dtcs(frames)? {
        let rec = encode_can_dtc(
            &dtc.slot,
            source_id,
            origin_ptr,
            log_hash,
            dtc.frame_offset,
            dtc.can_id,
            dtc.pgn,
            dtc.spn,
            dtc.fmi,
            dtc.occurrence_count,
            dtc.conversion_method,
            DTC_KIND_J1939_DM1,
            dtc.source_address,
            0,
        )?;
        pending_dtcs.push((dtc.frame_offset, rec));
        match mapping.lookup_j1939(dtc.spn, dtc.fmi) {
            TriggerResult::NoMatch => report.no_match_count += 1,
            TriggerResult::Match { fault_node_id } => {
                let key = j1939_key(dtc.spn, dtc.fmi);
                let trig = encode_can_trigger(
                    key.as_bytes(),
                    source_id,
                    origin_ptr,
                    log_hash,
                    dtc.frame_offset,
                    fault_node_id,
                    dtc.spn,
                    0,
                    dtc.fmi as u16,
                    0,
                    TRIGGER_KIND_J1939,
                )?;
                pending_trigs.push((dtc.frame_offset, TRIGGER_KIND_J1939, trig));
            }
        }
    }

    for (idx, frame) in frames.iter().enumerate() {
        let offset = idx as u64;
        if !frame.extended && (0x7E8..=0x7EF).contains(&frame.id) {
            for raw in parse_obd_mode03(frame.payload())? {
                let bytes = raw.to_be_bytes();
                let rec = encode_can_dtc(
                    &bytes,
                    source_id,
                    origin_ptr,
                    log_hash,
                    offset,
                    frame.id,
                    0,
                    0,
                    0,
                    0,
                    0,
                    DTC_KIND_OBD_MODE03,
                    0,
                    raw,
                )?;
                pending_dtcs.push((offset, rec));
                match mapping.lookup_obd(raw) {
                    TriggerResult::NoMatch => report.no_match_count += 1,
                    TriggerResult::Match { fault_node_id } => {
                        let key = obd_key(raw);
                        let trig = encode_can_trigger(
                            key.as_bytes(),
                            source_id,
                            origin_ptr,
                            log_hash,
                            offset,
                            fault_node_id,
                            0,
                            0,
                            0,
                            raw,
                            TRIGGER_KIND_OBD,
                        )?;
                        pending_trigs.push((offset, TRIGGER_KIND_OBD, trig));
                    }
                }
            }
            continue;
        }
        if frame.extended {
            let id = parse_j1939_id(frame.id)?;
            if id.pgn == PGN_TP_CM || id.pgn == PGN_TP_DT {
                continue;
            }
            for ev in thresholds.evaluate(id.pgn, frame.payload())? {
                if ev.out_of_range {
                    match mapping.lookup_param(ev.spn) {
                        TriggerResult::NoMatch => report.no_match_count += 1,
                        TriggerResult::Match { fault_node_id } => {
                            let key = param_key(ev.spn);
                            let trig = encode_can_trigger(
                                key.as_bytes(),
                                source_id,
                                origin_ptr,
                                log_hash,
                                offset,
                                fault_node_id,
                                ev.spn,
                                ev.raw,
                                0,
                                0,
                                TRIGGER_KIND_PARAM,
                            )?;
                            pending_trigs.push((offset, TRIGGER_KIND_PARAM, trig));
                        }
                    }
                }
            }
        }
    }

    pending_dtcs.sort_by_key(|(offset, _)| *offset);
    report.dtcs = pending_dtcs.into_iter().map(|(_, r)| r).collect();
    pending_trigs.sort_by_key(|(offset, kind, _)| (*offset, *kind));
    report.triggers = pending_trigs.into_iter().map(|(_, _, r)| r).collect();
    Ok(report)
}

/// Parse a candump log, decode DTCs, apply the mapping and threshold
/// tables, and return encoded records with `log_hash` equal to SHA-256
/// of the exact source text bytes.
pub fn ingest_candump(
    text: &str,
    mapping: &MappingTable,
    thresholds: &ThresholdTable,
    source_id: [u8; 16],
    origin_ptr: [u8; 16],
) -> Result<IngestReport, CanError> {
    let frames = parse_candump(text)?;
    let log_hash = sha256(text.as_bytes());
    ingest_frames(
        &frames, log_hash, mapping, thresholds, source_id, origin_ptr,
    )
}
