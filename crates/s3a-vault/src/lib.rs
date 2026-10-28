//! S3A Digital Vault (S3A-VAULT)
//!
//! Sovereign cryptographic identity, verifiable credentials, delegation grants,
//! physical asset custody, and gate network addressing stored natively in standard S3A archives.

pub mod access_code;
pub mod address;
pub mod agent_pass;
pub mod backup;
pub mod delegation;
pub mod errors;
pub mod keystore;
pub mod records;
pub mod rotation;
pub mod share;
pub mod share_cli;
pub mod transfer;
pub mod vault;
pub mod verify;

pub use access_code::{AccessCode, AccessCodeSessionManager};
pub use address::{GateAddress, GateTarget};
pub use agent_pass::{
    check_agent_pass, issue_agent_pass, verify_agent_pass, AgentPass, PassConditions,
    AGENT_PASS_DOMAIN,
};
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
pub use verify::{ChainCheck, GrantCheck, TransferCheck, VaultVerifyReport};

pub fn version() -> &'static str {
    "0.1.0"
}
