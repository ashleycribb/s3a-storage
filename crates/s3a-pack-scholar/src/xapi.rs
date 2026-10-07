//! Authoritative xAPI Profile IRI ↔ ID Registry for Scholar Explorer & Provenance.
//!
//! Conforms to the ADL xAPI Profile Specification and IEEE 9274.1.1.
//! Maps wire-level 32-bit `verb_id` values to canonical IRIs and human-readable labels.

/// Canonical Scholar Explorer xAPI Profile IRI.
pub const SCHOLAR_XAPI_PROFILE_IRI: &str = "https://ashecribb.dev/xapi/profiles/scholar-explorer/v1";

/// Canonical Scholar Explorer Usage xAPI Profile IRI.
pub const SCHOLAR_EXPLORER_USAGE_PROFILE_IRI: &str = "https://ashecribb.dev/xapi/profiles/scholar-explorer-usage/v1";

pub const USAGE_VERB_SEARCHED: &str = "https://ashecribb.dev/xapi/verbs/searched";
pub const USAGE_VERB_PROMPTED: &str = "https://ashecribb.dev/xapi/verbs/prompted";
pub const USAGE_VERB_AI_RESPONDED: &str = "https://ashecribb.dev/xapi/verbs/ai-responded";
pub const USAGE_VERB_VIEWED_SOURCE: &str = "https://ashecribb.dev/xapi/verbs/viewed-source";
pub const USAGE_VERB_EXPORTED: &str = "https://ashecribb.dev/xapi/verbs/exported";

/// Canonical Scholar Explorer OWL Domain Ontology IRI.
pub const SCHOLAR_OWL_ONTOLOGY_IRI: &str = "https://ashecribb.dev/ontologies/scholar/v1";

/// Specification entry for a registered xAPI verb.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerbSpec {
    pub verb_id: u32,
    pub iri: &'static str,
    pub label: &'static str,
    pub verb_bit: u32, // Bit offset in AgentRegistryRecord.allowed_verb_mask
}

// Well-known Teacher Assistant (TA) Verbs
pub const VERB_ID_TA_APPROVED_DRAFT: u32 = 0x00E1;
pub const VERB_ID_TA_PROVIDED_ANALOGY: u32 = 0x00E2;
pub const VERB_ID_TA_REDIRECTED: u32 = 0x00E3;
pub const VERB_ID_TA_ESCALATED: u32 = 0x00E4;

// Well-known Research / Scholar Verbs
pub const VERB_ID_EXPERIENCED: u32 = 0x0001;
pub const VERB_ID_REVIEWED: u32 = 0x0002;
pub const VERB_ID_APPROVED: u32 = 0x0003;
pub const VERB_ID_REJECTED: u32 = 0x0004;
pub const VERB_ID_REQUESTED_HUMAN_REVIEW: u32 = 0x0005;
pub const VERB_ID_FLAGGED_HALLUCINATION: u32 = 0x0006;
pub const VERB_ID_CITED: u32 = 0x0007;
pub const VERB_ID_IMPORTED: u32 = 0x0008;
pub const VERB_ID_EXPORTED_SNAPSHOT: u32 = 0x0009;
pub const VERB_ID_FLAGGED_DISCREPANCY: u32 = 0x000A;
pub const VERB_ID_VERIFIED_CLAIM: u32 = 0x000B;

/// Authoritative xAPI Extension IRIs for Verified Research Trust & Citation Fingerprints.
pub const EXT_CONTENT_SHA256: &str = "https://ashecribb.dev/xapi/ext/content-sha256";
pub const EXT_CONTENT_KIND: &str = "https://ashecribb.dev/xapi/ext/content-kind";
pub const EXT_SOURCE_URL: &str = "https://ashecribb.dev/xapi/ext/source-url";
pub const EXT_CROSSREF_VERSION: &str = "https://ashecribb.dev/xapi/ext/crossref-version";
pub const EXT_RECHECK_STATUS: &str = "https://ashecribb.dev/xapi/ext/recheck-status";
pub const EXT_PAPER_ID: &str = "https://ashecribb.dev/xapi/ext/paper-id";

