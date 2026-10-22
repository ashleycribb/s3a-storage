//! Language-neutral conformance suite (`conformance/vectors-1.0/`).
//!
//! Every case is a file plus an `expect` object in `manifest.json`. A conforming
//! implementation reads the file and emits a *reader report* (JSON object); the
//! runner checks that every key in `expect` is present in the report with an equal
//! value. Extra report keys are ignored. See `conformance/README.md`.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use s3a_engine::MmapReader;

/// Newest suite version. Older versions stay frozen and keep passing.
pub const SUITE_VERSION: &str = "vectors-1.5";
pub const SUITE_VERSIONS: [&str; 6] = [
    "vectors-1.0",
    "vectors-1.1",
    "vectors-1.2",
    "vectors-1.3",
    "vectors-1.4",
    "vectors-1.5",
];

/// Reader report for an archive, as defined in `conformance/README.md`.
pub fn read_report(path: &Path) -> Value {
    let reader = match MmapReader::open(path) {
        Ok(r) => r,
        Err(e) => return json!({"outcome": "error", "stage": "open", "error": e.to_string()}),
    };
    let mut tile_types: BTreeMap<String, u64> = BTreeMap::new();
    let mut payload_sha: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut layout_bad = false;
    let registry = s3a_core::registry::global_registry();
    for i in 0..reader.tile_count() as usize {
        if let Some((h, _)) = reader.get_tile(i) {
            *tile_types.entry(h.tile_type.to_string()).or_default() += 1;
            if let Some((_, p)) = reader.get_tile(i) {
                payload_sha
                    .entry(h.tile_type.to_string())
                    .or_default()
                    .push(sha256_hex(p));
            }
            use s3a_core::registry::RegistryError as R;
            if matches!(
                registry.validate_read(h.tile_type, h.record_layout_version),
                Err(R::UnsupportedLayoutVersionTooOld { .. }
                    | R::UnsupportedLayoutVersionTooNew { .. })
            ) {
                layout_bad = true;
            }
        }
    }
    let rr = s3a_engine::record_roots::check(&reader);
    let record_roots = if !rr.ok() {
        "mismatch"
    } else if rr.present {
        "ok"
    } else {
        "absent"
    };
    let ab = s3a_engine::about_report(&reader);
    let about = ab.about.as_ref().map(|a| {
        json!({"spec_version": a.spec_version, "spec_url": a.spec_url,
               "spec_sha256": a.spec_sha256, "writer": a.writer})
    });
    let mut rep = json!({
        "about": about,
        "embedded_spec": ab.embedded_spec,
        "tile_count": reader.tile_count(),
        "generation": reader.generation(),
        "active_slot": reader.active_slot(),
        "tile_types": tile_types,
        "tile_payload_sha256": payload_sha,
        "record_roots": record_roots,
    });
    let unsealed_ok = reader.verify_checksums();
    drop(reader);
    let (outcome, extra) = match s3a_engine::verify_sealed_archive(path, None) {
        Ok(r) if r.is_sealed => {
            let o = if !r.repaired_tiles.is_empty() {
                "repaired"
            } else if r.rebuilt_directory || r.out_of_order_detected || !r.warnings.is_empty() {
                "ok_with_warning"
            } else {
                "ok"
            };
            (
                o,
                json!({"is_sealed": true, "repaired_tiles": r.repaired_tiles.len()}),
            )
        }
        Ok(_) | Err(_) if unsealed_ok.is_ok() => ("ok", json!({"is_sealed": false})),
        Ok(_) => ("error", json!({"is_sealed": false, "stage": "checksum"})),
        Err(e) => ("error", json!({"stage": "verify", "error": e.to_string()})),
    };
    let o = rep.as_object_mut().unwrap();
    o.insert("outcome".into(), outcome.into());
    for (k, v) in extra.as_object().unwrap() {
        o.insert(k.clone(), v.clone());
    }
    if ab.corrupt {
        o.insert("outcome".into(), "error".into());
        o.insert("error_class".into(), "SchemaAboutCorrupt".into());
    }
    // TILE-SPEC §14.5.4: declared algorithm IDs. Absent = SHA-256 seal hash, no declared signature.
    if let Some(a) = &ab.about {
        let seal = if a.seal_hash_alg.is_empty() {
            s3a_core::algorithms::DEFAULT_SEAL_HASH_ALG
        } else {
            a.seal_hash_alg.as_str()
        };
        let sig = if a.sig_alg.is_empty() {
            Value::Null
        } else {
            a.sig_alg.clone().into()
        };
        o.insert(
            "algorithms".into(),
            json!({"seal_hash": seal, "signature": sig}),
        );
        match s3a_engine::check_about_algorithms(a) {
            Ok(st) => {
                o.insert("algorithm_status".into(), st.as_str().into());
            }
            Err((class, msg)) => {
                o.insert("outcome".into(), "error".into());
                o.insert("error_class".into(), class.into());
                o.insert("error".into(), msg.into());
            }
        }
    }
    if (record_roots == "mismatch" || layout_bad) && outcome != "error" {
        o.insert("outcome".into(), "error".into());
    }
    if layout_bad {
        o.insert("error_class".into(), "UnsupportedLayoutVersion".into());
    }
    rep
}

