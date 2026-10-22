//! s3a-conformance: generate, read, and run the language-neutral conformance suite.
//!
//!   s3a-conformance gen  <suite_dir>   regenerate vectors + manifest.json
//!   s3a-conformance read <file.s3a>    print this implementation's reader report
//!   s3a-conformance merkle             read a merkle-proof case JSON on stdin, print report
//!   s3a-conformance run  <suite_dir>   check this implementation against the manifest

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use s3a_spec_vectors::conformance::{self, read_report, sha256_hex, SUITE_VERSION};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    match (a.get(1).map(|s| s.as_str()), a.get(2)) {
        (Some("gen"), Some(d)) => {
            if let Err(e) = gen(Path::new(d)) {
                eprintln!("gen failed: {e}");
                std::process::exit(1);
            }
        }
        (Some("read"), Some(f)) => println!("{}", read_report(Path::new(f))),
        (Some("merkle"), _) => {
            let mut s = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut s).unwrap();
            let c: Value = serde_json::from_str(&s).expect("case JSON on stdin");
            println!("{}", conformance::merkle_report(&c));
        }
        (Some("run"), Some(d)) => match conformance::run_suite(Path::new(d)) {
            Ok((p, f)) => {
                for x in &f {
                    println!("FAIL {x}");
                }
                println!("{p} passed, {} failed", f.len());
                std::process::exit(if f.is_empty() { 0 } else { 1 });
            }
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(2);
            }
        },
        _ => eprintln!(
            "usage: s3a-conformance gen|run <suite_dir> | read <file> | merkle < case.json"
        ),
    }
}

const LEGACY: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../s3a-engine/tests/fixtures/legacy_v1_0_telemetry.s3a"
);

fn pick(rep: &Value, keys: &[&str]) -> Value {
    let mut m = serde_json::Map::new();
    for k in keys {
        if let Some(v) = rep.get(*k) {
            m.insert(k.to_string(), v.clone());
        }
    }
    Value::Object(m)
}

struct Gen {
    dir: PathBuf,
    cases: Vec<Value>,
}

impl Gen {
    /// Add an archive case. `expect` must agree with our reader or gen fails.
    fn file(
        &mut self,
        id: &str,
        src: &Path,
        level: &str,
        desc: &str,
        expect: Value,
    ) -> Result<(), String> {
        let rel = format!("files/{id}.s3a");
        std::fs::copy(src, self.dir.join(&rel)).map_err(|e| format!("{id}: {e}"))?;
        let bytes = std::fs::read(self.dir.join(&rel)).unwrap();
        let rep = read_report(&self.dir.join(&rel));
        let d = conformance::diff(&expect, &rep);
        if !d.is_empty() {
            return Err(format!(
                "{id}: reader disagrees with expectation: {d:?} (report {rep})"
            ));
        }
        self.cases.push(json!({
            "id": id, "kind": "archive", "level": level, "file": rel,
            "sha256": sha256_hex(&bytes), "size": bytes.len(),
            "description": desc, "expect": expect,
        }));
        Ok(())
    }
}

