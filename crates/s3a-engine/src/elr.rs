//! S3A ELR (Enterprise Learner Record) Aggregator and Projection Engine.
//!
//! Implements the IEEE P2997 subset from ADL ELRR (https://github.com/adlnet/elrr-documentation):
//! - Core Tiles: Person (25), Organization (26), LearningResource (27),
//!   LearningEvent (28), CompetencyAssertion (29), CredentialAssertion (30), Goal (31).
//! - Deterministic rules engine (rules.md 1.1–3.4 + Scholar extension rules).
//! - Versioned ruleset with complete replay capability (rebuild from statement log).
//! - Privacy consent filtering and audit logging.

use std::collections::HashMap;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};
use xxhash_rust::xxh3::xxh3_128;

use s3a_core::*;
use crate::lrs_store::u64_pair_to_uuid;

/// Version of the ELRR aggregator ruleset.
pub const ELR_RULESET_VERSION_V1: u32 = 1;

/// Computes a stable, deterministic 128-bit UUID for an entity given a category and natural key.
pub fn deterministic_uuid(kind: &str, key: &str) -> [u64; 2] {
    let mut input = Vec::with_capacity(kind.len() + 1 + key.len());
    input.extend_from_slice(kind.as_bytes());
    input.push(b':');
    input.extend_from_slice(key.as_bytes());
    let h = xxh3_128(&input);
    let mut pair = [0u64; 2];
    pair[0] = (h & 0xFFFF_FFFF_FFFF_FFFF) as u64;
    pair[1] = (h >> 64) as u64;
    // Set UUID v4 variant & version bits to be a standard well-formed UUID
    pair[0] = (pair[0] & 0xFFFF_FFFF_FFFF_0FFF) | 0x0000_0000_0000_4000;
    pair[1] = (pair[1] & 0x3FFF_FFFF_FFFF_FFFF) | 0x8000_0000_0000_0000;
    pair
}

/// Person entity (Tile 25: TILE_ELR_PERSON).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ElrPerson {
    pub id: [u64; 2],
    pub name: String,
    pub identifier: String, // mbox, openid, or account key
    pub source_statement_id: [u64; 2],
    pub timestamp_ms: u64,
    pub flags: u32,
}

impl ElrPerson {
    pub fn to_tile_bytes(&self) -> (ElrRecordHeader, Vec<u8>) {
        let payload = serde_json::to_vec(&serde_json::json!({
            "name": self.name,
            "identifier": self.identifier,
        })).unwrap_or_default();

        let hdr = ElrRecordHeader {
            entity_id: self.id,
            source_statement_id: self.source_statement_id,
            related_id: [0, 0],
            timestamp_ms: self.timestamp_ms,
            flags: self.flags,
            status: 0,
            numeric_value: 0.0,
            payload_len: payload.len() as u32,
            payload_crc32: crc32c::crc32c(&payload),
            sub_type: 0,
            _reserved: [0, 0],
        };
        (hdr, payload)
    }

    pub fn from_tile_payload(hdr: &ElrRecordHeader, payload: &[u8]) -> Result<Self, String> {
        let val: serde_json::Value = serde_json::from_slice(payload)
            .map_err(|e| format!("Invalid Person JSON payload: {}", e))?;
        Ok(Self {
            id: hdr.entity_id,
            name: val.get("name").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            identifier: val.get("identifier").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            source_statement_id: hdr.source_statement_id,
            timestamp_ms: hdr.timestamp_ms,
            flags: hdr.flags,
        })
    }
}

/// Organization entity (Tile 26: TILE_ELR_ORGANIZATION).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ElrOrganization {
    pub id: [u64; 2],
    pub name: String,
    pub org_type: String,
    pub source_statement_id: [u64; 2],
    pub timestamp_ms: u64,
    pub flags: u32,
}

impl ElrOrganization {
    pub fn to_tile_bytes(&self) -> (ElrRecordHeader, Vec<u8>) {
        let payload = serde_json::to_vec(&serde_json::json!({
            "name": self.name,
            "org_type": self.org_type,
        })).unwrap_or_default();

        let hdr = ElrRecordHeader {
            entity_id: self.id,
            source_statement_id: self.source_statement_id,
            related_id: [0, 0],
            timestamp_ms: self.timestamp_ms,
            flags: self.flags,
            status: 0,
            numeric_value: 0.0,
            payload_len: payload.len() as u32,
            payload_crc32: crc32c::crc32c(&payload),
            sub_type: 0,
            _reserved: [0, 0],
        };
        (hdr, payload)
    }

    pub fn from_tile_payload(hdr: &ElrRecordHeader, payload: &[u8]) -> Result<Self, String> {
        let val: serde_json::Value = serde_json::from_slice(payload)
            .map_err(|e| format!("Invalid Organization JSON payload: {}", e))?;
        Ok(Self {
            id: hdr.entity_id,
            name: val.get("name").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            org_type: val.get("org_type").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            source_statement_id: hdr.source_statement_id,
            timestamp_ms: hdr.timestamp_ms,
            flags: hdr.flags,
        })
    }
}

/// Learning Resource entity (Tile 27: TILE_ELR_LEARNING_RESOURCE).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ElrLearningResource {
    pub id: [u64; 2],
    pub iri: String,
    pub name: String,
    pub description: String,
    pub resource_type: String,
    pub source_statement_id: [u64; 2],
    pub timestamp_ms: u64,
    pub flags: u32,
}

