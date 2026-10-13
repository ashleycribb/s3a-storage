# S3A Swarm: High-Concurrency Multi-Agent Version Control
### Cloudflare "Build the Next GitHub" Challenge Submission (October 14 Deadline)

This repository demonstrates the concurrency engine for the next generation of GitHub built on **S3A Hyper-Tiles**, **S3A VCS**, **S3A Vault**, and the **Harness Decision Layer**, deployed across **Cloudflare Workers**, **Durable Objects**, and **Cloudflare Artifacts**.

Traditional Git architecture relies on centralized lockfiles (`.git/index.lock`), single-commit trees, and serialized branch heads. When hundreds of autonomous AI agents commit concurrently, traditional Git suffers catastrophic non-fast-forward push rejections and rebase storms.

**S3A VCS solves this problem at the storage foundation:**
1. **Disjoint Tile Union**: Non-overlapping record tiles merge unconditionally as a set union with **zero locks** and **zero rebase storms**.
2. **Deterministic Record Merge**: Concurrent edits to the same data record resolve deterministically via **Lamport timestamp (Last-Write-Wins)** with **cryptographic Ed25519 signature tie-breaking**. No coordination or central leader consensus required.
3. **Epistemic Invariant (Ask-Human)**: AI models and agents are strictly forbidden from auto-resolving empirical claim or scientific evidence disagreements. Conflicting claims route to an `ask_human` supervisory queue requiring a signed `VcsResolutionRecord`.
4. **Scoped Delegation Grants**: Agents hold cryptographically signed delegation grants restricting which paths/partitions they may touch. Out-of-bounds writes are rejected fail-safe and audited.
5. **Shadow Branch Evaluation**: With `--shadow`, commits land on isolated shadow branches and only promote to `main` after passing automated integrity and schema evaluation.
6. **Cloudflare Edge Integration**: Worker gateway `s3a-gate` + Durable Object `RepoCoordinator` per project + Cloudflare Artifacts Git repositories for per-agent isolation and merged baseline Git export.

---

## 1000-Agent Benchmark Results (Local PC Run)

Executed locally on Windows using `s3a-swarm run --agents 1000 --commits 5`:

| Metric | Result |
| :--- | :--- |
| **Simulated Agents** | **1,000** concurrent tokio tasks |
| **Total Commits Attempted** | **5,000** signed commits |
| **Total Duration** | **0.46 seconds** |
| **Push Throughput** | **10,933.11 commits / second** |
| **p50 Push Latency** | **0.084 ms** (84 microseconds) |
| **p99 Push Latency** | **0.140 ms** (140 microseconds) |
| **Auto-Merged (Union + LWW)** | **4,851** commits |
| **Rejected by Grant** | **100** commits (2.0% fail-safe boundary enforcement) |
| **Sent to Human Queue** | **49** claim disagreements (0.98% epistemic conflicts) |
| **Commit Signatures Verified** | **4,851 / 4,851 (100% valid)** |
| **Final Root Seal Hash** | `170964ecfd26acee5f8e48e0dec5f78a9414f7796875368c1f13071199e88d21` |

Local benchmark numbers are saved to [`demo/results.json`](results.json).

---

## Remote Benchmark against Cloudflare Workers & Artifacts

To run against the live Cloudflare deployment with 200 agents (concurrency 50, staying safely within the 10,000 operations/month limit):

```bash
cargo run --release --bin s3a-swarm -- run --remote https://s3a-gate.<subdomain>.workers.dev --agents 200 --commits 5 --results demo/results-cloudflare.json
```

This sends signed HTTP pushes containing the agent's Ed25519 signature, `gate://` address, and delegation grant to the `/project/:id/push` endpoint. The Durable Object checks grants, writes to the agent's Cloudflare Artifacts repository, merges to baseline, and writes real HTTP latencies into [`demo/results-cloudflare.json`](results-cloudflare.json).

---

## Deterministic Merge Rules

### Rule 1: Disjoint Record Set Union
Hyper-Tiles containing unique record keys across agent workspaces merge directly without lock contention. Two agents modifying different partitions (e.g. `agent_42/telemetry` vs `agent_99/telemetry`) never contend or cause non-fast-forward errors.

### Rule 2: Same-Record Data Edits (LWW + Ed25519 Tie-Break)
When concurrent agents modify the exact same non-claim record key:
1. **Primary Ordering**: `committed_timestamp` (Last-Write-Wins).
2. **Deterministic Tie-Break**: If timestamps are identical, the engine performs lexicographical comparison of the commit's 64-byte Ed25519 signature (`incoming.signature > existing.signature`).
Because Ed25519 signatures are cryptographically pseudo-random, unique, and strictly unforgeable, every node in a distributed network converges on the exact same winner with **zero central coordination**.

