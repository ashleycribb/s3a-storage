//! Short-lived signed Access Codes for gate session capability grants (§11.4).

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use crate::address::GateAddress;
use crate::errors::VaultError;

/// Short-lived capability grant issued to callers dialing a gate (§11.4).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccessCode {
    pub code_id: [u8; 16],
    pub target_gate: String,
    pub issuer_pubkey: [u8; 32],
    pub allowed_scopes: u32,
    pub allowed_actions: u32,
    pub expires_at: u64,
    pub max_use_count: u32,
    pub signature: String, // hex encoded 64-byte Ed25519 signature
}

impl AccessCode {
    /// Create and sign a new AccessCode.
    pub fn issue(
        code_id: [u8; 16],
        target_gate: &GateAddress,
        issuer_signing_key: &SigningKey,
        allowed_scopes: u32,
        allowed_actions: u32,
        expires_at: u64,
        max_use_count: u32,
    ) -> Self {
        let issuer_pubkey = issuer_signing_key.verifying_key().to_bytes();
        let target_gate_str = target_gate.to_address_string();

        let mut code = Self {
            code_id,
            target_gate: target_gate_str,
            issuer_pubkey,
            allowed_scopes,
            allowed_actions,
            expires_at,
            max_use_count,
            signature: String::new(),
        };

        let signable = code.signable_bytes();
        let sig = issuer_signing_key.sign(&signable);
        code.signature = hex::encode(sig.to_bytes());
        code
    }

    /// Signable bytes for cryptographic verification.
    pub fn signable_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&self.code_id);
        bytes.extend_from_slice(self.target_gate.as_bytes());
        bytes.extend_from_slice(&self.issuer_pubkey);
        bytes.extend_from_slice(&self.allowed_scopes.to_le_bytes());
        bytes.extend_from_slice(&self.allowed_actions.to_le_bytes());
        bytes.extend_from_slice(&self.expires_at.to_le_bytes());
        bytes.extend_from_slice(&self.max_use_count.to_le_bytes());
        bytes
    }

    /// Verify signature and structure.
    pub fn verify_signature(&self) -> Result<(), VaultError> {
        let verifying_key = VerifyingKey::from_bytes(&self.issuer_pubkey).map_err(|_| {
            VaultError::InvalidSignature("Invalid issuer public key in access code")
        })?;
        let mut sig_bytes = [0u8; 64];
        hex::decode_to_slice(&self.signature, &mut sig_bytes)
            .map_err(|_| VaultError::InvalidSignature("Invalid hex signature in access code"))?;
        let sig = Signature::from_bytes(&sig_bytes);
        verifying_key
            .verify_strict(&self.signable_bytes(), &sig)
            .map_err(|_| VaultError::InvalidSignature("Access code signature mismatch"))?;
        Ok(())
    }

    /// Format as compact token string.
    pub fn to_token(&self) -> Result<String, VaultError> {
        let json = serde_json::to_string(self)
            .map_err(|_| VaultError::CorruptedData("Serialization failed"))?;
        Ok(format!("s3a-ac.{}", hex::encode(json)))
    }

    /// Parse from compact token string.
    pub fn from_token(token: &str) -> Result<Self, VaultError> {
        let clean = token.trim();
        if !clean.starts_with("s3a-ac.") {
            return Err(VaultError::CorruptedData("Invalid access code prefix"));
        }
        let hex_payload = &clean["s3a-ac.".len()..];
        let json_bytes = hex::decode(hex_payload)
            .map_err(|_| VaultError::CorruptedData("Invalid hex payload in token"))?;
        let code: Self = serde_json::from_slice(&json_bytes)
            .map_err(|_| VaultError::CorruptedData("Invalid access code JSON in token"))?;
        code.verify_signature()?;
        Ok(code)
    }
}

/// In-memory tracker for Access Code session usage and revocations.
#[derive(Default)]
pub struct AccessCodeSessionManager {
    revoked_codes: HashSet<[u8; 16]>,
    use_counts: std::collections::HashMap<[u8; 16], u32>,
}

impl AccessCodeSessionManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn revoke(&mut self, code_id: [u8; 16]) {
        self.revoked_codes.insert(code_id);
    }

    pub fn is_revoked(&self, code_id: &[u8; 16]) -> bool {
        self.revoked_codes.contains(code_id)
    }

    /// Validate code and record a single use. Returns Ok if session is permitted.
    pub fn consume(
        &mut self,
        code: &AccessCode,
        target_gate: &GateAddress,
        now: u64,
    ) -> Result<(), VaultError> {
        // 1. Signature validity
        code.verify_signature()?;

        // 2. Revocation check
        if self.is_revoked(&code.code_id) {
            return Err(VaultError::GrantExpiredOrExhausted);
        }

        // 3. Target gate match
        if code.target_gate != target_gate.to_address_string() {
            return Err(VaultError::InvalidAddressFormat(
                "Target gate mismatch".into(),
            ));
        }

        // 4. Expiry
        if now > code.expires_at {
            return Err(VaultError::GrantExpiredOrExhausted);
        }

        // 5. Use count limit
        let current_uses = self.use_counts.entry(code.code_id).or_insert(0);
        if *current_uses >= code.max_use_count {
            return Err(VaultError::GrantExpiredOrExhausted);
        }

        *current_uses += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::records::{ACTION_READ, ACTION_WRITE};
    use rand_core::OsRng;

    #[test]
    fn test_access_code_issuance_token_and_consumption() {
        let signing_key = SigningKey::generate(&mut OsRng);
        let gate = GateAddress::from_public_key(&signing_key.verifying_key().to_bytes());
        let code_id = [42u8; 16];
        let now = 1000;
        let expires = 1500;
        let max_uses = 2;

        let code = AccessCode::issue(
            code_id,
            &gate,
            &signing_key,
            0xFFFF,
            ACTION_READ | ACTION_WRITE,
            expires,
            max_uses,
        );

        // Token serialization round-trip
        let token = code.to_token().unwrap();
        let parsed = AccessCode::from_token(&token).unwrap();
        assert_eq!(code, parsed);

        // Session consumption
        let mut session_mgr = AccessCodeSessionManager::new();

        // 1st use succeeds
        assert!(session_mgr.consume(&code, &gate, now + 10).is_ok());
        // 2nd use succeeds
        assert!(session_mgr.consume(&code, &gate, now + 20).is_ok());
        // 3rd use fails: max_uses reached
        assert!(matches!(
            session_mgr.consume(&code, &gate, now + 30),
            Err(VaultError::GrantExpiredOrExhausted)
        ));

        // Test expiry
        let code2 = AccessCode::issue(
            [43u8; 16],
            &gate,
            &signing_key,
            0xFFFF,
            ACTION_READ,
            expires,
            10,
        );
        let mut session_mgr2 = AccessCodeSessionManager::new();
        assert!(matches!(
            session_mgr2.consume(&code2, &gate, expires + 1),
            Err(VaultError::GrantExpiredOrExhausted)
        ));

        // Test revocation
        let code3 = AccessCode::issue(
            [44u8; 16],
            &gate,
            &signing_key,
            0xFFFF,
            ACTION_READ,
            expires,
            10,
        );
        let mut session_mgr3 = AccessCodeSessionManager::new();
        session_mgr3.revoke(code3.code_id);
        assert!(matches!(
            session_mgr3.consume(&code3, &gate, now),
            Err(VaultError::GrantExpiredOrExhausted)
        ));
    }
}