impl ElrLearningResource {
    pub fn to_tile_bytes(&self) -> (ElrRecordHeader, Vec<u8>) {
        let payload = serde_json::to_vec(&serde_json::json!({
            "iri": self.iri,
            "name": self.name,
            "description": self.description,
            "resource_type": self.resource_type,
        })).unwrap_or_default();

        let hdr = ElrRecordHeader {
            entity_id: self.id,
            source_statement_id: self.source_statement_id,
            related_id: [0, 0],
            timestamp_ms: self.timestamp_ms,
            flags: self.flags,
            status: 0,
            numeric_value: 0.0,
            payload_len: payload.len() as u32,
            payload_crc32: crc32c::crc32c(&payload),
            sub_type: 0,
            _reserved: [0, 0],
        };
        (hdr, payload)
    }

    pub fn from_tile_payload(hdr: &ElrRecordHeader, payload: &[u8]) -> Result<Self, String> {
        let val: serde_json::Value = serde_json::from_slice(payload)
            .map_err(|e| format!("Invalid LearningResource JSON payload: {}", e))?;
        Ok(Self {
            id: hdr.entity_id,
            iri: val.get("iri").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            name: val.get("name").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            description: val.get("description").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            resource_type: val.get("resource_type").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            source_statement_id: hdr.source_statement_id,
            timestamp_ms: hdr.timestamp_ms,
            flags: hdr.flags,
        })
    }
}

/// Learning Event entity (Tile 28: TILE_ELR_LEARNING_EVENT).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ElrLearningEvent {
    pub id: [u64; 2],
    pub person_id: [u64; 2],
    pub resource_id: [u64; 2],
    pub status: u32, // ELR_STATUS_*
    pub score_scaled: Option<f32>,
    pub success: Option<bool>,
    pub completion: Option<bool>,
    pub verb_iri: String,
    pub source_statement_id: [u64; 2],
    pub timestamp_ms: u64,
    pub flags: u32,
}

impl ElrLearningEvent {
    pub fn to_tile_bytes(&self) -> (ElrRecordHeader, Vec<u8>) {
        let payload = serde_json::to_vec(&serde_json::json!({
            "score_scaled": self.score_scaled,
            "success": self.success,
            "completion": self.completion,
            "verb_iri": self.verb_iri,
        })).unwrap_or_default();

        let hdr = ElrRecordHeader {
            entity_id: self.id,
            source_statement_id: self.source_statement_id,
            related_id: self.person_id,
            timestamp_ms: self.timestamp_ms,
            flags: self.flags,
            status: self.status,
            numeric_value: self.score_scaled.unwrap_or(0.0),
            payload_len: payload.len() as u32,
            payload_crc32: crc32c::crc32c(&payload),
            sub_type: 0,
            _reserved: self.resource_id,
        };
        (hdr, payload)
    }

    pub fn from_tile_payload(hdr: &ElrRecordHeader, payload: &[u8]) -> Result<Self, String> {
        let val: serde_json::Value = serde_json::from_slice(payload)
            .map_err(|e| format!("Invalid LearningEvent JSON payload: {}", e))?;
        Ok(Self {
            id: hdr.entity_id,
            person_id: hdr.related_id,
            resource_id: hdr._reserved,
            status: hdr.status,
            score_scaled: val.get("score_scaled").and_then(|v| v.as_f64()).map(|f| f as f32),
            success: val.get("success").and_then(|v| v.as_bool()),
            completion: val.get("completion").and_then(|v| v.as_bool()),
            verb_iri: val.get("verb_iri").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            source_statement_id: hdr.source_statement_id,
            timestamp_ms: hdr.timestamp_ms,
            flags: hdr.flags,
        })
    }
}

/// Competency Assertion entity (Tile 29: TILE_ELR_COMPETENCY_ASSERTION).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ElrCompetencyAssertion {
    pub id: [u64; 2],
    pub person_id: [u64; 2],
    pub competency_id: String,
    pub framework: String,
    pub confidence: f32,
    pub asserted_by: String,
    pub status: u32,
    pub source_statement_id: [u64; 2],
    pub timestamp_ms: u64,
    pub flags: u32,
}

impl ElrCompetencyAssertion {
    pub fn to_tile_bytes(&self) -> (ElrRecordHeader, Vec<u8>) {
        let payload = serde_json::to_vec(&serde_json::json!({
            "competency_id": self.competency_id,
            "framework": self.framework,
            "asserted_by": self.asserted_by,
        })).unwrap_or_default();

        let hdr = ElrRecordHeader {
            entity_id: self.id,
            source_statement_id: self.source_statement_id,
            related_id: self.person_id,
            timestamp_ms: self.timestamp_ms,
            flags: self.flags,
            status: self.status,
            numeric_value: self.confidence,
            payload_len: payload.len() as u32,
            payload_crc32: crc32c::crc32c(&payload),
            sub_type: 0,
            _reserved: [0, 0],
        };
        (hdr, payload)
    }

    pub fn from_tile_payload(hdr: &ElrRecordHeader, payload: &[u8]) -> Result<Self, String> {
        let val: serde_json::Value = serde_json::from_slice(payload)
            .map_err(|e| format!("Invalid CompetencyAssertion JSON payload: {}", e))?;
        Ok(Self {
            id: hdr.entity_id,
            person_id: hdr.related_id,
            competency_id: val.get("competency_id").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            framework: val.get("framework").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            confidence: hdr.numeric_value,
            asserted_by: val.get("asserted_by").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            status: hdr.status,
            source_statement_id: hdr.source_statement_id,
            timestamp_ms: hdr.timestamp_ms,
            flags: hdr.flags,
        })
    }
}

/// Credential Assertion entity (Tile 30: TILE_ELR_CREDENTIAL_ASSERTION).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ElrCredentialAssertion {
    pub id: [u64; 2],
    pub person_id: [u64; 2],
    pub credential_id: String,
    pub issuer: String,
    pub badge_url: String,
    pub source_statement_id: [u64; 2],
    pub timestamp_ms: u64,
    pub flags: u32,
}

