//! Algorithm identifier registry (TILE-SPEC §14.5.4, docs/spec/registry.md "Algorithm IDs").
//!
//! An archive may declare the algorithms it uses in its about entry (`seal_hash_alg=`,
//! `sig_alg=`). Absent keys mean the algorithms every S3A file has always used, so files
//! written before this registry existed keep their exact meaning.

use alloc::string::String;

/// What an algorithm ID is used for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlgKind {
    /// Hash used for payload hashes, the seal chain and record roots.
    SealHash,
    /// Signature over seal hashes.
    Signature,
}

/// Registry status of an algorithm ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AlgStatus {
    /// Writers SHOULD use it; readers MUST implement it.
    Recommended,
    /// Writers MAY use it; readers MUST implement it.
    Allowed,
    /// Writers MUST NOT create new data with it; readers MUST still verify it on old files.
    Deprecated,
    /// The ID is assigned but its S3A encoding is not specified yet. Readers MUST reject.
    Reserved,
}

impl AlgStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            AlgStatus::Recommended => "recommended",
            AlgStatus::Allowed => "allowed",
            AlgStatus::Deprecated => "deprecated",
            AlgStatus::Reserved => "reserved",
        }
    }
}

/// One registry entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlgEntry {
    pub id: &'static str,
    pub kind: AlgKind,
    pub status: AlgStatus,
}

/// The registry. IDs are never reused or removed; only their status changes (RFC 7696 §2.1).
pub const ALGORITHMS: &[AlgEntry] = &[
    AlgEntry {
        id: "sha256",
        kind: AlgKind::SealHash,
        status: AlgStatus::Recommended,
    },
    AlgEntry {
        id: "none",
        kind: AlgKind::Signature,
        status: AlgStatus::Allowed,
    },
    AlgEntry {
        id: "ed25519",
        kind: AlgKind::Signature,
        status: AlgStatus::Recommended,
    },
    AlgEntry {
        id: "ml-dsa-44",
        kind: AlgKind::Signature,
        status: AlgStatus::Reserved,
    },
    AlgEntry {
        id: "ml-dsa-65",
        kind: AlgKind::Signature,
        status: AlgStatus::Reserved,
    },
    AlgEntry {
        id: "ml-dsa-87",
        kind: AlgKind::Signature,
        status: AlgStatus::Reserved,
    },
];

/// Seal hash every archive uses when it declares none.
pub const DEFAULT_SEAL_HASH_ALG: &str = "sha256";

/// Why an algorithm ID cannot be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlgError {
    /// Not in the registry (or registered for the other kind). Readers MUST reject.
    Unknown(&'static str, String),
    /// Registered but reserved. Readers MUST reject.
    Reserved(&'static str),
    /// Writers only: deprecated IDs MUST NOT be used for new data.
    Deprecated(&'static str),
}

impl core::fmt::Display for AlgError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            AlgError::Unknown(k, id) => write!(f, "unknown {k} algorithm id '{id}'"),
            AlgError::Reserved(id) => {
                write!(f, "algorithm id '{id}' is reserved and not yet supported")
            }
            AlgError::Deprecated(id) => write!(f, "algorithm id '{id}' is deprecated for new data"),
        }
    }
}

fn kind_name(kind: AlgKind) -> &'static str {
    match kind {
        AlgKind::SealHash => "seal_hash",
        AlgKind::Signature => "signature",
    }
}

/// Looks up `id` for `kind`.
pub fn lookup(kind: AlgKind, id: &str) -> Option<&'static AlgEntry> {
    ALGORITHMS.iter().find(|e| e.kind == kind && e.id == id)
}

/// Reader rule: recommended, allowed and deprecated IDs verify; unknown and reserved IDs reject.
pub fn check_readable(kind: AlgKind, id: &str) -> Result<AlgStatus, AlgError> {
    match lookup(kind, id) {
        None => Err(AlgError::Unknown(kind_name(kind), id.into())),
        Some(e) if e.status == AlgStatus::Reserved => Err(AlgError::Reserved(e.id)),
        Some(e) => Ok(e.status),
    }
}

/// Writer rule: only recommended and allowed IDs may be written.
pub fn check_writable(kind: AlgKind, id: &str) -> Result<AlgStatus, AlgError> {
    let s = check_readable(kind, id)?;
    if s == AlgStatus::Deprecated {
        return Err(AlgError::Deprecated(lookup(kind, id).unwrap().id));
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_rules() {
        assert_eq!(
            check_readable(AlgKind::SealHash, "sha256"),
            Ok(AlgStatus::Recommended)
        );
        assert_eq!(
            check_readable(AlgKind::Signature, "ed25519"),
            Ok(AlgStatus::Recommended)
        );
        assert!(matches!(
            check_readable(AlgKind::Signature, "ml-dsa-65"),
            Err(AlgError::Reserved(_))
        ));
        assert!(matches!(
            check_readable(AlgKind::SealHash, "md5"),
            Err(AlgError::Unknown(..))
        ));
        // An ID registered for the other kind is unknown here.
        assert!(matches!(
            check_readable(AlgKind::SealHash, "ed25519"),
            Err(AlgError::Unknown(..))
        ));
        assert!(check_writable(AlgKind::SealHash, "sha256").is_ok());
    }

    #[test]
    fn ids_are_unique_per_kind() {
        for (i, a) in ALGORITHMS.iter().enumerate() {
            for b in &ALGORITHMS[i + 1..] {
                assert!(!(a.kind == b.kind && a.id == b.id), "duplicate {}", a.id);
            }
        }
    }
}
