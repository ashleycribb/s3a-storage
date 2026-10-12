//! Gate Directory & Relay Mailbox Service (§11).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use ed25519_dalek::{Signature, VerifyingKey};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};

use s3a_vault::address::GateAddress;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryEntry {
    pub name: String,
    pub address: String,
    pub public_key: String, // hex
    pub endpoints: Vec<String>,
    pub sequence: u32,
    pub valid_from: u64,
    pub valid_until: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterGateRequest {
    pub name: String,
    pub address: String,
    pub public_key: String,
    pub endpoints: Vec<String>,
    pub challenge: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrieveMailboxRequest {
    pub challenge: String,
    pub signature: String,
}

/// In-memory Gate Directory & Store-and-Forward Relay Mailbox service.
#[derive(Default)]
pub struct GateDirectoryService {
    /// Active challenge nonces mapping hex_nonce -> expires_at
    challenges: HashMap<String, u64>,
    /// Maps gate_name -> DirectoryEntry
    names: HashMap<String, DirectoryEntry>,
    /// Maps gate_address -> DirectoryEntry
    addresses: HashMap<String, DirectoryEntry>,
    /// Relay mailbox queues mapping gate_address -> queued tile buffers
    mailboxes: HashMap<String, Vec<Vec<u8>>>,
    /// Simple rate-limiter: client -> (window_sec, request_count)
    rate_limits: HashMap<String, (u64, u32)>,
}

pub type SharedGateDirectory = Arc<Mutex<GateDirectoryService>>;

impl GateDirectoryService {
    pub fn new() -> Self {
        Self::default()
    }

    /// Issue a fresh 32-byte cryptographic challenge nonce valid for 60 seconds.
    pub fn create_challenge(&mut self) -> String {
        let mut nonce = [0u8; 32];
        OsRng.fill_bytes(&mut nonce);
        let hex_nonce = hex::encode(nonce);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        self.challenges.insert(hex_nonce.clone(), now + 60);
        hex_nonce
    }

    /// Verify challenge nonce and prove ownership of private key via Ed25519 signature.
    pub fn verify_challenge(
        &mut self,
        challenge_hex: &str,
        public_key_bytes: &[u8; 32],
        signature_hex: &str,
    ) -> Result<(), &'static str> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // 1. Validate challenge existence and expiry
        let expires_at = self
            .challenges
            .remove(challenge_hex)
            .ok_or("Challenge expired or not found")?;

        if now > expires_at {
            return Err("Challenge expired");
        }

        // 2. Decode challenge nonce
        let challenge_bytes = hex::decode(challenge_hex)
            .map_err(|_| "Invalid challenge hex format")?;

        // 3. Decode signature
        let mut sig_bytes = [0u8; 64];
        hex::decode_to_slice(signature_hex, &mut sig_bytes)
            .map_err(|_| "Invalid signature hex format")?;
        let sig = Signature::from_bytes(&sig_bytes);

        // 4. Verify Ed25519 signature
        let vk = VerifyingKey::from_bytes(public_key_bytes)
            .map_err(|_| "Invalid public key format")?;

        vk.verify_strict(&challenge_bytes, &sig)
            .map_err(|_| "Forged challenge rejected: signature verification failed")?;

        Ok(())
    }

    /// Register or re-point a gate name.
    /// Invariant: Names are first-come, but only the address's key can sign/re-point.
    pub fn register_gate(
        &mut self,
        req: RegisterGateRequest,
        client_id: &str,
    ) -> Result<DirectoryEntry, &'static str> {
        // Rate limiting check: max 60 registrations per minute per client
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let entry = self.rate_limits.entry(client_id.to_string()).or_insert((now, 0));
        if entry.0 == now {
            entry.1 += 1;
            if entry.1 > 60 {
                return Err("Rate limit exceeded");
            }
        } else {
            *entry = (now, 1);
        }

        let mut pk_bytes = [0u8; 32];
        hex::decode_to_slice(&req.public_key, &mut pk_bytes)
            .map_err(|_| "Invalid public key hex")?;

        // Validate address matches derived address
        let expected_addr = GateAddress::from_public_key(&pk_bytes).to_address_string();
        if req.address != expected_addr {
            return Err("Gate address does not match public key");
        }

        // Verify challenge-response signature
        self.verify_challenge(&req.challenge, &pk_bytes, &req.signature)?;

        // Name conflict check
        let clean_name = req.name.trim().to_ascii_lowercase();
        if let Some(existing) = self.names.get(&clean_name) {
            if existing.public_key != req.public_key {
                return Err("Gate name already registered by another authority");
            }
        }

        let seq = self.names.get(&clean_name).map(|e| e.sequence + 1).unwrap_or(1);

        let dir_entry = DirectoryEntry {
            name: clean_name.clone(),
            address: req.address.clone(),
            public_key: req.public_key.clone(),
            endpoints: req.endpoints,
            sequence: seq,
            valid_from: now,
            valid_until: now + 86400 * 30, // 30 days
        };

        self.names.insert(clean_name, dir_entry.clone());
        self.addresses.insert(req.address, dir_entry.clone());

        Ok(dir_entry)
    }

    /// Resolve a gate by name or address.
    pub fn resolve(&self, target: &str) -> Option<DirectoryEntry> {
        let clean = target.trim().to_ascii_lowercase();
        if clean.starts_with("s3a-") {
            self.addresses.get(&clean).cloned()
        } else {
            self.names.get(&clean).cloned()
        }
    }

    /// Deposit sealed tile buffer into an offline vault's relay mailbox (§11.5).
    pub fn deposit_mailbox(&mut self, target_address: &str, tile_bytes: Vec<u8>) -> Result<(), &'static str> {
        let clean_addr = target_address.trim().to_ascii_lowercase();
        let queue = self.mailboxes.entry(clean_addr).or_default();
        if queue.len() >= 1000 {
            return Err("Relay mailbox full");
        }
        queue.push(tile_bytes);
        Ok(())
    }

    /// Retrieve and drain queued sealed tiles for an address, requiring challenge-response authentication.
    pub fn retrieve_mailbox(
        &mut self,
        target_address: &str,
        req: RetrieveMailboxRequest,
    ) -> Result<Vec<Vec<u8>>, &'static str> {
        let clean_addr = target_address.trim().to_ascii_lowercase();
        let entry = self
            .addresses
            .get(&clean_addr)
            .ok_or("Target address not registered in directory")?;

        let mut pk_bytes = [0u8; 32];
        hex::decode_to_slice(&entry.public_key, &mut pk_bytes)
            .map_err(|_| "Invalid stored public key")?;

        // Authenticate request
        self.verify_challenge(&req.challenge, &pk_bytes, &req.signature)?;

        // Drain mailbox queue
        let tiles = self.mailboxes.remove(&clean_addr).unwrap_or_default();
        Ok(tiles)
    }
}

