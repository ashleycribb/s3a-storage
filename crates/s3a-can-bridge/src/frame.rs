/// One parsed CAN frame from a log or a live reader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanFrame {
    pub timestamp_ns: u64,
    pub interface: String,
    pub id: u32,
    pub extended: bool,
    pub dlc: u8,
    pub data: [u8; 8],
}

impl CanFrame {
    /// Payload bytes actually present on the wire (`data[..dlc]`).
    pub fn payload(&self) -> &[u8] {
        let n = (self.dlc as usize).min(8);
        &self.data[..n]
    }
}

/// Parse a decimal seconds timestamp into nanoseconds without IEEE rounding.
///
/// The integer part is multiplied by 1_000_000_000. The fractional digits
/// are padded to 9 places as nanoseconds. More than 9 fractional digits is
/// rejected.
pub(crate) fn parse_decimal_timestamp_ns(s: &str) -> Option<u64> {
    let (int_part, frac) = match s.split_once('.') {
        Some((a, b)) => (a, b),
        None => (s, ""),
    };
    if int_part.is_empty() || !int_part.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if frac.len() > 9 || !frac.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let secs: u64 = int_part.parse().ok()?;
    let mut frac_ns: u64 = if frac.is_empty() {
        0
    } else {
        frac.parse().ok()?
    };
    for _ in frac.len()..9 {
        frac_ns = frac_ns.checked_mul(10)?;
    }
    secs.checked_mul(1_000_000_000)?.checked_add(frac_ns)
}

/// Parse a hex CAN ID using the locked width rule: 1..=3 digits is an
/// 11-bit standard ID, 8 digits is a 29-bit extended ID.
pub(crate) fn parse_id_by_width(hex: &str) -> Option<(u32, bool)> {
    let hex = hex.trim();
    if hex.is_empty() || !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let extended = match hex.len() {
        1..=3 => false,
        8 => true,
        _ => return None,
    };
    let id = u32::from_str_radix(hex, 16).ok()?;
    if extended {
        if id > 0x1FFF_FFFF {
            return None;
        }
    } else if id > 0x7FF {
        return None;
    }
    Some((id, extended))
}

pub(crate) fn parse_data_hex(hex: &str) -> Option<([u8; 8], u8)> {
    let hex = hex.trim();
    if !hex.len().is_multiple_of(2) || hex.len() > 16 || !hex.bytes().all(|c| c.is_ascii_hexdigit())
    {
        return None;
    }
    let dlc = (hex.len() / 2) as u8;
    let mut data = [0u8; 8];
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        data[i] = u8::from_str_radix(std::str::from_utf8(chunk).ok()?, 16).ok()?;
    }
    Some((data, dlc))
}
