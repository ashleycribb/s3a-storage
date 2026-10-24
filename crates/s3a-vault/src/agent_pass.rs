//! Owner-signed agent passes (Block deep dive #1: Buzz NIP-OA, adapted to Ed25519).
//!
//! A pass is an owner's signed statement that one agent key may call a listed set of
//! tool kinds within listed scopes during a time window. It is authorization evidence
//! only: the agent still acts under its own key.
//!
//! Rules:
//! * Signed bytes are `AGENT_PASS_DOMAIN || canonical(conditions)` (domain separation, so a
//!   pass signature can never be replayed as a commit, grant or share signature).
//! * Judgment actions (see [`crate::delegation::is_judgment_action`]) can never be granted:
//!   issuing or verifying a pass that lists one fails.
//! * Revoking the owner key in a [`RevocationRegistry`] invalidates every pass it signed,
//!   regardless of the revocation time.
use crate::delegation::is_judgment_action;
use crate::errors::VaultError;
use crate::keystore::KeyStore;
use crate::rotation::RevocationRegistry;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};

pub const AGENT_PASS_DOMAIN: &[u8] = b"s3a-agent-pass-v1\0";

/// The signed conditions. Lists are sorted and de-duplicated before signing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PassConditions {
    pub pass_id: String,
    /// Hex Ed25519 public key of the agent this pass is for.
    pub agent_pubkey: String,
    /// Hex Ed25519 public key of the owner who signs.
    pub owner_pubkey: String,
    /// Tool kinds (tool names) the agent may call.
    pub tool_kinds: Vec<String>,
    /// Scopes the agent may act in. A call that names a scope must name one of these.
    pub scopes: Vec<String>,
    pub not_before: u64,
    pub not_after: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentPass {
    pub conditions: PassConditions,
    /// Hex Ed25519 signature by `owner_pubkey`.
    pub signature: String,
}

impl PassConditions {
    fn normalized(&self) -> Self {
        let mut c = self.clone();
        c.tool_kinds.sort();
        c.tool_kinds.dedup();
        c.scopes.sort();
        c.scopes.dedup();
        c
    }

    /// Bytes the owner signs: domain label followed by canonical JSON.
    pub fn signing_bytes(&self) -> Vec<u8> {
        let mut b = AGENT_PASS_DOMAIN.to_vec();
        b.extend(serde_json::to_vec(&self.normalized()).expect("serializable"));
        b
    }

    fn check_shape(&self) -> Result<(), VaultError> {
        if self.tool_kinds.iter().any(|k| is_judgment_action(k)) {
            return Err(VaultError::RequiresHumanApproval(
                "Judgment actions can never be granted in an agent pass",
            ));
        }
        if self.not_after <= self.not_before {
            return Err(VaultError::CorruptedData("agent pass time window is empty"));
        }
        if self.agent_pubkey == self.owner_pubkey {
            return Err(VaultError::SelfGrantForbidden);
        }
        Ok(())
    }
}

fn key32(hex_str: &str, what: &'static str) -> Result<[u8; 32], VaultError> {
    hex::decode(hex_str)
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or(VaultError::InvalidSignature(what))
}

/// Issue a pass signed by the owner identity held in `keystore`.
pub fn issue_agent_pass(
    keystore: &impl KeyStore,
    owner_id: &[u8; 16],
    mut conditions: PassConditions,
) -> Result<AgentPass, VaultError> {
    conditions.owner_pubkey = hex::encode(keystore.get_public_key(owner_id)?);
    let conditions = conditions.normalized();
    conditions.check_shape()?;
    key32(&conditions.agent_pubkey, "invalid agent public key")?;
    let sig = keystore.sign(owner_id, &conditions.signing_bytes())?;
    Ok(AgentPass {
        conditions,
        signature: hex::encode(sig),
    })
}