use std::io::Cursor;
use tiny_http::{Header, Method, Response, StatusCode};

pub fn handle_gate_directory(
    method: &Method,
    path: &str,
    body: &[u8],
    service: &Arc<Mutex<GateDirectoryService>>,
) -> Response<Cursor<Vec<u8>>> {
    if path == "/gate/challenge" {
        if method == &Method::Get || method == &Method::Post {
            let challenge = service.lock().unwrap().create_challenge();
            let json = serde_json::json!({ "challenge": challenge });
            let bytes = serde_json::to_vec(&json).unwrap();
            return make_json_response(200, &bytes);
        } else {
            return make_err_response(405, "Method Not Allowed");
        }
    }

    if path == "/gate/register" {
        if method == &Method::Post {
            let req: Result<RegisterGateRequest, _> = serde_json::from_slice(body);
            match req {
                Ok(r) => {
                    match service.lock().unwrap().register_gate(r, "client") {
                        Ok(entry) => {
                            let bytes = serde_json::to_vec(&entry).unwrap_or_default();
                            make_json_response(200, &bytes)
                        }
                        Err(e) => {
                            let status = if e.contains("already registered") { 409 } else { 400 };
                            make_err_response(status, e)
                        }
                    }
                }
                Err(_) => make_err_response(400, "Invalid JSON payload for RegisterGateRequest"),
            }
        } else {
            make_err_response(405, "Method Not Allowed")
        }
    } else if path.starts_with("/gate/resolve/") {
        if method == &Method::Get || method == &Method::Head {
            let target = &path["/gate/resolve/".len()..];
            if let Some(entry) = service.lock().unwrap().resolve(target) {
                let bytes = serde_json::to_vec(&entry).unwrap_or_default();
                make_json_response(200, &bytes)
            } else {
                make_err_response(404, "Gate not found")
            }
        } else {
            make_err_response(405, "Method Not Allowed")
        }
    } else if path.starts_with("/gate/mailbox/") {
        let rest = &path["/gate/mailbox/".len()..];
        if rest.ends_with("/retrieve") {
            let target_addr = &rest[..rest.len() - "/retrieve".len()];
            if method == &Method::Post {
                let req: Result<RetrieveMailboxRequest, _> = serde_json::from_slice(body);
                match req {
                    Ok(r) => {
                        match service.lock().unwrap().retrieve_mailbox(target_addr, r) {
                            Ok(tiles) => {
                                let hex_tiles: Vec<String> = tiles.into_iter().map(hex::encode).collect();
                                let bytes = serde_json::to_vec(&hex_tiles).unwrap_or_default();
                                make_json_response(200, &bytes)
                            }
                            Err(e) => make_err_response(401, e),
                        }
                    }
                    Err(_) => make_err_response(400, "Invalid JSON for RetrieveMailboxRequest"),
                }
            } else {
                make_err_response(405, "Method Not Allowed")
            }
        } else {
            // Deposit mailbox
            let target_addr = rest;
            if method == &Method::Post {
                match service.lock().unwrap().deposit_mailbox(target_addr, body.to_vec()) {
                    Ok(_) => Response::new(
                        StatusCode(202),
                        vec![
                            Header::from_bytes(&b"Content-Type"[..], &b"text/plain"[..]).unwrap(),
                            Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap(),
                        ],
                        Cursor::new(b"Accepted".to_vec()),
                        Some(8),
                        None,
                    ),
                    Err(e) => make_err_response(400, e),
                }
            } else {
                make_err_response(405, "Method Not Allowed")
            }
        }
    } else {
        make_err_response(404, "Endpoint not found")
    }
}

