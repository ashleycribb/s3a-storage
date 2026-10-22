//! Read-only frontier queries over the commit DAG (inspired by agenthub's
//! `leaves` / `children` / `lineage`). Nothing here writes to the archive.
//!
//! Commits are keyed by `seal_hash`; `parent_seal_hashes[..parent_count]` are the edges.
//! All queries build one child index in a single O(N) pass.

use std::collections::{HashMap, HashSet};

use crate::errors::VcsError;
use crate::records::VcsCommitRecord;
use crate::repo::VcsRepo;

/// How `lineage` walks merge commits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineageMode {
    /// Follow only the primary (first) parent: one deterministic path to a root.
    #[default]
    FirstParent,
    /// Every ancestor reachable through any parent, BFS order, deduplicated.
    All,
}

fn parents(c: &VcsCommitRecord) -> impl Iterator<Item = &[u8; 32]> {
    c.parent_seal_hashes
        .iter()
        .take((c.parent_count as usize).min(2))
        .filter(|h| **h != [0u8; 32])
}

/// True if `agent` (hex, case-insensitive) prefixes the author's gate address prefix
/// or Ed25519 public key.
pub fn authored_by(c: &VcsCommitRecord, agent: &str) -> bool {
    let a = agent
        .trim()
        .trim_start_matches("gate://")
        .trim_start_matches("s3a-")
        .to_ascii_lowercase();
    !a.is_empty()
        && (hex::encode(c.author_address_prefix).starts_with(&a)
            || hex::encode(c.author_pubkey).starts_with(&a))
}

/// In-memory index of a commit set.
pub struct Frontier<'a> {
    commits: &'a [VcsCommitRecord],
    by_seal: HashMap<[u8; 32], usize>,
    children: HashMap<[u8; 32], Vec<usize>>,
}

impl<'a> Frontier<'a> {
    pub fn new(commits: &'a [VcsCommitRecord]) -> Self {
        let mut by_seal = HashMap::with_capacity(commits.len());
        let mut children: HashMap<[u8; 32], Vec<usize>> = HashMap::new();
        for (i, c) in commits.iter().enumerate() {
            by_seal.insert(c.seal_hash, i);
            for p in parents(c) {
                let v = children.entry(*p).or_default();
                if !v.contains(&i) {
                    v.push(i);
                }
            }
        }
        Self {
            commits,
            by_seal,
            children,
        }
    }

    /// Resolve a full hex seal hash or commit id.
    pub fn resolve(&self, hash: &str) -> Result<[u8; 32], VcsError> {
        let not_found = || VcsError::CommitNotFound(hash.to_string());
        let bytes = hex::decode(hash.trim()).map_err(|_| not_found())?;
        if bytes.len() != 32 {
            return Err(not_found());
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        if self.by_seal.contains_key(&arr) {
            return Ok(arr);
        }
        self.commits
            .iter()
            .find(|c| c.commit_id == arr)
            .map(|c| c.seal_hash)
            .ok_or_else(not_found)
    }

    /// Commits with no children, in stored order.
    pub fn leaves(&self) -> Vec<VcsCommitRecord> {
        self.commits
            .iter()
            .filter(|c| !self.children.contains_key(&c.seal_hash))
            .copied()
            .collect()
    }

    /// Direct children of `seal`, in stored order.
    pub fn children(&self, seal: &[u8; 32]) -> Result<Vec<VcsCommitRecord>, VcsError> {
        if !self.by_seal.contains_key(seal) {
            return Err(VcsError::CommitNotFound(hex::encode(seal)));
        }
        Ok(self
            .children
            .get(seal)
            .map(|v| v.iter().map(|&i| self.commits[i]).collect())
            .unwrap_or_default())
    }

    /// Ancestry starting at (and including) `seal`.
    pub fn lineage(
        &self,
        seal: &[u8; 32],
        mode: LineageMode,
    ) -> Result<Vec<VcsCommitRecord>, VcsError> {
        let Some(&start) = self.by_seal.get(seal) else {
            return Err(VcsError::CommitNotFound(hex::encode(seal)));
        };
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut queue = std::collections::VecDeque::from([start]);
        while let Some(i) = queue.pop_front() {
            let c = &self.commits[i];
            if !seen.insert(c.seal_hash) {
                continue;
            }
            out.push(*c);
            for (n, p) in parents(c).enumerate() {
                if mode == LineageMode::FirstParent && n > 0 {
                    break;
                }
                if let Some(&j) = self.by_seal.get(p) {
                    queue.push_back(j);
                }
            }
        }
        Ok(out)
    }
}

fn filter(v: Vec<VcsCommitRecord>, agent: Option<&str>) -> Vec<VcsCommitRecord> {
    match agent {
        Some(a) => v.into_iter().filter(|c| authored_by(c, a)).collect(),
        None => v,
    }
}

impl VcsRepo {
    fn resolve_frontier(f: &Frontier, repo: &VcsRepo, h: &str) -> Result<[u8; 32], VcsError> {
        if let Some(r) = repo.get_ref(h)? {
            return Ok(r.target_seal_hash);
        }
        f.resolve(h)
    }

