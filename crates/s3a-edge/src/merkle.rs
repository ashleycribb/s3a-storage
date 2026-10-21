//! RFC 6962-style Merkle commitments over record bytes (no_std, no alloc).
//!
//! * Leaf hash:  `SHA-256(0x00 || record)`
//! * Node hash:  `SHA-256(0x01 || left || right)`
//! * Split point: largest power of two strictly less than `n` (RFC 6962 §2.1).
//! * Odd leaves are **never duplicated** (avoids the Bitcoin CVE-2012-2459 ambiguity).
//!
//! See `docs/adr/0012-lessons-from-bitcoin-block-design.md`.

use sha2::{Digest, Sha256};

/// Maximum proof length supported (trees of up to 2^64 leaves).
pub const MAX_PROOF_LEN: usize = 64;

/// `SHA-256("")`, the RFC 6962 hash of an empty tree.
pub fn empty_root() -> [u8; 32] {
    Sha256::digest([]).into()
}

pub fn leaf_hash(record: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update([0x00]);
    h.update(record);
    h.finalize().into()
}

pub fn node_hash(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update([0x01]);
    h.update(left);
    h.update(right);
    h.finalize().into()
}

fn split(n: usize) -> usize {
    let mut k = 1usize;
    while k << 1 < n {
        k <<= 1;
    }
    k
}

/// Merkle Tree Hash over `n` leaves; `leaf(i)` returns record `i`'s bytes.
pub fn root_with<'a, F: Fn(usize) -> &'a [u8]>(n: usize, leaf: &F) -> [u8; 32] {
    fn go<'a, F: Fn(usize) -> &'a [u8]>(lo: usize, n: usize, leaf: &F) -> [u8; 32] {
        if n == 1 {
            return leaf_hash(leaf(lo));
        }
        let k = split(n);
        node_hash(&go(lo, k, leaf), &go(lo + k, n - k, leaf))
    }
    if n == 0 {
        empty_root()
    } else {
        go(0, n, leaf)
    }
}

/// Merkle root over a slice of records.
pub fn merkle_root(records: &[&[u8]]) -> [u8; 32] {
    root_with(records.len(), &|i| records[i])
}

/// Merkle root over `data` split into fixed-size records of `record_size` bytes.
/// Returns `None` if `record_size` is 0 or does not divide `data.len()`.
pub fn merkle_root_fixed(data: &[u8], record_size: usize) -> Option<[u8; 32]> {
    if record_size == 0 || !data.len().is_multiple_of(record_size) {
        return None;
    }
    let n = data.len() / record_size;
    Some(root_with(n, &|i| {
        &data[i * record_size..(i + 1) * record_size]
    }))
}

/// Writes the RFC 6962 audit path for leaf `index` into `out`; returns the path length.
/// Returns `None` if `index >= n` or `out` is too small.
pub fn inclusion_proof_with<'a, F: Fn(usize) -> &'a [u8]>(
    n: usize,
    index: usize,
    leaf: &F,
    out: &mut [[u8; 32]],
) -> Option<usize> {
    if index >= n {
        return None;
    }
    // Iterative top-down walk; siblings are collected top-down then reversed (path is leaf-up).
    let (mut lo, mut size, mut m, mut len) = (0usize, n, index, 0usize);
    while size > 1 {
        let k = split(size);
        if len >= out.len() {
            return None;
        }
        if m < k {
            out[len] = root_with(size - k, &|i| leaf(lo + k + i));
            size = k;
        } else {
            out[len] = root_with(k, &|i| leaf(lo + i));
            lo += k;
            m -= k;
            size -= k;
        }
        len += 1;
    }
    out[..len].reverse();
    Some(len)
}

/// Audit path for fixed-size records.
pub fn inclusion_proof_fixed(
    data: &[u8],
    record_size: usize,
    index: usize,
    out: &mut [[u8; 32]],
) -> Option<usize> {
    if record_size == 0 || !data.len().is_multiple_of(record_size) {
        return None;
    }
    let n = data.len() / record_size;
    inclusion_proof_with(
        n,
        index,
        &|i| &data[i * record_size..(i + 1) * record_size],
        out,
    )
}