impl ElrCredentialAssertion {
    pub fn to_tile_bytes(&self) -> (ElrRecordHeader, Vec<u8>) {
        let payload = serde_json::to_vec(&serde_json::json!({
            "credential_id": self.credential_id,
            "issuer": self.issuer,
            "badge_url": self.badge_url,
        })).unwrap_or_default();

        let hdr = ElrRecordHeader {
            entity_id: self.id,
            source_statement_id: self.source_statement_id,
            related_id: self.person_id,
            timestamp_ms: self.timestamp_ms,
            flags: self.flags,
            status: 0,
            numeric_value: 0.0,
            payload_len: payload.len() as u32,
            payload_crc32: crc32c::crc32c(&payload),
            sub_type: 0,
            _reserved: [0, 0],
        };
        (hdr, payload)
    }

    pub fn from_tile_payload(hdr: &ElrRecordHeader, payload: &[u8]) -> Result<Self, String> {
        let val: serde_json::Value = serde_json::from_slice(payload)
            .map_err(|e| format!("Invalid CredentialAssertion JSON payload: {}", e))?;
        Ok(Self {
            id: hdr.entity_id,
            person_id: hdr.related_id,
            credential_id: val.get("credential_id").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            issuer: val.get("issuer").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            badge_url: val.get("badge_url").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            source_statement_id: hdr.source_statement_id,
            timestamp_ms: hdr.timestamp_ms,
            flags: hdr.flags,
        })
    }
}

/// Goal entity (Tile 31: TILE_ELR_GOAL).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ElrGoal {
    pub id: [u64; 2],
    pub person_id: [u64; 2],
    pub goal_type: u32, // ELR_GOAL_TYPE_*
    pub status: u32,    // ELR_GOAL_STATUS_*
    pub target_date_ms: u64,
    pub name: String,
    pub description: String,
    pub source_statement_id: [u64; 2],
    pub timestamp_ms: u64,
    pub flags: u32,
}

impl ElrGoal {
    pub fn to_tile_bytes(&self) -> (ElrRecordHeader, Vec<u8>) {
        let payload = serde_json::to_vec(&serde_json::json!({
            "name": self.name,
            "description": self.description,
            "target_date_ms": self.target_date_ms,
        })).unwrap_or_default();

        let hdr = ElrRecordHeader {
            entity_id: self.id,
            source_statement_id: self.source_statement_id,
            related_id: self.person_id,
            timestamp_ms: self.timestamp_ms,
            flags: self.flags,
            status: self.status,
            numeric_value: 0.0,
            payload_len: payload.len() as u32,
            payload_crc32: crc32c::crc32c(&payload),
            sub_type: self.goal_type,
            _reserved: [0, 0],
        };
        (hdr, payload)
    }

    pub fn from_tile_payload(hdr: &ElrRecordHeader, payload: &[u8]) -> Result<Self, String> {
        let val: serde_json::Value = serde_json::from_slice(payload)
            .map_err(|e| format!("Invalid Goal JSON payload: {}", e))?;
        Ok(Self {
            id: hdr.entity_id,
            person_id: hdr.related_id,
            goal_type: hdr.sub_type,
            status: hdr.status,
            target_date_ms: val.get("target_date_ms").and_then(|v| v.as_u64()).unwrap_or(0),
            name: val.get("name").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            description: val.get("description").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            source_statement_id: hdr.source_statement_id,
            timestamp_ms: hdr.timestamp_ms,
            flags: hdr.flags,
        })
    }
}

/// Set of entities projected from a single accepted statement.
#[derive(Debug, Clone, Default)]
pub struct ElrProjectionDelta {
    pub person: Option<ElrPerson>,
    pub organization: Option<ElrOrganization>,
    pub learning_resource: Option<ElrLearningResource>,
    pub learning_event: Option<ElrLearningEvent>,
    pub competency_assertion: Option<ElrCompetencyAssertion>,
    pub credential_assertion: Option<ElrCredentialAssertion>,
    pub goal: Option<ElrGoal>,
}

/// In-memory projection store state.
#[derive(Default)]
pub struct ElrStoreState {
    pub persons: RwLock<HashMap<[u64; 2], ElrPerson>>,
    pub organizations: RwLock<HashMap<[u64; 2], ElrOrganization>>,
    pub learning_resources: RwLock<HashMap<[u64; 2], ElrLearningResource>>,
    pub learning_events: RwLock<HashMap<[u64; 2], ElrLearningEvent>>,
    pub competencies: RwLock<HashMap<[u64; 2], ElrCompetencyAssertion>>,
    pub credentials: RwLock<HashMap<[u64; 2], ElrCredentialAssertion>>,
    pub goals: RwLock<HashMap<[u64; 2], ElrGoal>>,
    pub person_marks: RwLock<HashMap<[u64; 2], u32>>,
}

impl ElrStoreState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_person_consent_withdrawn(&self, person_id: &[u64; 2]) -> bool {
        let marks = self.person_marks.read().unwrap();
        marks.get(person_id).map(|&m| (m & (1 << MARK_CONSENT_WITHDRAWN)) != 0).unwrap_or(false)
    }

    pub fn apply_delta(&self, delta: ElrProjectionDelta) {
        if let Some(p) = delta.person {
            self.persons.write().unwrap().insert(p.id, p);
        }
        if let Some(o) = delta.organization {
            self.organizations.write().unwrap().insert(o.id, o);
        }
        if let Some(lr) = delta.learning_resource {
            self.learning_resources.write().unwrap().insert(lr.id, lr);
        }
        if let Some(le) = delta.learning_event {
            self.learning_events.write().unwrap().insert(le.id, le);
        }
        if let Some(ca) = delta.competency_assertion {
            self.competencies.write().unwrap().insert(ca.id, ca);
        }
        if let Some(cr) = delta.credential_assertion {
            self.credentials.write().unwrap().insert(cr.id, cr);
        }
        if let Some(g) = delta.goal {
            self.goals.write().unwrap().insert(g.id, g);
        }
    }
}

