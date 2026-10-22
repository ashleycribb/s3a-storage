# S3A Git-Style Version Control Specification (S3A-VCS)

**Document Version:** 1.0-draft  
**Status:** IMPLEMENTED (Phase 1)  
**Date:** October 2026  
**License:** [Creative Commons Attribution 4.0 International (CC-BY-4.0)](LICENSE)  
**Patents:** [Royalty-Free Patent Non-Assertion Pledge](PATENTS.md)  
**Parent Specifications:** [S3A Hyper-Tile Storage Format Specification](S3A-TILE-SPEC.md), [S3A Digital Vault Specification](S3A-VAULT.md)

---

## Specification Audit & Implementation Notice

> [!IMPORTANT]
> **SPECIFICATION AUDIT STATUS: IMPLEMENTED (Phase 1)**  
> In accordance with the S3A Specification Audit Policy, Git-style version control for S3A archives has been **fully implemented and verified**. This implementation includes:
> 1. Version control commit records (`crates/s3a-vcs`) binding a cryptographic seal hash, author identity gate address, commit message, parent seal hashes (supporting DAG branching and multi-parent merge commits), and Ed25519 signature from the author's vault key.
> 2. Reference management (`refs/heads/*`, `refs/tags/*`) stored directly inside the archive as signed ref records, supporting mutable branches and immutable tags.
> 3. Standard Git-like operations: `init`, `commit -m`, `log`, `branch`, `checkout` (point-in-time immutable view), `tag`, `diff` (tile-level aggregate counts and record IDs without restricted payload leakage), and `merge` (append-only record union with deterministic conflict detection).
> 4. Sovereign vault synchronization (`push`/`pull`) between vault gate endpoints over `gate://` protocol using tile-hash inventory negotiations (transmitting strictly missing sealed tiles), enforcing fast-forward safety unless `--merge` is specified, and honoring active consent scopes.
> 5. Read-only Git export (`vcs export-git <dir>`) writing a valid Git repository with commit history and CSV/JSON record snapshots decipherable by standard Git tooling (`git log`).
> 6. Human Judgment Safety Rule: Merge conflicts involving claim or evidence records invoke `ask_human` checkpoints; automated tools are strictly prohibited from auto-resolving conflicting claims.
> 7. All test suites pass across `cargo test --workspace`.

---

## 1. Conformance & Architectural Principles

### 1.1 Conformance Terminology

The key words **"MUST"**, **"MUST NOT"**, **"REQUIRED"**, **"SHALL"**, **"SHALL NOT"**, **"SHOULD"**, **"SHOULD NOT"**, **"RECOMMENDED"**, **"NOT RECOMMENDED"**, **"MAY"**, and **"OPTIONAL"** in this document are to be interpreted as described in BCP 14 [RFC 2119] [RFC 8174].

### 1.2 Core Architectural Principles

1. **Commit = Sealed State + Cryptographic Identity**:
   Every commit in S3A corresponds directly to a valid **Sealed Footer v2** container state. A commit record seals the archive, commits to the authoritative `seal_hash`, and records author provenance via the author's vault Gate Address (`gate://s3a-XXXX...`) signed with their Ed25519 private key.
2. **First-Class DAG Topology**:
   Commits reference one or more parent seal hashes (`parent_seal_hashes`). Root commits have zero parents; regular linear commits have one parent; merge commits have two or more parents.
3. **Archive-Native Storage**:
   VCS commit metadata, branches, and tags are stored natively inside the S3A archive using assigned tile types (`TileType::VCS_COMMIT = 80`, `TileType::VCS_REF = 81`). Existing readers safely skip these records via the unknown-type skip rule (§7.2).
4. **Zero Privacy Leaks on Diff**:
   Diff operations evaluate differences across commits at the tile and record identifier level (`added`, `withdrawn`, `erased` counts and IDs). Confidential payload fields, participant biometric records, and private data are never leaked in diff summaries.
5. **Human Judgment for Claims and Evidence**:
   In accordance with S3A safety governance, conflicts involving epistemic claims (`CLAIM_KIND_EMPIRICAL`), scientific notes, or contradictory assertions MUST NEVER be auto-resolved by AI agents or automated merge heuristics. Such conflicts trigger an interactive human judgment checkpoint (`ask_human`).

---

## 2. Binary Layout & Data Structures

### 2.1 Tile Type Assignments

| Tile ID (Dec) | Tile ID (Hex) | Canonical Name | Owner / Crate | Description |
|:---:|:---:|:---|:---|:---|
| **80** | `0x0050` | `vcs_commit` | `s3a-vcs` | Version control commit metadata, author signature, and parent DAG |
| **81** | `0x0051` | `vcs_ref` | `s3a-vcs` | Branch and tag reference definitions (name → seal hash) |

