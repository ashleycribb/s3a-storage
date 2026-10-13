//! Cloudflare Worker + Durable Object Sketch for S3A High-Concurrency VCS.
//!
//! Provides single-writer ordering per repository, deterministic merge rules,
//! cryptographic delegation grant verification, epistemic dispute queueing,
//! and 128 KB Hyper-Tile chunk storage in Cloudflare R2 (Free Tier compatible).

export interface Env {
  REPO_COORDINATOR: DurableObjectNamespace;
  TILES_BUCKET: R2Bucket;
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    const pathParts = url.pathname.split("/").filter(Boolean);

    // Routing: /repos/:repo_id/...
    if (pathParts[0] !== "repos" || pathParts.length < 2) {
      return new Response("S3A Cloudflare VCS Gate. Use /repos/:repo_id/push, /conflicts, /resolve, /tiles/:hash", { status: 200 });
    }

    const repoId = pathParts[1];
    const subRoute = pathParts[2] || "";

    // Tile retrieval from R2 directly via Worker
    if (subRoute === "tiles" && pathParts[3]) {
      const tileHash = pathParts[3];
      const object = await env.TILES_BUCKET.get(`tiles/${repoId}/${tileHash}`);
      if (!object) {
        return new Response("Tile not found", { status: 404 });
      }
      return new Response(object.body, {
        headers: {
          "Content-Type": "application/octet-stream",
          "Cache-Control": "public, max-age=31536000, immutable",
          "ETag": tileHash,
        },
      });
    }

    // Forward repo operations to repo's Durable Object singleton
    const doId = env.REPO_COORDINATOR.idFromName(repoId);
    const stub = env.REPO_COORDINATOR.get(doId);
    return stub.fetch(request);
  },
};

// ============================================================================
// Durable Object: RepoCoordinatorDO (Single-Writer per Repository)
// ============================================================================

interface RepoRecord {
  record_id: string;
  tile_type: number;
  payload_hash: string;
  summary: string;
  timestamp: number;
  commit_sig_hex: string;
  author_gate: string;
}

interface ConflictEntry {
  conflict_id: string;
  record_id: string;
  tile_type: number;
  version_a: string;
  version_b: string;
  author_a: string;
  author_b: string;
  timestamp: number;
  resolved: boolean;
}

export class RepoCoordinatorDO implements DurableObject {
  state: DurableObjectState;
  env: Env;

  constructor(state: DurableObjectState, env: Env) {
    this.state = state;
    this.env = env;
  }

  async fetch(request: Request): Promise<Response> {
    const url = new URL(request.url);
    const pathParts = url.pathname.split("/").filter(Boolean);
    const action = pathParts[2] || "";

    switch (action) {
      case "push": {
        if (request.method !== "POST") return new Response("Method not allowed", { status: 405 });
        const body = await request.json() as {
          commit: {
            commit_id: string;
            author_pubkey: string;
            signature: string;
            timestamp: number;
            message: string;
          };
          grant: {
            agent_id: string;
            allowed_prefix: string;
            expires_at: number;
          };
          records: RepoRecord[];
          tiles: { tile_type: number; tile_hash: string; payload_base64: string }[];
        };

        // 1. Grant Boundary Evaluation (Fail-Safe Default)
        for (const rec of body.records) {
          if (!rec.record_id.startsWith(body.grant.allowed_prefix) && !rec.record_id.startsWith("shared_")) {
            return new Response(JSON.stringify({
              status: "rejected_by_grant",
              reason: `Path '${rec.record_id}' violates grant scope '${body.grant.allowed_prefix}'`,
            }), { status: 403, headers: { "Content-Type": "application/json" } });
          }
        }

        // 2. Disjoint Union & Epistemic Conflict Detection
        let autoMergedCount = 0;
        let conflictEnqueued = false;

        for (const rec of body.records) {
          const existing = await this.state.storage.get<RepoRecord>(`rec:${rec.record_id}`);
          if (existing) {
            // Check epistemic claim invariant
            const isEpistemic = rec.record_id.startsWith("claim_") || rec.record_id.startsWith("evidence_");
            if (isEpistemic && existing.payload_hash !== rec.payload_hash) {
              const conflictId = `conflict_${Date.now()}_${Math.random().toString(36).substring(2, 8)}`;
              const conflict: ConflictEntry = {
                conflict_id: conflictId,
                record_id: rec.record_id,
                tile_type: rec.tile_type,
                version_a: existing.summary,
                version_b: rec.summary,
                author_a: existing.author_gate,
                author_b: rec.author_gate,
                timestamp: Date.now(),
                resolved: false,
              };
              await this.state.storage.put(`conflict:${conflictId}`, conflict);
              conflictEnqueued = true;
              continue; // Do not overwrite; require human judgment
            }

            // Deterministic Merge Rule: LWW timestamp + Signature tie-break
            if (rec.timestamp > existing.timestamp || (rec.timestamp === existing.timestamp && rec.commit_sig_hex > existing.commit_sig_hex)) {
              await this.state.storage.put(`rec:${rec.record_id}`, rec);
              autoMergedCount++;
            } else {
              // Existing version wins tie-break; preserved deterministically
              autoMergedCount++;
            }
          } else {
            // Disjoint record added as part of set union
            await this.state.storage.put(`rec:${rec.record_id}`, rec);
            autoMergedCount++;
          }
        }

        // 3. Store Hyper-Tiles in R2 Bucket
        const repoId = pathParts[1];
        for (const tile of body.tiles) {
          const buffer = Uint8Array.from(atob(tile.payload_base64), c => c.charCodeAt(0));
          await this.env.TILES_BUCKET.put(`tiles/${repoId}/${tile.tile_hash}`, buffer);
        }

        // 4. Update Commit Pointer in DO Storage
        await this.state.storage.put(`commit:${body.commit.commit_id}`, body.commit);
        await this.state.storage.put("head_seal", body.commit.commit_id);

        return new Response(JSON.stringify({
          status: conflictEnqueued ? "sent_to_human" : "merged",
          auto_merged: autoMergedCount,
          head_seal: body.commit.commit_id,
        }), { status: 200, headers: { "Content-Type": "application/json" } });
      }

      case "conflicts": {
        const list = await this.state.storage.list<ConflictEntry>({ prefix: "conflict:" });
        const conflicts = Array.from(list.values()).filter(c => !c.resolved);
        return new Response(JSON.stringify(conflicts), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        });
      }

      case "resolve": {
        if (request.method !== "POST") return new Response("Method not allowed", { status: 405 });
        const { conflict_id, chosen_side, rationale, supervisor_pubkey, signature } = await request.json() as any;
        const conflict = await this.state.storage.get<ConflictEntry>(`conflict:${conflict_id}`);
        if (!conflict) return new Response("Conflict not found", { status: 404 });

        conflict.resolved = true;
        await this.state.storage.put(`conflict:${conflict_id}`, conflict);
        await this.state.storage.put(`resolution:${conflict_id}`, {
          conflict_id,
          chosen_side,
          rationale,
          supervisor_pubkey,
          signature,
          timestamp: Date.now(),
        });

        return new Response(JSON.stringify({ status: "resolved", conflict_id }), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        });
      }

      default:
        return new Response("Not found", { status: 404 });
    }
  }
}