fn make_json_response(status: u16, body: &[u8]) -> Response<Cursor<Vec<u8>>> {
    Response::new(
        StatusCode(status),
        vec![
            Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap(),
            Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap(),
        ],
        Cursor::new(body.to_vec()),
        Some(body.len()),
        None,
    )
}

fn make_err_response(status: u16, msg: &str) -> Response<Cursor<Vec<u8>>> {
    let json = serde_json::json!({ "error": msg }).to_string();
    make_json_response(status, json.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    #[test]
    fn test_gate_directory_challenge_registration_and_relay() {
        let mut dir = GateDirectoryService::new();
        let signing_key = SigningKey::generate(&mut OsRng);
        let pubkey_bytes = signing_key.verifying_key().to_bytes();
        let pubkey_hex = hex::encode(pubkey_bytes);
        let addr = GateAddress::from_public_key(&pubkey_bytes).to_address_string();

        // 1. Request challenge
        let challenge = dir.create_challenge();
        let challenge_bytes = hex::decode(&challenge).unwrap();

        // 2. Sign challenge
        let sig = signing_key.sign(&challenge_bytes);
        let sig_hex = hex::encode(sig.to_bytes());

        // 3. Register gate
        let reg_req = RegisterGateRequest {
            name: "station-alpha".into(),
            address: addr.clone(),
            public_key: pubkey_hex.clone(),
            endpoints: vec!["https://station-alpha.org".into()],
            challenge: challenge.clone(),
            signature: sig_hex.clone(),
        };

        let registered = dir.register_gate(reg_req, "127.0.0.1").expect("Registration must succeed");
        assert_eq!(registered.name, "station-alpha");
        assert_eq!(registered.address, addr);

        // Resolve by name and address
        let resolved_name = dir.resolve("station-alpha").expect("Must resolve by name");
        assert_eq!(resolved_name.address, addr);
        let resolved_addr = dir.resolve(&addr).expect("Must resolve by address");
        assert_eq!(resolved_addr.name, "station-alpha");

        // 4. Forged challenge rejection
        let challenge2 = dir.create_challenge();
        let bad_sig_hex = hex::encode([0xFFu8; 64]);
        let forged_req = RegisterGateRequest {
            name: "station-beta".into(),
            address: addr.clone(),
            public_key: pubkey_hex.clone(),
            endpoints: vec![],
            challenge: challenge2,
            signature: bad_sig_hex,
        };
        assert!(dir.register_gate(forged_req, "127.0.0.1").is_err());

        // 5. Offline relay mailbox deposit and retrieval
        let sample_tile = b"SEALED_TILE_PAYLOAD_12345".to_vec();
        dir.deposit_mailbox(&addr, sample_tile.clone()).unwrap();

        // Retrieve mailbox with challenge-response
        let retrieve_chal = dir.create_challenge();
        let chal_bytes = hex::decode(&retrieve_chal).unwrap();
        let retrieve_sig = signing_key.sign(&chal_bytes);

        let retrieved = dir
            .retrieve_mailbox(
                &addr,
                RetrieveMailboxRequest {
                    challenge: retrieve_chal,
                    signature: hex::encode(retrieve_sig.to_bytes()),
                },
            )
            .expect("Retrieve must succeed with valid auth");

        assert_eq!(retrieved.len(), 1);
        assert_eq!(retrieved[0], sample_tile);

        // Queue is drained
        let retrieve_chal2 = dir.create_challenge();
        let chal2_bytes = hex::decode(&retrieve_chal2).unwrap();
        let retrieve_sig2 = signing_key.sign(&chal2_bytes);
        let empty = dir
            .retrieve_mailbox(
                &addr,
                RetrieveMailboxRequest {
                    challenge: retrieve_chal2,
                    signature: hex::encode(retrieve_sig2.to_bytes()),
                },
            )
            .unwrap();
        assert_eq!(empty.len(), 0);
    }
}