/// Verify signature, shape, time window and owner revocation at time `now`.
pub fn verify_agent_pass(
    pass: &AgentPass,
    now: u64,
    revocations: &RevocationRegistry,
) -> Result<(), VaultError> {
    let c = &pass.conditions;
    c.check_shape()?;
    let owner = key32(&c.owner_pubkey, "invalid owner public key")?;
    key32(&c.agent_pubkey, "invalid agent public key")?;
    let sig: [u8; 64] = hex::decode(&pass.signature)
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or(VaultError::InvalidSignature(
            "malformed agent pass signature",
        ))?;
    VerifyingKey::from_bytes(&owner)
        .map_err(|_| VaultError::InvalidSignature("invalid owner public key"))?
        .verify_strict(&c.signing_bytes(), &Signature::from_bytes(&sig))
        .map_err(|_| VaultError::InvalidSignature("agent pass signature mismatch"))?;
    if revocations.is_revoked(&owner) {
        return Err(VaultError::KeyRevokedError(owner));
    }
    if now < c.not_before || now > c.not_after {
        return Err(VaultError::GrantExpiredOrExhausted);
    }
    Ok(())
}

/// Verify the pass and check that it covers `tool_kind` (and `scope`, if the call names one)
/// for `agent_pubkey`.
pub fn check_agent_pass(
    pass: &AgentPass,
    agent_pubkey_hex: &str,
    tool_kind: &str,
    scope: Option<&str>,
    now: u64,
    revocations: &RevocationRegistry,
) -> Result<(), VaultError> {
    if is_judgment_action(tool_kind) {
        return Err(VaultError::RequiresHumanApproval(
            "Judgment action cannot be delegated to an agent",
        ));
    }
    verify_agent_pass(pass, now, revocations)?;
    let c = &pass.conditions;
    if !c.agent_pubkey.eq_ignore_ascii_case(agent_pubkey_hex) {
        return Err(VaultError::InvalidSignature(
            "agent pass is for a different agent key",
        ));
    }
    if !c.tool_kinds.iter().any(|k| k == tool_kind) {
        return Err(VaultError::GrantExpiredOrExhausted);
    }
    if let Some(s) = scope {
        if !c.scopes.iter().any(|x| x == s) {
            return Err(VaultError::GrantExpiredOrExhausted);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keystore::MemoryKeyStore;
    use crate::records::RevokedKeyRecord;

    const OWNER: [u8; 16] = [1; 16];
    const AGENT: [u8; 16] = [2; 16];

    fn setup() -> (MemoryKeyStore, String) {
        let mut ks = MemoryKeyStore::new();
        ks.generate_keypair(&OWNER).unwrap();
        let agent = hex::encode(ks.generate_keypair(&AGENT).unwrap());
        (ks, agent)
    }
    fn conds(agent: &str, kinds: &[&str]) -> PassConditions {
        PassConditions {
            pass_id: "p1".into(),
            agent_pubkey: agent.into(),
            owner_pubkey: String::new(),
            tool_kinds: kinds.iter().map(|s| s.to_string()).collect(),
            scopes: vec!["lab-a".into()],
            not_before: 100,
            not_after: 200,
        }
    }

    #[test]
    fn valid_pass_covers_listed_tool_and_scope_only() {
        let (ks, agent) = setup();
        let p = issue_agent_pass(&ks, &OWNER, conds(&agent, &["s3a_query"])).unwrap();
        let r = RevocationRegistry::new();
        check_agent_pass(&p, &agent, "s3a_query", Some("lab-a"), 150, &r).unwrap();
        check_agent_pass(&p, &agent, "s3a_query", None, 150, &r).unwrap();
        assert!(check_agent_pass(&p, &agent, "s3a_write", None, 150, &r).is_err());
        assert!(check_agent_pass(&p, &agent, "s3a_query", Some("lab-b"), 150, &r).is_err());
        assert!(check_agent_pass(&p, &"00".repeat(32), "s3a_query", None, 150, &r).is_err());
    }

    #[test]
    fn expired_and_not_yet_valid_pass_refused() {
        let (ks, agent) = setup();
        let p = issue_agent_pass(&ks, &OWNER, conds(&agent, &["s3a_query"])).unwrap();
        let r = RevocationRegistry::new();
        assert!(matches!(
            verify_agent_pass(&p, 201, &r),
            Err(VaultError::GrantExpiredOrExhausted)
        ));
        assert!(matches!(
            verify_agent_pass(&p, 99, &r),
            Err(VaultError::GrantExpiredOrExhausted)
        ));
    }

    #[test]
    fn tampered_pass_refused() {
        let (ks, agent) = setup();
        let mut p = issue_agent_pass(&ks, &OWNER, conds(&agent, &["s3a_query"])).unwrap();
        p.conditions.tool_kinds.push("s3a_write".into());
        assert!(matches!(
            verify_agent_pass(&p, 150, &RevocationRegistry::new()),
            Err(VaultError::InvalidSignature(_))
        ));
        let mut p2 = issue_agent_pass(&ks, &OWNER, conds(&agent, &["s3a_query"])).unwrap();
        p2.conditions.not_after = 10_000;
        assert!(verify_agent_pass(&p2, 150, &RevocationRegistry::new()).is_err());
    }

    #[test]
    fn domain_separation_signature_not_reusable() {
        let (ks, agent) = setup();
        let c = conds(&agent, &["s3a_query"]);
        // A signature over the bare JSON (no domain label) must not verify as a pass.
        let mut c2 = c.clone();
        c2.owner_pubkey = hex::encode(ks.get_public_key(&OWNER).unwrap());
        let raw = serde_json::to_vec(&c2.normalized()).unwrap();
        let sig = ks.sign(&OWNER, &raw).unwrap();
        let p = AgentPass {
            conditions: c2.normalized(),
            signature: hex::encode(sig),
        };
        assert!(verify_agent_pass(&p, 150, &RevocationRegistry::new()).is_err());
    }

    #[test]
    fn revoked_owner_invalidates_all_passes() {
        let (ks, agent) = setup();
        let p1 = issue_agent_pass(&ks, &OWNER, conds(&agent, &["s3a_query"])).unwrap();
        let mut c = conds(&agent, &["s3a_write"]);
        c.pass_id = "p2".into();
        let p2 = issue_agent_pass(&ks, &OWNER, c).unwrap();
        let mut r = RevocationRegistry::new();
        let mut rec = RevokedKeyRecord {
            revocation_id: [9; 16],
            revoked_public_key: ks.get_public_key(&OWNER).unwrap(),
            revoked_at: 500u64.to_le_bytes(),
            reason_code: 1,
            record_crc32: 0,
        };
        rec.record_crc32 = rec.compute_crc();
        r.register_revocation(&rec).unwrap();
        for p in [&p1, &p2] {
            // Revoked later than `now`, still refused: revocation kills every pass.
            assert!(matches!(
                verify_agent_pass(p, 150, &r),
                Err(VaultError::KeyRevokedError(_))
            ));
        }
    }

    #[test]
    fn judgment_kinds_can_never_be_granted() {
        let (ks, agent) = setup();
        for k in ["erasure_execute", "consent_withdraw", "revoke_delegation"] {
            assert!(matches!(
                issue_agent_pass(&ks, &OWNER, conds(&agent, &["s3a_query", k])),
                Err(VaultError::RequiresHumanApproval(_))
            ));
        }
        // Even a hand-signed pass listing a judgment kind fails verification.
        let mut c = conds(&agent, &["erasure_execute"]);
        c.owner_pubkey = hex::encode(ks.get_public_key(&OWNER).unwrap());
        let sig = ks.sign(&OWNER, &c.signing_bytes()).unwrap();
        let p = AgentPass {
            conditions: c.normalized(),
            signature: hex::encode(sig),
        };
        assert!(verify_agent_pass(&p, 150, &RevocationRegistry::new()).is_err());
    }

    #[test]
    fn self_pass_refused() {
        let (ks, _) = setup();
        let own = hex::encode(ks.get_public_key(&OWNER).unwrap());
        assert!(matches!(
            issue_agent_pass(&ks, &OWNER, conds(&own, &["s3a_query"])),
            Err(VaultError::SelfGrantForbidden)
        ));
    }
}
