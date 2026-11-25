use crate::error::CanError;

/// Format a 16-bit SAE J2012 DTC as five ASCII bytes (`P0133`, `C0133`,
/// `B0133`, or `U0133`).
pub fn format_obd_dtc(raw: u16) -> [u8; 5] {
    let letter = match raw >> 14 {
        0 => b'P',
        1 => b'C',
        2 => b'B',
        _ => b'U',
    };
    let mut out = [0u8; 5];
    out[0] = letter;
    let digits = [
        ((raw >> 12) & 0x3) as u8,
        ((raw >> 8) & 0xF) as u8,
        ((raw >> 4) & 0xF) as u8,
        (raw & 0xF) as u8,
    ];
    for (i, d) in digits.into_iter().enumerate() {
        out[i + 1] = if d < 10 { b'0' + d } else { b'A' + (d - 10) };
    }
    out
}

/// Parse a five-character OBD code (`P0133`) into the 16-bit SAE encoding.
pub fn parse_obd_code(code: &str) -> Result<u16, CanError> {
    let b = code.as_bytes();
    if b.len() != 5 {
        return Err(CanError::MalformedMapping { line: 0 });
    }
    let hi = match b[0] {
        b'P' | b'p' => 0u16,
        b'C' | b'c' => 1,
        b'B' | b'b' => 2,
        b'U' | b'u' => 3,
        _ => return Err(CanError::MalformedMapping { line: 0 }),
    };
    let mut raw = hi << 14;
    for (i, ch) in b[1..].iter().enumerate() {
        let v = match ch {
            b'0'..=b'9' => ch - b'0',
            b'A'..=b'F' => ch - b'A' + 10,
            b'a'..=b'f' => ch - b'a' + 10,
            _ => return Err(CanError::MalformedMapping { line: 0 }),
        };
        if i == 0 {
            if v > 3 {
                return Err(CanError::MalformedMapping { line: 0 });
            }
            raw |= (v as u16) << 12;
        } else {
            let shift = 8 - (i as u16 - 1) * 4;
            raw |= (v as u16) << shift;
        }
    }
    Ok(raw)
}

/// Parse a single-frame ISO-TP OBD-II Mode 03 response (`PCI`, `0x43`, DTC pairs).
///
/// `0x0000` slots are padding and are not returned. First-frame ISO-TP
/// (PCI nibble `0x1`) and any other PCI type fail closed.
pub fn parse_obd_mode03(payload: &[u8]) -> Result<Vec<u16>, CanError> {
    if payload.is_empty() {
        return Err(CanError::InvalidObdResponse);
    }
    let pci = payload[0];
    if pci >> 4 != 0 {
        return Err(CanError::InvalidObdResponse);
    }
    let len = (pci & 0x0F) as usize;
    if len == 0 || payload.len() < 1 + len {
        return Err(CanError::InvalidObdResponse);
    }
    let body = &payload[1..1 + len];
    if body[0] != 0x43 {
        return Err(CanError::InvalidObdResponse);
    }
    let dtc_bytes = &body[1..];
    if !dtc_bytes.len().is_multiple_of(2) {
        return Err(CanError::InvalidObdResponse);
    }
    let mut out = Vec::new();
    for chunk in dtc_bytes.as_chunks::<2>().0 {
        let raw = u16::from_be_bytes([chunk[0], chunk[1]]);
        if raw != 0 {
            out.push(raw);
        }
    }
    Ok(out)
}