fn truncate_str(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        s
    } else {
        match s.char_indices().take_while(|(idx, _)| *idx < max_len).last() {
            Some((idx, ch)) => &s[..idx + ch.len_utf8()],
            None => "",
        }
    }
}

/// Applies the versioned ELRR aggregator rules (rules.md 1.1–3.4 + Scholar extension rules)
/// deterministically to an accepted xAPI statement.
pub fn project_statement_to_elr(
    statement_val: &serde_json::Value,
    statement_id: [u64; 2],
    timestamp_ms: u64,
) -> ElrProjectionDelta {
    let mut delta = ElrProjectionDelta::default();

    // 1. Extract Actor -> ElrPerson
    let actor = match statement_val.get("actor") {
        Some(a) => a,
        None => return delta,
    };

    let identifier = if let Some(mbox) = actor.get("mbox").and_then(|v| v.as_str()) {
        mbox.to_string()
    } else if let Some(sha1) = actor.get("mbox_sha1sum").and_then(|v| v.as_str()) {
        format!("sha1:{}", sha1)
    } else if let Some(openid) = actor.get("openid").and_then(|v| v.as_str()) {
        openid.to_string()
    } else if let Some(acc) = actor.get("account") {
        let hp = acc.get("homePage").and_then(|v| v.as_str()).unwrap_or_default();
        let name = acc.get("name").and_then(|v| v.as_str()).unwrap_or_default();
        format!("{}:{}", hp, name)
    } else {
        "anonymous".to_string()
    };

    let name = actor.get("name")
        .and_then(|n| n.as_str())
        .unwrap_or(&identifier)
        .to_string();

    let person_id = deterministic_uuid("person", &identifier);
    let person = ElrPerson {
        id: person_id,
        name,
        identifier: identifier.clone(),
        source_statement_id: statement_id,
        timestamp_ms,
        flags: ELR_FLAG_ACTIVE,
    };
    delta.person = Some(person);

    // 2. Extract Object -> ElrLearningResource (if Activity)
    let obj = match statement_val.get("object") {
        Some(o) => o,
        None => return delta,
    };

    let obj_type = obj.get("objectType").and_then(|v| v.as_str()).unwrap_or("Activity");
    let obj_id = obj.get("id").and_then(|v| v.as_str()).unwrap_or_default();

    let is_activity = obj_type == "Activity";
    let is_goal_obj = obj_type == "Goal" || obj_id.contains("/goals/") || obj_id.contains("goal");

    let resource_id = if is_activity {
        let res_id = deterministic_uuid("resource", obj_id);
        let raw_res_name = obj.get("definition")
            .and_then(|d| d.get("name"))
            .and_then(|n| n.as_object())
            .and_then(|m| m.values().next())
            .and_then(|v| v.as_str())
            .unwrap_or(obj_id);
        let raw_res_desc = obj.get("definition")
            .and_then(|d| d.get("description"))
            .and_then(|n| n.as_object())
            .and_then(|m| m.values().next())
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let res_type = obj.get("definition")
            .and_then(|d| d.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("http://adlnet.gov/expapi/activities/course");

        let res_name = truncate_str(raw_res_name, 1024);
        let res_desc = truncate_str(raw_res_desc, 8192);

        delta.learning_resource = Some(ElrLearningResource {
            id: res_id,
            iri: obj_id.to_string(),
            name: res_name.to_string(),
            description: res_desc.to_string(),
            resource_type: res_type.to_string(),
            source_statement_id: statement_id,
            timestamp_ms,
            flags: ELR_FLAG_ACTIVE,
        });
        res_id
    } else {
        [0, 0]
    };

    // Extract Verb & Result
    let verb_iri = statement_val.get("verb")
        .and_then(|v| v.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or_default();

    let result = statement_val.get("result");
    let success = result.and_then(|r| r.get("success")).and_then(|s| s.as_bool());
    let completion = result.and_then(|r| r.get("completion")).and_then(|c| c.as_bool());
    let score_scaled = result.and_then(|r| r.get("score"))
        .and_then(|s| s.get("scaled"))
        .and_then(|sc| sc.as_f64())
        .map(|f| f as f32);

    // =========================================================================
    // SECTION 1: LEARNING RECORD STATEMENTS (rules.md 1.1 - 1.9)
    // =========================================================================
    let mut learning_event_status = None;

    if verb_iri.ends_with("/completed") {
        match success {
            None => learning_event_status = Some(ELR_STATUS_COMPLETED),      // Rule 1.1
            Some(true) => learning_event_status = Some(ELR_STATUS_PASSED),   // Rule 1.2
            Some(false) => learning_event_status = Some(ELR_STATUS_FAILED),  // Rule 1.3
        }
    } else if verb_iri.ends_with("/passed") {
        learning_event_status = Some(ELR_STATUS_PASSED);                     // Rule 1.4
    } else if verb_iri.ends_with("/failed") {
        learning_event_status = Some(ELR_STATUS_FAILED);                     // Rule 1.5
    } else if verb_iri.ends_with("/satisfied") {
        learning_event_status = Some(ELR_STATUS_SATISFIED);                  // Rule 1.6
    } else if verb_iri.ends_with("/attempted") {
        learning_event_status = Some(ELR_STATUS_ATTEMPTED);                  // Rule 1.7
    } else if verb_iri.ends_with("/registered") {
        learning_event_status = Some(ELR_STATUS_REGISTERED);                 // Rule 1.8
    } else if verb_iri.ends_with("/scheduled") {
        learning_event_status = Some(ELR_STATUS_SCHEDULED);                  // Rule 1.9
    }

    // =========================================================================
    // =========================================================================
    // SECTION 2: COMPETENCIES & CREDENTIALS (rules.md 2.1 - 2.3)
    // =========================================================================
    let instructor_name = statement_val.get("context")
        .and_then(|c| c.get("instructor"))
        .and_then(|i| i.get("name"))
        .and_then(|n| n.as_str());

    if verb_iri.ends_with("/mastered") || verb_iri.ends_with("/achieved") {
        // Rule 2.1: Mastered / Achieved Competency
        let comp_id = deterministic_uuid("competency", &format!("{}:{}", u64_pair_to_uuid(person_id), obj_id));
        let asserted_by = instructor_name.unwrap_or("LRS Aggregator Rules Engine").to_string();
        delta.competency_assertion = Some(ElrCompetencyAssertion {
            id: comp_id,
            person_id,
            competency_id: obj_id.to_string(),
            framework: "Total Learning Architecture (TLA)".to_string(),
            confidence: score_scaled.unwrap_or(1.0),
            asserted_by,
            status: ELR_STATUS_PASSED,
            source_statement_id: statement_id,
            timestamp_ms,
            flags: ELR_FLAG_ACTIVE,
        });
    } else if verb_iri.ends_with("/earned") || verb_iri.ends_with("/conferred") {
        // Rule 2.2: Earned / Conferred Credential
        let cred_id = deterministic_uuid("credential", &format!("{}:{}", u64_pair_to_uuid(person_id), obj_id));
        let issuer = instructor_name.unwrap_or("ADL Credential Registry").to_string();
        delta.credential_assertion = Some(ElrCredentialAssertion {
            id: cred_id,
            person_id,
            credential_id: obj_id.to_string(),
            issuer,
            badge_url: obj_id.to_string(),
            source_statement_id: statement_id,
            timestamp_ms,
            flags: ELR_FLAG_ACTIVE,
        });
    } else if verb_iri.ends_with("/waived") {
        // Rule 2.3: Waived Competency
        let comp_id = deterministic_uuid("competency", &format!("{}:{}", u64_pair_to_uuid(person_id), obj_id));
        let asserted_by = instructor_name.unwrap_or("LRS Aggregator Rules Engine (Waived)").to_string();
        delta.competency_assertion = Some(ElrCompetencyAssertion {
            id: comp_id,
            person_id,
            competency_id: obj_id.to_string(),
            framework: "Total Learning Architecture (TLA)".to_string(),
            confidence: 1.0,
            asserted_by,
            status: ELR_STATUS_SATISFIED,
            source_statement_id: statement_id,
            timestamp_ms,
            flags: ELR_FLAG_ACTIVE,
        });
    }

    // =========================================================================
    // SECTION 3: GOALS (rules.md 3.1 - 3.4)
    // =========================================================================
    if is_goal_obj || verb_iri.contains("goal") || verb_iri.ends_with("/assigned") || verb_iri.ends_with("/pursued") || verb_iri.ends_with("/chose") || verb_iri.ends_with("/planned") || verb_iri.ends_with("/dismissed") {
        let raw_goal_name = obj.get("definition")
            .and_then(|d| d.get("name"))
            .and_then(|n| n.as_object())
            .and_then(|m| m.values().next())
            .and_then(|v| v.as_str())
            .unwrap_or(obj_id);
        let goal_name = truncate_str(raw_goal_name, 1024).to_string();

        let raw_goal_desc = obj.get("definition")
            .and_then(|d| d.get("description"))
            .and_then(|n| n.as_object())
            .and_then(|m| m.values().next())
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let goal_desc = truncate_str(raw_goal_desc, 4096).to_string();

        if verb_iri.ends_with("/assigned") {
            // Rule 3.1: Goal assigned
            let goal_id = deterministic_uuid("goal", &format!("{}:{}", u64_pair_to_uuid(person_id), obj_id));
            let desc = if goal_desc.is_empty() { "Assigned organizational learning goal".to_string() } else { goal_desc };
            delta.goal = Some(ElrGoal {
                id: goal_id,
                person_id,
                goal_type: ELR_GOAL_TYPE_ASSIGNED,
                status: ELR_GOAL_STATUS_ACTIVE,
                target_date_ms: timestamp_ms + (30 * 86400 * 1000),
                name: goal_name,
                description: desc,
                source_statement_id: statement_id,
                timestamp_ms,
                flags: ELR_FLAG_ACTIVE,
            });
        } else if verb_iri.ends_with("/pursued") || verb_iri.ends_with("/chose") || verb_iri.ends_with("/planned") {
            // Rule 3.2: Self-selected goal
            let goal_id = deterministic_uuid("goal", &format!("{}:{}", u64_pair_to_uuid(person_id), obj_id));
            let desc = if goal_desc.is_empty() { "Self-directed learning goal".to_string() } else { goal_desc };
            delta.goal = Some(ElrGoal {
                id: goal_id,
                person_id,
                goal_type: ELR_GOAL_TYPE_SELF,
                status: ELR_GOAL_STATUS_ACTIVE,
                target_date_ms: timestamp_ms + (30 * 86400 * 1000),
                name: goal_name,
                description: desc,
                source_statement_id: statement_id,
                timestamp_ms,
                flags: ELR_FLAG_ACTIVE,
            });
        } else if verb_iri.ends_with("/recommended") {
            // Rule 3.3: Recommended goal
            let goal_id = deterministic_uuid("goal", &format!("{}:{}", u64_pair_to_uuid(person_id), obj_id));
            let desc = if goal_desc.is_empty() { "Recommended learning goal".to_string() } else { goal_desc };
            delta.goal = Some(ElrGoal {
                id: goal_id,
                person_id,
                goal_type: ELR_GOAL_TYPE_RECOMMENDED,
                status: ELR_GOAL_STATUS_ACTIVE,
                target_date_ms: timestamp_ms + (30 * 86400 * 1000),
                name: goal_name,
                description: desc,
                source_statement_id: statement_id,
                timestamp_ms,
                flags: ELR_FLAG_ACTIVE,
            });
        } else if verb_iri.ends_with("/abandoned") || verb_iri.ends_with("/dismissed") || verb_iri.ends_with("/removed") {
            // Rule 3.4: Goal removed / abandoned
            let goal_id = deterministic_uuid("goal", &format!("{}:{}", u64_pair_to_uuid(person_id), obj_id));
            let desc = if goal_desc.is_empty() { "Abandoned/removed goal".to_string() } else { goal_desc };
            delta.goal = Some(ElrGoal {
                id: goal_id,
                person_id,
                goal_type: ELR_GOAL_TYPE_SELF,
                status: ELR_GOAL_STATUS_REMOVED,
                target_date_ms: 0,
                name: goal_name,
                description: desc,
                source_statement_id: statement_id,
                timestamp_ms,
                flags: ELR_FLAG_ACTIVE,
            });
        }
    }

    // =========================================================================
    // SECTION 4: SCHOLAR EXTENSION RULES
    // =========================================================================
    if verb_iri == "https://ashecribb.dev/xapi/verbs/reviewed" {
        learning_event_status = Some(ELR_STATUS_COMPLETED);
        let comp_id = deterministic_uuid("competency", &format!("{}:peer-review", u64_pair_to_uuid(person_id)));
        delta.competency_assertion = Some(ElrCompetencyAssertion {
            id: comp_id,
            person_id,
            competency_id: "https://ashecribb.dev/competencies/peer-review".to_string(),
            framework: "Scholar Research Integrity Framework".to_string(),
            confidence: 1.0,
            asserted_by: "Scholar Verification System".to_string(),
            status: ELR_STATUS_PASSED,
            source_statement_id: statement_id,
            timestamp_ms,
            flags: ELR_FLAG_ACTIVE,
        });
    } else if verb_iri == "https://ashecribb.dev/xapi/verbs/approved" {
        learning_event_status = Some(ELR_STATUS_PASSED);
    } else if verb_iri == "https://ashecribb.dev/xapi/verbs/rejected" {
        learning_event_status = Some(ELR_STATUS_FAILED);
    } else if verb_iri == "https://ashecribb.dev/xapi/verbs/flagged-hallucination" {
        let comp_id = deterministic_uuid("competency", &format!("{}:hallucination-detection", u64_pair_to_uuid(person_id)));
        delta.competency_assertion = Some(ElrCompetencyAssertion {
            id: comp_id,
            person_id,
            competency_id: "https://ashecribb.dev/competencies/hallucination-detection".to_string(),
            framework: "Scholar Research Integrity Framework".to_string(),
            confidence: 1.0,
            asserted_by: "Scholar Fact Checking".to_string(),
            status: ELR_STATUS_PASSED,
            source_statement_id: statement_id,
            timestamp_ms,
            flags: ELR_FLAG_ACTIVE,
        });
    } else if verb_iri == "https://ashecribb.dev/xapi/verbs/ta-approved-draft" {
        let comp_id = deterministic_uuid("competency", &format!("{}:research-methodology", u64_pair_to_uuid(person_id)));
        delta.competency_assertion = Some(ElrCompetencyAssertion {
            id: comp_id,
            person_id,
            competency_id: "https://ashecribb.dev/competencies/research-methodology".to_string(),
            framework: "Scholar Research Integrity Framework".to_string(),
            confidence: 1.0,
            asserted_by: "Teacher Assistant Supervisory Gate".to_string(),
            status: ELR_STATUS_PASSED,
            source_statement_id: statement_id,
            timestamp_ms,
            flags: ELR_FLAG_ACTIVE,
        });
    } else if verb_iri == "https://ashecribb.dev/xapi/verbs/requested-human-review" {
        learning_event_status = Some(ELR_STATUS_ATTEMPTED);
    } else if verb_iri == "http://purl.org/spar/cito/cites" {
        learning_event_status = Some(ELR_STATUS_COMPLETED);
    }

    if let Some(st) = learning_event_status {
        let ev_key = format!("{}:{}:{}", u64_pair_to_uuid(person_id), u64_pair_to_uuid(resource_id), u64_pair_to_uuid(statement_id));
        let ev_id = deterministic_uuid("event", &ev_key);
        delta.learning_event = Some(ElrLearningEvent {
            id: ev_id,
            person_id,
            resource_id,
            status: st,
            score_scaled,
            success,
            completion,
            verb_iri: verb_iri.to_string(),
            source_statement_id: statement_id,
            timestamp_ms,
            flags: ELR_FLAG_ACTIVE,
        });
    }

    delta
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rules_1_1_to_1_9_events() {
        let stmt_id = [100, 200];
        let t_ms = 1_700_000_000_000;

        let verbs_and_expected = vec![
            ("http://adlnet.gov/expapi/verbs/completed", ELR_STATUS_COMPLETED),
            ("http://adlnet.gov/expapi/verbs/passed", ELR_STATUS_PASSED),
            ("http://adlnet.gov/expapi/verbs/failed", ELR_STATUS_FAILED),
            ("http://adlnet.gov/expapi/verbs/satisfied", ELR_STATUS_SATISFIED),
            ("http://adlnet.gov/expapi/verbs/attempted", ELR_STATUS_ATTEMPTED),
            ("http://adlnet.gov/expapi/verbs/registered", ELR_STATUS_REGISTERED),
            ("http://adlnet.gov/expapi/verbs/scheduled", ELR_STATUS_SCHEDULED),
        ];

        for (verb_iri, expected_status) in verbs_and_expected {
            let mut result_json = serde_json::json!({
                "score": {"scaled": 0.95},
                "completion": true
            });
            if verb_iri.ends_with("/passed") {
                result_json["success"] = serde_json::Value::Bool(true);
            } else if verb_iri.ends_with("/failed") || verb_iri.ends_with("/abandoned") {
                result_json["success"] = serde_json::Value::Bool(false);
            }

            let val = serde_json::json!({
                "id": "11111111-2222-3333-4444-555566667777",
                "actor": {
                    "mbox": "mailto:learner@example.com",
                    "name": "Jane Learner"
                },
                "verb": {"id": verb_iri},
                "object": {
                    "id": "http://example.com/course/101",
                    "definition": {
                        "name": {"en": "Biology 101"},
                        "description": {"en": "Intro to Bio"}
                    }
                },
                "result": result_json
            });

            let delta = project_statement_to_elr(&val, stmt_id, t_ms);
            assert!(delta.person.is_some());
            let p = delta.person.unwrap();
            assert_eq!(p.name, "Jane Learner");
            assert_eq!(p.identifier, "mailto:learner@example.com");

            assert!(delta.learning_resource.is_some());
            let lr = delta.learning_resource.unwrap();
            assert_eq!(lr.name, "Biology 101");
            assert_eq!(lr.description, "Intro to Bio");

            assert!(delta.learning_event.is_some());
            let ev = delta.learning_event.unwrap();
            assert_eq!(ev.status, expected_status);
            assert_eq!(ev.score_scaled, Some(0.95));
            assert_eq!(ev.completion, Some(true));
            assert_eq!(ev.verb_iri, verb_iri);
        }
    }

    #[test]
    fn test_rules_2_1_to_2_3_competencies_and_credentials() {
        let stmt_id = [300, 400];
        let t_ms = 1_700_000_100_000;

        // Competency assertion
        let comp_val = serde_json::json!({
            "id": "22222222-3333-4444-5555-666677778888",
            "actor": {"mbox": "mailto:alice@example.com"},
            "verb": {"id": "http://adlnet.gov/expapi/verbs/mastered"},
            "object": {
                "id": "https://example.com/competencies/rust-memory-safety",
                "definition": {
                    "name": {"en": "Rust Memory Safety"},
                    "type": "http://adlnet.gov/expapi/activities/competency"
                }
            },
            "context": {
                "instructor": {"name": "Prof. Smith"}
            },
            "result": {
                "score": {"scaled": 0.98}
            }
        });

        let delta = project_statement_to_elr(&comp_val, stmt_id, t_ms);
        assert!(delta.competency_assertion.is_some());
        let ca = delta.competency_assertion.unwrap();
        assert_eq!(ca.competency_id, "https://example.com/competencies/rust-memory-safety");
        assert_eq!(ca.confidence, 0.98);
        assert_eq!(ca.asserted_by, "Prof. Smith");
        assert_eq!(ca.status, ELR_STATUS_PASSED);

        // Credential assertion
        let cred_val = serde_json::json!({
            "id": "33333333-4444-5555-6666-777788889999",
            "actor": {"mbox": "mailto:bob@example.com"},
            "verb": {"id": "http://adlnet.gov/expapi/verbs/earned"},
            "object": {
                "id": "https://badges.example.com/badge/systems-architect",
                "definition": {
                    "name": {"en": "Systems Architect Badge"},
                    "description": {"en": "Advanced distributed systems architecture"}
                }
            },
            "context": {
                "instructor": {"name": "Accreditation Board"}
            }
        });

        let delta_cred = project_statement_to_elr(&cred_val, stmt_id, t_ms);
        assert!(delta_cred.credential_assertion.is_some());
        let cr = delta_cred.credential_assertion.unwrap();
        assert_eq!(cr.credential_id, "https://badges.example.com/badge/systems-architect");
        assert_eq!(cr.issuer, "Accreditation Board");
    }

    #[test]
    fn test_rules_3_1_to_3_4_goals() {
        let stmt_id = [500, 600];
        let t_ms = 1_700_000_200_000;

        // Assigned goal
        let assign_goal_val = serde_json::json!({
            "id": "44444444-5555-6666-7777-888899990000",
            "actor": {"mbox": "mailto:charlie@example.com"},
            "verb": {"id": "http://adlnet.gov/expapi/verbs/assigned"},
            "object": {
                "id": "https://example.com/goals/complete-thesis",
                "definition": {
                    "name": {"en": "Complete Master Thesis"},
                    "description": {"en": "Finish experimental validation and thesis manuscript"}
                }
            }
        });

        let delta_assign = project_statement_to_elr(&assign_goal_val, stmt_id, t_ms);
        assert!(delta_assign.goal.is_some());
        let g = delta_assign.goal.unwrap();
        assert_eq!(g.name, "Complete Master Thesis");
        assert_eq!(g.goal_type, ELR_GOAL_TYPE_ASSIGNED);
        assert_eq!(g.status, ELR_GOAL_STATUS_ACTIVE);

        // Removed goal
        let remove_goal_val = serde_json::json!({
            "id": "55555555-6666-7777-8888-999900001111",
            "actor": {"mbox": "mailto:charlie@example.com"},
            "verb": {"id": "http://adlnet.gov/expapi/verbs/dismissed"},
            "object": {
                "id": "https://example.com/goals/complete-thesis",
                "definition": {"name": {"en": "Complete Master Thesis"}}
            }
        });
        let delta_remove = project_statement_to_elr(&remove_goal_val, stmt_id, t_ms);
        assert!(delta_remove.goal.is_some());
        let g_rem = delta_remove.goal.unwrap();
        assert_eq!(g_rem.status, ELR_GOAL_STATUS_REMOVED);

        // Self-selected goal
        let self_goal_val = serde_json::json!({
            "id": "66666666-7777-8888-9999-000011112222",
            "actor": {"mbox": "mailto:charlie@example.com"},
            "verb": {"id": "http://adlnet.gov/expapi/verbs/planned"},
            "object": {
                "id": "https://example.com/goals/learn-rust",
                "definition": {"name": {"en": "Learn Rust in 30 Days"}}
            }
        });
        let delta_self = project_statement_to_elr(&self_goal_val, stmt_id, t_ms);
        assert!(delta_self.goal.is_some());
        let g_self = delta_self.goal.unwrap();
        assert_eq!(g_self.goal_type, ELR_GOAL_TYPE_SELF);
        assert_eq!(g_self.status, ELR_GOAL_STATUS_ACTIVE);
    }

    #[test]
    fn test_scholar_extension_rules() {
        let stmt_id = [700, 800];
        let t_ms = 1_700_000_300_000;

        // Reviewed
        let rev_val = serde_json::json!({
            "id": "77777777-8888-9999-aaaa-bbbbccccdddd",
            "actor": {"mbox": "mailto:researcher@lab.edu"},
            "verb": {"id": "https://ashecribb.dev/xapi/verbs/reviewed"},
            "object": {"id": "https://doi.org/10.1000/182"}
        });
        let d_rev = project_statement_to_elr(&rev_val, stmt_id, t_ms);
        assert_eq!(d_rev.learning_event.unwrap().status, ELR_STATUS_COMPLETED);

        // Approved
        let app_val = serde_json::json!({
            "id": "88888888-9999-aaaa-bbbb-ccccddddeeee",
            "actor": {"mbox": "mailto:pi@lab.edu"},
            "verb": {"id": "https://ashecribb.dev/xapi/verbs/approved"},
            "object": {"id": "https://doi.org/10.1000/182"}
        });
        let d_app = project_statement_to_elr(&app_val, stmt_id, t_ms);
        assert_eq!(d_app.learning_event.unwrap().status, ELR_STATUS_PASSED);

        // Flagged hallucination
        let flag_val = serde_json::json!({
            "id": "99999999-aaaa-bbbb-cccc-ddddeeeeffff",
            "actor": {"mbox": "mailto:reviewer@lab.edu"},
            "verb": {"id": "https://ashecribb.dev/xapi/verbs/flagged-hallucination"},
            "object": {"id": "https://doi.org/10.1000/182"}
        });
        let d_flag = project_statement_to_elr(&flag_val, stmt_id, t_ms);
        let ca = d_flag.competency_assertion.unwrap();
        assert_eq!(ca.competency_id, "https://ashecribb.dev/competencies/hallucination-detection");
        assert_eq!(ca.framework, "Scholar Research Integrity Framework");

        // TA approved draft
        let ta_val = serde_json::json!({
            "id": "aaaaaaaa-bbbb-cccc-dddd-eeeeffff0000",
            "actor": {"mbox": "mailto:student@lab.edu"},
            "verb": {"id": "https://ashecribb.dev/xapi/verbs/ta-approved-draft"},
            "object": {"id": "https://doi.org/10.1000/182"}
        });
        let d_ta = project_statement_to_elr(&ta_val, stmt_id, t_ms);
        let ca_ta = d_ta.competency_assertion.unwrap();
        assert_eq!(ca_ta.competency_id, "https://ashecribb.dev/competencies/research-methodology");
    }

    #[test]
    fn test_elr_entity_payload_roundtrip() {
        let (hdr_p, payload_p) = ElrPerson {
            id: [1, 2],
            name: "Ada Lovelace".into(),
            identifier: "mailto:ada@example.com".into(),
            source_statement_id: [10, 20],
            timestamp_ms: 12345678,
            flags: 0,
        }.to_tile_bytes();
        let decoded_p = ElrPerson::from_tile_payload(&hdr_p, &payload_p).unwrap();
        assert_eq!(decoded_p.name, "Ada Lovelace");
        assert_eq!(decoded_p.identifier, "mailto:ada@example.com");

        let (hdr_g, payload_g) = ElrGoal {
            id: [3, 4],
            person_id: [1, 2],
            goal_type: ELR_GOAL_TYPE_ASSIGNED,
            status: ELR_GOAL_STATUS_ACTIVE,
            target_date_ms: 999999,
            name: "Complete Project".into(),
            description: "Finish code".into(),
            source_statement_id: [10, 20],
            timestamp_ms: 12345678,
            flags: 0,
        }.to_tile_bytes();
        let decoded_g = ElrGoal::from_tile_payload(&hdr_g, &payload_g).unwrap();
        assert_eq!(decoded_g.name, "Complete Project");
        assert_eq!(decoded_g.goal_type, ELR_GOAL_TYPE_ASSIGNED);
    }
}
