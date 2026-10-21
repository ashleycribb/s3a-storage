//! S3A Learner API (IEEE P2997 Subset / ADL ELRR REST Service).
//!
//! Implements the Learner REST API mirroring https://github.com/adlnet/elrr-documentation:
//! - service-api.md endpoints for Person, Organization, LearningResource, LearningEvent,
//!   CompetencyAssertion, CredentialAssertion, Goal.
//! - GET/POST/PUT; DELETE = tombstone + audit.
//! - auth.md JWT client tokens + admin override mode.
//! - Writes structured audit records to the audit archive.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

use sha2::{Digest, Sha256};
use tiny_http::{Header, Method, Response, StatusCode};

use s3a_core::*;
use s3a_engine::concurrent_writer::ConcurrentTileWriter;
use s3a_engine::{elr::*, format_iso8601_ms, u64_pair_to_uuid, uuid_to_u64_pair};

use crate::{constant_time_eq, format_http_date, AppState};

// -----------------------------------------------------------------------------
// HMAC-SHA256 & BASE64URL HELPERS
// -----------------------------------------------------------------------------

pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        let mut hasher = Sha256::new();
        hasher.update(key);
        let digest = hasher.finalize();
        k[..32].copy_from_slice(&digest);
    } else {
        k[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }

    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(message);
    let inner_digest = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner_digest);
    let outer_digest = outer.finalize();

    let mut res = [0u8; 32];
    res.copy_from_slice(&outer_digest);
    res
}

pub fn base64url_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity((input.len() * 4).div_ceil(3));
    let mut i = 0;
    while i < input.len() {
        let b0 = input[i] as u32;
        let b1 = if i + 1 < input.len() {
            input[i + 1] as u32
        } else {
            0
        };
        let b2 = if i + 2 < input.len() {
            input[i + 2] as u32
        } else {
            0
        };

        let triple = (b0 << 16) | (b1 << 8) | b2;

        out.push(TABLE[((triple >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3F) as usize] as char);
        if i + 1 < input.len() {
            out.push(TABLE[((triple >> 6) & 0x3F) as usize] as char);
        }
        if i + 2 < input.len() {
            out.push(TABLE[(triple & 0x3F) as usize] as char);
        }
        i += 3;
    }
    out
}

pub fn base64url_decode(input: &str) -> Result<Vec<u8>, ()> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut buf = Vec::new();
    let mut acc = 0u32;
    let mut bits = 0;

    for &b in input.as_bytes() {
        if b == b'=' || b.is_ascii_whitespace() {
            continue;
        }
        let val = TABLE.iter().position(|&x| x == b).ok_or(())? as u32;
        acc = (acc << 6) | val;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            buf.push(((acc >> bits) & 0xFF) as u8);
        }
    }
    Ok(buf)
}

// -----------------------------------------------------------------------------
// JWT CLAIMS & AUTH
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JwtClaims {
    pub sub: String,
    pub jti: String,
    pub exp: u64,
    pub scopes: Vec<String>,
}

impl JwtClaims {
    pub fn has_scope(&self, req_scope: &str) -> bool {
        self.scopes.iter().any(|s| s == "admin" || s == req_scope)
    }
}

pub fn generate_jwt(secret: &str, sub: &str, scopes: &[String], exp_secs: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let claims = JwtClaims {
        sub: sub.to_string(),
        jti: s3a_engine::lrs_store::generate_uuid_v4(),
        exp: now + exp_secs,
        scopes: scopes.to_vec(),
    };

    let header_json = r#"{"alg":"HS256","typ":"JWT"}"#;
    let payload_json = serde_json::to_string(&claims).unwrap_or_default();

    let header_b64 = base64url_encode(header_json.as_bytes());
    let payload_b64 = base64url_encode(payload_json.as_bytes());
    let unsigned = format!("{}.{}", header_b64, payload_b64);

    let sig = hmac_sha256(secret.as_bytes(), unsigned.as_bytes());
    let sig_b64 = base64url_encode(&sig);

    format!("{}.{}", unsigned, sig_b64)
}

pub fn verify_jwt(secret: &str, token: &str) -> Result<JwtClaims, &'static str> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err("Invalid JWT token format");
    }

    let unsigned = format!("{}.{}", parts[0], parts[1]);
    let expected_sig = hmac_sha256(secret.as_bytes(), unsigned.as_bytes());
    let actual_sig = base64url_decode(parts[2]).map_err(|_| "Invalid signature encoding")?;

    if !constant_time_eq(&expected_sig, &actual_sig) {
        return Err("Invalid token signature");
    }

    let payload_bytes = base64url_decode(parts[1]).map_err(|_| "Invalid payload encoding")?;
    let claims: JwtClaims =
        serde_json::from_slice(&payload_bytes).map_err(|_| "Invalid claims JSON")?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    if claims.exp > 0 && claims.exp < now {
        return Err("Token expired");
    }

    Ok(claims)
}

// -----------------------------------------------------------------------------
// AUDIT LOGGING FOR LEARNER API
// -----------------------------------------------------------------------------

pub fn log_learner_audit(
    writer: &ConcurrentTileWriter,
    action_type: &str,
    svc_method: &str,
    entity_type: &str,
    entity_id: &str,
    jwt_id: &str,
    status_code: u16,
) {
    let now = s3a_engine::lrs_store::now_ms();
    let audit_rec = serde_json::json!({
        "id": s3a_engine::lrs_store::generate_uuid_v4(),
        "timestamp": format_iso8601_ms(now),
        "action_type": action_type,
        "svc_method": svc_method,
        "entity_type": entity_type,
        "entity_id": entity_id,
        "jwt_id": jwt_id,
        "status": status_code,
    });
    let bytes = serde_json::to_vec(&audit_rec).unwrap_or_default();
    let mut hdr = HyperTileHeader::new(0, TILE_STATEMENT_BODY);
    hdr.record_count = 1;
    hdr.payload_bytes = bytes.len() as u32;
    hdr.data_crc32 = crc32c::crc32c(&bytes);
    let _ = writer.write_raw_hyper_tile(&hdr, &bytes);
}

// -----------------------------------------------------------------------------
// HTTP RESPONSE HELPERS
// -----------------------------------------------------------------------------

fn json_response<T: serde::Serialize>(status_code: u16, data: &T) -> Response<Cursor<Vec<u8>>> {
    let body = serde_json::to_vec(data).unwrap_or_default();
    let len = body.len();
    Response::new(
        StatusCode(status_code),
        vec![
            Header::from_bytes(&b"Content-Type"[..], &b"application/json; charset=utf-8"[..]).unwrap(),
            Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap(),
            Header::from_bytes(&b"Access-Control-Allow-Headers"[..], &b"Content-Type, Authorization, X-Experience-API-Version, If-Match, If-None-Match, Access-Control-Request-Private-Network"[..]).unwrap(),
            Header::from_bytes(&b"Access-Control-Allow-Methods"[..], &b"GET, POST, PUT, DELETE, HEAD, OPTIONS"[..]).unwrap(),
            Header::from_bytes(&b"Access-Control-Expose-Headers"[..], &b"ETag, Last-Modified, X-Experience-API-Version, X-Experience-API-Consistent-Through"[..]).unwrap(),
            Header::from_bytes(&b"Date"[..], format_http_date().as_bytes()).unwrap(),
        ],
        Cursor::new(body),
        Some(len),
        None,
    )
}

