//! CRC routines for s3a-edge: Castagnoli CRC32C and CCITT CRC16 in pure #![no_std].

/// Castagnoli CRC32C polynomial: 0x82F63B78 (reflected representation).
pub const CRC32C_POLYNOMIAL: u32 = 0x82F6_3B78;

/// Generates the 256-word Castagnoli CRC32C lookup table at compile time.
pub const fn make_crc32c_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut j = 0;
        while j < 8 {
            if (crc & 1) != 0 {
                crc = (crc >> 1) ^ CRC32C_POLYNOMIAL;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

pub const CRC32C_TABLE: [u32; 256] = make_crc32c_table();

/// Computes Castagnoli CRC32C checksum of data.
pub fn crc32c(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        let idx = ((crc ^ (b as u32)) & 0xFF) as usize;
        crc = (crc >> 8) ^ CRC32C_TABLE[idx];
    }
    !crc
}

/// Appends data to an existing Castagnoli CRC32C checksum.
pub fn crc32c_append(crc: u32, data: &[u8]) -> u32 {
    let mut current = !crc;
    for &b in data {
        let idx = ((current ^ (b as u32)) & 0xFF) as usize;
        current = (current >> 8) ^ CRC32C_TABLE[idx];
    }
    !current
}

/// Computes CCITT CRC16 checksum (polynomial 0x1021, init 0xFFFF).
pub fn crc16_ccitt(data: &[u8]) -> u16 {
    let mut crc = 0xFFFFu16;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            if (crc & 0x8000) != 0 {
                crc = (crc << 1) ^ 0x1021;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crc32c_standard_vectors() {
        // Standard check vector for Castagnoli CRC32C with input "123456789": 0xE3069283
        assert_eq!(crc32c(b"123456789"), 0xE3069283);
        assert_eq!(crc32c(b""), 0);

        // Incremental append matches single-pass
        let part1 = b"12345";
        let part2 = b"6789";
        let c1 = crc32c(part1);
        let c_combined = crc32c_append(c1, part2);
        assert_eq!(c_combined, 0xE3069283);
    }

    #[test]
    fn test_crc16_ccitt_standard_vector() {
        // Standard CCITT 0x1021 (init 0xFFFF) for "123456789" is 0x29B1
        assert_eq!(crc16_ccitt(b"123456789"), 0x29B1);
    }
}
