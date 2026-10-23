//! S3A Clean Pack: raw-sealed intake, company rulebook, signed cleaned versions.
//!
//! Flow (see `docs/clean-pack.md`):
//! 1. [`raw::intake`] writes untouched source bytes into a sealed raw archive with a
//!    per-item fingerprint (SHA-256 + source metadata). The raw archive is never modified.
//! 2. [`clean::clean`] applies the company [`rulebook::Rulebook`] deterministically and
//!    produces a [`clean::CleanedVersion`]: cleaned records that each cite their raw item,
//!    a change list (rule id + version, made by, approved by), and a review queue for
//!    anything the rulebook does not cover (UNCLASSIFIED / NEEDS_RULE).
//! 3. The cleaned version is hashed and Ed25519-signed, and written to its own sealed archive.
//! 4. [`clean::verify`] checks hash, signature and that every record traces to a raw item.
//!
//! The pack only uses existing generic core tile types (BLOB, RECORD, PROVENANCE); it
//! adds no tile IDs and makes no `s3a-core` changes. AI suggestions are out of scope.

pub mod archive;
pub mod clean;
pub mod cli;
pub mod intake;
pub mod intake_log;
pub mod raw;
pub mod rulebook;

pub use clean::{clean, verify, CleanedVersion, QueueEntry, QueueTag};
pub use raw::{intake, RawItem, RawSnapshot, SourceMeta};
pub use rulebook::{Rulebook, RulebookUpdate};

/// Hex SHA-256 helper.
pub fn sha256_hex(b: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(b))
}

/// Canonical JSON (serde_json with BTreeMap-ordered maps and struct field order).
pub fn canonical<T: serde::Serialize>(v: &T) -> Vec<u8> {
    // Round-trip through Value so all maps are key-sorted.
    let val = serde_json::to_value(v).expect("serializable");
    serde_json::to_vec(&val).expect("serializable")
}