/// Authoritative Table of Registered Scholar Verbs.
pub static SCHOLAR_VERBS: &[VerbSpec] = &[
    // Teacher Assistant (TA) Verbs (Bits 0..3)
    VerbSpec {
        verb_id: VERB_ID_TA_APPROVED_DRAFT,
        iri: "https://ashecribb.dev/xapi/verbs/ta-approved-draft",
        label: "ta-approved-draft",
        verb_bit: 1 << 0,
    },
    VerbSpec {
        verb_id: VERB_ID_TA_PROVIDED_ANALOGY,
        iri: "https://ashecribb.dev/xapi/verbs/ta-provided-analogy",
        label: "ta-provided-analogy",
        verb_bit: 1 << 1,
    },
    VerbSpec {
        verb_id: VERB_ID_TA_REDIRECTED,
        iri: "https://ashecribb.dev/xapi/verbs/ta-redirected",
        label: "ta-redirected",
        verb_bit: 1 << 2,
    },
    VerbSpec {
        verb_id: VERB_ID_TA_ESCALATED,
        iri: "https://ashecribb.dev/xapi/verbs/ta-escalated",
        label: "ta-escalated",
        verb_bit: 1 << 3,
    },
    // Core Research Verbs (Bits 4..12)
    VerbSpec {
        verb_id: VERB_ID_EXPERIENCED,
        iri: "http://adlnet.gov/expapi/verbs/experienced",
        label: "experienced",
        verb_bit: 1 << 4,
    },
    VerbSpec {
        verb_id: VERB_ID_REVIEWED,
        iri: "https://ashecribb.dev/xapi/verbs/reviewed",
        label: "reviewed",
        verb_bit: 1 << 5,
    },
    VerbSpec {
        verb_id: VERB_ID_APPROVED,
        iri: "https://ashecribb.dev/xapi/verbs/approved",
        label: "approved",
        verb_bit: 1 << 6,
    },
    VerbSpec {
        verb_id: VERB_ID_REJECTED,
        iri: "https://ashecribb.dev/xapi/verbs/rejected",
        label: "rejected",
        verb_bit: 1 << 7,
    },
    VerbSpec {
        verb_id: VERB_ID_REQUESTED_HUMAN_REVIEW,
        iri: "https://ashecribb.dev/xapi/verbs/requested-human-review",
        label: "requested-human-review",
        verb_bit: 1 << 8,
    },
    VerbSpec {
        verb_id: VERB_ID_FLAGGED_HALLUCINATION,
        iri: "https://ashecribb.dev/xapi/verbs/flagged-hallucination",
        label: "flagged-hallucination",
        verb_bit: 1 << 9,
    },
    VerbSpec {
        verb_id: VERB_ID_CITED,
        iri: "http://purl.org/spar/cito/cites",
        label: "cited",
        verb_bit: 1 << 10,
    },
    VerbSpec {
        verb_id: VERB_ID_IMPORTED,
        iri: "https://ashecribb.dev/xapi/verbs/imported",
        label: "imported",
        verb_bit: 1 << 11,
    },
    VerbSpec {
        verb_id: VERB_ID_EXPORTED_SNAPSHOT,
        iri: "https://ashecribb.dev/xapi/verbs/exported-snapshot",
        label: "exported-snapshot",
        verb_bit: 1 << 12,
    },
    VerbSpec {
        verb_id: VERB_ID_FLAGGED_DISCREPANCY,
        iri: "https://ashecribb.dev/xapi/verbs/flagged-discrepancy",
        label: "flagged-discrepancy",
        verb_bit: 1 << 13,
    },
    VerbSpec {
        verb_id: VERB_ID_VERIFIED_CLAIM,
        iri: "https://ashecribb.dev/xapi/verbs/verified-claim",
        label: "verified-claim",
        verb_bit: 1 << 14,
    },
];

/// Resolves a VerbSpec by its 32-bit wire integer identifier.
pub fn resolve_verb_by_id(verb_id: u32) -> Option<&'static VerbSpec> {
    SCHOLAR_VERBS.iter().find(|v| v.verb_id == verb_id)
}

/// Resolves a VerbSpec by its canonical IRI string.
pub fn resolve_verb_by_iri(iri: &str) -> Option<&'static VerbSpec> {
    SCHOLAR_VERBS.iter().find(|v| v.iri == iri)
}

/// Resolves a VerbSpec by its human-readable label slug.
pub fn resolve_verb_by_label(label: &str) -> Option<&'static VerbSpec> {
    SCHOLAR_VERBS.iter().find(|v| v.label == label)
}

/// Computes a stable 64-bit FNV-1a hash of an IRI string.
pub fn hash_iri(iri: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in iri.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3u64);
    }
    hash
}

