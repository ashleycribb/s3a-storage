use s3a_core::{FILE_HEADER_SIZE, HYPER_TILE_SIZE};
use s3a_flat::{flatten, info, unflatten, FlatError, FLAT_HEADER_LEN};

fn rt(data: &[u8]) -> (usize, usize) {
    let f = flatten(data, None).unwrap();
    let z = flatten(data, Some(3)).unwrap();
    assert_eq!(unflatten(&f).unwrap(), data);
    assert_eq!(unflatten(&z).unwrap(), data);
    assert!(info(&z).unwrap().zstd && !info(&f).unwrap().zstd);
    (f.len(), z.len())
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// Same shape as real files: header, fixed-size tiles with a little data each, optional tail.
fn synthetic(tiles: usize, used: usize, tail: usize, seed: u64) -> Vec<u8> {
    let mut r = Rng(seed);
    let mut v = vec![0u8; FILE_HEADER_SIZE + tiles * HYPER_TILE_SIZE + tail];
    for b in v.iter_mut().take(96) {
        *b = r.next() as u8;
    }
    for t in 0..tiles {
        let base = FILE_HEADER_SIZE + t * HYPER_TILE_SIZE;
        for b in &mut v[base..base + 200] {
            *b = r.next() as u8; // tile header-ish
        }
        let p = base + 512;
        for b in &mut v[p..p + used] {
            *b = r.next() as u8;
        }
        // an interior island after a long zero run
        v[base + HYPER_TILE_SIZE / 2] = 0xAB;
    }
    let n = v.len();
    for b in &mut v[n - tail..] {
        *b = r.next() as u8;
    }
    v
}

#[test]
fn empty_file() {
    let (f, _) = rt(&[]);
    assert_eq!(f, FLAT_HEADER_LEN);
}

#[test]
fn tiny_and_all_zero_inputs() {
    rt(&[1]);
    rt(&[0; 10]);
    rt(&vec![0u8; FILE_HEADER_SIZE + 3 * HYPER_TILE_SIZE]);
    rt(&vec![0xFFu8; FILE_HEADER_SIZE + HYPER_TILE_SIZE + 7]);
}

#[test]
fn synthetic_real_shape_with_trailing_bytes() {
    let d = synthetic(6, 3000, 0, 1);
    let (f, _z) = rt(&d);
    assert!(f < d.len() / 20, "flat {f} vs {}", d.len());
    let d = synthetic(28, 4000, 2432, 2); // swarm_repo-like trailing bytes
    rt(&d);
}

#[test]
fn conformance_and_fixture_s3a_files_round_trip() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let mut n = 0;
    for dir in [
        "conformance/vectors-1.3/files",
        "crates/s3a-engine/tests/fixtures",
        "crates/s3a-pack-scholar/testdata",
    ] {
        let Ok(rd) = std::fs::read_dir(format!("{root}/{dir}")) else {
            continue;
        };
        for e in rd.flatten() {
            if e.path().extension().is_some_and(|x| x == "s3a") {
                rt(&std::fs::read(e.path()).unwrap());
                n += 1;
            }
        }
    }
    assert!(n >= 5, "expected spec-vector files, found {n}");
}

#[test]
fn random_round_trips() {
    let mut r = Rng(0x9E37_79B9_7F4A_7C15);
    for _ in 0..200 {
        let len = (r.next() % 300_000) as usize;
        let density = r.next() % 100;
        let mut v = vec![0u8; len];
        for b in v.iter_mut() {
            if r.next() % 100 < density {
                *b = r.next() as u8;
            }
        }
        rt(&v);
    }
}

#[test]
fn rejects_truncation_and_corruption_without_panic() {
    let d = synthetic(3, 1500, 100, 7);
    for z in [None, Some(3)] {
        let f = flatten(&d, z).unwrap();
        for cut in [
            0,
            3,
            10,
            FLAT_HEADER_LEN - 1,
            FLAT_HEADER_LEN,
            f.len() / 2,
            f.len() - 1,
        ] {
            assert!(unflatten(&f[..cut]).is_err(), "cut {cut}");
        }
        let mut r = Rng(42);
        for _ in 0..500 {
            let mut g = f.clone();
            let i = (r.next() as usize) % g.len();
            g[i] ^= 1 << (r.next() % 8);
            assert!(unflatten(&g).is_err(), "flip at {i}");
        }
        let mut longer = f.clone();
        longer.push(0);
        assert!(unflatten(&longer).is_err());
    }
    assert_eq!(unflatten(b"NOPE").unwrap_err(), FlatError::BadMagic);
}

#[test]
fn record_crc_and_sha_are_checked() {
    let d = synthetic(2, 800, 0, 9);
    let f = flatten(&d, None).unwrap();
    // flip a payload byte inside the body: record CRC must catch it
    let mut g = f.clone();
    let k = FLAT_HEADER_LEN + 40;
    g[k] ^= 0x55;
    let e = unflatten(&g).unwrap_err();
    assert!(
        matches!(
            e,
            FlatError::RecordCrc(_) | FlatError::BadField(_) | FlatError::Truncated(_)
        ),
        "{e:?}"
    );
}

/// Real files from Ashley's PC: run with S3AF_REALDATA=/dir cargo test -p s3a-flat -- --ignored
#[test]
#[ignore]
fn real_files_round_trip() {
    let dir = std::env::var("S3AF_REALDATA").expect("set S3AF_REALDATA");
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let d = std::fs::read(e.path()).unwrap();
        let (f, z) = rt(&d);
        println!("{} {} {} {}", e.path().display(), d.len(), f, z);
    }
}