/// Verifies an RFC 6962 / RFC 9162 §2.1.3.2 inclusion proof.
pub fn verify_inclusion(
    record: &[u8],
    index: u64,
    tree_size: u64,
    proof: &[[u8; 32]],
    root: &[u8; 32],
) -> bool {
    if index >= tree_size {
        return false;
    }
    let (mut fnn, mut sn) = (index, tree_size - 1);
    let mut r = leaf_hash(record);
    for p in proof {
        if sn == 0 {
            return false;
        }
        if fnn & 1 == 1 || fnn == sn {
            r = node_hash(p, &r);
            if fnn & 1 == 0 {
                while fnn & 1 == 0 && fnn != 0 {
                    fnn >>= 1;
                    sn >>= 1;
                }
            }
        } else {
            r = node_hash(&r, p);
        }
        fnn >>= 1;
        sn >>= 1;
    }
    sn == 0 && &r == root
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex32(s: &str) -> [u8; 32] {
        let mut o = [0u8; 32];
        for i in 0..32 {
            o[i] = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap();
        }
        o
    }

    // Leaves and roots from the certificate-transparency reference test suite:
    // https://github.com/google/certificate-transparency/blob/master/cpp/merkletree/merkle_tree_test.cc
    const LEAVES: [&[u8]; 8] = [
        b"",
        b"\x00",
        b"\x10",
        b"\x20\x21",
        b"\x30\x31",
        b"\x40\x41\x42\x43",
        b"\x50\x51\x52\x53\x54\x55\x56\x57",
        b"\x60\x61\x62\x63\x64\x65\x66\x67\x68\x69\x6a\x6b\x6c\x6d\x6e\x6f",
    ];
    const ROOTS: [&str; 8] = [
        "6e340b9cffb37a989ca544e6bb780a2c78901d3fb33738768511a30617afa01d",
        "fac54203e7cc696cf0dfcb42c92a1d9dbaf70ad9e621f4bd8d98662f00e3c125",
        "aeb6bcfe274b70a14fb067a5e5578264db0fa9b51af5e0ba159158f329e06e77",
        "d37ee418976dd95753c1c73862b9398fa2a2cf9b4ff0fdfe8b30cd95209614b7",
        "4e3bbb1f7b478dcfe71fb631631519a3bca12c9aefca1612bfce4c13a86264d4",
        "76e67dadbcdf1e10e1b74ddc608abd2f98dfb16fbce75277b5232a127f2087ef",
        "ddb89be403809e325750d3d263cd78929c2942b7942a34b77e122c9594a74c8c",
        "5dc9da79a70659a9ad559cb701ded9a2ab9d823aad2f4960cfe370eff4604328",
    ];

    #[test]
    fn rfc6962_reference_roots() {
        assert_eq!(
            empty_root(),
            hex32("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        );
        for n in 1..=8 {
            assert_eq!(merkle_root(&LEAVES[..n]), hex32(ROOTS[n - 1]), "n={n}");
        }
    }

    fn check_all(n: usize) {
        let leaves = &LEAVES[..n];
        let root = merkle_root(leaves);
        let f = |i: usize| leaves[i];
        #[allow(clippy::needless_range_loop)]
        for i in 0..n {
            let mut buf = [[0u8; 32]; MAX_PROOF_LEN];
            let len = inclusion_proof_with(n, i, &f, &mut buf).unwrap();
            let p = &buf[..len];
            assert!(verify_inclusion(leaves[i], i as u64, n as u64, p, &root));
            // tampered record
            assert!(!verify_inclusion(b"tampered", i as u64, n as u64, p, &root));
            // wrong index / size
            if n > 1 {
                assert!(!verify_inclusion(
                    leaves[i],
                    ((i + 1) % n) as u64,
                    n as u64,
                    p,
                    &root
                ));
            }
            // tampered proof element
            if len > 0 {
                let mut bad = [[0u8; 32]; MAX_PROOF_LEN];
                bad[..len].copy_from_slice(p);
                bad[0][0] ^= 1;
                assert!(!verify_inclusion(
                    leaves[i],
                    i as u64,
                    n as u64,
                    &bad[..len],
                    &root
                ));
                // truncated proof
                assert!(!verify_inclusion(
                    leaves[i],
                    i as u64,
                    n as u64,
                    &p[..len - 1],
                    &root
                ));
            }
        }
    }

    #[test]
    fn proofs_1_2_3_7_leaves() {
        for n in [1, 2, 3, 7] {
            check_all(n);
        }
        for n in 1..=8 {
            check_all(n);
        }
    }

    #[test]
    fn odd_leaf_not_duplicated() {
        // Bitcoin-style duplication would make [a,b,c] and [a,b,c,c] collide (CVE-2012-2459).
        let a: &[u8] = b"a";
        let b: &[u8] = b"b";
        let c: &[u8] = b"c";
        assert_ne!(merkle_root(&[a, b, c]), merkle_root(&[a, b, c, c]));
        // 3-leaf root = node(node(a,b), leaf(c)), not node(.., node(c,c)).
        let expect = node_hash(&node_hash(&leaf_hash(a), &leaf_hash(b)), &leaf_hash(c));
        assert_eq!(merkle_root(&[a, b, c]), expect);
    }

    #[test]
    fn leaf_and_node_are_domain_separated() {
        let l = leaf_hash(b"x");
        let mut cat = [0u8; 64];
        cat[..32].copy_from_slice(&l);
        cat[32..].copy_from_slice(&l);
        // An interior node can never be presented as a 64-byte leaf.
        assert_ne!(leaf_hash(&cat), node_hash(&l, &l));
    }

    #[test]
    fn fixed_size_records() {
        let data = [1u8, 2, 3, 4, 5, 6];
        let r = merkle_root_fixed(&data, 2).unwrap();
        assert_eq!(r, merkle_root(&[&data[0..2], &data[2..4], &data[4..6]]));
        assert!(merkle_root_fixed(&data, 4).is_none());
        let mut buf = [[0u8; 32]; MAX_PROOF_LEN];
        let len = inclusion_proof_fixed(&data, 2, 2, &mut buf).unwrap();
        assert!(verify_inclusion(&data[4..6], 2, 3, &buf[..len], &r));
    }
}
