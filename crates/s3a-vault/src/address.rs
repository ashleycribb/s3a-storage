//! Gate Addresses and Dialing URI protocol (§11).

use crate::errors::VaultError;
use sha2::{Digest, Sha256};

/// Prefix of a v2 (full 256-bit) gate address: `s3a2-<64 hex>-<4 hex check>`.
pub const V2_PREFIX: &str = "s3a2-";

/// A Gate Address derived from an Ed25519 public key (§11.1).
///
/// Two formats exist:
/// * **v2 (default)** `s3a2-<64 hex>-CCCC`: the full SHA-256 of the public key
///   plus a 16-bit CRC32C typo check. Binds to exactly one key.
/// * **legacy v1** `s3a-XXXX-XXXX-XXXX-XXXX-CC`: only the first 64 bits of the
///   digest. Still parsed for old links, but flagged as weaker: a determined
///   attacker can grind a different key with the same 64-bit prefix. A legacy
///   address is never proof of identity on its own; callers must check the
///   full key with [`GateAddress::matches_key`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GateAddress {
    /// 8-byte digest prefix of SHA-256(public_key)
    pub prefix: [u8; 8],
    /// 1-byte checksum (CRC32C(prefix) & 0xFF), kept for v1 compatibility
    pub checksum: u8,
    /// Full SHA-256(public_key); `None` for a parsed legacy v1 address.
    pub full: Option<[u8; 32]>,
}

fn v2_check(d: &[u8; 32]) -> u16 {
    (crc32c::crc32c(d) & 0xFFFF) as u16
}

impl GateAddress {
    /// Derive a (v2) Gate Address from an Ed25519 public key.
    pub fn from_public_key(pubkey: &[u8; 32]) -> Self {
        let digest: [u8; 32] = Sha256::digest(pubkey).into();
        let mut prefix = [0u8; 8];
        prefix.copy_from_slice(&digest[0..8]);
        let checksum = (crc32c::crc32c(&prefix) & 0xFF) as u8;
        Self {
            prefix,
            checksum,
            full: Some(digest),
        }
    }

    /// True when this is a short 64-bit legacy address (weaker; see type docs).
    pub fn is_legacy(&self) -> bool {
        self.full.is_none()
    }

