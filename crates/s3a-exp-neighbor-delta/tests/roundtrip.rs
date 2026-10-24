use s3a_exp_neighbor_delta::*;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}
const ELEMS: [Elem; 3] = [Elem::U8, Elem::U32, Elem::F32];

fn rt(tiles: &[Vec<u8>]) {
    let refs: Vec<&[u8]> = tiles.iter().map(|t| t.as_slice()).collect();
    for p in Predictor::ALL {
        for e in ELEMS {
            let enc = encode(&refs, p, e, 3).unwrap();
            let dec = decode(&enc).unwrap();
            assert_eq!(dec, tiles, "{p:?} {e:?}");
        }
    }
}

#[test]
fn property_random_roundtrip() {
    let mut r = Rng(0x9E3779B97F4A7C15);
    for _ in 0..200 {
        let len = (r.next() % 97) as usize; // includes non-multiples of 4
        let count = (r.next() % 6) as usize;
        let tiles: Vec<Vec<u8>> = (0..count)
            .map(|_| (0..len).map(|_| r.next() as u8).collect())
            .collect();
        rt(&tiles);
    }
}

#[test]
fn empty_input() {
    rt(&[]);
    rt(&[vec![]]);
}

#[test]
fn single_tile() {
    rt(&[(0..4096u32).flat_map(|v| v.to_le_bytes()).collect()]);
}

#[test]
fn special_floats_bit_exact() {
    let vals = [
        f32::NAN, -f32::NAN, f32::from_bits(0x7fc0_1234), f32::INFINITY, f32::NEG_INFINITY,
        0.0, -0.0, f32::MIN_POSITIVE, f32::from_bits(1), f32::MAX, f32::MIN,
    ];
    let t: Vec<u8> = vals.iter().flat_map(|v| v.to_bits().to_le_bytes()).collect();
    let mut t2 = t.clone();
    t2.reverse();
    rt(&[t.clone(), t2, t]);
}

#[test]
fn integer_wraparound() {
    let a: Vec<u8> = [0u32, u32::MAX, 1, u32::MAX - 1].iter().flat_map(|v| v.to_le_bytes()).collect();
    let b: Vec<u8> = [u32::MAX, 0, u32::MAX, 0].iter().flat_map(|v| v.to_le_bytes()).collect();
    rt(&[a.clone(), b.clone(), a, b]);
}

#[test]
fn rejects_corrupt_streams() {
    let t = vec![1u8; 64];
    let mut enc = encode(&[&t], Predictor::PrevTile, Elem::U32, 3).unwrap();
    assert_eq!(decode(&enc[..10]), Err(Error::Truncated));
    enc[0] = b'X';
    assert_eq!(decode(&enc), Err(Error::BadMagic));
    let mut enc = encode(&[&t], Predictor::PrevTile, Elem::U32, 3).unwrap();
    enc[5] = 99;
    assert!(decode(&enc).is_err());
    assert!(encode(&[&t, &t[..8]], Predictor::None, Elem::U8, 3).is_err());
}

#[test]
fn real_fixture_roundtrip_read_only() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../s3a-engine/tests/fixtures/legacy_v1_0_telemetry.s3a");
    let file = std::fs::read(path).unwrap();
    let tiles: Vec<Vec<u8>> = hyper_tile_payloads(&file).into_iter().map(|t| t.to_vec()).collect();
    assert!(!tiles.is_empty());
    rt(&tiles);
}