fn gen(dir: &Path) -> Result<(), String> {
    let files = dir.join("files");
    let _ = std::fs::remove_dir_all(&files);
    std::fs::create_dir_all(&files).map_err(|e| e.to_string())?;
    let tmp = std::env::temp_dir().join(format!("s3a_conf_gen_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let names = s3a_spec_vectors::generate_all_vectors(&tmp).map_err(|e| e.to_string())?;
    let mut g = Gen {
        dir: dir.to_path_buf(),
        cases: Vec::new(),
    };

    // 1. Existing spec-vectors, expectations taken from their expected.json.
    for n in &names {
        let exp: Value = serde_json::from_str(
            &std::fs::read_to_string(tmp.join(format!("{n}.expected.json"))).unwrap(),
        )
        .unwrap();
        let mut outcome = exp["expected_outcome"].as_str().unwrap_or("ok").to_string();
        let mut e = serde_json::Map::new();
        if outcome == "fallback_recovered" {
            // Newest header slot is corrupt; reader must fall back to the older slot.
            outcome = "ok".into();
        }
        if outcome == "ok_with_warning" && exp.get("is_sealed") == Some(&Value::Bool(false)) {
            // Footer gone: the bytes are an ordinary unsealed archive, which is plain "ok"
            // (an unsealed file cannot be told apart from a sealed one that lost its footer).
            outcome = "ok".into();
        }
        let sealed = exp.get("is_sealed") == Some(&Value::Bool(true));
        e.insert("outcome".into(), outcome.clone().into());
        if outcome != "error" {
            if let Some(h) = exp.get("header") {
                for k in ["tile_count", "generation", "active_slot"] {
                    if let Some(v) = h.get(k) {
                        e.insert(k.into(), v.clone());
                    }
                }
            }
            if let Some(v) = exp.get("is_sealed") {
                e.insert("is_sealed".into(), v.clone());
            }
            if sealed {
                if let Some(v) = exp.get("repaired_tiles_count") {
                    e.insert("repaired_tiles".into(), v.clone());
                }
            }
        }
        let desc = exp["description"].as_str().unwrap_or("").to_string();
        g.file(
            n,
            &tmp.join(format!("{n}.s3a")),
            "reader",
            &desc,
            Value::Object(e),
        )?;
    }

    // 2. Legacy v1.0 file and its migration to the current format.
    let lrep = read_report(Path::new(LEGACY));
    g.file("legacy_v1_0_telemetry", Path::new(LEGACY), "reader",
        "Unmodified legacy v1.0 archive (tile header CRC covers bytes 0..36 only). Current readers must reject it with a checksum error and never write it; use `s3a-cli migrate` (see legacy_v1_0_migrated)",
        pick(&lrep, &["outcome", "tile_count", "tile_types"]))?;
    let mig = tmp.join("migrated.s3a");
    s3a_engine::migrate::migrate_file(Path::new(LEGACY), &mig, false).map_err(|e| e.to_string())?;
    let mrep = read_report(&mig);
    if mrep["tile_types"] != lrep["tile_types"] {
        return Err(format!(
            "migration changed tile types: {} vs {}",
            lrep["tile_types"], mrep["tile_types"]
        ));
    }
    g.file(
        "legacy_v1_0_migrated",
        &mig,
        "reader",
        "legacy_v1_0_telemetry after `s3a-cli migrate`; same tile types, current format",
        pick(&mrep, &["outcome", "tile_count", "tile_types"]),
    )?;

    // 3. Record-root tile (83): ok, and a CRC-consistent record tamper (mismatch).
    let rr_in = std::fs::read(LEGACY).unwrap();
    let rr = s3a_engine::seal_buffer_v2(
        &rr_in,
        &s3a_engine::SealOptionsV2::default().with_record_roots(),
    )
    .map_err(|e| e.to_string())?;
    let p = tmp.join("rr_ok.s3a");
    std::fs::write(&p, &rr).unwrap();
    g.file(
        "record_roots_ok",
        &p,
        "reader",
        "Sealed with record-root tile 83 (RFC 6962 roots per tile); all roots match",
        json!({"outcome": "ok", "is_sealed": true, "record_roots": "ok"}),
    )?;
    let mut bad = rr.clone();
    let off = s3a_core::FILE_HEADER_SIZE;
    let hsz = std::mem::size_of::<s3a_core::HyperTileHeader>();
    let mut h: s3a_core::HyperTileHeader = bytemuck::pod_read_unaligned(&bad[off..off + hsz]);
    let pstart = off + s3a_core::HYPER_TILE_HEADER_SIZE;
    bad[pstart + 3] ^= 0xff; // tile 0, record 0
    h.data_crc32 = crc32c::crc32c(&bad[pstart..pstart + h.payload_bytes as usize]);
    h.header_crc32 = s3a_engine::compute_tile_header_crc32(&h);
    bad[off..off + hsz].copy_from_slice(bytemuck::bytes_of(&h));
    let p = tmp.join("rr_bad.s3a");
    std::fs::write(&p, &bad).unwrap();
    g.file("record_roots_mismatch", &p, "reader",
        "record_roots_ok with one record byte changed and tile CRCs recomputed; record-root check must fail and the archive must not verify",
        json!({"outcome": "error", "record_roots": "mismatch"}))?;
    g.file(
        "record_roots_absent",
        &tmp.join("sealed_ok.s3a"),
        "reader",
        "Sealed archive without tile 83",
        json!({"record_roots": "absent"}),
    )?;

    // 4. RFC 6962 inclusion proofs (pure hash vectors; no archive needed).
    use s3a_edge::merkle;
    let rs = 8usize;
    for &(n, k) in &[(1usize, 0usize), (2, 1), (5, 4), (7, 3), (8, 0)] {
        let data: Vec<u8> = (0..n * rs).map(|i| (i * 7 % 251) as u8).collect();
        let root = merkle::merkle_root_fixed(&data, rs).unwrap();
        let mut buf = [[0u8; 32]; merkle::MAX_PROOF_LEN];
        let len = merkle::inclusion_proof_fixed(&data, rs, k, &mut buf).unwrap();
        let leaf = &data[k * rs..(k + 1) * rs];
        let proof: Vec<String> = buf[..len].iter().map(hex::encode).collect();
        let base = json!({"kind": "merkle-proof", "level": "reader", "leaf_hex": hex::encode(leaf),
            "index": k, "tree_size": n, "proof_hex": proof, "root_hex": hex::encode(root)});
        let mut add = |id: String, mut c: Value, valid: bool, desc: &str| {
            c["id"] = id.into();
            c["description"] = desc.into();
            c["expect"] = json!({"valid": valid});
            assert_eq!(
                conformance::merkle_report(&c)["valid"],
                valid,
                "{}",
                c["id"]
            );
            g.cases.push(c);
        };
        add(
            format!("merkle_n{n}_i{k}_valid"),
            base.clone(),
            true,
            "Valid RFC 6962 audit path",
        );
        let mut t = base.clone();
        let mut lb = leaf.to_vec();
        lb[0] ^= 1;
        t["leaf_hex"] = hex::encode(lb).into();
        add(
            format!("merkle_n{n}_i{k}_tampered_leaf"),
            t,
            false,
            "Leaf byte flipped",
        );
        if n > 1 {
            let mut t = base.clone();
            t["index"] = ((k + 1) % n).into();
            add(
                format!("merkle_n{n}_i{k}_wrong_index"),
                t,
                false,
                "Correct path, wrong index",
            );
        }
        let mut t = base.clone();
        t["index"] = n.into();
        add(
            format!("merkle_n{n}_i{k}_index_out_of_range"),
            t,
            false,
            "index >= tree_size",
        );
    }
    // CVE-2012-2459: duplicating the last leaf must not give the same root.
    let data: Vec<u8> = (0..3 * rs).map(|i| i as u8).collect();
    let mut dup = data.clone();
    dup.extend_from_slice(&data[2 * rs..]);
    let r3 = merkle::merkle_root_fixed(&data, rs).unwrap();
    let r4 = merkle::merkle_root_fixed(&dup, rs).unwrap();
    assert_ne!(r3, r4);
    let mut buf = [[0u8; 32]; merkle::MAX_PROOF_LEN];
    let len = merkle::inclusion_proof_fixed(&dup, rs, 3, &mut buf).unwrap();
    let c = json!({"id": "merkle_cve_2012_2459_duplicate_leaf", "kind": "merkle-proof", "level": "reader",
        "description": "Proof for a duplicated 4th leaf checked against the 3-leaf root must fail (Bitcoin CVE-2012-2459)",
        "leaf_hex": hex::encode(&dup[3 * rs..]), "index": 3, "tree_size": 4,
        "proof_hex": buf[..len].iter().map(hex::encode).collect::<Vec<_>>(), "root_hex": hex::encode(r3),
        "expect": {"valid": false}});
    assert_eq!(conformance::merkle_report(&c)["valid"], false);
    g.cases.push(c);

    let manifest = json!({
        "suite": "s3a-hypertile-conformance",
        "version": SUITE_VERSION,
        "spec": "specifications/ (S3A Hyper-Tile v1.1-draft)",
        "levels": {
            "reader": "Must produce a report matching `expect` for every case with level=reader.",
            "writer": "Reader level, plus files it writes must yield outcome=ok under the reference reader (see README)."
        },
        "report_keys": ["outcome", "tile_count", "generation", "active_slot", "is_sealed", "repaired_tiles", "tile_types", "record_roots", "valid"],
        "case_count": g.cases.len(),
        "cases": g.cases,
    });
    std::fs::write(
        dir.join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap() + "\n",
    )
    .map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&tmp);
    println!(
        "wrote {} cases to {}",
        manifest["case_count"],
        dir.display()
    );
    Ok(())
}