fn empty_response(status_code: u16) -> Response<Cursor<Vec<u8>>> {
    Response::new(
        StatusCode(status_code),
        vec![
            Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap(),
            Header::from_bytes(&b"Access-Control-Allow-Headers"[..], &b"Content-Type, Authorization, X-Experience-API-Version, If-Match, If-None-Match, Access-Control-Request-Private-Network"[..]).unwrap(),
            Header::from_bytes(&b"Access-Control-Allow-Methods"[..], &b"GET, POST, PUT, DELETE, HEAD, OPTIONS"[..]).unwrap(),
            Header::from_bytes(&b"Access-Control-Expose-Headers"[..], &b"ETag, Last-Modified, X-Experience-API-Version, X-Experience-API-Consistent-Through"[..]).unwrap(),
            Header::from_bytes(&b"Date"[..], format_http_date().as_bytes()).unwrap(),
        ],
        Cursor::new(Vec::new()),
        Some(0),
        None,
    )
}

fn error_response(status_code: u16, msg: &str) -> Response<Cursor<Vec<u8>>> {
    let err_obj = serde_json::json!({ "error": msg });
    json_response(status_code, &err_obj)
}

// -----------------------------------------------------------------------------
// MAIN LEARNER API DISPATCHER
// -----------------------------------------------------------------------------

