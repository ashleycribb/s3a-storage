//! S3A Digital Vault (S3A-VAULT)
//!
//! Sovereign cryptographic identity, verifiable credentials, delegation grants,
//! physical asset custody, and gate network addressing stored natively in standard S3A archives.

pub mod access_code;
pub mod address;
pub mod backup;
pub mod delegation;
pub mod errors;
pub mod keystore;
pub mod records;
pub mod rotation;
pub mod transfer;
pub mod vault;

pub use access_code::{AccessCode, AccessCodeSessionManager};
pub use address::{GateAddress, GateTarget};
pub use backup::{create_backup, restore_backup};
pub use delegation::{
    create_delegation_grant, evaluate_agent_action, is_judgment_action, verify_delegation_grant,
};
pub use errors::VaultError;
pub use keystore::{FileKeyStore, KeyStore, MemoryKeyStore};
pub use records::*;
pub use rotation::{create_key_rotation_record, verify_key_rotation_record, RevocationRegistry};
pub use transfer::{
    compute_transfer_hash, create_transfer_record, resolve_chain_ownership, verify_transfer_record,
};
pub use vault::Vault;

pub fn version() -> &'static str {
    "0.1.0"
}
