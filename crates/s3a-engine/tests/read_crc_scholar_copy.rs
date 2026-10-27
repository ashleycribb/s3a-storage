//! Opt-in check against a real archive copy (never committed):
//! `S3A_SCHOLAR_COPY=/path/copy.s3a cargo test -p s3a-engine --release --test read_crc_scholar_copy -- --ignored --nocapture`
use s3a_engine::*;
use std::time::Instant;

#[test]
#[ignore]
fn scholar_copy_reads_every_tile() {
    let Ok(path) = std::env::var("S3A_SCHOLAR_COPY") else {
        eprintln!("S3A_SCHOLAR_COPY not set; skipping");
        return;
    };
    let r = MmapReader::open(&path).expect("open");
    r.verify_checksums().expect("verify_checksums");
    let n = r.tile_count() as usize;
    for i in 0..n {
        r.try_get_tile(i)
            .unwrap_or_else(|e| panic!("try_get_tile {i}: {e}"));
        assert!(r.get_tile(i).is_some(), "get_tile {i}");
        assert!(r.get_promoted_tile(i).is_some(), "get_promoted_tile {i}");
    }
    let iters = 20_000;
    let t = Instant::now();
    for _ in 0..iters {
        for i in 0..n {
            std::hint::black_box(r.get_tile(i));
        }
    }
    let ns = t.elapsed().as_nanos() as f64 / (iters * n) as f64;
    eprintln!("tiles={n} get_tile avg={ns:.1} ns/read");
}
