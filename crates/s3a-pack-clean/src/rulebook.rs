//! Company rulebook: a versioned, human-readable YAML description of the company's data.
//!
//! Rulebook changes are recorded as [`RulebookUpdate`]s signed by a human approver key
//! listed in the *current* rulebook. Keys listed under `agents` can never sign an update.
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Rulebook {
    pub rulebook_version: String,
    #[serde(default)]
    pub company: String,
    /// Hex Ed25519 public keys of humans allowed to sign rulebook updates.
    #[serde(default)]
    pub approvers: Vec<String>,
    /// Hex Ed25519 public keys of AI agents; these may never sign rulebook updates.
    #[serde(default)]
    pub agents: Vec<String>,
    #[serde(default)]
    pub categories: Vec<Category>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Category {
    pub name: String,
    #[serde(rename = "match")]
    pub matcher: Matcher,
    #[serde(default)]
    pub fields: Vec<Field>,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Matcher {
    /// Lower-case file extensions, e.g. `[csv]`. Only CSV content is parsed in v1.
    pub extensions: Vec<String>,
    /// Optional lower-case file-name prefix (naming convention).
    #[serde(default)]
    pub name_prefix: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Field {
    pub name: String,
    /// `string`, `integer`, `decimal`, `date` (YYYY-MM-DD) or `bool`.
    #[serde(rename = "type", default = "default_type")]
    pub ty: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub allowed: Vec<String>,
}
fn default_type() -> String {
    "string".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Rule {
    pub id: String,
    pub version: u32,
    /// `trim`, `uppercase`, `lowercase`, `map`, `date_iso`.
    pub op: String,
    /// Field the rule applies to; all fields when absent.
    #[serde(default)]
    pub field: Option<String>,
    /// For `map`: exact value replacements.
    #[serde(default)]
    pub map: BTreeMap<String, String>,
    /// For `date_iso`: accepted input layouts (`MM/DD/YYYY`, `DD/MM/YYYY`, `DD.MM.YYYY`, `YYYY-MM-DD`).
    #[serde(default)]
    pub formats: Vec<String>,
}

impl Rulebook {
    pub fn parse(yaml: &[u8]) -> Result<Self, String> {
        let rb: Rulebook =
            serde_yaml::from_slice(yaml).map_err(|e| format!("rulebook YAML: {e}"))?;
        rb.validate()?;
        Ok(rb)
    }

    fn validate(&self) -> Result<(), String> {
        if self.rulebook_version.trim().is_empty() {
            return Err("rulebook_version is required".into());
        }
        const OPS: [&str; 5] = ["trim", "uppercase", "lowercase", "map", "date_iso"];
        const TYPES: [&str; 5] = ["string", "integer", "decimal", "date", "bool"];
        for c in &self.categories {
            for r in &c.rules {
                if !OPS.contains(&r.op.as_str()) {
                    return Err(format!("rule {}: unknown op '{}'", r.id, r.op));
                }
            }
            for f in &c.fields {
                if !TYPES.contains(&f.ty.as_str()) {
                    return Err(format!("field {}: unknown type '{}'", f.name, f.ty));
                }
            }
        }
        for a in &self.approvers {
            if self.agents.contains(a) {
                return Err(format!("key {a} cannot be both approver and agent"));
            }
        }
        Ok(())
    }
}

/// A human-signed record moving the rulebook from one version to the next.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RulebookUpdate {
    pub from_sha256: String,
    pub to_sha256: String,
    pub to_version: String,
    pub reason: String,
    pub signer_pubkey: String,
    pub signature: String,
}

impl RulebookUpdate {
    fn body(&self) -> Vec<u8> {
        crate::canonical(&(
            "s3a-rulebook-update-v1",
            &self.from_sha256,
            &self.to_sha256,
            &self.to_version,
            &self.reason,
            &self.signer_pubkey,
        ))
    }

    /// Sign an update from `old` to `new` rulebook bytes.
    pub fn sign(old: &[u8], new: &[u8], reason: &str, key: &SigningKey) -> Result<Self, String> {
        let nb = Rulebook::parse(new)?;
        let mut u = Self {
            from_sha256: crate::sha256_hex(old),
            to_sha256: crate::sha256_hex(new),
            to_version: nb.rulebook_version,
            reason: reason.to_string(),
            signer_pubkey: hex::encode(key.verifying_key().to_bytes()),
            signature: String::new(),
        };
        u.signature = hex::encode(key.sign(&u.body()).to_bytes());
        Ok(u)
    }

    /// Verify against the old and new rulebook bytes. The signer must be a human approver
    /// of the old rulebook and must not be listed as an agent.
    pub fn verify(&self, old: &[u8], new: &[u8]) -> Result<(), String> {
        let ob = Rulebook::parse(old)?;
        let nb = Rulebook::parse(new)?;
        if crate::sha256_hex(old) != self.from_sha256 || crate::sha256_hex(new) != self.to_sha256 {
            return Err("rulebook bytes do not match update hashes".into());
        }
        if nb.rulebook_version != self.to_version || nb.rulebook_version == ob.rulebook_version {
            return Err("rulebook_version must change and match the update".into());
        }
        if ob.agents.contains(&self.signer_pubkey) || nb.agents.contains(&self.signer_pubkey) {
            return Err("rulebook updates may not be signed by an AI agent key".into());
        }
        if !ob.approvers.contains(&self.signer_pubkey) {
            return Err("signer is not a human approver of the current rulebook".into());
        }
        let pk: [u8; 32] = hex::decode(&self.signer_pubkey)
            .ok()
            .and_then(|v| v.try_into().ok())
            .ok_or("bad signer key")?;
        let sig: [u8; 64] = hex::decode(&self.signature)
            .ok()
            .and_then(|v| v.try_into().ok())
            .ok_or("bad signature")?;
        VerifyingKey::from_bytes(&pk)
            .map_err(|_| "bad signer key")?
            .verify(&self.body(), &Signature::from_bytes(&sig))
            .map_err(|_| "bad signature".to_string())
    }
}