### Rule 3: Epistemic Invariant (Ask-Human Queue)
When concurrent commits touch epistemic or empirical claim records (`ScholarTileType::NOTE_CLAIMS`, `RESEARCH_GRAPH`, `AI_TRACE_LOG`, or record keys matching `claim_*` / `evidence_*`), the engine **never auto-resolves**.
- The conflicting versions are held in the `AskHumanQueue`.
- Human supervisors inspect versions through the web dashboard or CLI and sign a `VcsResolutionRecord` to finalize the merge.

---

## Real Git Verification with Cloudflare Artifacts

S3A is backed by real Git repositories created through the Cloudflare Artifacts Workers binding (`env.ARTIFACTS`):

1. **Agent Repositories**: Each agent session receives an isolated Artifacts repository (e.g., `s3a-agent-agent-042`).
2. **Baseline Repository**: Merged commits are synced to `s3a-baseline-<project>`.

### Cloning the Baseline Repository with Standard Git
To clone and inspect the merged Git history from any terminal:

```bash
# Set your Cloudflare Artifacts bearer token (generated via Workers binding or wrangler)
export REPO_TOKEN="<ARTIFACTS_TOKEN>"
export BASELINE_REMOTE="https://artifacts.cloudflare.net/git/default/s3a-baseline-s3a-swarm-main.git"

# Clone using standard git with Authorization header
git -c http.extraHeader="Authorization: Bearer $REPO_TOKEN" clone "$BASELINE_REMOTE"
cd s3a-baseline-s3a-swarm-main

# View the real Git commit history created by the swarm
git log --oneline -n 10
```

---

## Cloudflare Worker Deployment

The Cloudflare Worker implementation is located in the [`cloudflare/`](../cloudflare/) directory:

```bash
cd cloudflare

# 1. Authenticate with Cloudflare
npx wrangler login

# 2. Deploy Worker and Durable Object
npx wrangler deploy
```

The Worker provides:
- **Root Dashboard (`/`)**: Real-time counter metrics, active agents linking to their Artifacts repositories, the human review queue with Approve/Reject buttons, and recent merges stream.
- **`POST /project/:id/push`**: Ingests signed commits, validates delegation grants, updates agent Artifacts repo, and executes deterministic merge.
- **`GET /project/:id/status`**: Returns project health, counters, and Artifacts repo status.
- **`GET /project/:id/human-queue`**: Returns pending claim conflicts.
- **`POST /project/:id/resolve/:conflict`**: Records signed human supervisor resolution into provenance.
- **`GET /project/:id/agent/:addr`**: Returns agent Artifacts repository and commit log.

---

## Workspace Test Totals

The entire S3A storage and version control workspace is **100% green**:

| Crate / Test Target | Tests Passed | Status |
| :--- | :---: | :---: |
| **`s3a-swarm`** (concurrency, merge, grants, eval) | **5** | Pass |
| **`s3a-vcs`** (commit DAG, branches, union merge, git export) | **13** | Pass |
| **`s3a-vault`** (identities, gate addresses, delegation, rotation) | **9** | Pass |
| **`s3a-harness`** (decision layer, ask_human pause/resume, eval) | **55** | Pass |
| **`s3a-engine`** (hyper-tile read/write, RS parity, compactor, sealed) | **83** | Pass |
| **`s3a-spec-vectors`** (conformance test vectors) | **32** | Pass |
| **`s3a-simd`** (SIMD filtering, metric distance, DOP14) | **13** | Pass |
| **`s3a-wasm-snapshot`** (WASM delta export, xAPI verb registry) | **10** | Pass |
| **`s3a-pack-scholar`, `da`, `gis`, `robotics`, `wearable`, `ontology`** | **180** | Pass |
| **Total Workspace Tests** | **400** | **ALL PASSED (0 Failed)** |

---

## Exact Run Commands Summary

### 1. Run the Local 1000-Agent Concurrency Benchmark
```bash
cargo run --release --bin s3a-swarm -- run --agents 1000 --commits 5 --out demo/swarm_repo.s3a --results demo/results.json
```

### 2. Verify Every Commit Cryptographic Signature
```bash
cargo run --release --bin s3a-swarm -- verify demo/swarm_repo.s3a
```

### 3. List Pending Human Supervisor Conflicts
```bash
cargo run --release --bin s3a-swarm -- human list
```

### 4. Resolve an Epistemic Conflict with a Signed Human Resolution
```bash
cargo run --release --bin s3a-swarm -- human resolve conflict_001 --choice target --rationale "Empirical replicate confirmed finding"
```

### 5. Run with Automated Shadow Branch Evaluation
```bash
cargo run --release --bin s3a-swarm -- run --agents 100 --commits 5 --shadow --out demo/shadow_repo.s3a --results demo/shadow_results.json
```

### 6. Run Workspace Test Suite
```bash
cargo test --workspace
```