pub fn handle_learner_api(
    method: &Method,
    path: &str,
    params: &HashMap<String, String>,
    headers: &HashMap<String, String>,
    body: &[u8],
    state: &Arc<AppState>,
) -> Response<Cursor<Vec<u8>>> {
    // 1. Auth check
    let mut is_admin = state.config.admin_api_override;
    let mut client_sub = if is_admin {
        "admin-override".to_string()
    } else {
        "anonymous".to_string()
    };
    let mut jwt_id = if is_admin {
        "admin-override".to_string()
    } else {
        "none".to_string()
    };
    let mut scopes = if is_admin {
        vec!["admin".to_string(), "read".to_string(), "write".to_string()]
    } else {
        Vec::new()
    };

    let auth_hdr = headers.get("authorization");
    if let Some(auth_val) = auth_hdr {
        if auth_val.starts_with("Bearer ") {
            let token = auth_val[7..].trim();
            match verify_jwt(&state.config.jwt_secret, token) {
                Ok(claims) => {
                    client_sub = claims.sub.clone();
                    jwt_id = claims.jti.clone();
                    is_admin = claims.has_scope("admin") || state.config.admin_api_override;
                    scopes = claims.scopes;
                }
                Err(err) => {
                    let resp = error_response(401, &format!("Unauthorized: {}", err));
                    log_learner_audit(
                        &state.audit_writer,
                        "AUTH",
                        &format!("{} {}", method.as_str(), path),
                        "Token",
                        "",
                        &jwt_id,
                        401,
                    );
                    return resp;
                }
            }
        } else if auth_val.starts_with("Basic ") {
            // Check basic auth for admin override or configured credentials
            let encoded = auth_val[6..].trim();
            if let Ok(decoded) = crate::base64_decode(encoded) {
                if let Ok(user_pass) = String::from_utf8(decoded) {
                    if let Some((user, pass)) = user_pass.split_once(':') {
                        for cred in &state.config.credentials {
                            if constant_time_eq(user.as_bytes(), cred.user.as_bytes())
                                && constant_time_eq(pass.as_bytes(), cred.pass.as_bytes())
                            {
                                client_sub = cred.user.clone();
                                jwt_id = format!("basic:{}", cred.user);
                                if cred.has_scope("all") || cred.has_scope("admin") {
                                    is_admin = true;
                                    scopes = vec![
                                        "admin".to_string(),
                                        "read".to_string(),
                                        "write".to_string(),
                                    ];
                                } else {
                                    scopes = cred.scopes.clone();
                                }
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    // Dev mode fallback
    if state.config.insecure_dev {
        is_admin = true;
        scopes = vec!["admin".to_string(), "read".to_string(), "write".to_string()];
        client_sub = "insecure-dev".to_string();
    }

    // 2. Token generation endpoint: POST /admin/token
    if path == "/admin/token" || path == "/admin/token/" {
        if method != &Method::Post {
            return error_response(405, "Method Not Allowed");
        }
        let req_json: serde_json::Value = match serde_json::from_slice(body) {
            Ok(v) => v,
            Err(_) => serde_json::json!({}),
        };
        let client_id = req_json
            .get("client_id")
            .and_then(|v| v.as_str())
            .unwrap_or(&client_sub);
        let requested_scopes: Vec<String> = req_json
            .get("scopes")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|s| s.as_str().map(|str_val| str_val.to_string()))
                    .collect()
            })
            .unwrap_or_else(|| vec!["read".to_string(), "write".to_string()]);

        // If client requested "admin" scope, verify caller is admin
        if requested_scopes.contains(&"admin".to_string()) && !is_admin {
            return error_response(403, "Forbidden: caller cannot issue admin tokens");
        }

        let token = generate_jwt(&state.config.jwt_secret, client_id, &requested_scopes, 3600);
        let resp_payload = serde_json::json!({
            "access_token": token,
            "token_type": "Bearer",
            "expires_in": 3600,
            "scopes": requested_scopes
        });
        log_learner_audit(
            &state.audit_writer,
            "CREATE",
            "POST /admin/token",
            "Token",
            client_id,
            &jwt_id,
            200,
        );
        return json_response(200, &resp_payload);
    }

    // -------------------------------------------------------------------------
    // ADMIN ENDPOINT: Delete by Actor Account / Device ID
    // -------------------------------------------------------------------------
    if path == "/admin/delete-by-actor-account"
        || path == "/admin/delete-by-actor-account/"
        || path == "/admin/purge/actor"
        || path == "/admin/purge/actor/"
        || path == "/admin/purge-by-device-id"
        || path == "/admin/purge-by-device-id/"
    {
        if method != &Method::Post && method != &Method::Delete {
            return error_response(405, "Method Not Allowed");
        }
        if !is_admin {
            let resp = error_response(403, "Forbidden: 'admin' scope required");
            log_learner_audit(
                &state.audit_writer,
                "ADMIN",
                &format!("{} {}", method.as_str(), path),
                "ActorAccount",
                "",
                &jwt_id,
                403,
            );
            return resp;
        }

        let body_val: serde_json::Value =
            serde_json::from_slice(body).unwrap_or(serde_json::json!({}));
        let target_device_id = body_val
            .get("deviceId")
            .or_else(|| body_val.get("device_id"))
            .or_else(|| body_val.get("name"))
            .or_else(|| body_val.get("account").and_then(|a| a.get("name")))
            .and_then(|v| v.as_str())
            .or_else(|| params.get("deviceId").map(|s| s.as_str()))
            .or_else(|| params.get("device_id").map(|s| s.as_str()))
            .or_else(|| params.get("name").map(|s| s.as_str()))
            .or_else(|| params.get("account").map(|s| s.as_str()))
            .unwrap_or_default();

        if target_device_id.is_empty() {
            return error_response(400, "Missing device ID or actor account name to purge");
        }

        let home_page = body_val
            .get("account")
            .and_then(|a| a.get("homePage"))
            .and_then(|h| h.as_str())
            .or_else(|| params.get("homePage").map(|s| s.as_str()));

        let target_archive = body_val
            .get("archive")
            .and_then(|a| a.as_str())
            .or_else(|| params.get("archive").map(|s| s.as_str()))
            .unwrap_or("usage.s3a");

        let store = match state.get_or_open_store(target_archive) {
            Ok(s) => s,
            Err(e) => {
                return error_response(
                    500,
                    &format!("Failed to open archive '{}': {:?}", target_archive, e),
                )
            }
        };

        match store.purge_by_actor_account(target_device_id, home_page, "s3a-admin") {
            Ok(receipts) => {
                let resp_payload = serde_json::json!({
                    "success": true,
                    "device_id": target_device_id,
                    "archive": target_archive,
                    "purged_count": receipts.len(),
                    "receipts": receipts,
                });
                log_learner_audit(
                    &state.audit_writer,
                    "PURGE_ACTOR",
                    &format!("{} {}", method.as_str(), path),
                    "ActorAccount",
                    target_device_id,
                    &jwt_id,
                    200,
                );
                return json_response(200, &resp_payload);
            }
            Err(e) => {
                let resp = error_response(500, &format!("Failed to purge actor account: {:?}", e));
                log_learner_audit(
                    &state.audit_writer,
                    "PURGE_ACTOR",
                    &format!("{} {}", method.as_str(), path),
                    "ActorAccount",
                    target_device_id,
                    &jwt_id,
                    500,
                );
                return resp;
            }
        }
    }

    // -------------------------------------------------------------------------
    // ADMIN ENDPOINT: Compaction & Retention Purge (> 90 days)
    // -------------------------------------------------------------------------
    if path == "/admin/compact"
        || path == "/admin/compact/"
        || path == "/admin/purge/retention"
        || path == "/admin/purge/retention/"
    {
        if method != &Method::Post {
            return error_response(405, "Method Not Allowed");
        }
        if !is_admin {
            let resp = error_response(403, "Forbidden: 'admin' scope required");
            log_learner_audit(
                &state.audit_writer,
                "ADMIN",
                &format!("{} {}", method.as_str(), path),
                "Retention",
                "",
                &jwt_id,
                403,
            );
            return resp;
        }

        let body_val: serde_json::Value =
            serde_json::from_slice(body).unwrap_or(serde_json::json!({}));
        let target_archive = body_val
            .get("archive")
            .and_then(|a| a.as_str())
            .or_else(|| params.get("archive").map(|s| s.as_str()))
            .unwrap_or("usage.s3a");

        let retention_days: u64 = body_val
            .get("retention_days")
            .and_then(|d| d.as_u64())
            .or_else(|| params.get("retention_days").and_then(|d| d.parse().ok()))
            .unwrap_or(90);

        let store = match state.get_or_open_store(target_archive) {
            Ok(s) => s,
            Err(e) => {
                return error_response(
                    500,
                    &format!("Failed to open archive '{}': {:?}", target_archive, e),
                )
            }
        };

        match store.purge_and_compact_retention(retention_days, "s3a-admin") {
            Ok(result) => {
                let resp_payload = serde_json::json!({
                    "success": true,
                    "archive": result.archive,
                    "retention_days": result.retention_days,
                    "cutoff_ms": result.cutoff_ms,
                    "purged_count": result.purged_count,
                    "receipts": result.receipts,
                });
                log_learner_audit(
                    &state.audit_writer,
                    "COMPACT_RETENTION",
                    &format!("{} {}", method.as_str(), path),
                    "Retention",
                    target_archive,
                    &jwt_id,
                    200,
                );
                return json_response(200, &resp_payload);
            }
            Err(e) => {
                let resp = error_response(500, &format!("Retention compaction failed: {:?}", e));
                log_learner_audit(
                    &state.audit_writer,
                    "COMPACT_RETENTION",
                    &format!("{} {}", method.as_str(), path),
                    "Retention",
                    target_archive,
                    &jwt_id,
                    500,
                );
                return resp;
            }
        }
    }

    // Check permissions for data endpoints
    let has_read = is_admin || scopes.iter().any(|s| s == "read" || s == "admin");
    let has_write = is_admin || scopes.iter().any(|s| s == "write" || s == "admin");

    if (method == &Method::Get || method == &Method::Head) && !has_read {
        let resp = error_response(403, "Forbidden: 'read' scope required");
        log_learner_audit(
            &state.audit_writer,
            "READ",
            &format!("{} {}", method.as_str(), path),
            "Resource",
            "",
            &jwt_id,
            403,
        );
        return resp;
    }
    if (method == &Method::Post || method == &Method::Put) && !has_write {
        let resp = error_response(403, "Forbidden: 'write' scope required");
        log_learner_audit(
            &state.audit_writer,
            "WRITE",
            &format!("{} {}", method.as_str(), path),
            "Resource",
            "",
            &jwt_id,
            403,
        );
        return resp;
    }
    if method == &Method::Delete && !has_write {
        let resp = error_response(
            403,
            "Forbidden: 'write' or 'admin' scope required for DELETE",
        );
        log_learner_audit(
            &state.audit_writer,
            "DELETE",
            &format!("{} {}", method.as_str(), path),
            "Resource",
            "",
            &jwt_id,
            403,
        );
        return resp;
    }

    let include_hidden = is_admin
        && params
            .get("include_hidden")
            .map(|v| v == "true")
            .unwrap_or(false);

    // Parse entity routes: /api/<entity> or /api/<entity>/<id>
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() < 2 || parts[0] != "api" {
        return error_response(404, "Not Found");
    }

    let entity_type = parts[1];
    let id_opt = if parts.len() >= 3 {
        Some(parts[2])
    } else {
        None
    };

    match entity_type {
        "person" => handle_person_entity(method, id_opt, include_hidden, body, state, &jwt_id),
        "organization" => handle_organization_entity(method, id_opt, body, state, &jwt_id),
        "learning-resource" => handle_resource_entity(method, id_opt, body, state, &jwt_id),
        "learning-event" => {
            handle_event_entity(method, id_opt, params, include_hidden, body, state, &jwt_id)
        }
        "competency" => {
            handle_competency_entity(method, id_opt, params, include_hidden, body, state, &jwt_id)
        }
        "credential" => {
            handle_credential_entity(method, id_opt, params, include_hidden, body, state, &jwt_id)
        }
        "goal" => handle_goal_entity(method, id_opt, params, include_hidden, body, state, &jwt_id),
        _ => error_response(404, "Not Found"),
    }
}

// -----------------------------------------------------------------------------
// ENTITY HANDLERS (Person, Org, Resource, Event, Competency, Credential, Goal)
// -----------------------------------------------------------------------------

fn handle_person_entity(
    method: &Method,
    id_opt: Option<&str>,
    include_hidden: bool,
    body: &[u8],
    state: &Arc<AppState>,
    jwt_id: &str,
) -> Response<Cursor<Vec<u8>>> {
    match (method, id_opt) {
        (&Method::Get, None) => {
            let list = state.store.list_persons(include_hidden);
            log_learner_audit(
                &state.audit_writer,
                "READ",
                "GET /api/person",
                "Person",
                "",
                jwt_id,
                200,
            );
            json_response(200, &list)
        }
        (&Method::Get, Some(id_str)) => {
            let id = match uuid_to_u64_pair(id_str) {
                Ok(p) => p,
                Err(_) => return error_response(400, "Invalid Person UUID"),
            };
            match state.store.get_person(id, include_hidden) {
                Some(p) => {
                    log_learner_audit(
                        &state.audit_writer,
                        "READ",
                        &format!("GET /api/person/{}", id_str),
                        "Person",
                        id_str,
                        jwt_id,
                        200,
                    );
                    json_response(200, &p)
                }
                None => {
                    log_learner_audit(
                        &state.audit_writer,
                        "READ",
                        &format!("GET /api/person/{}", id_str),
                        "Person",
                        id_str,
                        jwt_id,
                        404,
                    );
                    error_response(404, "Person Not Found")
                }
            }
        }
        (&Method::Post, _) => {
            let mut person: ElrPerson = match serde_json::from_slice(body) {
                Ok(p) => p,
                Err(e) => return error_response(400, &format!("Invalid Person JSON: {}", e)),
            };
            if person.id == [0, 0] {
                person.id = deterministic_uuid("person", &person.identifier);
            }
            if person.timestamp_ms == 0 {
                person.timestamp_ms = s3a_engine::lrs_store::now_ms();
            }
            let entity_id_str = u64_pair_to_uuid(person.id);
            match state.store.put_person(person.clone()) {
                Ok(_) => {
                    log_learner_audit(
                        &state.audit_writer,
                        "CREATE",
                        "POST /api/person",
                        "Person",
                        &entity_id_str,
                        jwt_id,
                        201,
                    );
                    json_response(201, &person)
                }
                Err(e) => error_response(500, &format!("Storage failure: {}", e)),
            }
        }
        (&Method::Put, Some(id_str)) => {
            let id = match uuid_to_u64_pair(id_str) {
                Ok(p) => p,
                Err(_) => return error_response(400, "Invalid Person UUID"),
            };
            let mut person: ElrPerson = match serde_json::from_slice(body) {
                Ok(p) => p,
                Err(e) => return error_response(400, &format!("Invalid Person JSON: {}", e)),
            };
            person.id = id;
            if person.timestamp_ms == 0 {
                person.timestamp_ms = s3a_engine::lrs_store::now_ms();
            }
            match state.store.put_person(person.clone()) {
                Ok(_) => {
                    log_learner_audit(
                        &state.audit_writer,
                        "UPDATE",
                        &format!("PUT /api/person/{}", id_str),
                        "Person",
                        id_str,
                        jwt_id,
                        200,
                    );
                    json_response(200, &person)
                }
                Err(e) => error_response(500, &format!("Storage failure: {}", e)),
            }
        }
        (&Method::Delete, Some(id_str)) => {
            let id = match uuid_to_u64_pair(id_str) {
                Ok(p) => p,
                Err(_) => return error_response(400, "Invalid Person UUID"),
            };
            match state.store.delete_person(id) {
                Ok(_) => {
                    log_learner_audit(
                        &state.audit_writer,
                        "DELETE",
                        &format!("DELETE /api/person/{}", id_str),
                        "Person",
                        id_str,
                        jwt_id,
                        204,
                    );
                    empty_response(204)
                }
                Err(e) => error_response(500, &format!("Deletion failure: {}", e)),
            }
        }
        _ => error_response(405, "Method Not Allowed"),
    }
}

fn handle_organization_entity(
    method: &Method,
    id_opt: Option<&str>,
    body: &[u8],
    state: &Arc<AppState>,
    jwt_id: &str,
) -> Response<Cursor<Vec<u8>>> {
    match (method, id_opt) {
        (&Method::Get, None) => {
            let list = state.store.list_organizations();
            log_learner_audit(
                &state.audit_writer,
                "READ",
                "GET /api/organization",
                "Organization",
                "",
                jwt_id,
                200,
            );
            json_response(200, &list)
        }
        (&Method::Get, Some(id_str)) => {
            let id = match uuid_to_u64_pair(id_str) {
                Ok(p) => p,
                Err(_) => return error_response(400, "Invalid Organization UUID"),
            };
            match state.store.get_organization(id) {
                Some(o) => {
                    log_learner_audit(
                        &state.audit_writer,
                        "READ",
                        &format!("GET /api/organization/{}", id_str),
                        "Organization",
                        id_str,
                        jwt_id,
                        200,
                    );
                    json_response(200, &o)
                }
                None => error_response(404, "Organization Not Found"),
            }
        }
        (&Method::Post, _) => {
            let mut org: ElrOrganization = match serde_json::from_slice(body) {
                Ok(o) => o,
                Err(e) => return error_response(400, &format!("Invalid Organization JSON: {}", e)),
            };
            if org.id == [0, 0] {
                org.id = deterministic_uuid("organization", &org.name);
            }
            if org.timestamp_ms == 0 {
                org.timestamp_ms = s3a_engine::lrs_store::now_ms();
            }
            let entity_id_str = u64_pair_to_uuid(org.id);
            match state.store.put_organization(org.clone()) {
                Ok(_) => {
                    log_learner_audit(
                        &state.audit_writer,
                        "CREATE",
                        "POST /api/organization",
                        "Organization",
                        &entity_id_str,
                        jwt_id,
                        201,
                    );
                    json_response(201, &org)
                }
                Err(e) => error_response(500, &format!("Storage failure: {}", e)),
            }
        }
        (&Method::Put, Some(id_str)) => {
            let id = match uuid_to_u64_pair(id_str) {
                Ok(p) => p,
                Err(_) => return error_response(400, "Invalid Organization UUID"),
            };
            let mut org: ElrOrganization = match serde_json::from_slice(body) {
                Ok(o) => o,
                Err(e) => return error_response(400, &format!("Invalid Organization JSON: {}", e)),
            };
            org.id = id;
            if org.timestamp_ms == 0 {
                org.timestamp_ms = s3a_engine::lrs_store::now_ms();
            }
            match state.store.put_organization(org.clone()) {
                Ok(_) => {
                    log_learner_audit(
                        &state.audit_writer,
                        "UPDATE",
                        &format!("PUT /api/organization/{}", id_str),
                        "Organization",
                        id_str,
                        jwt_id,
                        200,
                    );
                    json_response(200, &org)
                }
                Err(e) => error_response(500, &format!("Storage failure: {}", e)),
            }
        }
        (&Method::Delete, Some(id_str)) => {
            let id = match uuid_to_u64_pair(id_str) {
                Ok(p) => p,
                Err(_) => return error_response(400, "Invalid Organization UUID"),
            };
            match state.store.delete_organization(id) {
                Ok(_) => {
                    log_learner_audit(
                        &state.audit_writer,
                        "DELETE",
                        &format!("DELETE /api/organization/{}", id_str),
                        "Organization",
                        id_str,
                        jwt_id,
                        204,
                    );
                    empty_response(204)
                }
                Err(e) => error_response(500, &format!("Deletion failure: {}", e)),
            }
        }
        _ => error_response(405, "Method Not Allowed"),
    }
}

fn handle_resource_entity(
    method: &Method,
    id_opt: Option<&str>,
    body: &[u8],
    state: &Arc<AppState>,
    jwt_id: &str,
) -> Response<Cursor<Vec<u8>>> {
    match (method, id_opt) {
        (&Method::Get, None) => {
            let list = state.store.list_learning_resources();
            log_learner_audit(
                &state.audit_writer,
                "READ",
                "GET /api/learning-resource",
                "LearningResource",
                "",
                jwt_id,
                200,
            );
            json_response(200, &list)
        }
        (&Method::Get, Some(id_str)) => {
            let id = match uuid_to_u64_pair(id_str) {
                Ok(p) => p,
                Err(_) => return error_response(400, "Invalid LearningResource UUID"),
            };
            match state.store.get_learning_resource(id) {
                Some(r) => {
                    log_learner_audit(
                        &state.audit_writer,
                        "READ",
                        &format!("GET /api/learning-resource/{}", id_str),
                        "LearningResource",
                        id_str,
                        jwt_id,
                        200,
                    );
                    json_response(200, &r)
                }
                None => error_response(404, "LearningResource Not Found"),
            }
        }
        (&Method::Post, _) => {
            let mut res: ElrLearningResource = match serde_json::from_slice(body) {
                Ok(r) => r,
                Err(e) => {
                    return error_response(400, &format!("Invalid LearningResource JSON: {}", e))
                }
            };
            if res.id == [0, 0] {
                res.id = deterministic_uuid("resource", &res.iri);
            }
            if res.timestamp_ms == 0 {
                res.timestamp_ms = s3a_engine::lrs_store::now_ms();
            }
            let entity_id_str = u64_pair_to_uuid(res.id);
            match state.store.put_learning_resource(res.clone()) {
                Ok(_) => {
                    log_learner_audit(
                        &state.audit_writer,
                        "CREATE",
                        "POST /api/learning-resource",
                        "LearningResource",
                        &entity_id_str,
                        jwt_id,
                        201,
                    );
                    json_response(201, &res)
                }
                Err(e) => error_response(500, &format!("Storage failure: {}", e)),
            }
        }
        (&Method::Put, Some(id_str)) => {
            let id = match uuid_to_u64_pair(id_str) {
                Ok(p) => p,
                Err(_) => return error_response(400, "Invalid LearningResource UUID"),
            };
            let mut res: ElrLearningResource = match serde_json::from_slice(body) {
                Ok(r) => r,
                Err(e) => {
                    return error_response(400, &format!("Invalid LearningResource JSON: {}", e))
                }
            };
            res.id = id;
            if res.timestamp_ms == 0 {
                res.timestamp_ms = s3a_engine::lrs_store::now_ms();
            }
            match state.store.put_learning_resource(res.clone()) {
                Ok(_) => {
                    log_learner_audit(
                        &state.audit_writer,
                        "UPDATE",
                        &format!("PUT /api/learning-resource/{}", id_str),
                        "LearningResource",
                        id_str,
                        jwt_id,
                        200,
                    );
                    json_response(200, &res)
                }
                Err(e) => error_response(500, &format!("Storage failure: {}", e)),
            }
        }
        (&Method::Delete, Some(id_str)) => {
            let id = match uuid_to_u64_pair(id_str) {
                Ok(p) => p,
                Err(_) => return error_response(400, "Invalid LearningResource UUID"),
            };
            match state.store.delete_learning_resource(id) {
                Ok(_) => {
                    log_learner_audit(
                        &state.audit_writer,
                        "DELETE",
                        &format!("DELETE /api/learning-resource/{}", id_str),
                        "LearningResource",
                        id_str,
                        jwt_id,
                        204,
                    );
                    empty_response(204)
                }
                Err(e) => error_response(500, &format!("Deletion failure: {}", e)),
            }
        }
        _ => error_response(405, "Method Not Allowed"),
    }
}

fn handle_event_entity(
    method: &Method,
    id_opt: Option<&str>,
    params: &HashMap<String, String>,
    include_hidden: bool,
    body: &[u8],
    state: &Arc<AppState>,
    jwt_id: &str,
) -> Response<Cursor<Vec<u8>>> {
    match (method, id_opt) {
        (&Method::Get, None) => {
            let person_filter = params
                .get("person_id")
                .and_then(|s| uuid_to_u64_pair(s).ok());
            let list = state
                .store
                .list_learning_events(person_filter, include_hidden);
            log_learner_audit(
                &state.audit_writer,
                "READ",
                "GET /api/learning-event",
                "LearningEvent",
                "",
                jwt_id,
                200,
            );
            json_response(200, &list)
        }
        (&Method::Get, Some(id_str)) => {
            let id = match uuid_to_u64_pair(id_str) {
                Ok(p) => p,
                Err(_) => return error_response(400, "Invalid LearningEvent UUID"),
            };
            match state.store.get_learning_event(id, include_hidden) {
                Some(e) => {
                    log_learner_audit(
                        &state.audit_writer,
                        "READ",
                        &format!("GET /api/learning-event/{}", id_str),
                        "LearningEvent",
                        id_str,
                        jwt_id,
                        200,
                    );
                    json_response(200, &e)
                }
                None => error_response(404, "LearningEvent Not Found"),
            }
        }
        (&Method::Post, _) => {
            let mut ev: ElrLearningEvent = match serde_json::from_slice(body) {
                Ok(e) => e,
                Err(e) => {
                    return error_response(400, &format!("Invalid LearningEvent JSON: {}", e))
                }
            };
            if ev.id == [0, 0] {
                let ev_key = format!(
                    "{}:{}:{}",
                    u64_pair_to_uuid(ev.person_id),
                    u64_pair_to_uuid(ev.resource_id),
                    u64_pair_to_uuid(ev.source_statement_id)
                );
                ev.id = deterministic_uuid("event", &ev_key);
            }
            if ev.timestamp_ms == 0 {
                ev.timestamp_ms = s3a_engine::lrs_store::now_ms();
            }
            let entity_id_str = u64_pair_to_uuid(ev.id);
            match state.store.put_learning_event(ev.clone()) {
                Ok(_) => {
                    log_learner_audit(
                        &state.audit_writer,
                        "CREATE",
                        "POST /api/learning-event",
                        "LearningEvent",
                        &entity_id_str,
                        jwt_id,
                        201,
                    );
                    json_response(201, &ev)
                }
                Err(e) => error_response(500, &format!("Storage failure: {}", e)),
            }
        }
        (&Method::Put, Some(id_str)) => {
            let id = match uuid_to_u64_pair(id_str) {
                Ok(p) => p,
                Err(_) => return error_response(400, "Invalid LearningEvent UUID"),
            };
            let mut ev: ElrLearningEvent = match serde_json::from_slice(body) {
                Ok(e) => e,
                Err(e) => {
                    return error_response(400, &format!("Invalid LearningEvent JSON: {}", e))
                }
            };
            ev.id = id;
            if ev.timestamp_ms == 0 {
                ev.timestamp_ms = s3a_engine::lrs_store::now_ms();
            }
            match state.store.put_learning_event(ev.clone()) {
                Ok(_) => {
                    log_learner_audit(
                        &state.audit_writer,
                        "UPDATE",
                        &format!("PUT /api/learning-event/{}", id_str),
                        "LearningEvent",
                        id_str,
                        jwt_id,
                        200,
                    );
                    json_response(200, &ev)
                }
                Err(e) => error_response(500, &format!("Storage failure: {}", e)),
            }
        }
        (&Method::Delete, Some(id_str)) => {
            let id = match uuid_to_u64_pair(id_str) {
                Ok(p) => p,
                Err(_) => return error_response(400, "Invalid LearningEvent UUID"),
            };
            match state.store.delete_learning_event(id) {
                Ok(_) => {
                    log_learner_audit(
                        &state.audit_writer,
                        "DELETE",
                        &format!("DELETE /api/learning-event/{}", id_str),
                        "LearningEvent",
                        id_str,
                        jwt_id,
                        204,
                    );
                    empty_response(204)
                }
                Err(e) => error_response(500, &format!("Deletion failure: {}", e)),
            }
        }
        _ => error_response(405, "Method Not Allowed"),
    }
}

fn handle_competency_entity(
    method: &Method,
    id_opt: Option<&str>,
    params: &HashMap<String, String>,
    include_hidden: bool,
    body: &[u8],
    state: &Arc<AppState>,
    jwt_id: &str,
) -> Response<Cursor<Vec<u8>>> {
    match (method, id_opt) {
        (&Method::Get, None) => {
            let person_filter = params
                .get("person_id")
                .and_then(|s| uuid_to_u64_pair(s).ok());
            let list = state.store.list_competencies(person_filter, include_hidden);
            log_learner_audit(
                &state.audit_writer,
                "READ",
                "GET /api/competency",
                "CompetencyAssertion",
                "",
                jwt_id,
                200,
            );
            json_response(200, &list)
        }
        (&Method::Get, Some(id_str)) => {
            let id = match uuid_to_u64_pair(id_str) {
                Ok(p) => p,
                Err(_) => return error_response(400, "Invalid Competency UUID"),
            };
            match state.store.get_competency(id, include_hidden) {
                Some(c) => {
                    log_learner_audit(
                        &state.audit_writer,
                        "READ",
                        &format!("GET /api/competency/{}", id_str),
                        "CompetencyAssertion",
                        id_str,
                        jwt_id,
                        200,
                    );
                    json_response(200, &c)
                }
                None => error_response(404, "Competency Not Found"),
            }
        }
        (&Method::Post, _) => {
            let mut comp: ElrCompetencyAssertion = match serde_json::from_slice(body) {
                Ok(c) => c,
                Err(e) => return error_response(400, &format!("Invalid Competency JSON: {}", e)),
            };
            if comp.id == [0, 0] {
                let comp_key = format!(
                    "{}:{}",
                    u64_pair_to_uuid(comp.person_id),
                    comp.competency_id
                );
                comp.id = deterministic_uuid("competency", &comp_key);
            }
            if comp.timestamp_ms == 0 {
                comp.timestamp_ms = s3a_engine::lrs_store::now_ms();
            }
            let entity_id_str = u64_pair_to_uuid(comp.id);
            match state.store.put_competency(comp.clone()) {
                Ok(_) => {
                    log_learner_audit(
                        &state.audit_writer,
                        "CREATE",
                        "POST /api/competency",
                        "CompetencyAssertion",
                        &entity_id_str,
                        jwt_id,
                        201,
                    );
                    json_response(201, &comp)
                }
                Err(e) => error_response(500, &format!("Storage failure: {}", e)),
            }
        }
        (&Method::Put, Some(id_str)) => {
            let id = match uuid_to_u64_pair(id_str) {
                Ok(p) => p,
                Err(_) => return error_response(400, "Invalid Competency UUID"),
            };
            let mut comp: ElrCompetencyAssertion = match serde_json::from_slice(body) {
                Ok(c) => c,
                Err(e) => return error_response(400, &format!("Invalid Competency JSON: {}", e)),
            };
            comp.id = id;
            if comp.timestamp_ms == 0 {
                comp.timestamp_ms = s3a_engine::lrs_store::now_ms();
            }
            match state.store.put_competency(comp.clone()) {
                Ok(_) => {
                    log_learner_audit(
                        &state.audit_writer,
                        "UPDATE",
                        &format!("PUT /api/competency/{}", id_str),
                        "CompetencyAssertion",
                        id_str,
                        jwt_id,
                        200,
                    );
                    json_response(200, &comp)
                }
                Err(e) => error_response(500, &format!("Storage failure: {}", e)),
            }
        }
        (&Method::Delete, Some(id_str)) => {
            let id = match uuid_to_u64_pair(id_str) {
                Ok(p) => p,
                Err(_) => return error_response(400, "Invalid Competency UUID"),
            };
            match state.store.delete_competency(id) {
                Ok(_) => {
                    log_learner_audit(
                        &state.audit_writer,
                        "DELETE",
                        &format!("DELETE /api/competency/{}", id_str),
                        "CompetencyAssertion",
                        id_str,
                        jwt_id,
                        204,
                    );
                    empty_response(204)
                }
                Err(e) => error_response(500, &format!("Deletion failure: {}", e)),
            }
        }
        _ => error_response(405, "Method Not Allowed"),
    }
}

fn handle_credential_entity(
    method: &Method,
    id_opt: Option<&str>,
    params: &HashMap<String, String>,
    include_hidden: bool,
    body: &[u8],
    state: &Arc<AppState>,
    jwt_id: &str,
) -> Response<Cursor<Vec<u8>>> {
    match (method, id_opt) {
        (&Method::Get, None) => {
            let person_filter = params
                .get("person_id")
                .and_then(|s| uuid_to_u64_pair(s).ok());
            let list = state.store.list_credentials(person_filter, include_hidden);
            log_learner_audit(
                &state.audit_writer,
                "READ",
                "GET /api/credential",
                "CredentialAssertion",
                "",
                jwt_id,
                200,
            );
            json_response(200, &list)
        }
        (&Method::Get, Some(id_str)) => {
            let id = match uuid_to_u64_pair(id_str) {
                Ok(p) => p,
                Err(_) => return error_response(400, "Invalid Credential UUID"),
            };
            match state.store.get_credential(id, include_hidden) {
                Some(c) => {
                    log_learner_audit(
                        &state.audit_writer,
                        "READ",
                        &format!("GET /api/credential/{}", id_str),
                        "CredentialAssertion",
                        id_str,
                        jwt_id,
                        200,
                    );
                    json_response(200, &c)
                }
                None => error_response(404, "Credential Not Found"),
            }
        }
        (&Method::Post, _) => {
            let mut cred: ElrCredentialAssertion = match serde_json::from_slice(body) {
                Ok(c) => c,
                Err(e) => return error_response(400, &format!("Invalid Credential JSON: {}", e)),
            };
            if cred.id == [0, 0] {
                let cred_key = format!(
                    "{}:{}",
                    u64_pair_to_uuid(cred.person_id),
                    cred.credential_id
                );
                cred.id = deterministic_uuid("credential", &cred_key);
            }
            if cred.timestamp_ms == 0 {
                cred.timestamp_ms = s3a_engine::lrs_store::now_ms();
            }
            let entity_id_str = u64_pair_to_uuid(cred.id);
            match state.store.put_credential(cred.clone()) {
                Ok(_) => {
                    log_learner_audit(
                        &state.audit_writer,
                        "CREATE",
                        "POST /api/credential",
                        "CredentialAssertion",
                        &entity_id_str,
                        jwt_id,
                        201,
                    );
                    json_response(201, &cred)
                }
                Err(e) => error_response(500, &format!("Storage failure: {}", e)),
            }
        }
        (&Method::Put, Some(id_str)) => {
            let id = match uuid_to_u64_pair(id_str) {
                Ok(p) => p,
                Err(_) => return error_response(400, "Invalid Credential UUID"),
            };
            let mut cred: ElrCredentialAssertion = match serde_json::from_slice(body) {
                Ok(c) => c,
                Err(e) => return error_response(400, &format!("Invalid Credential JSON: {}", e)),
            };
            cred.id = id;
            if cred.timestamp_ms == 0 {
                cred.timestamp_ms = s3a_engine::lrs_store::now_ms();
            }
            match state.store.put_credential(cred.clone()) {
                Ok(_) => {
                    log_learner_audit(
                        &state.audit_writer,
                        "UPDATE",
                        &format!("PUT /api/credential/{}", id_str),
                        "CredentialAssertion",
                        id_str,
                        jwt_id,
                        200,
                    );
                    json_response(200, &cred)
                }
                Err(e) => error_response(500, &format!("Storage failure: {}", e)),
            }
        }
        (&Method::Delete, Some(id_str)) => {
            let id = match uuid_to_u64_pair(id_str) {
                Ok(p) => p,
                Err(_) => return error_response(400, "Invalid Credential UUID"),
            };
            match state.store.delete_credential(id) {
                Ok(_) => {
                    log_learner_audit(
                        &state.audit_writer,
                        "DELETE",
                        &format!("DELETE /api/credential/{}", id_str),
                        "CredentialAssertion",
                        id_str,
                        jwt_id,
                        204,
                    );
                    empty_response(204)
                }
                Err(e) => error_response(500, &format!("Deletion failure: {}", e)),
            }
        }
        _ => error_response(405, "Method Not Allowed"),
    }
}

fn handle_goal_entity(
    method: &Method,
    id_opt: Option<&str>,
    params: &HashMap<String, String>,
    include_hidden: bool,
    body: &[u8],
    state: &Arc<AppState>,
    jwt_id: &str,
) -> Response<Cursor<Vec<u8>>> {
    match (method, id_opt) {
        (&Method::Get, None) => {
            let person_filter = params
                .get("person_id")
                .and_then(|s| uuid_to_u64_pair(s).ok());
            let list = state.store.list_goals(person_filter, include_hidden);
            log_learner_audit(
                &state.audit_writer,
                "READ",
                "GET /api/goal",
                "Goal",
                "",
                jwt_id,
                200,
            );
            json_response(200, &list)
        }
        (&Method::Get, Some(id_str)) => {
            let id = match uuid_to_u64_pair(id_str) {
                Ok(p) => p,
                Err(_) => return error_response(400, "Invalid Goal UUID"),
            };
            match state.store.get_goal(id, include_hidden) {
                Some(g) => {
                    log_learner_audit(
                        &state.audit_writer,
                        "READ",
                        &format!("GET /api/goal/{}", id_str),
                        "Goal",
                        id_str,
                        jwt_id,
                        200,
                    );
                    json_response(200, &g)
                }
                None => error_response(404, "Goal Not Found"),
            }
        }
        (&Method::Post, _) => {
            let mut goal: ElrGoal = match serde_json::from_slice(body) {
                Ok(g) => g,
                Err(e) => return error_response(400, &format!("Invalid Goal JSON: {}", e)),
            };
            if goal.id == [0, 0] {
                let goal_key = format!("{}:{}", u64_pair_to_uuid(goal.person_id), goal.name);
                goal.id = deterministic_uuid("goal", &goal_key);
            }
            if goal.timestamp_ms == 0 {
                goal.timestamp_ms = s3a_engine::lrs_store::now_ms();
            }
            let entity_id_str = u64_pair_to_uuid(goal.id);
            match state.store.put_goal(goal.clone()) {
                Ok(_) => {
                    log_learner_audit(
                        &state.audit_writer,
                        "CREATE",
                        "POST /api/goal",
                        "Goal",
                        &entity_id_str,
                        jwt_id,
                        201,
                    );
                    json_response(201, &goal)
                }
                Err(e) => error_response(500, &format!("Storage failure: {}", e)),
            }
        }
        (&Method::Put, Some(id_str)) => {
            let id = match uuid_to_u64_pair(id_str) {
                Ok(p) => p,
                Err(_) => return error_response(400, "Invalid Goal UUID"),
            };
            let mut goal: ElrGoal = match serde_json::from_slice(body) {
                Ok(g) => g,
                Err(e) => return error_response(400, &format!("Invalid Goal JSON: {}", e)),
            };
            goal.id = id;
            if goal.timestamp_ms == 0 {
                goal.timestamp_ms = s3a_engine::lrs_store::now_ms();
            }
            match state.store.put_goal(goal.clone()) {
                Ok(_) => {
                    log_learner_audit(
                        &state.audit_writer,
                        "UPDATE",
                        &format!("PUT /api/goal/{}", id_str),
                        "Goal",
                        id_str,
                        jwt_id,
                        200,
                    );
                    json_response(200, &goal)
                }
                Err(e) => error_response(500, &format!("Storage failure: {}", e)),
            }
        }
        (&Method::Delete, Some(id_str)) => {
            let id = match uuid_to_u64_pair(id_str) {
                Ok(p) => p,
                Err(_) => return error_response(400, "Invalid Goal UUID"),
            };
            match state.store.delete_goal(id) {
                Ok(_) => {
                    log_learner_audit(
                        &state.audit_writer,
                        "DELETE",
                        &format!("DELETE /api/goal/{}", id_str),
                        "Goal",
                        id_str,
                        jwt_id,
                        204,
                    );
                    empty_response(204)
                }
                Err(e) => error_response(500, &format!("Deletion failure: {}", e)),
            }
        }
        _ => error_response(405, "Method Not Allowed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jwt_generation_and_verification() {
        let secret = "test-secret-key-32-bytes-long!!";
        let sub = "learner-client-42";
        let scopes = vec!["read".to_string(), "write".to_string()];

        let token = generate_jwt(secret, sub, &scopes, 3600);
        let claims = verify_jwt(secret, &token).expect("JWT verification should succeed");
        assert_eq!(claims.sub, sub);
        assert_eq!(claims.scopes, scopes);
        assert!(claims.has_scope("read"));
        assert!(claims.has_scope("write"));
        assert!(!claims.has_scope("admin"));

        // Tampered token must fail verification
        let mut tampered = token.clone();
        tampered.push('a');
        assert!(verify_jwt(secret, &tampered).is_err());

        // Wrong secret must fail
        assert!(verify_jwt("wrong-secret-key-wrong-wrong-wrong", &token).is_err());
    }

    #[test]
    fn test_admin_scope_authorizes_all() {
        let secret = "test-secret-key-32-bytes-long!!";
        let token = generate_jwt(secret, "admin-user", &["admin".to_string()], 3600);
        let claims = verify_jwt(secret, &token).unwrap();
        assert!(claims.has_scope("read"));
        assert!(claims.has_scope("write"));
        assert!(claims.has_scope("admin"));
        assert!(claims.has_scope("anything-else"));
    }

    #[test]
    fn test_learner_api_endpoints_crud_and_auth() {
        let temp_dir = std::env::temp_dir();
        let rand_id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let data_path = temp_dir.join(format!("test_learner_api_{}.s3a", rand_id));
        let audit_path = temp_dir.join(format!("test_learner_api_{}.audit.s3a", rand_id));

        let store = s3a_engine::LrsStore::open_or_create(&data_path).unwrap();
        let audit_writer = Arc::new(ConcurrentTileWriter::create(&audit_path).unwrap());

        let mut config = crate::ServerConfig::default();
        config.archive_path = data_path.clone();
        config.audit_archive_path = audit_path.clone();
        config.jwt_secret = "test-secret-key-for-learner-api!!".to_string();

        let stores = Arc::new(std::sync::RwLock::new(HashMap::new()));
        let default_key = data_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("test.s3a")
            .to_string();
        stores.write().unwrap().insert(default_key, store.clone());

        let state = Arc::new(AppState {
            default_store: store.clone(),
            store,
            stores,
            audit_writer: audit_writer.clone(),
            config: config.clone(),
            agent_registry: Vec::new(),
            gate_directory: Arc::new(std::sync::Mutex::new(
                crate::gate_directory::GateDirectoryService::new(),
            )),
            batch_sync: Arc::new(std::sync::Mutex::new(
                crate::batch_sync::BatchSyncService::new(
                    crate::batch_sync::BatchSyncLimits::default(),
                ),
            )),
        });

        // 1. Unauthenticated request without admin override fails with 403 (or 401)
        let unauth_resp = handle_learner_api(
            &Method::Get,
            "/api/person",
            &HashMap::new(),
            &HashMap::new(),
            &[],
            &state,
        );
        assert_eq!(unauth_resp.status_code().0, 403);

        // 2. Issue an Admin Token via /admin/token (using insecure-dev or dev mode)
        let mut dev_state = (*state).clone();
        dev_state.config.insecure_dev = true;
        let dev_state = Arc::new(dev_state);

        let token_req_body = serde_json::to_vec(&serde_json::json!({
            "client_id": "test-admin-client",
            "scopes": ["admin", "read", "write"]
        }))
        .unwrap();

        let token_resp = handle_learner_api(
            &Method::Post,
            "/admin/token",
            &HashMap::new(),
            &HashMap::new(),
            &token_req_body,
            &dev_state,
        );
        assert_eq!(token_resp.status_code().0, 200);

        // Parse token from response
        let admin_token = generate_jwt(&config.jwt_secret, "test-admin", &["admin".into()], 3600);
        let read_token = generate_jwt(&config.jwt_secret, "test-reader", &["read".into()], 3600);
        let write_token = generate_jwt(&config.jwt_secret, "test-writer", &["write".into()], 3600);

        let mut admin_headers = HashMap::new();
        admin_headers.insert(
            "authorization".to_string(),
            format!("Bearer {}", admin_token),
        );

        let mut read_headers = HashMap::new();
        read_headers.insert(
            "authorization".to_string(),
            format!("Bearer {}", read_token),
        );

        let mut write_headers = HashMap::new();
        write_headers.insert(
            "authorization".to_string(),
            format!("Bearer {}", write_token),
        );

        // 3. Reader cannot write (403)
        let person_body = serde_json::to_vec(&serde_json::json!({
            "identifier": "mailto:dr.watson@bakerst.org",
            "name": "Dr. John Watson",
            "flags": 0
        }))
        .unwrap();

        let forbidden_write = handle_learner_api(
            &Method::Post,
            "/api/person",
            &HashMap::new(),
            &read_headers,
            &person_body,
            &state,
        );
        assert_eq!(forbidden_write.status_code().0, 403);

        // 4. Writer creates person (201)
        let create_resp = handle_learner_api(
            &Method::Post,
            "/api/person",
            &HashMap::new(),
            &write_headers,
            &person_body,
            &state,
        );
        assert_eq!(create_resp.status_code().0, 201);

        let person_id = deterministic_uuid("person", "mailto:dr.watson@bakerst.org");
        let person_uuid = u64_pair_to_uuid(person_id);

        // 5. Writer cannot read (403)
        let forbidden_read = handle_learner_api(
            &Method::Get,
            &format!("/api/person/{}", person_uuid),
            &HashMap::new(),
            &write_headers,
            &[],
            &state,
        );
        assert_eq!(forbidden_read.status_code().0, 403);

        // 6. Reader reads person (200)
        let ok_read = handle_learner_api(
            &Method::Get,
            &format!("/api/person/{}", person_uuid),
            &HashMap::new(),
            &read_headers,
            &[],
            &state,
        );
        assert_eq!(ok_read.status_code().0, 200);

        // 7. Admin updates person (200)
        let update_body = serde_json::to_vec(&serde_json::json!({
            "identifier": "mailto:dr.watson@bakerst.org",
            "name": "Dr. John H. Watson, MD",
            "flags": 0
        }))
        .unwrap();
        let update_resp = handle_learner_api(
            &Method::Put,
            &format!("/api/person/{}", person_uuid),
            &HashMap::new(),
            &admin_headers,
            &update_body,
            &state,
        );
        assert_eq!(update_resp.status_code().0, 200);

        // 8. Admin override mode bypasses bearer requirement
        let mut override_state = (*state).clone();
        override_state.config.admin_api_override = true;
        let override_state = Arc::new(override_state);

        let override_read = handle_learner_api(
            &Method::Get,
            &format!("/api/person/{}", person_uuid),
            &HashMap::new(),
            &HashMap::new(), // No Authorization header!
            &[],
            &override_state,
        );
        assert_eq!(override_read.status_code().0, 200);

        // 9. Admin deletes person (204)
        let delete_resp = handle_learner_api(
            &Method::Delete,
            &format!("/api/person/{}", person_uuid),
            &HashMap::new(),
            &admin_headers,
            &[],
            &state,
        );
        assert_eq!(delete_resp.status_code().0, 204);

        // 10. Check that deleted person returns 404
        let not_found_resp = handle_learner_api(
            &Method::Get,
            &format!("/api/person/{}", person_uuid),
            &HashMap::new(),
            &admin_headers,
            &[],
            &state,
        );
        assert_eq!(not_found_resp.status_code().0, 404);

        // Clean up temp files
        let _ = std::fs::remove_file(&data_path);
        let _ = std::fs::remove_file(&audit_path);
    }
}
