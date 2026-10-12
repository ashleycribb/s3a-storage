//! Error types for S3A Vault

use std::fmt;

#[derive(Debug)]
pub enum VaultError {
    InvalidCrc(&'static str),
    InvalidSignature(&'static str),
    SelfGrantForbidden,
    WidenedGrantForbidden,
    GrantExpiredOrExhausted,
    RequiresHumanApproval(&'static str),
    KeyRevokedError([u8; 32]),
    InvalidAddressChecksum,
    InvalidAddressFormat(String),
    DoubleSendDetected([u8; 32]),
    ConsentViolationError(&'static str),
    KeyStoreError(String),
    ItemNotFound([u8; 16]),
    GrantNotFound([u8; 16]),
    IoError(std::io::Error),
    EngineError(String),
    CorruptedData(&'static str),
}

impl fmt::Display for VaultError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCrc(msg) => write!(f, "Invalid CRC32C checksum: {}", msg),
            Self::InvalidSignature(msg) => write!(f, "Cryptographic signature verification failed: {}", msg),
            Self::SelfGrantForbidden => write!(f, "Self-granting by autonomous agents is strictly forbidden"),
            Self::WidenedGrantForbidden => write!(f, "Sub-grant widens parent delegation permissions"),
            Self::GrantExpiredOrExhausted => write!(f, "Delegation grant has expired or reached max operation limit"),
            Self::RequiresHumanApproval(action) => write!(f, "Non-delegable judgment action requires interactive human approval: {}", action),
            Self::KeyRevokedError(key) => write!(f, "Action rejected: key {} is revoked", hex::encode(key)),
            Self::InvalidAddressChecksum => write!(f, "Gate address checksum mismatch / typo detected"),
            Self::InvalidAddressFormat(msg) => write!(f, "Invalid gate address format: {}", msg),
            Self::DoubleSendDetected(hash) => write!(f, "Double-send detected for item {}", hex::encode(hash)),
            Self::ConsentViolationError(msg) => write!(f, "Consent governance violation: {}", msg),
            Self::KeyStoreError(msg) => write!(f, "KeyStore error: {}", msg),
            Self::ItemNotFound(id) => write!(f, "Vault item {} not found", hex::encode(id)),
            Self::GrantNotFound(id) => write!(f, "Delegation grant {} not found", hex::encode(id)),
            Self::IoError(e) => write!(f, "I/O error: {}", e),
            Self::EngineError(msg) => write!(f, "Storage engine error: {}", msg),
            Self::CorruptedData(msg) => write!(f, "Corrupted archive data: {}", msg),
        }
    }
}

impl std::error::Error for VaultError {}

impl From<std::io::Error> for VaultError {
    fn from(e: std::io::Error) -> Self {
        Self::IoError(e)
    }
}
