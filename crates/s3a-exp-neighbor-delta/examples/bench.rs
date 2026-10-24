//! cargo run --release -p s3a-exp-neighbor-delta --example bench [-- extra.s3a ...]
//! Extra files are read only (pass a COPY, never an original store).
use s3a_exp_neighbor_delta::*;
use std::time::Instant;

const PAY: usize = s3a_core::HYPER_TILE_PAYLOAD_SIZE;
const N: usize = 32; // tiles per synthetic dataset (~4 MiB)

fn f32_tiles(f: impl Fn(usize, usize, &mut u64) -> f32) -> Vec<Vec<u8>> {
    let mut s = 0x1234_5678_9abc_def0u64;
    (0..N).map(|t| {
        let mut v = Vec::with_capacity(PAY);
        for i in 0..PAY / 4 { v.extend_from_slice(&f(t, i, &mut s).to_le_bytes()); }
        v
    }).collect()
}
fn rnd(s: &mut u64) -> f32 {
    *s ^= *s << 13; *s ^= *s >> 7; *s ^= *s << 17;
    (*s >> 40) as f32 / (1u64 << 24) as f32 - 0.5
}

fn main() {
    let mut sets: Vec<(String, Elem, Vec<Vec<u8>>)> = vec![
        ("synthetic smooth f32".into(), Elem::F32,
         f32_tiles(|t, i, _| ((t * PAY / 4 + i) as f32 * 1e-4).sin())),
        ("synthetic noisy f32".into(), Elem::F32, f32_tiles(|_, _, s| rnd(s))),
        ("synthetic sensor f32 (slow drift + 1% noise, quantised 1/256)".into(), Elem::F32,
         f32_tiles(|t, i, s| {
             let x = 20.0 + ((t * PAY / 4 + i) as f32 * 1e-5).sin() * 5.0 + rnd(s) * 0.05;
             (x * 256.0).round() / 256.0
         })),
        ("synthetic counter u32".into(), Elem::U32, (0..N).map(|t| {
            (0..PAY / 4).flat_map(|i| (((t * PAY / 4 + i) as u32) * 3).to_le_bytes()).collect()
        }).collect()),
    ];
    let mut files: Vec<String> = vec![
        concat!(env!("CARGO_MANIFEST_DIR"), "/../s3a-engine/tests/fixtures/legacy_v1_0_telemetry.s3a").into(),
        concat!(env!("CARGO_MANIFEST_DIR"), "/../s3a-pack-scholar/testdata/pre_change_tile18_fixture.s3a").into(),
    ];
    files.extend(std::env::args().skip(1));
    for f in files {
        let data = std::fs::read(&f).expect("read");
        let tiles: Vec<Vec<u8>> = hyper_tile_payloads(&data).into_iter().map(|t| t.to_vec()).collect();
        let name = f.rsplit('/').next().unwrap().to_string();
        if tiles.is_empty() { println!("| {name} | (no Hyper-Tiles: {} bytes) |", data.len()); continue; }
        for e in [Elem::U8, Elem::U32, Elem::F32] { sets.push((format!("{name} as {e:?}"), e, tiles.clone())); }
    }
    println!("| dataset | raw bytes | predictor | zstd-3 bytes | ratio vs none | enc MB/s | dec MB/s |");
    println!("|---|---|---|---|---|---|---|");
    for (name, e, tiles) in &sets {
        let refs: Vec<&[u8]> = tiles.iter().map(|t| t.as_slice()).collect();
        let raw: usize = refs.iter().map(|t| t.len()).sum();
        let mut base = 0usize;
        for p in Predictor::ALL {
            let reps = 3;
            let t0 = Instant::now();
            let mut enc = Vec::new();
            for _ in 0..reps { enc = encode(&refs, p, *e, 3).unwrap(); }
            let et = t0.elapsed().as_secs_f64() / reps as f64;
            let t1 = Instant::now();
            for _ in 0..reps { assert_eq!(decode(&enc).unwrap(), *tiles); }
            let dt = t1.elapsed().as_secs_f64() / reps as f64;
            if p == Predictor::None { base = enc.len(); }
            let mb = raw as f64 / 1e6;
            println!("| {name} | {raw} | {p:?} | {} | {:.3} | {:.0} | {:.0} |",
                enc.len(), enc.len() as f64 / base as f64, mb / et, mb / dt);
        }
    }
}
