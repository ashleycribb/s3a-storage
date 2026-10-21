//! Gate Addresses and Dialing URI protocol (§11).

use crate::errors::VaultError;
use sha2::{Digest, Sha256};

/// An immutable Gate Address derived from an Ed25519 public key (§11.1).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GateAddress {
    /// 8-byte digest prefix of SHA-256(public_key)
    pub prefix: [u8; 8],
    /// 1-byte checksum (CRC32C & 0xFF)
    pub checksum: u8,
}

impl GateAddress {
    /// Derive a Gate Address from an Ed25519 public key.
    pub fn from_public_key(pubkey: &[u8; 32]) -> Self {
        let digest = Sha256::digest(pubkey);
        let mut prefix = [0u8; 8];
        prefix.copy_from_slice(&digest[0..8]);
        let checksum = (crc32c::crc32c(&prefix) & 0xFF) as u8;
        Self { prefix, checksum }
    }

    /// Format as human-readable segmented address (e.g., `s3a-7f9a-c21d-e4b8-8910-93`).
    pub fn to_address_string(&self) -> String {
        let hex_prefix = hex::encode(self.prefix);
        format!(
            "s3a-{}-{}-{}-{}-{:02x}",
            &hex_prefix[0..4],
            &hex_prefix[4..8],
            &hex_prefix[8..12],
            &hex_prefix[12..16],
            self.checksum
        )
    }

    /// Format as canonical URI: `gate://s3a-XXXX-XXXX-XXXX-XXXX-CC`.
    pub fn to_uri(&self) -> String {
        format!("gate://{}", self.to_address_string())
    }

    /// Parse and validate a segmented address string.
    /// Rejects typos, checksum mismatches, and malformed formats.
    pub fn parse_address(input: &str) -> Result<Self, VaultError> {
        let clean = input.trim().to_ascii_lowercase();
        let parts: Vec<&str> = clean.split('-').collect();

        if parts.len() != 6 || parts[0] != "s3a" {
            return Err(VaultError::InvalidAddressFormat(format!(
                "Address must match s3a-XXXX-XXXX-XXXX-XXXX-CC, got '{}'",
                input
            )));
        }

        // Each data segment must be 4 hex chars
        for &seg in &parts[1..5] {
            if seg.len() != 4 || !seg.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(VaultError::InvalidAddressFormat(format!(
                    "Invalid segment '{}'",
                    seg
                )));
            }
        }

        // Checksum segment must be 2 hex chars
        if parts[5].len() != 2 || !parts[5].chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(VaultError::InvalidAddressFormat(format!(
                "Invalid checksum segment '{}'",
                parts[5]
            )));
        }

        let combined_hex = format!("{}{}{}{}", parts[1], parts[2], parts[3], parts[4]);
        let mut prefix = [0u8; 8];
        hex::decode_to_slice(&combined_hex, &mut prefix)
            .map_err(|e| VaultError::InvalidAddressFormat(e.to_string()))?;

        let expected_checksum = (crc32c::crc32c(&prefix) & 0xFF) as u8;
        let mut checksum_bytes = [0u8; 1];
        hex::decode_to_slice(parts[5], &mut checksum_bytes)
            .map_err(|e| VaultError::InvalidAddressFormat(e.to_string()))?;

        if checksum_bytes[0] != expected_checksum {
            return Err(VaultError::InvalidAddressChecksum);
        }

        Ok(Self {
            prefix,
            checksum: expected_checksum,
        })
    }

    /// Parse a URI (`gate://...`).
    pub fn parse_uri(uri: &str) -> Result<Self, VaultError> {
        let clean = uri.trim();
        if !clean.starts_with("gate://") {
            return Err(VaultError::InvalidAddressFormat(format!(
                "URI must begin with gate://, got '{}'",
                uri
            )));
        }
        let addr_part = &clean["gate://".len()..];
        Self::parse_address(addr_part)
    }
}

/// Endpoint reference for dialing (address or named gate).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateTarget {
    Address(GateAddress),
    Name(String),
}

impl GateTarget {
    pub fn parse(uri: &str) -> Result<Self, VaultError> {
        let clean = uri.trim();
        if !clean.starts_with("gate://") {
            return Err(VaultError::InvalidAddressFormat(format!(
                "URI must begin with gate://, got '{}'",
                uri
            )));
        }
        let target = &clean["gate://".len()..];
        if target.starts_with("s3a-") {
            let addr = GateAddress::parse_address(target)?;
            Ok(GateTarget::Address(addr))
        } else {
            if target.is_empty() {
                return Err(VaultError::InvalidAddressFormat("Empty gate target".into()));
            }
            Ok(GateTarget::Name(target.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gate_address_roundtrip_and_typo_rejection() {
        let sample_pubkey: [u8; 32] = [42u8; 32];
        let addr = GateAddress::from_public_key(&sample_pubkey);
        let addr_str = addr.to_address_string();
        let uri = addr.to_uri();

        assert!(addr_str.starts_with("s3a-"));
        assert!(uri.starts_with("gate://s3a-"));

        // Round-trip parse address
        let parsed = GateAddress::parse_address(&addr_str).expect("Valid address must parse");
        assert_eq!(parsed, addr);

        // Round-trip parse URI
        let parsed_uri = GateAddress::parse_uri(&uri).expect("Valid URI must parse");
        assert_eq!(parsed_uri, addr);

        // Parse through GateTarget
        match GateTarget::parse(&uri).expect("GateTarget parse must succeed") {
            GateTarget::Address(a) => assert_eq!(a, addr),
            GateTarget::Name(_) => panic!("Expected Address target"),
        }

        // Test named target
        match GateTarget::parse("gate://lab-alpha.biology").expect("Named target must parse") {
            GateTarget::Name(n) => assert_eq!(n, "lab-alpha.biology"),
            GateTarget::Address(_) => panic!("Expected Name target"),
        }

        // Typo rejection: flip a character in one segment
        let mut corrupted_chars: Vec<char> = addr_str.chars().collect();
        // Flip char at position 5
        corrupted_chars[5] = if corrupted_chars[5] == '0' { '1' } else { '0' };
        let corrupted_str: String = corrupted_chars.into_iter().collect();
        let err = GateAddress::parse_address(&corrupted_str);
        assert!(matches!(err, Err(VaultError::InvalidAddressChecksum)));

        // Typo in checksum segment
        let mut corrupted_chk: Vec<char> = addr_str.chars().collect();
        let len = corrupted_chk.len();
        corrupted_chk[len - 1] = if corrupted_chk[len - 1] == 'a' {
            'b'
        } else {
            'a'
        };
        let corrupted_chk_str: String = corrupted_chk.into_iter().collect();
        assert!(matches!(
            GateAddress::parse_address(&corrupted_chk_str),
            Err(VaultError::InvalidAddressChecksum)
        ));

        // Format error: invalid segment length
        assert!(matches!(
            GateAddress::parse_address("s3a-12-34-56-78-90"),
            Err(VaultError::InvalidAddressFormat(_))
        ));
    }
}