    /// Read-only: commits nobody has built on yet.
    pub fn leaves(&self, agent: Option<&str>) -> Result<Vec<VcsCommitRecord>, VcsError> {
        let all = self.list_commits()?;
        Ok(filter(Frontier::new(&all).leaves(), agent))
    }

    /// Read-only: direct children of a commit (hex seal/commit id or ref name).
    pub fn children(
        &self,
        hash: &str,
        agent: Option<&str>,
    ) -> Result<Vec<VcsCommitRecord>, VcsError> {
        let all = self.list_commits()?;
        let f = Frontier::new(&all);
        let s = Self::resolve_frontier(&f, self, hash)?;
        Ok(filter(f.children(&s)?, agent))
    }

    /// Read-only: ancestry of a commit back to root(s).
    pub fn lineage(
        &self,
        hash: &str,
        mode: LineageMode,
        agent: Option<&str>,
    ) -> Result<Vec<VcsCommitRecord>, VcsError> {
        let all = self.list_commits()?;
        let f = Frontier::new(&all);
        let s = Self::resolve_frontier(&f, self, hash)?;
        Ok(filter(f.lineage(&s, mode)?, agent))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::Zeroable;

    fn h(n: u8) -> [u8; 32] {
        [n; 32]
    }
    fn mk(n: u8, ps: &[u8], author: u8) -> VcsCommitRecord {
        let mut c = VcsCommitRecord::zeroed();
        c.seal_hash = h(n);
        c.commit_id = [n.wrapping_add(100); 32];
        c.parent_count = ps.len() as u8;
        for (i, p) in ps.iter().enumerate() {
            c.parent_seal_hashes[i] = h(*p);
        }
        c.author_address_prefix = [author; 8];
        c
    }
    fn seals(v: &[VcsCommitRecord]) -> Vec<u8> {
        v.iter().map(|c| c.seal_hash[0]).collect()
    }

    #[test]
    fn linear_chain() {
        let cs = vec![mk(1, &[], 0xaa), mk(2, &[1], 0xaa), mk(3, &[2], 0xaa)];
        let f = Frontier::new(&cs);
        assert_eq!(seals(&f.leaves()), vec![3]);
        assert_eq!(seals(&f.children(&h(1)).unwrap()), vec![2]);
        assert!(f.children(&h(3)).unwrap().is_empty());
        assert_eq!(
            seals(&f.lineage(&h(3), LineageMode::FirstParent).unwrap()),
            vec![3, 2, 1]
        );
    }

    #[test]
    fn branch_fork() {
        let cs = vec![mk(1, &[], 0xaa), mk(2, &[1], 0xaa), mk(3, &[1], 0xbb)];
        let f = Frontier::new(&cs);
        assert_eq!(seals(&f.leaves()), vec![2, 3]);
        assert_eq!(seals(&f.children(&h(1)).unwrap()), vec![2, 3]);
    }

    #[test]
    fn merge_two_parents() {
        // 1 -> 2 -> 4(merge 2,3); 1 -> 3
        let cs = vec![
            mk(1, &[], 0xaa),
            mk(2, &[1], 0xaa),
            mk(3, &[1], 0xbb),
            mk(4, &[2, 3], 0xaa),
        ];
        let f = Frontier::new(&cs);
        assert_eq!(seals(&f.leaves()), vec![4]);
        assert_eq!(seals(&f.children(&h(3)).unwrap()), vec![4]);
        assert_eq!(
            seals(&f.lineage(&h(4), LineageMode::FirstParent).unwrap()),
            vec![4, 2, 1]
        );
        assert_eq!(
            seals(&f.lineage(&h(4), LineageMode::All).unwrap()),
            vec![4, 2, 3, 1]
        );
    }

    #[test]
    fn unknown_hash_errors() {
        let cs = vec![mk(1, &[], 0xaa)];
        let f = Frontier::new(&cs);
        assert!(matches!(
            f.children(&h(9)),
            Err(VcsError::CommitNotFound(_))
        ));
        assert!(f.lineage(&h(9), LineageMode::All).is_err());
        assert!(f.resolve("zz").is_err());
        assert!(f.resolve(&hex::encode(h(9))).is_err());
        // commit_id also resolves
        assert_eq!(f.resolve(&hex::encode([101u8; 32])).unwrap(), h(1));
    }

    #[test]
    fn empty_repo() {
        let cs: Vec<VcsCommitRecord> = vec![];
        let f = Frontier::new(&cs);
        assert!(f.leaves().is_empty());
        assert!(f.children(&h(1)).is_err());
    }

    #[test]
    fn agent_filter() {
        let cs = vec![mk(1, &[], 0xaa), mk(2, &[1], 0xaa), mk(3, &[1], 0xbb)];
        let f = Frontier::new(&cs);
        assert_eq!(seals(&filter(f.leaves(), Some("bbbb"))), vec![3]);
        assert_eq!(seals(&filter(f.leaves(), Some("gate://AA"))), vec![2]);
        assert!(filter(f.leaves(), Some("cc")).is_empty());
    }
}
