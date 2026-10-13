# S3A Cloudflare Worker + Durable Object Sketch

Free-tier serverless coordinator architecture for S3A Git-Style Version Control using Cloudflare Workers, Durable Objects, and R2 object storage.

## Architecture

```
                                      +-------------------------------+
                                      |    1000 Autonomous Agents     |
                                      +-------------------------------+
                                                      |
                                                      | Concurrent Signed Pushes (Ed25519)
                                                      v
+---------------------------------------------------------------------------------------------------+
| Cloudflare Global Edge Network                                                                    |
|                                                                                                   |
|  [ Worker Router ]                                                                                |
|         |                                                                                         |
|         +---> Durable Object: RepoCoordinatorDO (Single-writer per repository)                    |
|                    |                                                                              |
|                    |-- 1. Evaluates Scoped Delegation Grants (rejects unauthorized writes)        |
|                    |-- 2. Detects Epistemic Conflicts (routes claims to ask_human queue)          |
|                    |-- 3. Merges Disjoint Hyper-Tiles as Set Union (zero locks/rebase storms)     |
|                    |-- 4. Deterministic Same-Record Merge (LWW timestamp + Ed25519 tie-break)     |
|                    |                                                                              |
|                    v                                                                              |
|  [ Cloudflare R2 Storage ] (Stores 128 KB S3A Hyper-Tiles: tiles/{repo_id}/{tile_hash})          |
+---------------------------------------------------------------------------------------------------+
```

## Free Tier Eligibility

- **Cloudflare Workers**: 100,000 requests/day free.
- **Durable Objects**: Standard pricing tier included.
- **R2 Storage**: 10 GB storage, 1M Class A operations (write), 10M Class B operations (read) free per month.
- **Zero Paid Services Required**.

## Endpoints

1. `POST /repos/:repo_id/push`: Receives signed commits, validates delegation grants, updates head seal.
2. `GET /repos/:repo_id/conflicts`: Lists pending epistemic/empirical conflicts requiring human review.
3. `POST /repos/:repo_id/resolve`: Submits signed `VcsResolutionRecord` from a human supervisor.
4. `GET /repos/:repo_id/tiles/:tile_hash`: Fetches raw 128 KB Hyper-Tiles from R2 with edge caching.

## Local Development & Deployment

```bash
# Install wrangler (if needed)
npm install

# Test locally with Cloudflare Miniflare
npx wrangler dev

# Deploy to Cloudflare
npx wrangler deploy
```
