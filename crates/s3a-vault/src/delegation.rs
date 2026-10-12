//! Delegation grants, governance invariants, and anti-escalation enforcement (§3, §12.5).

use ed25519_dalek::{Signature, VerifyingKey};
use crate::errors::VaultError;
use crate::keystore::KeyStore;
use crate::records::{DelegationGrantRecord, VaultOwnerKind};

/// Check if an action string corresponds to a Non-Delegable Judgment Action (§3.2, §12.5).
pub fn is_judgment_action(action: &str) -> bool {
    let lower = action.trim().to_ascii_lowercase();
    lower.contains("erasure")
        || lower.contains("consent_withdraw")
        || lower.contains("consent")
        || lower.contains("claim_support")
        || lower.contains("methodological_rigor")
        || lower.contains("overstatement")
        || lower.contains("extrapolation")
        || lower.contains("actuator_override")
        || lower.contains("legal_waiver")
        || lower.contains("revoke_delegation")
        || lower.contains("grant_revoke")
}

/// Helper to create and sign a DelegationGrantRecord.
pub fn create_delegation_grant(
    grant_id: [u8; 16],
    granter_id: [u8; 16],
    _granter_kind: VaultOwnerKind,
    agent_id: [u8; 16],
    agent_pubkey: [u8; 32],
    allowed_actions: u32,
    allowed_scopes: u32,
    max_operations: u32,
    issued_at: u64,
    expires_at: u64,
    parent_grant: Option<&DelegationGrantRecord>,
    keystore: &impl KeyStore,
) -> Result<DelegationGrantRecord, VaultError> {
    let granter_pubkey = keystore.get_public_key(&granter_id)?;

    // 1. Anti-Escalation Rule: No Self-Granting (§3.2)
    if granter_pubkey == agent_pubkey {
        return Err(VaultError::SelfGrantForbidden);
    }

    // 2. Anti-Escalation Rule: No Widening (§3.2)
    if let Some(parent) = parent_grant {
        if (allowed_actions & !parent.allowed_actions) != 0 {
            return Err(VaultError::WidenedGrantForbidden);
        }
        if (allowed_scopes & !parent.allowed_scopes) != 0 {
            return Err(VaultError::WidenedGrantForbidden);
        }
    }

    let mut record = DelegationGrantRecord {
        grant_id,
        granter_pubkey,
        agent_id,
        agent_pubkey,
        allowed_actions,
        allowed_scopes,
        max_operations,
        _pad: [0u8; 4],
        issued_at: issued_at.to_le_bytes(),
        expires_at: expires_at.to_le_bytes(),
        granter_signature: [0u8; 64],
    };

    let sig_bytes = keystore.sign(&granter_id, record.signed_bytes())?;
    record.granter_signature = sig_bytes;
    Ok(record)
}

/// Verify a DelegationGrantRecord cryptographic signature and basic validity.
pub fn verify_delegation_grant(grant: &DelegationGrantRecord) -> Result<(), VaultError> {
    // Self-granting check
    if grant.granter_pubkey == grant.agent_pubkey {
        return Err(VaultError::SelfGrantForbidden);
    }

    let verifying_key = VerifyingKey::from_bytes(&grant.granter_pubkey)
        .map_err(|_| VaultError::InvalidSignature("Invalid granter public key"))?;
    let signature = Signature::from_bytes(&grant.granter_signature);

    verifying_key
        .verify_strict(grant.signed_bytes(), &signature)
        .map_err(|_| VaultError::InvalidSignature("Delegation grant signature mismatch"))?;

    Ok(())
}