    /// Warning text for legacy addresses, `None` for v2.
    pub fn legacy_warning(&self) -> Option<&'static str> {
        self.is_legacy().then_some(
            "legacy 64-bit gate address: weaker, could be spoofed by a ground key; verify the full key",
        )
    }

    /// The legacy v1 form of this address (64-bit prefix).
    pub fn to_legacy_string(&self) -> String {
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

    /// Canonical string: v2 when the full digest is known, legacy otherwise.
    pub fn to_address_string(&self) -> String {
        match &self.full {
            Some(d) => format!("{}{}-{:04x}", V2_PREFIX, hex::encode(d), v2_check(d)),
            None => self.to_legacy_string(),
        }
    }

    /// Format as canonical URI: `gate://<address>`.
    pub fn to_uri(&self) -> String {
        format!("gate://{}", self.to_address_string())
    }

    /// Does `pubkey` own this address? v2 compares all 256 bits; legacy only
    /// the 64-bit prefix (weaker; callers should surface [`Self::legacy_warning`]).
    pub fn matches_key(&self, pubkey: &[u8; 32]) -> bool {
        let derived = Self::from_public_key(pubkey);
        match &self.full {
            Some(d) => *d == derived.full.unwrap(),
            None => self.prefix == derived.prefix,
        }
    }

    /// Do two addresses name the same identity? Full digests are compared when
    /// both are v2; if either is legacy only the 64-bit prefix can be compared.
    pub fn same_identity(&self, other: &Self) -> bool {
        match (&self.full, &other.full) {
            (Some(a), Some(b)) => a == b,
            _ => self.prefix == other.prefix,
        }
    }

    /// Parse and validate a segmented address string (v2 or legacy v1).
    /// Rejects typos, checksum mismatches, and malformed formats.
    pub fn parse_address(input: &str) -> Result<Self, VaultError> {
        let clean = input.trim().to_ascii_lowercase();
        if let Some(rest) = clean.strip_prefix(V2_PREFIX) {
            return Self::parse_v2(rest, input);
        }
        Self::parse_legacy(&clean, input)
    }

    fn parse_v2(rest: &str, input: &str) -> Result<Self, VaultError> {
        let bad = || {
            VaultError::InvalidAddressFormat(format!(
                "Address must match s3a2-<64 hex>-CCCC, got '{}'",
                input
            ))
        };
        let (hexd, chk) = rest.split_once('-').ok_or_else(bad)?;
        if hexd.len() != 64 || chk.len() != 4 {
            return Err(bad());
        }
        let mut d = [0u8; 32];
        hex::decode_to_slice(hexd, &mut d).map_err(|_| bad())?;
        let c = u16::from_str_radix(chk, 16).map_err(|_| bad())?;
        if !chk.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(bad());
        }
        if c != v2_check(&d) {
            return Err(VaultError::InvalidAddressChecksum);
        }
        let mut prefix = [0u8; 8];
        prefix.copy_from_slice(&d[0..8]);
        Ok(Self {
            prefix,
            checksum: (crc32c::crc32c(&prefix) & 0xFF) as u8,
            full: Some(d),
        })
    }

    fn parse_legacy(clean: &str, input: &str) -> Result<Self, VaultError> {
        let parts: Vec<&str> = clean.split('-').collect();

        if parts.len() != 6 || parts[0] != "s3a" {
            return Err(VaultError::InvalidAddressFormat(format!(
                "Address must match s3a2-<64 hex>-CCCC or legacy s3a-XXXX-XXXX-XXXX-XXXX-CC, got '{}'",
                input
            )));
        }

        for &seg in &parts[1..5] {
            if seg.len() != 4 || !seg.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(VaultError::InvalidAddressFormat(format!(
                    "Invalid segment '{}'",
                    seg
                )));
            }
        }

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
            full: None,
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
        if target.starts_with("s3a-") || target.starts_with(V2_PREFIX) {
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

        assert!(addr_str.starts_with("s3a2-"));
        assert!(uri.starts_with("gate://s3a2-"));

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

    #[test]
    fn v2_is_256_bit_and_versioned() {
        let a = GateAddress::from_public_key(&[7u8; 32]);
        let s = a.to_address_string();
        assert!(s.starts_with("s3a2-"));
        assert_eq!(s.len(), 5 + 64 + 1 + 4);
        assert!(!a.is_legacy() && a.legacy_warning().is_none());
        assert_eq!(
            GateAddress::parse_address(&s.to_uppercase().replace("S3A2", "s3a2")).unwrap(),
            a
        );
    }

    #[test]
    fn legacy_still_parses_but_is_flagged() {
        let pk = [9u8; 32];
        let a = GateAddress::from_public_key(&pk);
        let legacy = GateAddress::parse_address(&a.to_legacy_string()).unwrap();
        assert!(legacy.is_legacy() && legacy.legacy_warning().is_some());
        assert_eq!(legacy.to_address_string(), a.to_legacy_string());
        assert!(legacy.matches_key(&pk));
        assert!(legacy.same_identity(&a));
        match GateTarget::parse(&format!("gate://{}", a.to_legacy_string())).unwrap() {
            GateTarget::Address(x) => assert!(x.is_legacy()),
            _ => panic!(),
        }
    }

    #[test]
    fn forged_key_with_same_short_prefix_rejected_by_v2() {
        let real_pk = [1u8; 32];
        let real = GateAddress::from_public_key(&real_pk);
        // Simulate a ground 64-bit collision: an attacker key whose address
        // shares the real prefix but differs in the remaining 192 bits.
        let forged_pk = [2u8; 32];
        let mut forged = GateAddress::from_public_key(&forged_pk);
        forged.prefix = real.prefix;
        forged.checksum = real.checksum;
        // A legacy address cannot tell them apart...
        let legacy = GateAddress::parse_address(&real.to_legacy_string()).unwrap();
        assert!(legacy.same_identity(&forged));
        // ...the v2 address can.
        assert!(!real.same_identity(&forged));
        assert!(!real.matches_key(&forged_pk));
        assert!(real.matches_key(&real_pk));
        let parsed = GateAddress::parse_address(&real.to_address_string()).unwrap();
        assert!(!parsed.matches_key(&forged_pk));
    }

    #[test]
    fn v2_typo_and_format_rejected() {
        let s = GateAddress::from_public_key(&[3u8; 32]).to_address_string();
        let mut c: Vec<char> = s.chars().collect();
        c[10] = if c[10] == '0' { '1' } else { '0' };
        let t: String = c.into_iter().collect();
        assert!(matches!(
            GateAddress::parse_address(&t),
            Err(VaultError::InvalidAddressChecksum)
        ));
        assert!(matches!(
            GateAddress::parse_address("s3a2-abcd-0000"),
            Err(VaultError::InvalidAddressFormat(_))
        ));
    }
}
