use crate::error::CanError;
use crate::frame::{parse_data_hex, parse_decimal_timestamp_ns, parse_id_by_width, CanFrame};

fn is_comment_or_blank(line: &str) -> bool {
    let t = line.trim();
    t.is_empty() || t.starts_with('#') || t.starts_with("//")
}

/// Parse a `candump -L` / SocketCAN ASCII log.
///
/// Lines are `(<seconds>) <iface> <id>#<hexdata>`. `#` comments and blank
/// lines are skipped.
pub fn parse_candump(text: &str) -> Result<Vec<CanFrame>, CanError> {
    let mut frames = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line_no = i + 1;
        if is_comment_or_blank(raw) {
            continue;
        }
        let line = raw.trim();
        let err = CanError::MalformedLog { line: line_no };
        if !line.starts_with('(') {
            return Err(err);
        }
        let close = line.find(')').ok_or(err.clone())?;
        let ts = parse_decimal_timestamp_ns(&line[1..close]).ok_or(err.clone())?;
        let rest = line[close + 1..].trim();
        let mut parts = rest.splitn(2, char::is_whitespace);
        let iface = parts.next().ok_or(err.clone())?;
        let id_data = parts
            .next()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or(err.clone())?;
        let hash = id_data.find('#').ok_or(err.clone())?;
        let (id, extended) = parse_id_by_width(&id_data[..hash]).ok_or(err.clone())?;
        let (data, dlc) = parse_data_hex(&id_data[hash + 1..]).ok_or(err.clone())?;
        frames.push(CanFrame {
            timestamp_ns: ts,
            interface: iface.to_string(),
            id,
            extended,
            dlc,
            data,
        });
    }
    Ok(frames)
}

/// Parse the locked CSV format: header
/// `timestamp_s,interface,can_id,dlc,data_hex` and matching data rows.
/// A `0x` prefix on `can_id` is rejected. `#` comments are skipped.
pub fn parse_can_csv(text: &str) -> Result<Vec<CanFrame>, CanError> {
    let mut header_seen = false;
    let mut frames = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line_no = i + 1;
        if is_comment_or_blank(raw) {
            continue;
        }
        let line = raw.trim();
        if !header_seen {
            if line != "timestamp_s,interface,can_id,dlc,data_hex" {
                return Err(CanError::MalformedLog { line: line_no });
            }
            header_seen = true;
            continue;
        }
        let err = CanError::MalformedLog { line: line_no };
        let cols: Vec<&str> = line.split(',').collect();
        if cols.len() != 5 {
            return Err(err);
        }
        let ts = parse_decimal_timestamp_ns(cols[0].trim()).ok_or(err.clone())?;
        let iface = cols[1].trim();
        if iface.is_empty() {
            return Err(err);
        }
        let id_str = cols[2].trim();
        if id_str.starts_with("0x") || id_str.starts_with("0X") {
            return Err(err);
        }
        let (id, extended) = parse_id_by_width(id_str).ok_or(err.clone())?;
        let dlc: u8 = cols[3].trim().parse().map_err(|_| err.clone())?;
        let (data, data_dlc) = parse_data_hex(cols[4].trim()).ok_or(err.clone())?;
        if dlc != data_dlc || dlc > 8 {
            return Err(err);
        }
        frames.push(CanFrame {
            timestamp_ns: ts,
            interface: iface.to_string(),
            id,
            extended,
            dlc,
            data,
        });
    }
    if !header_seen {
        return Err(CanError::MalformedLog { line: 1 });
    }
    Ok(frames)
}

/// Parse a simple Vector ASCII (`.asc`) log. A `base hex` line must appear
/// before any frame. The date line is ignored (it is not converted to a
/// Unix epoch). Frame timestamps are the leading numeric field, parsed
/// with the same decimal-nanosecond rule as candump.
pub fn parse_vector_asc(text: &str) -> Result<Vec<CanFrame>, CanError> {
    let mut base_hex = false;
    let mut frames = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line_no = i + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('#') {
            continue;
        }
        let tokens: Vec<&str> = line.split_whitespace().collect();
        if tokens.first() == Some(&"date") {
            continue;
        }
        if tokens.first() == Some(&"base") {
            if tokens.get(1) != Some(&"hex") {
                return Err(CanError::MalformedLog { line: line_no });
            }
            base_hex = true;
            continue;
        }
        if !base_hex {
            return Err(CanError::MalformedLog { line: line_no });
        }
        // timestamp channel id[x] Rx|Tx d dlc data...
        let err = CanError::MalformedLog { line: line_no };
        if tokens.len() < 6 {
            return Err(err);
        }
        let ts = parse_decimal_timestamp_ns(tokens[0]).ok_or(err.clone())?;
        let channel = tokens[1];
        let (id_tok, extended) = if let Some(stripped) = tokens[2].strip_suffix('x') {
            (stripped, true)
        } else if let Some(stripped) = tokens[2].strip_suffix('X') {
            (stripped, true)
        } else {
            (tokens[2], false)
        };
        if !id_tok.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(err);
        }
        let id = u32::from_str_radix(id_tok, 16).map_err(|_| err.clone())?;
        if extended {
            if id > 0x1FFF_FFFF {
                return Err(err);
            }
        } else if id > 0x7FF {
            return Err(err);
        }
        let dir = tokens[3];
        if dir != "Rx" && dir != "Tx" && dir != "rx" && dir != "tx" {
            return Err(err);
        }
        if tokens[4] != "d" && tokens[4] != "D" {
            return Err(err);
        }
        let dlc: u8 = tokens[5].parse().map_err(|_| err.clone())?;
        if dlc as usize != tokens.len() - 6 || dlc > 8 {
            return Err(err);
        }
        let mut data = [0u8; 8];
        for (j, tok) in tokens[6..].iter().enumerate() {
            if tok.len() > 2 || !tok.bytes().all(|c| c.is_ascii_hexdigit()) {
                return Err(err);
            }
            data[j] = u8::from_str_radix(tok, 16).map_err(|_| err.clone())?;
        }
        frames.push(CanFrame {
            timestamp_ns: ts,
            interface: channel.to_string(),
            id,
            extended,
            dlc,
            data,
        });
    }
    Ok(frames)
}