/// Validate whether an agent operation is permitted under an active grant.
pub fn evaluate_agent_action(
    grant: &DelegationGrantRecord,
    action_mask: u32,
    action_name: &str,
    current_time: u64,
    invocations: u32,
) -> Result<(), VaultError> {
    // 1. Check Non-Delegable Judgment Invariant (§3.2, §12.5)
    if is_judgment_action(action_name) {
        return Err(VaultError::RequiresHumanApproval("Judgment action cannot be delegated to an agent"));
    }

    // 2. Check Expiration
    if current_time > grant.get_expires_at() {
        return Err(VaultError::GrantExpiredOrExhausted);
    }

    // 3. Check Invocations Limit
    if invocations >= grant.max_operations {
        return Err(VaultError::GrantExpiredOrExhausted);
    }

    // 4. Check Allowed Action Mask
    if (grant.allowed_actions & action_mask) == 0 {
        return Err(VaultError::GrantExpiredOrExhausted);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keystore::MemoryKeyStore;
    use crate::records::{ACTION_READ, ACTION_WRITE, ACTION_SEND_ITEM};

    #[test]
    fn test_delegation_chain_and_governance_rules() {
        let mut ks = MemoryKeyStore::new();

        let human_id = [1u8; 16];
        let _human_pubkey = ks.generate_keypair(&human_id).unwrap();

        let agent1_id = [2u8; 16];
        let agent1_pubkey = ks.generate_keypair(&agent1_id).unwrap();

        let agent2_id = [3u8; 16];
        let agent2_pubkey = ks.generate_keypair(&agent2_id).unwrap();

        let grant_id1 = [10u8; 16];
        let now = 1000;
        let expires = 2000;

        // 1. Valid grant from Human to Agent1
        let grant1 = create_delegation_grant(
            grant_id1,
            human_id,
            VaultOwnerKind::Person,
            agent1_id,
            agent1_pubkey,
            ACTION_READ | ACTION_WRITE,
            0xFFFF,
            50,
            now,
            expires,
            None,
            &ks,
        )
        .expect("Valid grant should succeed");

        assert!(verify_delegation_grant(&grant1).is_ok());

        // 2. Self-grant by Agent1 must be rejected
        let self_grant = create_delegation_grant(
            [11u8; 16],
            agent1_id,
            VaultOwnerKind::Agent,
            agent1_id,
            agent1_pubkey,
            ACTION_READ,
            0xFFFF,
            10,
            now,
            expires,
            Some(&grant1),
            &ks,
        );
        assert!(matches!(self_grant, Err(VaultError::SelfGrantForbidden)));

        // 3. Widening by Agent1 to Agent2 (Agent1 only has READ | WRITE, tries to give SEND_ITEM)
        let widened_grant = create_delegation_grant(
            [12u8; 16],
            agent1_id,
            VaultOwnerKind::Agent,
            agent2_id,
            agent2_pubkey,
            ACTION_READ | ACTION_SEND_ITEM,
            0xFFFF,
            10,
            now,
            expires,
            Some(&grant1),
            &ks,
        );
        assert!(matches!(widened_grant, Err(VaultError::WidenedGrantForbidden)));

        // 4. Valid sub-grant (narrower or equal)
        let sub_grant = create_delegation_grant(
            [13u8; 16],
            agent1_id,
            VaultOwnerKind::Agent,
            agent2_id,
            agent2_pubkey,
            ACTION_READ,
            0xFFFF,
            10,
            now,
            expires,
            Some(&grant1),
            &ks,
        )
        .expect("Sub-grant with narrower permissions should succeed");
        assert!(verify_delegation_grant(&sub_grant).is_ok());

        // 5. Expiration check
        assert!(evaluate_agent_action(&grant1, ACTION_READ, "read_record", now + 10, 0).is_ok());
        assert!(matches!(
            evaluate_agent_action(&grant1, ACTION_READ, "read_record", expires + 1, 0),
            Err(VaultError::GrantExpiredOrExhausted)
        ));

        // 6. Max operations limit
        assert!(matches!(
            evaluate_agent_action(&grant1, ACTION_READ, "read_record", now + 10, 50),
            Err(VaultError::GrantExpiredOrExhausted)
        ));

        // 7. Non-delegable judgment action must be blocked
        assert!(matches!(
            evaluate_agent_action(&grant1, ACTION_WRITE, "erasure_tile_purge", now + 10, 0),
            Err(VaultError::RequiresHumanApproval(_))
        ));
        assert!(matches!(
            evaluate_agent_action(&grant1, ACTION_WRITE, "claim_support_evaluation", now + 10, 0),
            Err(VaultError::RequiresHumanApproval(_))
        ));
    }
}