/// RFC 6962 inclusion-proof report: `{"valid": bool}`.
pub fn merkle_report(case: &Value) -> Value {
    let h = |s: &str| hex::decode(s).unwrap_or_default();
    let leaf = h(case["leaf_hex"].as_str().unwrap_or(""));
    let root: [u8; 32] = h(case["root_hex"].as_str().unwrap_or(""))
        .try_into()
        .unwrap_or([0; 32]);
    let proof: Vec<[u8; 32]> = case["proof_hex"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|x| h(x.as_str().unwrap_or("")).try_into().unwrap_or([0; 32]))
                .collect()
        })
        .unwrap_or_default();
    let idx = case["index"].as_u64().unwrap_or(0);
    let size = case["tree_size"].as_u64().unwrap_or(0);
    json!({"valid": s3a_edge::merkle::verify_inclusion(&leaf, idx, size, &proof, &root)})
}

pub fn sha256_hex(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

/// Compare `expect` against `report`; returns mismatching keys.
pub fn diff(expect: &Value, report: &Value) -> Vec<String> {
    let mut bad = Vec::new();
    if let Some(m) = expect.as_object() {
        for (k, v) in m {
            // Dotted keys address nested objects: "tile_types.9999".
            let got = k.split('.').try_fold(report, |cur, part| cur.get(part));
            if got != Some(v) {
                bad.push(format!(
                    "{k}: expected {v}, got {}",
                    got.unwrap_or(&Value::Null)
                ));
            }
        }
    }
    bad
}

/// Reference rewrite for `kind: "rewrite"` cases: compaction (`s3a-cli compact`).
pub fn rewrite(input: &Path, output: &Path) -> Result<(), String> {
    let _ = std::fs::remove_file(output);
    s3a_engine::Compactor::compact(&[input.to_path_buf()], output.to_path_buf())
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Run the suite in `dir` against this crate's reader. Returns (passed, failures).
pub fn run_suite(dir: &Path) -> Result<(usize, Vec<String>), String> {
    let m: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("manifest.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let mut pass = 0;
    let mut fails = Vec::new();
    for c in m["cases"].as_array().ok_or("manifest: no cases")? {
        let id = c["id"].as_str().unwrap_or("?");
        let report = match c["kind"].as_str() {
            Some("merkle-proof") => merkle_report(c),
            kind => {
                let f = dir.join(c["file"].as_str().unwrap_or(""));
                let bytes = std::fs::read(&f).map_err(|e| format!("{id}: {e}"))?;
                if c["sha256"].as_str() != Some(sha256_hex(&bytes).as_str()) {
                    fails.push(format!("{id}: file sha256 does not match manifest"));
                    continue;
                }
                if kind == Some("rewrite") {
                    let out = std::env::temp_dir()
                        .join(format!("s3a_conf_rw_{}_{id}.s3a", std::process::id()));
                    if let Err(e) = rewrite(&f, &out) {
                        fails.push(format!("{id}: rewrite failed: {e}"));
                        continue;
                    }
                    let r = read_report(&out);
                    let _ = std::fs::remove_file(&out);
                    r
                } else {
                    read_report(&f)
                }
            }
        };
        let d = diff(&c["expect"], &report);
        if d.is_empty() {
            pass += 1;
        } else {
            fails.push(format!("{id}: {}", d.join("; ")));
        }
    }
    Ok((pass, fails))
}