### 2.2 VcsCommitRecord Binary Layout

A commit record binds an archive's cryptographic seal hash to human provenance and DAG parentage:

```rust
#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct VcsCommitRecord {
    pub commit_id: [u8; 32],            // SHA-256 digest of commit payload
    pub seal_hash: [u8; 32],            // Authoritative sealed footer v2 seal hash
    pub author_pubkey: [u8; 32],        // Ed25519 public key of author
    pub author_address_prefix: [u8; 8], // GateAddress prefix
    pub author_checksum: u8,            // GateAddress checksum
    pub parent_count: u8,               // Number of parent commits (0=root, 1=linear, 2=merge)
    pub _pad: [u8; 6],
    pub parent_seal_hashes: [[u8; 32]; 2], // Primary and secondary parent seal hashes
    pub committed_timestamp: u64,       // Unix epoch timestamp (seconds)
    pub signature: [u8; 64],            // Ed25519 signature over commit envelope
    pub message_bytes: [u8; 256],       // UTF-8 commit message (null-padded)
}
```

#### Envelope Signing Hash
The commit envelope signed by `author_pubkey` commits to:
$$\text{Envelope} = \text{"S3ACOMMIT1"} \,\|\, \text{seal\_hash} \,\|\, \text{parent\_seal\_hashes} \,\|\, \text{committed\_timestamp} \,\|\, \text{message\_bytes}$$

### 2.3 VcsRefRecord Binary Layout

Branch heads and tags are stored as explicit ref records:

```rust
pub const REF_KIND_BRANCH: u16 = 1;
pub const REF_KIND_TAG: u16 = 2;

#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq, Eq)]
pub struct VcsRefRecord {
    pub ref_kind: u16,                  // 1 = Branch (mutable), 2 = Tag (immutable)
    pub _pad: [u8; 6],
    pub target_seal_hash: [u8; 32],     // Seal hash pointed to by this ref
    pub author_pubkey: [u8; 32],        // Creator or updater public key
    pub updated_timestamp: u64,         // Unix epoch timestamp of ref creation/update
    pub signature: [u8; 64],            // Ed25519 signature over ref payload
    pub name: [u8; 64],                 // UTF-8 ref name (e.g., "main", "feature-sensor", "v1.0.0")
}
```

---

## 3. Version Control Operations

### 3.1 Initializing a Repository (`vcs init`)
- Initializes dual file header and foundational directory.
- Creates initial branch reference pointing to Genesis (`refs/heads/main`).
- Creates sovereign `.s3a/` or `.vault.s3a` metadata container.

### 3.2 Committing Changes (`vcs commit -m <message>`)
1. Scans pending uncommitted or modified hyper-tiles in the active container.
2. Generates updated Sealed Footer v2 (`seal_buffer_v2`), computing authoritative `seal_hash`.
3. Resolves active branch HEAD from current ref state as `parent_seal_hashes[0]`.
4. Derives author Gate Address from active vault keystore (`FileKeyStore`).
5. Constructs `VcsCommitRecord`, signs envelope with author's Ed25519 private key.
6. Updates active branch ref (`refs/heads/<branch>`) to point to the new `seal_hash`.

### 3.3 Inspecting History (`vcs log`)
- Traverses the DAG backwards starting from `HEAD` (or specified ref/commit) via `parent_seal_hashes`.
- Displays commit ID, seal hash, author Gate Address, timestamp, branch/tag pointers, and commit message.

### 3.3a Frontier Queries (`vcs leaves`, `vcs children`, `vcs lineage`)
Read-only DAG views (inspired by agenthub). One O(N) pass builds a child index from `parent_seal_hashes[..parent_count]`; nothing is written.
- `vcs leaves <archive>`: commits with no children (unbuilt-on work).
- `vcs children <archive> <hash|ref>`: direct children, stored order.
- `vcs lineage <archive> <hash|ref> [--all]`: ancestry including the start. Default follows the first parent only (one deterministic path); `--all` returns every ancestor via any parent, BFS order, deduplicated.
- `--agent <hex>` filters results by author gate-address prefix or pubkey prefix; `--json` for machine output. Unknown hashes return `CommitNotFound`.

### 3.4 Branching & Tagging (`vcs branch`, `vcs tag`)
- **Branches (`vcs branch <name>`)**: Creates a mutable reference pointing to the current commit's `seal_hash`.
- **Tags (`vcs tag <name>`)**: Creates an immutable snapshot reference. If a tag with the same name already exists pointing to a different commit, the update is rejected.

