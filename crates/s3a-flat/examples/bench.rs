//! cargo run --release -p s3a-flat --example bench -- <file.s3a>...
use std::time::Instant;
fn main() {
    println!("| file | original B | flat B | flat+zstd3 B | flat ratio | flat+zstd ratio | flatten MB/s | unflatten MB/s | zstd flatten MB/s | zstd unflatten MB/s | round trip |");
    println!("|---|---|---|---|---|---|---|---|---|---|---|");
    for p in std::env::args().skip(1) {
        let d = std::fs::read(&p).expect("read");
        let mb = d.len() as f64 / 1e6;
        let reps = 5;
        let time = |f: &dyn Fn() -> Vec<u8>| {
            let t = Instant::now();
            let mut o = Vec::new();
            for _ in 0..reps {
                o = f();
            }
            (o, mb / (t.elapsed().as_secs_f64() / reps as f64))
        };
        let (f, fs) = time(&|| s3a_flat::flatten(&d, None).unwrap());
        let (u, us) = time(&|| s3a_flat::unflatten(&f).unwrap());
        let (z, zs) = time(&|| s3a_flat::flatten(&d, Some(3)).unwrap());
        let (uz, uzs) = time(&|| s3a_flat::unflatten(&z).unwrap());
        let ok = u == d && uz == d;
        let name = p.rsplit('/').next().unwrap();
        println!("| {name} | {} | {} | {} | {:.4} | {:.4} | {fs:.0} | {us:.0} | {zs:.0} | {uzs:.0} | {} |",
            d.len(), f.len(), z.len(), f.len() as f64 / d.len().max(1) as f64, z.len() as f64 / d.len().max(1) as f64, if ok { "yes (sha256 + CRC32C)" } else { "NO" });
    }
}
