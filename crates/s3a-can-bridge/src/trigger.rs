use crate::error::{CanError, TriggerResult};
use crate::obd::parse_obd_code;
use std::collections::BTreeMap;

/// Checked-in mapping from a decoded fault key onto a pack-to node id.
#[derive(Debug, Clone, Default)]
pub struct MappingTable {
    j1939: BTreeMap<(u32, u8), u64>,
    obd: BTreeMap<u16, u64>,
    param: BTreeMap<u32, u64>,
}

impl MappingTable {
    /// Load a mapping CSV (`kind,key,fault_node_id`). `#` comments and
    /// blank lines are skipped. Duplicate keys and unknown kinds fail closed.
    pub fn from_csv(text: &str) -> Result<Self, CanError> {
        let mut table = MappingTable::default();
        for (i, raw) in text.lines().enumerate() {
            let line_no = i + 1;
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let cols: Vec<&str> = line.split(',').map(str::trim).collect();
            if cols.len() != 3 {
                return Err(CanError::MalformedMapping { line: line_no });
            }
            let node: u64 = cols[2]
                .parse()
                .map_err(|_| CanError::MalformedMapping { line: line_no })?;
            if node == 0 {
                return Err(CanError::ZeroFaultNodeId);
            }
            match cols[0] {
                "j1939" => {
                    let (spn, fmi) = parse_j1939_key(cols[1], line_no)?;
                    if table.j1939.insert((spn, fmi), node).is_some() {
                        return Err(CanError::DuplicateMappingKey);
                    }
                }
                "obd" => {
                    let raw = parse_obd_code(cols[1])
                        .map_err(|_| CanError::MalformedMapping { line: line_no })?;
                    if table.obd.insert(raw, node).is_some() {
                        return Err(CanError::DuplicateMappingKey);
                    }
                }
                "param" => {
                    let spn: u32 = cols[1]
                        .parse()
                        .map_err(|_| CanError::MalformedMapping { line: line_no })?;
                    if table.param.insert(spn, node).is_some() {
                        return Err(CanError::DuplicateMappingKey);
                    }
                }
                _ => return Err(CanError::UnknownMappingKind),
            }
        }
        Ok(table)
    }

    pub fn lookup_j1939(&self, spn: u32, fmi: u8) -> TriggerResult {
        match self.j1939.get(&(spn, fmi)).copied() {
            Some(fault_node_id) => TriggerResult::Match { fault_node_id },
            None => TriggerResult::NoMatch,
        }
    }

    pub fn lookup_obd(&self, raw: u16) -> TriggerResult {
        match self.obd.get(&raw).copied() {
            Some(fault_node_id) => TriggerResult::Match { fault_node_id },
            None => TriggerResult::NoMatch,
        }
    }

    pub fn lookup_param(&self, spn: u32) -> TriggerResult {
        match self.param.get(&spn).copied() {
            Some(fault_node_id) => TriggerResult::Match { fault_node_id },
            None => TriggerResult::NoMatch,
        }
    }
}

fn parse_j1939_key(key: &str, line: usize) -> Result<(u32, u8), CanError> {
    let (spn, fmi) = key
        .split_once(':')
        .ok_or(CanError::MalformedMapping { line })?;
    let spn: u32 = spn
        .parse()
        .map_err(|_| CanError::MalformedMapping { line })?;
    let fmi: u8 = fmi
        .parse()
        .map_err(|_| CanError::MalformedMapping { line })?;
    if fmi > 31 {
        return Err(CanError::MalformedMapping { line });
    }
    Ok((spn, fmi))
}

/// One threshold-table evaluation of a parameter in a PGN payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParamEvaluation {
    pub spn: u32,
    pub raw: u32,
    pub out_of_range: bool,
}

#[derive(Debug, Clone)]
struct ThresholdRow {
    spn: u32,
    pgn: u32,
    start_bit: u32,
    bit_length: u32,
    min_raw: u32,
    max_raw: u32,
}

/// Checked-in raw-value thresholds for out-of-range parameter triggers.
#[derive(Debug, Clone, Default)]
pub struct ThresholdTable {
    rows: Vec<ThresholdRow>,
}

impl ThresholdTable {
    /// Load a threshold CSV
    /// (`spn,pgn,start_bit,bit_length,min_raw,max_raw`).
    pub fn from_csv(text: &str) -> Result<Self, CanError> {
        let mut rows = Vec::new();
        for (i, raw) in text.lines().enumerate() {
            let line_no = i + 1;
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let err = CanError::MalformedThreshold { line: line_no };
            let cols: Vec<&str> = line.split(',').map(str::trim).collect();
            if cols.len() != 6 {
                return Err(err);
            }
            let spn: u32 = cols[0].parse().map_err(|_| err.clone())?;
            let pgn: u32 = cols[1].parse().map_err(|_| err.clone())?;
            let start_bit: u32 = cols[2].parse().map_err(|_| err.clone())?;
            let bit_length: u32 = cols[3].parse().map_err(|_| err.clone())?;
            let min_raw: u32 = cols[4].parse().map_err(|_| err.clone())?;
            let max_raw: u32 = cols[5].parse().map_err(|_| err.clone())?;
            if bit_length == 0 || bit_length > 32 {
                return Err(err);
            }
            if start_bit.checked_add(bit_length).is_none() || start_bit + bit_length > 64 {
                return Err(err);
            }
            if min_raw > max_raw {
                return Err(err);
            }
            if rows
                .iter()
                .any(|r: &ThresholdRow| r.spn == spn && r.pgn == pgn)
            {
                return Err(err);
            }
            rows.push(ThresholdRow {
                spn,
                pgn,
                start_bit,
                bit_length,
                min_raw,
                max_raw,
            });
        }
        Ok(ThresholdTable { rows })
    }

    /// Evaluate every threshold row whose PGN matches `pgn`.
    ///
    /// J1939 parameter groups are 8-byte data fields. A payload shorter
    /// than 8 bytes is `ShortPayload` (fail closed), even if the named
    /// signal would fit in fewer bytes.
    pub fn evaluate(&self, pgn: u32, payload: &[u8]) -> Result<Vec<ParamEvaluation>, CanError> {
        let matching: Vec<&ThresholdRow> = self.rows.iter().filter(|r| r.pgn == pgn).collect();
        if matching.is_empty() {
            return Ok(Vec::new());
        }
        if payload.len() < 8 {
            return Err(CanError::ShortPayload);
        }
        let mut out = Vec::new();
        for row in matching {
            let raw = extract_signal(payload, row.start_bit, row.bit_length)?;
            out.push(ParamEvaluation {
                spn: row.spn,
                raw,
                out_of_range: raw < row.min_raw || raw > row.max_raw,
            });
        }
        Ok(out)
    }
}

/// Extract an unsigned little-endian (Intel / SAE J1939-71) signal.
fn extract_signal(payload: &[u8], start_bit: u32, bit_length: u32) -> Result<u32, CanError> {
    if bit_length == 0 || bit_length > 32 {
        return Err(CanError::ShortPayload);
    }
    let end_bit = start_bit + bit_length;
    let needed = end_bit.div_ceil(8) as usize;
    if payload.len() < needed {
        return Err(CanError::ShortPayload);
    }
    let mut raw = 0u32;
    for i in 0..bit_length {
        let bit = start_bit + i;
        let byte = (bit / 8) as usize;
        let off = bit % 8;
        if (payload[byte] >> off) & 1 == 1 {
            raw |= 1 << i;
        }
    }
    Ok(raw)
}
