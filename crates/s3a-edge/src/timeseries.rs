//! Monotonic Time-Series Encodings (§5.1): Delta Timestamps, FoR Quantization & Bit-Packing.

use crate::records::CompactEdgeSensorRecord;

/// Quantizes a 32-bit floating point observation into an integer using Frame-of-Reference (FoR):
/// `Q = round((V - base_offset) / scale_step)`.
pub fn quantize_f32(val: f32, base_offset: f32, scale_step: f32) -> i32 {
    if scale_step == 0.0 {
        return 0;
    }
    let diff = (val - base_offset) / scale_step;
    if diff >= 0.0 {
        (diff + 0.5) as i32
    } else {
        (diff - 0.5) as i32
    }
}

/// Dequantizes an integer observation back into floating point:
/// `V = base_offset + (Q * scale_step)`.
pub fn dequantize_i32(q: i32, base_offset: f32, scale_step: f32) -> f32 {
    base_offset + (q as f32) * scale_step
}

/// Computes delta milliseconds from `min_timestamp_ms`:
/// `delta_ts = (timestamp_ms - min_timestamp_ms) as u16`.
/// Clamped to u16::MAX (65,535 ms = ~65.5s) if exceeding range.
pub fn compute_delta_ts_ms(timestamp_ms: u64, min_timestamp_ms: u64) -> u16 {
    if timestamp_ms < min_timestamp_ms {
        0
    } else {
        let delta = timestamp_ms - min_timestamp_ms;
        if delta > u16::MAX as u64 {
            u16::MAX
        } else {
            delta as u16
        }
    }
}

/// Bit-packs an array of unsigned integers (up to 16 bits each) into a destination byte slice
/// using exactly `bit_width` bits per integer (1 <= bit_width <= 16).
/// Returns the number of bytes written to `out`.
pub fn bit_pack_u16(values: &[u16], bit_width: u8, out: &mut [u8]) -> usize {
    if bit_width == 0 || bit_width > 16 || values.is_empty() {
        return 0;
    }

    let total_bits = values.len() * (bit_width as usize);
    let bytes_needed = total_bits.div_ceil(8);
    if out.len() < bytes_needed {
        return 0;
    }

    // Zero the target buffer
    for b in &mut out[..bytes_needed] {
        *b = 0;
    }

    let mut bit_pos = 0;
    let mask = (1u32 << bit_width) - 1;

    for &v in values {
        let val = (v as u32) & mask;
        let mut bits_left = bit_width as usize;
        let mut cur_val = val;

        while bits_left > 0 {
            let byte_idx = bit_pos / 8;
            let bit_offset = bit_pos % 8;
            let space_in_byte = 8 - bit_offset;
            let bits_to_write = bits_left.min(space_in_byte);

            let chunk = (cur_val & ((1 << bits_to_write) - 1)) as u8;
            out[byte_idx] |= chunk << bit_offset;

            cur_val >>= bits_to_write;
            bits_left -= bits_to_write;
            bit_pos += bits_to_write;
        }
    }

    bytes_needed
}

/// Unpacks bit-packed integers from `input` into `out` (up to `count` values).
/// Returns the number of unpacked values written.
pub fn bit_unpack_u16(input: &[u8], count: usize, bit_width: u8, out: &mut [u16]) -> usize {
    if bit_width == 0 || bit_width > 16 || count == 0 || out.len() < count {
        return 0;
    }

    let mut bit_pos = 0;
    let mask = (1u32 << bit_width) - 1;

    for i in 0..count {
        let mut val = 0u32;
        let mut bits_read = 0;

        while bits_read < (bit_width as usize) {
            let byte_idx = bit_pos / 8;
            if byte_idx >= input.len() {
                return i;
            }
            let bit_offset = bit_pos % 8;
            let space_in_byte = 8 - bit_offset;
            let bits_to_take = ((bit_width as usize) - bits_read).min(space_in_byte);

            let byte_val = input[byte_idx] >> bit_offset;
            let chunk = (byte_val as u32) & ((1 << bits_to_take) - 1);
            val |= chunk << bits_read;

            bits_read += bits_to_take;
            bit_pos += bits_to_take;
        }

        out[i] = (val & mask) as u16;
    }

    count
}

/// Computes the minimal bit width required to represent all values in `slice`.
pub fn required_bit_width_u16(slice: &[u16]) -> u8 {
    let mut max_val = 0u16;
    for &v in slice {
        if v > max_val {
            max_val = v;
        }
    }
    if max_val == 0 {
        return 1;
    }
    16 - max_val.leading_zeros() as u8
}

/// Packs an array of `CompactEdgeSensorRecord`s into a compressed delta-packed representation.
/// Layout:
/// - `header`: 4 bytes [count: u16, bit_width_delta: u8, bit_width_val: u8]
/// - bit-packed delta timestamps
/// - bit-packed delta values
pub fn compress_sensor_records(records: &[CompactEdgeSensorRecord], out: &mut [u8]) -> usize {
    if records.is_empty() || out.len() < 4 {
        return 0;
    }

    // Find bounds for delta timestamps
    let mut deltas = [0u16; 256];
    let n = records.len().min(256);
    for i in 0..n {
        deltas[i] = records[i].delta_ts_ms;
    }
    let bw_delta = required_bit_width_u16(&deltas[..n]).max(1);

    // Delta between consecutive quantized values
    let mut val_deltas = [0u16; 256];
    let mut prev = 0i32;
    for i in 0..n {
        let diff = records[i].quantized_val - prev;
        prev = records[i].quantized_val;
        // Zig-zag encode signed diff to unsigned:
        let zigzag = ((diff << 1) ^ (diff >> 31)) as u32;
        val_deltas[i] = (zigzag & 0xFFFF) as u16;
    }
    let bw_val = required_bit_width_u16(&val_deltas[..n]).max(1);

    out[0..2].copy_from_slice(&(n as u16).to_le_bytes());
    out[2] = bw_delta;
    out[3] = bw_val;

    let mut cursor = 4;
    let delta_bytes = bit_pack_u16(&deltas[..n], bw_delta, &mut out[cursor..]);
    cursor += delta_bytes;

    let val_bytes = bit_pack_u16(&val_deltas[..n], bw_val, &mut out[cursor..]);
    cursor += val_bytes;

    cursor
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quantization_roundtrip() {
        let base = 20.0f32;
        let step = 0.05f32;

        let raw = 23.45f32;
        let q = quantize_f32(raw, base, step);
        let restored = dequantize_i32(q, base, step);
        assert!((restored - raw).abs() < step);
    }

    #[test]
    fn test_bit_pack_unpack_roundtrip() {
        let input = [0u16, 1, 5, 12, 15, 7, 3, 9, 14, 2];
        let bw = 4u8; // 4 bits each
        let mut packed = [0u8; 16];
        let packed_len = bit_pack_u16(&input, bw, &mut packed);
        assert_eq!(packed_len, 5); // 10 values * 4 bits = 40 bits = 5 bytes

        let mut unpacked = [0u16; 10];
        let unpacked_count = bit_unpack_u16(&packed[..packed_len], 10, bw, &mut unpacked);
        assert_eq!(unpacked_count, 10);
        assert_eq!(&input[..], &unpacked[..]);
    }
}