### 3.5 Checking Out State (`vcs checkout <ref>`)
- Resolves the target ref or commit ID to its corresponding `seal_hash`.
- Configures a read-only snapshot view bounded by the target generation/seal.

### 3.6 Inspecting Differences (`vcs diff <a> <b>`)
- Compares tile directories and record lists between two commits $A$ and $B$.
- Computes aggregate counts per tile type:
  - Records added.
  - Records withdrawn (via `ConsentRecord` or `MarkRecord`).
  - Records erased (via `ErasureRecord`).
- **Privacy Guarantee**: Diff never dumps or prints raw sensor values, biometric streams, or participant personal data; reports strictly summary counts and non-sensitive logical identifiers.

### 3.7 Merging Branches (`vcs merge <source_branch>`)
- Computes nearest common ancestor in the DAG.
- Evaluates fast-forward possibility:
  - If target branch is an ancestor of source branch, advances target pointer (fast-forward).
  - Otherwise, executes append-only union merge:
    - Non-conflicting unique records from both parents are unioned.
    - If two records share the same logical key/ID but possess differing content:
      - For empirical claims or research notes: triggers `ask_human` judgment checkpoint.
      - For standard data: reports merge conflict with offending record IDs; never silently overwrites.
    - Upon clean union, seals a merge commit with `parent_count = 2`.

---

## 4. Vault Synchronization over `gate://` Protocol

### 4.1 Missing Tile Inventory Protocol
When pushing or pulling between vaults:
1. **Inventory Request**: Source requests target's known tile payload hashes for the desired commit DAG.
2. **Missing Shards Determination**: Target responds with list of unknown tile hashes.
3. **Delta Transfer**: Source transmits strictly the missing sealed hyper-tiles.
4. **Signature & Chain Verification**: Receiving vault validates:
   - All received tile CRC32C checksums and SHA-256 payload hashes.
   - Sealed footer hash chain integrity.
   - Commit Ed25519 signature validity.
5. **Consent Gate**: If the incoming records fall outside authorized participant consent scopes for the receiving vault, transmission is rejected.
6. **Fast-Forward Check**: Rejects non-fast-forward push operations unless `--merge` is explicitly supplied.

---

## 5. Export to Standard Git (`vcs export-git`)

To allow researchers and software developers to inspect S3A history using familiar Git tooling:
- `s3a-cli vcs export-git <output_dir>` builds a valid local `.git` repository.
- Replays every sealed commit in chronological order from root to HEAD.
- Writes structured tables (JSON / CSV) representing record contents at each floor.
- Runs standard `git commit` matching the original S3A author, timestamp, and message.
- Results can be inspected immediately with standard `git log`, `git show`, and `git diff`.

---

## 6. Verification & Test Suite Matrix

| Test Name | File Location | Purpose |
|:---|:---|:---|
| `test_vcs_commit_signature_verify_and_forgery_rejected` | `crates/s3a-vcs/tests/vcs_tests.rs` | Verifies valid commit signatures pass and altered envelopes are rejected |
| `test_vcs_log_dag_order` | `crates/s3a-vcs/tests/vcs_tests.rs` | Confirms topological DAG log ordering from HEAD to genesis |
| `test_vcs_branch_and_checkout_isolation` | `crates/s3a-vcs/tests/vcs_tests.rs` | Tests branch creation, branch divergence, and checkout state isolation |
| `test_vcs_diff_counts_without_restricted_payloads` | `crates/s3a-vcs/tests/vcs_tests.rs` | Proves diff computes added/withdrawn/erased counts without leaking payloads |
| `test_vcs_merge_union_and_conflict_detection` | `crates/s3a-vcs/tests/vcs_tests.rs` | Verifies append-only union merge and conflicting record ID detection |
| `test_vcs_sync_inventory_missing_tiles_only` | `crates/s3a-vcs/tests/vcs_tests.rs` | Ensures push/pull inventory transmits only missing tiles |
| `test_vcs_non_fast_forward_rejected` | `crates/s3a-vcs/tests/vcs_tests.rs` | Rejects divergent push without merge flag |
| `test_vcs_consent_enforcement_on_push` | `crates/s3a-vcs/tests/vcs_tests.rs` | Enforces participant consent restrictions during vault transfer |
| `test_vcs_export_git_produces_valid_repo` | `crates/s3a-vcs/tests/vcs_tests.rs` | Verifies `git log` works against exported git repository |
| `test_manifest_created_timestamp_populated` | `crates/s3a-engine/tests/manifest_tile.rs` | Verifies manifest "Created" displays valid archive timestamp |
