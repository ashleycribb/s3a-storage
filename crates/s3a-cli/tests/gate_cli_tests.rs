//! Integration test for `s3a-cli gate register|resolve` against an in-process Gate Directory (§11).

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::{Arc, Mutex};
use std::thread;

use ed25519_dalek::{Signature, VerifyingKey};
use rand_core::{OsRng, RngCore};
use tiny_http::{Header, Method, Response, Server, StatusCode};

use s3a_cli::gate_cli::{register_gate, resolve_gate, DirectoryEntry, RegisterGateRequest};
use s3a_vault::address::GateAddress;
use s3a_vault::keystore::{FileKeyStore, KeyStore};

#[test]
fn test_gate_register_and_resolve_against_local_directory() {
    let server = Server::http("127.0.0.1:0").expect("Failed to bind ephemeral server");
    let port = server.server_addr().to_ip().unwrap().port();
    let directory_url = format!("http://127.0.0.1:{}/gate/", port);

    let active_challenge = Arc::new(Mutex::new(String::new()));
    let registry: Arc<Mutex<HashMap<String, DirectoryEntry>>> =
        Arc::new(Mutex::new(HashMap::new()));

    let srv_challenge = active_challenge.clone();
    let srv_registry = registry.clone();

    // Spawn mock in-process gate directory server
    let _server_handle = thread::spawn(move || {
        for mut req in server.incoming_requests() {
            let path = req.url().to_string();
            let method = req.method().clone();

            if path == "/gate/challenge" && (method == Method::Get || method == Method::Post) {
                let mut nonce = [0u8; 32];
                OsRng.fill_bytes(&mut nonce);
                let hex_nonce = hex::encode(nonce);
                *srv_challenge.lock().unwrap() = hex_nonce.clone();

                let json = serde_json::json!({ "challenge": hex_nonce });
                let bytes = serde_json::to_vec(&json).unwrap();
                let _ = req.respond(Response::new(
                    StatusCode(200),
                    vec![
                        Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap(),
                    ],
                    Cursor::new(bytes),
                    None,
                    None,
                ));
            } else if path == "/gate/register" && method == Method::Post {
                let mut body = Vec::new();
                let _ = req.as_reader().read_to_end(&mut body);
                let reg_req: Result<RegisterGateRequest, _> = serde_json::from_slice(&body);

                match reg_req {
                    Ok(r) => {
                        let expected_challenge = srv_challenge.lock().unwrap().clone();
                        if r.challenge != expected_challenge {
                            let _ = req.respond(Response::new(
                                StatusCode(401),
                                vec![],
                                Cursor::new(b"Challenge mismatch".to_vec()),
                                None,
                                None,
                            ));
                            continue;
                        }

                        let mut pk_bytes = [0u8; 32];
                        let mut sig_bytes = [0u8; 64];
                        if hex::decode_to_slice(&r.public_key, &mut pk_bytes).is_err()
                            || hex::decode_to_slice(&r.signature, &mut sig_bytes).is_err()
                        {
                            let _ = req.respond(Response::new(
                                StatusCode(400),
                                vec![],
                                Cursor::new(vec![]),
                                None,
                                None,
                            ));
                            continue;
                        }

                        let expected_addr =
                            GateAddress::from_public_key(&pk_bytes).to_address_string();
                        if r.address != expected_addr {
                            let _ = req.respond(Response::new(
                                StatusCode(400),
                                vec![],
                                Cursor::new(vec![]),
                                None,
                                None,
                            ));
                            continue;
                        }

                        let challenge_bytes = hex::decode(&r.challenge).unwrap();
                        let vk = VerifyingKey::from_bytes(&pk_bytes).unwrap();
                        let sig = Signature::from_bytes(&sig_bytes);

                        if vk.verify_strict(&challenge_bytes, &sig).is_err() {
                            let _ = req.respond(Response::new(
                                StatusCode(401),
                                vec![],
                                Cursor::new(b"Signature verification failed".to_vec()),
                                None,
                                None,
                            ));
                            continue;
                        }

                        let entry = DirectoryEntry {
                            name: r.name.clone(),
                            address: r.address,
                            public_key: r.public_key,
                            endpoints: r.endpoints,
                            sequence: 1,
                            valid_from: 1000,
                            valid_until: 2000,
                        };

                        srv_registry
                            .lock()
                            .unwrap()
                            .insert(r.name.to_lowercase(), entry.clone());
                        let bytes = serde_json::to_vec(&entry).unwrap();
                        let _ = req.respond(Response::new(
                            StatusCode(200),
                            vec![Header::from_bytes(
                                &b"Content-Type"[..],
                                &b"application/json"[..],
                            )
                            .unwrap()],
                            Cursor::new(bytes),
                            None,
                            None,
                        ));
                    }
                    Err(_) => {
                        let _ = req.respond(Response::new(
                            StatusCode(400),
                            vec![],
                            Cursor::new(vec![]),
                            None,
                            None,
                        ));
                    }
                }
            } else if path.starts_with("/gate/resolve/") && method == Method::Get {
                let name = &path["/gate/resolve/".len()..].to_lowercase();
                if let Some(entry) = srv_registry.lock().unwrap().get(name) {
                    let bytes = serde_json::to_vec(entry).unwrap();
                    let _ = req.respond(Response::new(
                        StatusCode(200),
                        vec![
                            Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
                                .unwrap(),
                        ],
                        Cursor::new(bytes),
                        None,
                        None,
                    ));
                } else {
                    let _ = req.respond(Response::new(
                        StatusCode(404),
                        vec![
                            Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
                                .unwrap(),
                        ],
                        Cursor::new(b"{\"error\":\"Gate not found\"}".to_vec()),
                        None,
                        None,
                    ));
                }
            } else {
                let _ = req.respond(Response::new(
                    StatusCode(405),
                    vec![],
                    Cursor::new(vec![]),
                    None,
                    None,
                ));
            }
        }
    });

    // Client verification:
    let temp_dir = std::env::temp_dir().join(format!("s3a_gate_test_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let ks_path = temp_dir.join("test_identity.keystore");
    let passphrase = "test-argon2-secure-passphrase";

    let mut ks =
        FileKeyStore::open_or_create(&ks_path, passphrase).expect("Failed to create FileKeyStore");
    let mut identity_id = [0u8; 16];
    OsRng.fill_bytes(&mut identity_id);
    let pubkey = ks
        .generate_keypair(&identity_id)
        .expect("Keypair generation failed");
    ks.save(passphrase).expect("Keystore save failed");

    let expected_address = GateAddress::from_public_key(&pubkey).to_address_string();

    // 1. Register gate name "test_station"
    let registered = register_gate(
        &directory_url,
        "test_station",
        &ks,
        &identity_id,
        vec!["https://station.local:8080".to_string()],
    )
    .expect("Registration failed");

    assert_eq!(registered.name, "test_station");
    assert_eq!(registered.address, expected_address);
    assert_eq!(registered.public_key, hex::encode(pubkey));
    assert_eq!(registered.endpoints, vec!["https://station.local:8080"]);

    // 2. Resolve gate name "test_station"
    let resolved = resolve_gate(&directory_url, "test_station").expect("Resolve failed");
    assert_eq!(resolved.name, "test_station");
    assert_eq!(resolved.address, expected_address);
    assert_eq!(resolved.public_key, hex::encode(pubkey));
    assert_eq!(resolved.endpoints, vec!["https://station.local:8080"]);

    // 3. Resolve unknown gate name returns error (404)
    let unknown_err = resolve_gate(&directory_url, "non_existent_gate").unwrap_err();
    assert!(unknown_err.contains("not found in directory"));

    let _ = std::fs::remove_dir_all(&temp_dir);
}