use bytemuck::bytes_of;
use s3a_core::{HyperTileHeader, S3ACoordinate};
use s3a_engine::concurrent_writer::ConcurrentTileWriter;
use s3a_engine::lrs_store::{uuid_to_u64_pair, hash_actor_value, now_ms};
use crate::pack::ScholarTileType;
use crate::records::{
    AgentDenied, AgentRegistryRecord, AiTraceRecord, HumanLrsRecord,
    STEP_TOOL_CALL,
};

/// Validates statement compliance with the Scholar Explorer and Research Traceability profile.
pub fn validate_scholar_profile(statement: &serde_json::Value) -> Result<(), String> {
    let verb_iri = statement.get("verb")
        .and_then(|v| v.get("id"))
        .and_then(|id| id.as_str())
        .unwrap_or_default();

    if let Some(spec) = resolve_verb_by_iri(verb_iri) {
        // Enforce StatementTemplate rules
        let obj = statement.get("object").and_then(|o| o.as_object())
            .ok_or_else(|| "Scholar profile statement must have an object".to_string())?;

        let ot = obj.get("objectType").and_then(|v| v.as_str()).unwrap_or("Activity");
        if ot == "Activity" {
            let act_id = obj.get("id").and_then(|v| v.as_str())
                .ok_or_else(|| "Scholar activity object must have an id".to_string())?;
            if !s3a_engine::lrs_store::is_valid_iri(act_id) {
                return Err(format!("Scholar activity object id is not valid IRI: {}", act_id));
            }
        }

        // Verb-specific requirements
        match spec.verb_id {
            VERB_ID_APPROVED | VERB_ID_TA_APPROVED_DRAFT => {
                let res = statement.get("result").and_then(|r| r.as_object())
                    .ok_or_else(|| format!("Verb '{}' requires result object", spec.label))?;
                if res.get("success").and_then(|s| s.as_bool()).is_none() {
                    return Err(format!("Verb '{}' requires result.success boolean", spec.label));
                }
            }
            VERB_ID_CITED => {
                if !obj.contains_key("id") {
                    return Err("Verb 'cited' requires object.id reference to cited work".to_string());
                }
            }
            _ => {}
        }
    }

    Ok(())
}

/// Checks if a verb IRI is one of the 5 permitted verbs in the `scholar-explorer-usage/v1` profile.
pub fn is_usage_profile_verb(verb_iri: &str) -> bool {
    let lower = verb_iri.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "https://ashecribb.dev/xapi/verbs/searched"
            | "https://ashecribb.dev/xapi/verbs/prompted"
            | "https://ashecribb.dev/xapi/verbs/ai-responded"
            | "https://ashecribb.dev/xapi/verbs/viewed-source"
            | "https://ashecribb.dev/xapi/verbs/exported"
    ) || lower.ends_with("/searched")
      || lower.ends_with("/prompted")
      || lower.ends_with("/ai-responded")
      || lower.ends_with("/viewed-source")
      || lower.ends_with("/exported")
}

/// Checks if a statement declares the scholar-explorer-usage/v1 profile in its context activities.
pub fn statement_has_usage_profile(statement: &serde_json::Value) -> bool {
    if let Some(ctx) = statement.get("context").and_then(|c| c.as_object()) {
        if let Some(ca) = ctx.get("contextActivities").and_then(|a| a.as_object()) {
            if let Some(categories) = ca.get("category").and_then(|c| c.as_array()) {
                for item in categories {
                    if let Some(id) = item.get("id").and_then(|i| i.as_str()) {
                        if id == SCHOLAR_EXPLORER_USAGE_PROFILE_IRI || id.contains("scholar-explorer-usage/v1") {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

/// Validates that a statement adheres strictly to the `scholar-explorer-usage/v1` profile.
/// Rejects any statement that does not declare the profile or uses a verb outside the 5 allowed verbs.
pub fn validate_scholar_explorer_usage_profile(statement: &serde_json::Value) -> Result<(), String> {
    if !statement_has_usage_profile(statement) {
        return Err("Statement does not declare the 'scholar-explorer-usage/v1' profile in context.contextActivities.category".to_string());
    }

    let verb_iri = statement.get("verb")
        .and_then(|v| v.get("id"))
        .and_then(|id| id.as_str())
        .unwrap_or_default();

    if !is_usage_profile_verb(verb_iri) {
        return Err(format!("Verb '{}' is not permitted by 'scholar-explorer-usage/v1' profile. Allowed verbs are: searched, prompted, ai-responded, viewed-source, exported", verb_iri));
    }

    if statement.get("object").is_none() {
        return Err("Statement must contain an 'object'".to_string());
    }

    Ok(())
}

/// Validates whether an AI actor statement conforms to the agent registry allowlist.
pub fn validate_ai_agent_allowlist(
    agent_registry: &[AgentRegistryRecord],
    statement_val: &serde_json::Value,
) -> Result<(), AgentDenied> {
    let actor_val = match statement_val.get("actor") {
        Some(a) => a,
        None => return Ok(()),
    };

    let is_agent = actor_val.get("account")
        .and_then(|acc| acc.get("homePage"))
        .and_then(|hp| hp.as_str())
        .map(|s| s.contains("agent") || s.contains("s3a"))
        .unwrap_or(false)
        || actor_val.get("name")
            .and_then(|n| n.as_str())
            .map(|s| s.starts_with("Agent:") || s.starts_with("AI:"))
            .unwrap_or(false);

    if !is_agent {
        return Ok(());
    }

    let actor_hash = hash_actor_value(actor_val);
    let name_hash = actor_val.get("name")
        .and_then(|n| n.as_str())
        .map(|n| n.trim_start_matches("Agent:").trim_start_matches("AI:").trim())
        .map(hash_iri);
    let acc_name_hash = actor_val.get("account")
        .and_then(|acc| acc.get("name"))
        .and_then(|n| n.as_str())
        .map(hash_iri);

    let verb_iri = statement_val.get("verb")
        .and_then(|v| v.get("id"))
        .and_then(|id| id.as_str())
        .unwrap_or_default();

    let verb_bit = resolve_verb_by_iri(verb_iri).map(|v| v.verb_bit);

    // If registered agent, assert permissions
    let agent_rec = agent_registry.iter().find(|r| {
        r.agent_id == actor_hash
            || name_hash.map(|h| r.agent_id == h).unwrap_or(false)
            || acc_name_hash.map(|h| r.agent_id == h).unwrap_or(false)
    });

    if let Some(agent_rec) = agent_rec {
        agent_rec.assert_allows_gated(verb_bit, None, false)?;
    } else {
        return Err(AgentDenied::AgentDisabled);
    }

    Ok(())
}

/// Enriches accepted statement into compact Scholar tiles:
/// - Human researcher statements -> Tile 10 (`ScholarTileType::HUMAN_LRS`)
/// - AI agent steps -> Tile 11 (`ScholarTileType::AI_TRACE_LOG`)
pub fn enrich_scholar_statement(
    writer: &ConcurrentTileWriter,
    statement_val: &serde_json::Value,
    _body_hash_xxh3: [u8; 16],
) -> std::io::Result<()> {
    let session_uuid = statement_val.get("context")
        .and_then(|c| c.get("registration"))
        .and_then(|r| r.as_str())
        .and_then(|s| uuid_to_u64_pair(s).ok())
        .unwrap_or([0, 0]);

    let timestamp_sec = statement_val.get("timestamp")
        .and_then(|t| t.as_str())
        .and_then(s3a_engine::lrs_store::chrono_parse_ms)
        .map(|ms| ms / 1000)
        .unwrap_or_else(|| (now_ms() / 1000) as u64);

    let actor_val = statement_val.get("actor");
    let actor_hash = actor_val.map(hash_actor_value).unwrap_or(0);

    let is_agent = actor_val.and_then(|a| a.get("account"))
        .and_then(|acc| acc.get("homePage"))
        .and_then(|hp| hp.as_str())
        .map(|s| s.contains("agent") || s.contains("s3a"))
        .unwrap_or(false)
        || actor_val.and_then(|a| a.get("name"))
            .and_then(|n| n.as_str())
            .map(|s| s.starts_with("Agent:") || s.starts_with("AI:"))
            .unwrap_or(false);

    if is_agent {
        let ai_rec = AiTraceRecord {
            session_uuid,
            timestamp_sec,
            agent_id: actor_hash,
            step_type: STEP_TOOL_CALL,
            confidence: 1.0,
            human_coord: S3ACoordinate::new(0, 0, 0),
            hilbert_index: 0,
            flags: 0,
        };
        let bytes = bytes_of(&ai_rec);
        let mut hdr = HyperTileHeader::new(0, ScholarTileType::AI_TRACE_LOG);
        hdr.record_count = 1;
        hdr.payload_bytes = bytes.len() as u32;
        hdr.data_crc32 = crc32c::crc32c(bytes);
        s3a_simd::bloom_filter_insert(&mut hdr.filter.bits, session_uuid[0] ^ session_uuid[1], hdr.filter.hash_seed);
        s3a_simd::bloom_filter_insert(&mut hdr.filter.bits, actor_hash, hdr.filter.hash_seed);
        writer.write_raw_hyper_tile(&hdr, bytes)?;
    } else {
        let verb_iri = statement_val.get("verb")
            .and_then(|v| v.get("id"))
            .and_then(|id| id.as_str())
            .unwrap_or_default();
        let verb_id = resolve_verb_by_iri(verb_iri).map(|v| v.verb_id).unwrap_or(0);

        let human_rec = HumanLrsRecord::new(
            session_uuid,
            timestamp_sec,
            actor_hash,
            verb_id,
            1.0,
            S3ACoordinate::new(0, 0, 0),
            0,
        );
        let bytes = bytes_of(&human_rec);
        let mut hdr = HyperTileHeader::new(0, ScholarTileType::HUMAN_LRS);
        hdr.record_count = 1;
        hdr.payload_bytes = bytes.len() as u32;
        hdr.data_crc32 = crc32c::crc32c(bytes);
        s3a_simd::bloom_filter_insert(&mut hdr.filter.bits, session_uuid[0] ^ session_uuid[1], hdr.filter.hash_seed);
        s3a_simd::bloom_filter_insert(&mut hdr.filter.bits, actor_hash, hdr.filter.hash_seed);
        writer.write_raw_hyper_tile(&hdr, bytes)?;
    }

    Ok(())
}

/// Constructs an authoritative xAPI statement when citing or saving a paper,
/// embedding the cryptographic content SHA-256 hash in context extensions,
/// and returning the statement alongside the matching CitationFingerprintRecord.
pub fn build_citation_statement(
    actor_mbox: &str,
    paper_id: u64,
    doi: &str,
    content_bytes: &[u8],
    content_kind: u32,
    source_url: Option<&str>,
    crossref_version: u32,
) -> (serde_json::Value, crate::records::CitationFingerprintRecord) {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content_bytes);
    let digest: [u8; 32] = hasher.finalize().into();
    let digest_hex = hex::encode(digest);

    let url_str = source_url.unwrap_or(doi);
    let now_sec = s3a_engine::lrs_store::now_ms() / 1000;

    let fingerprint = crate::records::CitationFingerprintRecord::new(
        paper_id,
        hash_iri(doi),
        digest,
        content_kind,
        crossref_version,
        now_sec,
        hash_iri(url_str),
    );

    let statement = serde_json::json!({
        "actor": {
            "mbox": actor_mbox
        },
        "verb": {
            "id": "http://purl.org/spar/cito/cites",
            "display": {
                "en-US": "cited"
            }
        },
        "object": {
            "id": if doi.starts_with("http") { doi.to_string() } else { format!("https://doi.org/{}", doi) },
            "objectType": "Activity",
            "definition": {
                "type": "https://ashecribb.dev/xapi/activities/academic-paper"
            }
        },
        "context": {
            "contextActivities": {
                "category": [
                    { "id": SCHOLAR_XAPI_PROFILE_IRI }
                ]
            },
            "extensions": {
                EXT_CONTENT_SHA256: digest_hex,
                EXT_CONTENT_KIND: fingerprint.content_kind_str(),
                EXT_SOURCE_URL: url_str,
                EXT_CROSSREF_VERSION: crossref_version,
                EXT_PAPER_ID: paper_id,
            }
        }
    });

    (statement, fingerprint)
}

/// Extracts a CitationFingerprintRecord from an xAPI statement's context extensions if present.
pub fn extract_citation_fingerprint_from_statement(
    statement: &serde_json::Value,
    paper_id: u64,
) -> Option<crate::records::CitationFingerprintRecord> {
    let ctx = statement.get("context")?.get("extensions")?;
    let sha_hex = ctx.get(EXT_CONTENT_SHA256)?.as_str()?;
    if sha_hex.len() != 64 {
        return None;
    }
    let mut digest = [0u8; 32];
    hex::decode_to_slice(sha_hex, &mut digest).ok()?;

    let kind_str = ctx.get(EXT_CONTENT_KIND).and_then(|v| v.as_str()).unwrap_or("pdf");
    let kind = crate::records::CitationFingerprintRecord::parse_content_kind(kind_str);
    let url_str = ctx.get(EXT_SOURCE_URL).and_then(|v| v.as_str()).unwrap_or("");
    let version = ctx.get(EXT_CROSSREF_VERSION).and_then(|v| v.as_u64()).unwrap_or(1) as u32;

    let obj_id = statement.get("object").and_then(|o| o.get("id")).and_then(|i| i.as_str()).unwrap_or("");
    let doi_hash = hash_iri(obj_id);
    let url_hash = hash_iri(url_str);
    let now_sec = s3a_engine::lrs_store::now_ms() / 1000;

    Some(crate::records::CitationFingerprintRecord::new(
        paper_id,
        doi_hash,
        digest,
        kind,
        version,
        now_sec,
        url_hash,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verb_resolution_roundtrip() {
        for spec in SCHOLAR_VERBS {
            let by_id = resolve_verb_by_id(spec.verb_id).expect("Verb ID must resolve");
            assert_eq!(by_id.iri, spec.iri);
            assert_eq!(by_id.label, spec.label);

            let by_iri = resolve_verb_by_iri(spec.iri).expect("Verb IRI must resolve");
            assert_eq!(by_iri.verb_id, spec.verb_id);

            let by_label = resolve_verb_by_label(spec.label).expect("Verb label must resolve");
            assert_eq!(by_label.verb_id, spec.verb_id);
        }
    }

    #[test]
    fn test_hash_iri_deterministic() {
        let hash1 = hash_iri(SCHOLAR_XAPI_PROFILE_IRI);
        let hash2 = hash_iri(SCHOLAR_XAPI_PROFILE_IRI);
        assert_eq!(hash1, hash2);
        assert_ne!(hash1, 0);
    }

    #[test]
    fn test_scholar_profile_validation_success_and_violation() {
        // Valid approved statement with result.success
        let valid_stmt = serde_json::json!({
            "actor": {"mbox": "mailto:dr.smith@university.edu"},
            "verb": {"id": "https://ashecribb.dev/xapi/verbs/approved"},
            "object": {
                "id": "https://doi.org/10.1000/182",
                "objectType": "Activity"
            },
            "result": {
                "success": true
            }
        });
        assert!(validate_scholar_profile(&valid_stmt).is_ok());

        // Violation: approved verb without result
        let invalid_no_result = serde_json::json!({
            "actor": {"mbox": "mailto:dr.smith@university.edu"},
            "verb": {"id": "https://ashecribb.dev/xapi/verbs/approved"},
            "object": {
                "id": "https://doi.org/10.1000/182"
            }
        });
        let err = validate_scholar_profile(&invalid_no_result).unwrap_err();
        assert!(err.contains("requires result object"));

        // Violation: approved verb without result.success
        let invalid_no_success = serde_json::json!({
            "actor": {"mbox": "mailto:dr.smith@university.edu"},
            "verb": {"id": "https://ashecribb.dev/xapi/verbs/approved"},
            "object": {
                "id": "https://doi.org/10.1000/182"
            },
            "result": {
                "response": "looks good"
            }
        });
        let err2 = validate_scholar_profile(&invalid_no_success).unwrap_err();
        assert!(err2.contains("requires result.success boolean"));

        // Violation: invalid activity IRI
        let invalid_iri = serde_json::json!({
            "actor": {"mbox": "mailto:dr.smith@university.edu"},
            "verb": {"id": "https://ashecribb.dev/xapi/verbs/approved"},
            "object": {
                "id": "not-an-iri"
            },
            "result": {"success": true}
        });
        let err3 = validate_scholar_profile(&invalid_iri).unwrap_err();
        assert!(err3.contains("not valid IRI"));
    }

    #[test]
    fn test_ai_agent_allowlist_enforcement_and_denial() {
        use crate::records::{
            seed_default_agents, AGENT_FLAG_ACTIVE, AGENT_FLAG_DISABLED,
            AGENT_FLAG_REQUIRES_HUMAN_GATE,
        };

        let registry = seed_default_agents();

        // 1. Human researcher statement is always allowed through
        let human_stmt = serde_json::json!({
            "actor": {"mbox": "mailto:researcher@lab.edu"},
            "verb": {"id": "https://ashecribb.dev/xapi/verbs/approved"}
        });
        assert_eq!(validate_ai_agent_allowlist(&registry, &human_stmt), Ok(()));

        // 2. Registered AI agent ('lab-assistant') with permitted verb ('experienced', bit 4)
        let agent_stmt_ok = serde_json::json!({
            "actor": {
                "account": {
                    "homePage": "https://s3a.local/agents",
                    "name": "lab-assistant"
                }
            },
            "verb": {"id": "http://adlnet.gov/expapi/verbs/experienced"}
        });
        assert_eq!(validate_ai_agent_allowlist(&registry, &agent_stmt_ok), Ok(()));

        // 3. Registered AI agent ('lab-assistant') with unpermitted verb ('rejected', bit 7)
        let agent_stmt_denied_verb = serde_json::json!({
            "actor": {
                "name": "Agent: lab-assistant",
                "account": {
                    "homePage": "https://s3a.local/agents",
                    "name": "lab-assistant"
                }
            },
            "verb": {"id": "https://ashecribb.dev/xapi/verbs/rejected"}
        });
        assert_eq!(
            validate_ai_agent_allowlist(&registry, &agent_stmt_denied_verb),
            Err(AgentDenied::VerbNotAllowlisted { verb_bit: 1 << 7 })
        );

        // 4. Unregistered AI agent is rejected (AgentDisabled / unlisted)
        let unknown_agent_stmt = serde_json::json!({
            "actor": {
                "name": "Agent: rogue-crawler",
                "account": {
                    "homePage": "https://s3a.local/agents",
                    "name": "rogue-crawler"
                }
            },
            "verb": {"id": "http://adlnet.gov/expapi/verbs/experienced"}
        });
        assert_eq!(
            validate_ai_agent_allowlist(&registry, &unknown_agent_stmt),
            Err(AgentDenied::AgentDisabled)
        );

        // 5. Disabled agent in registry
        let disabled_agent = AgentRegistryRecord::new(
            hash_iri("disabled-bot"),
            0,
            0,
            0xFFFF,
            0xFFFF,
            0,
            1,
            AGENT_FLAG_DISABLED,
            1,
        );
        let disabled_stmt = serde_json::json!({
            "actor": {
                "name": "AI: disabled-bot",
                "account": {
                    "homePage": "https://s3a.local/agents",
                    "name": "disabled-bot"
                }
            },
            "verb": {"id": "http://adlnet.gov/expapi/verbs/experienced"}
        });
        assert_eq!(
            validate_ai_agent_allowlist(&[disabled_agent], &disabled_stmt),
            Err(AgentDenied::AgentDisabled)
        );

        // 6. Gated agent requiring human supervision without clearance
        let gated_agent = AgentRegistryRecord::new(
            hash_iri("gated-bot"),
            0,
            0,
            0xFFFF,
            0xFFFF,
            0,
            1,
            AGENT_FLAG_ACTIVE | AGENT_FLAG_REQUIRES_HUMAN_GATE,
            1,
        );
        let gated_stmt = serde_json::json!({
            "actor": {
                "name": "AI: gated-bot",
                "account": {
                    "homePage": "https://s3a.local/agents",
                    "name": "gated-bot"
                }
            },
            "verb": {"id": "http://adlnet.gov/expapi/verbs/experienced"}
        });
        assert_eq!(
            validate_ai_agent_allowlist(&[gated_agent], &gated_stmt),
            Err(AgentDenied::RequiresHumanGate)
        );
    }

    #[test]
    fn test_enrich_scholar_statement_tiles_10_and_11() {
        let mut temp_path = std::env::temp_dir();
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        temp_path.push(format!("s3a_enrich_test_{}_{}.s3a", std::process::id(), nonce));

        let writer = ConcurrentTileWriter::create(&temp_path).unwrap();
        writer.enable_addon(s3a_core::ADDON_EDU).unwrap();

        // 1. Enrich human statement -> Tile 10 (HUMAN_LRS)
        let human_stmt = serde_json::json!({
            "id": "11111111-2222-3333-4444-555555555555",
            "actor": {"mbox": "mailto:dr.scholar@univ.edu"},
            "verb": {"id": "https://ashecribb.dev/xapi/verbs/reviewed"},
            "object": {"id": "https://doi.org/10.1000/182"},
            "context": {"registration": "66666666-7777-8888-9999-aaaaaaaaaaaa"}
        });
        enrich_scholar_statement(&writer, &human_stmt, [0u8; 16]).unwrap();

        // 2. Enrich AI agent statement -> Tile 11 (AI_TRACE_LOG)
        let ai_stmt = serde_json::json!({
            "id": "22222222-3333-4444-5555-666666666666",
            "actor": {
                "account": {
                    "homePage": "https://s3a.local/agents",
                    "name": "lab-assistant"
                }
            },
            "verb": {"id": "http://adlnet.gov/expapi/verbs/experienced"},
            "object": {"id": "https://doi.org/10.1000/182"},
            "context": {"registration": "66666666-7777-8888-9999-aaaaaaaaaaaa"}
        });
        enrich_scholar_statement(&writer, &ai_stmt, [0u8; 16]).unwrap();
        drop(writer);

        // Verify tiles written into archive
        let reader = s3a_engine::MmapReader::open(&temp_path).unwrap();
        assert_eq!(reader.tile_count(), 2);

        let (hdr0, payload0) = reader.get_tile(0).unwrap();
        assert_eq!(hdr0.tile_type, ScholarTileType::HUMAN_LRS);
        assert_eq!(hdr0.record_count, 1);
        let human_records: &[HumanLrsRecord] = bytemuck::cast_slice(payload0);
        assert_eq!(human_records.len(), 1);
        assert_eq!(human_records[0].verb_id, VERB_ID_REVIEWED);

        let (hdr1, payload1) = reader.get_tile(1).unwrap();
        assert_eq!(hdr1.tile_type, ScholarTileType::AI_TRACE_LOG);
        assert_eq!(hdr1.record_count, 1);
        let ai_records: &[AiTraceRecord] = bytemuck::cast_slice(payload1);
        assert_eq!(ai_records.len(), 1);
        assert_eq!(ai_records[0].step_type, STEP_TOOL_CALL);

        let _ = std::fs::remove_file(&temp_path);
    }

    #[test]
    fn test_scholar_explorer_usage_profile_validation() {
        let allowed_verbs = [
            USAGE_VERB_SEARCHED,
            USAGE_VERB_PROMPTED,
            USAGE_VERB_AI_RESPONDED,
            USAGE_VERB_VIEWED_SOURCE,
            USAGE_VERB_EXPORTED,
        ];

        // 1. All 5 permitted verbs must be accepted
        for &verb in &allowed_verbs {
            let stmt = serde_json::json!({
                "actor": {"account": {"name": "device_xyz", "homePage": "https://scholar.explorer"}},
                "verb": {"id": verb},
                "object": {"id": "https://example.com/item/1"},
                "context": {
                    "contextActivities": {
                        "category": [{"id": "https://ashecribb.dev/xapi/profiles/scholar-explorer-usage/v1"}]
                    }
                }
            });
            assert!(
                validate_scholar_explorer_usage_profile(&stmt).is_ok(),
                "Verb '{}' must be accepted by usage profile",
                verb
            );
        }

        // 2. Removed researcher verbs must be rejected
        let removed_verbs = [
            "https://ashecribb.dev/xapi/verbs/reviewed",
            "https://ashecribb.dev/xapi/verbs/verified-claim",
            "https://ashecribb.dev/xapi/verbs/raise-hand-to-ta",
            "http://adlnet.gov/expapi/verbs/experienced",
            "http://adlnet.gov/expapi/verbs/completed",
        ];
        for &verb in &removed_verbs {
            let stmt = serde_json::json!({
                "actor": {"account": {"name": "device_xyz", "homePage": "https://scholar.explorer"}},
                "verb": {"id": verb},
                "object": {"id": "https://example.com/item/1"},
                "context": {
                    "contextActivities": {
                        "category": [{"id": "https://ashecribb.dev/xapi/profiles/scholar-explorer-usage/v1"}]
                    }
                }
            });
            assert!(
                validate_scholar_explorer_usage_profile(&stmt).is_err(),
                "Verb '{}' must be rejected by usage profile",
                verb
            );
        }

        // 3. Rejection: missing profile category
        let no_profile = serde_json::json!({
            "actor": {"account": {"name": "device_xyz", "homePage": "https://scholar.explorer"}},
            "verb": {"id": USAGE_VERB_SEARCHED},
            "object": {"id": "https://example.com/item/1"}
        });
        assert!(validate_scholar_explorer_usage_profile(&no_profile).is_err());
    }

    #[test]
    fn test_build_citation_statement_and_extraction() {
        let content = b"%PDF-1.7 Test academic article source bytes for SHA-256 fingerprinting";
        let (stmt, fp) = build_citation_statement(
            "mailto:curator@institution.edu",
            42,
            "10.1000/182",
            content,
            crate::records::CONTENT_KIND_PDF,
            Some("https://example.com/paper.pdf"),
            1,
        );

        let ext_hash = stmt["context"]["extensions"][EXT_CONTENT_SHA256].as_str().unwrap();
        assert_eq!(ext_hash, fp.content_sha256_hex());
        assert_eq!(stmt["context"]["extensions"][EXT_CONTENT_KIND], "pdf");

        let extracted = extract_citation_fingerprint_from_statement(&stmt, 42).expect("Must extract fingerprint");
        assert_eq!(extracted.paper_id, 42);
        assert_eq!(extracted.content_sha256, fp.content_sha256);
        assert_eq!(extracted.content_kind, crate::records::CONTENT_KIND_PDF);
        assert_eq!(extracted.crossref_version, 1);
    }
}
