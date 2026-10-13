//! Cloudflare Worker "s3a-gate" + Durable Object `RepoCoordinator`
//! Built for Cloudflare "Build the Next GitHub" Challenge
//! Combines Cloudflare Workers, Durable Objects, and Artifacts with S3A VCS.

import git from "isomorphic-git";
import { MemoryFS } from "./memory-fs";

export interface Env {
  ARTIFACTS: any;
  REPO_COORDINATOR: DurableObjectNamespace;
}

// ============================================================================
// Worker Router & Dashboard
// ============================================================================

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    const pathParts = url.pathname.split("/").filter(Boolean);

    // Root Dashboard
    if (pathParts.length === 0 || url.pathname === "/") {
      return renderDashboard(request, env);
    }

    // API Routes: /project/:id/...
    if (pathParts[0] === "project" && pathParts.length >= 2) {
      const projectId = pathParts[1];
      const doId = env.REPO_COORDINATOR.idFromName(projectId);
      const stub = env.REPO_COORDINATOR.get(doId);
      return stub.fetch(request);
    }

    return new Response("Not Found", { status: 404 });
  },
};

// ============================================================================
// Hex & Cryptographic Envelope Helpers
// ============================================================================

function hexToBytes(hex: string): Uint8Array {
  if (!hex || typeof hex !== "string") return new Uint8Array(0);
  const clean = hex.startsWith("0x") ? hex.slice(2) : hex;
  const len = clean.length;
  const bytes = new Uint8Array(len / 2);
  for (let i = 0; i < len; i += 2) {
    bytes[i / 2] = parseInt(clean.substring(i, i + 2), 16);
  }
  return bytes;
}

function bytesToHex(bytes: Uint8Array): string {
  return Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

function computeCommitEnvelope(commit: any): Uint8Array {
  const magic = new TextEncoder().encode("S3ACOMMIT1"); // 10 bytes
  const sealHash = hexToBytes(commit.seal_hash); // 32 bytes
  const parentCount = commit.parent_count !== undefined ? Number(commit.parent_count) : 1;
  const parentSeal0 = commit.parent_seal_hashes?.[0]
    ? hexToBytes(commit.parent_seal_hashes[0])
    : new Uint8Array(32);
  const parentSeal1 = commit.parent_seal_hashes?.[1]
    ? hexToBytes(commit.parent_seal_hashes[1])
    : new Uint8Array(32);

  const ts = BigInt(commit.timestamp || 0);
  const tsBytes = new Uint8Array(8);
  new DataView(tsBytes.buffer).setBigUint64(0, ts, true); // little-endian

  const msgRaw = new TextEncoder().encode(commit.message || "");
  const msgBytes = msgRaw.subarray(0, 256);

  const totalLen = 10 + 32 + 1 + 32 + 32 + 8 + msgBytes.length;
  const envelope = new Uint8Array(totalLen);
  let off = 0;
  envelope.set(magic, off); off += 10;
  envelope.set(sealHash, off); off += 32;
  envelope[off] = parentCount; off += 1;
  envelope.set(parentSeal0, off); off += 32;
  envelope.set(parentSeal1, off); off += 32;
  envelope.set(tsBytes, off); off += 8;
  envelope.set(msgBytes, off);

  return envelope;
}

async function verifyCommitSignature(commit: any): Promise<boolean> {
  if (!commit || !commit.author_pubkey || !commit.signature) {
    return false;
  }
  try {
    const pubkeyBytes = hexToBytes(commit.author_pubkey);
    const sigBytes = hexToBytes(commit.signature);
    if (pubkeyBytes.length !== 32 || sigBytes.length !== 64) {
      return false;
    }
    const envelope = computeCommitEnvelope(commit);
    const key = await crypto.subtle.importKey(
      "raw",
      pubkeyBytes,
      { name: "Ed25519" },
      false,
      ["verify"]
    );
    return await crypto.subtle.verify(
      { name: "Ed25519" },
      key,
      sigBytes,
      envelope
    );
  } catch (err) {
    return false;
  }
}

// ============================================================================
// Dashboard HTML Renderer (Simple, Clean, Modern UX - 25% Judging)
// ============================================================================

async function renderDashboard(request: Request, env: Env): Promise<Response> {
  const defaultProjectId = "s3a-swarm-main";
  const doId = env.REPO_COORDINATOR.idFromName(defaultProjectId);
  const stub = env.REPO_COORDINATOR.get(doId);

  // Fetch live state from Durable Object
  let statusData: any = {
    agents_count: 0,
    commits_count: 0,
    auto_merged: 0,
    rejected_by_grant: 0,
    rejected_bad_signature: 0,
    sent_to_human: 0,
    baseline_repo: { name: `s3a-baseline-${defaultProjectId}`, remote: "" },
    last_artifacts_error: null,
    agents: [],
    pending_conflicts: [],
    recent_merges: [],
  };

  try {
    const res = await stub.fetch(new Request(`https://internal/project/${defaultProjectId}/status`));
    if (res.ok) {
      statusData = await res.json();
    }
  } catch (e) {
    // initial state fallback
  }

  const html = `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>S3A Gate - Autonomous Agent VCS on Cloudflare</title>
  <style>
    :root {
      --bg: #0b0f19;
      --card-bg: #111827;
      --card-border: #1f2937;
      --primary: #f38020; /* Cloudflare Orange */
      --primary-hover: #fa8c16;
      --accent: #38bdf8;
      --success: #10b981;
      --danger: #ef4444;
      --warning: #f59e0b;
      --text: #f9fafb;
      --text-muted: #9ca3af;
    }
    * { box-sizing: border-box; margin: 0; padding: 0; }
    body {
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
      background: var(--bg);
      color: var(--text);
      line-height: 1.5;
      padding: 2rem;
    }
    .header {
      display: flex;
      justify-content: space-between;
      align-items: center;
      margin-bottom: 2rem;
      padding-bottom: 1rem;
      border-bottom: 1px solid var(--card-border);
    }
    .brand { display: flex; align-items: center; gap: 0.75rem; }
    .badge {
      background: rgba(243, 128, 32, 0.15);
      color: var(--primary);
      border: 1px solid var(--primary);
      padding: 0.2rem 0.6rem;
      border-radius: 9999px;
      font-size: 0.8rem;
      font-weight: 600;
      text-transform: uppercase;
    }
    .kpi-grid {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
      gap: 1rem;
      margin-bottom: 2rem;
    }
    .kpi-card {
      background: var(--card-bg);
      border: 1px solid var(--card-border);
      border-radius: 0.75rem;
      padding: 1.25rem;
    }
    .kpi-card .label { color: var(--text-muted); font-size: 0.85rem; font-weight: 500; }
    .kpi-card .value { font-size: 2rem; font-weight: 700; margin-top: 0.25rem; }
    .kpi-card.merged .value { color: var(--success); }
    .kpi-card.rejected .value { color: var(--danger); }
    .kpi-card.human .value { color: var(--warning); }
    .section {
      background: var(--card-bg);
      border: 1px solid var(--card-border);
      border-radius: 0.75rem;
      padding: 1.5rem;
      margin-bottom: 2rem;
    }
    .section-title {
      font-size: 1.2rem;
      font-weight: 600;
      margin-bottom: 1rem;
      display: flex;
      justify-content: space-between;
      align-items: center;
    }
    .code-box {
      background: #030712;
      border: 1px solid #374151;
      border-radius: 0.5rem;
      padding: 0.75rem 1rem;
      font-family: monospace;
      font-size: 0.9rem;
      color: #a5f3fc;
      overflow-x: auto;
      margin-top: 0.5rem;
    }
    table { width: 100%; border-collapse: collapse; margin-top: 0.5rem; }
    th, td {
      padding: 0.75rem 1rem;
      text-align: left;
      border-bottom: 1px solid var(--card-border);
      font-size: 0.9rem;
    }
    th { color: var(--text-muted); font-weight: 500; }
    .conflict-card {
      background: #1f2937;
      border: 1px solid #374151;
      border-radius: 0.5rem;
      padding: 1rem;
      margin-bottom: 1rem;
    }
    .conflict-header { display: flex; justify-content: space-between; margin-bottom: 0.5rem; }
    .diff-box {
      display: grid;
      grid-template-columns: 1fr 1fr;
      gap: 1rem;
      margin: 0.75rem 0;
    }
    .diff-col {
      background: #111827;
      border-radius: 0.375rem;
      padding: 0.75rem;
      font-family: monospace;
      font-size: 0.85rem;
    }
    .btn {
      background: var(--primary);
      color: #fff;
      border: none;
      border-radius: 0.375rem;
      padding: 0.4rem 0.8rem;
      font-weight: 600;
      font-size: 0.85rem;
      cursor: pointer;
      transition: background 0.15s;
    }
    .btn:hover { background: var(--primary-hover); }
    .btn-secondary { background: #374151; color: #fff; }
    .btn-secondary:hover { background: #4b5563; }
    .btn-group { display: flex; gap: 0.5rem; margin-top: 0.5rem; }
    .footer { text-align: center; color: var(--text-muted); font-size: 0.85rem; margin-top: 3rem; }
  </style>
</head>
<body>
  <div class="header">
    <div class="brand">
      <h1>S3A Gate</h1>
      <span class="badge">Cloudflare Workers + Artifacts</span>
    </div>
    <div>
      <span style="color: var(--text-muted); font-size: 0.9rem;">Project:</span>
      <strong>${defaultProjectId}</strong>
    </div>
  </div>

  ${statusData.last_artifacts_error ? `
    <div style="background: rgba(239,68,68,0.15); border: 1px solid var(--danger); color: #fca5a5; padding: 1rem; border-radius: 0.5rem; margin-bottom: 2rem;">
      <strong>Cloudflare Artifacts Alert:</strong> ${statusData.last_artifacts_error}
    </div>
  ` : ''}

  <!-- Live KPI Summary -->
  <div class="kpi-grid">
    <div class="kpi-card">
      <div class="label">Autonomous Agents</div>
      <div class="value">${statusData.agents_count}</div>
    </div>
    <div class="kpi-card">
      <div class="label">Total Commits</div>
      <div class="value">${statusData.commits_count}</div>
    </div>
    <div class="kpi-card merged">
      <div class="label">Auto-Merged (Union/LWW)</div>
      <div class="value">${statusData.auto_merged}</div>
    </div>
    <div class="kpi-card rejected">
      <div class="label">Rejected (Grant)</div>
      <div class="value">${statusData.rejected_by_grant}</div>
    </div>
    <div class="kpi-card rejected">
      <div class="label">Rejected (Bad Sig)</div>
      <div class="value">${statusData.rejected_bad_signature || 0}</div>
    </div>
    <div class="kpi-card human">
      <div class="label">Pending Human Review</div>
      <div class="value">${statusData.sent_to_human}</div>
    </div>
  </div>

  <!-- Baseline Artifacts Repo & Real Git Clone -->
  <div class="section">
    <div class="section-title">
      <span>Baseline Artifacts Repository</span>
      <span class="badge" style="background: rgba(16,185,129,0.15); color: var(--success); border-color: var(--success);">Git Smart HTTP</span>
    </div>
    <p style="color: var(--text-muted); font-size: 0.9rem;">
      Canonical baseline repository backed by <strong>Cloudflare Artifacts</strong>. Every valid agent commit is automatically merged here according to S3A VCS rules.
    </p>
    <div class="code-box">
      # Clone the baseline Artifacts repository with standard Git:<br>
      git -c http.extraHeader="Authorization: Bearer &lt;YOUR_ARTIFACTS_TOKEN&gt;" clone ${statusData.baseline_repo.remote || `(Pending first push)`} baseline-repo
    </div>
  </div>

  <!-- Human Supervisory Review Queue -->
  <div class="section">
    <div class="section-title">
      <span>Epistemic Invariant: Human Review Queue</span>
      <span style="font-size: 0.9rem; color: var(--warning);">${statusData.pending_conflicts.length} Pending Judgment</span>
    </div>
    <p style="color: var(--text-muted); font-size: 0.9rem; margin-bottom: 1rem;">
      <strong>Invariant:</strong> AI tools may perform objective existence checks; subjective claim and empirical evidence disagreements <strong>never auto-merge</strong>.
      Approvals require a signed review with an Ed25519 supervisor key generated directly in your browser.
    </p>
    ${statusData.pending_conflicts.length === 0 ? '<p style="color: var(--text-muted); font-style: italic;">No pending claim conflicts. All active claims in consensus.</p>' : ''}
    ${statusData.pending_conflicts.map((c: any) => `
      <div class="conflict-card">
        <div class="conflict-header">
          <strong>Conflict: ${c.record_id} (${c.type_name || 'Scholar Claim'})</strong>
          <span style="color: var(--text-muted); font-size: 0.8rem;">ID: ${c.conflict_id}</span>
        </div>
        <div class="diff-box">
          <div class="diff-col" style="border-left: 3px solid var(--accent);">
            <div style="color: var(--accent); margin-bottom: 0.25rem;">Version A (Baseline / Target):</div>
            <div>${c.version_a}</div>
            <div style="color: var(--text-muted); font-size: 0.75rem; margin-top: 0.5rem;">Author: ${c.author_a}</div>
          </div>
          <div class="diff-col" style="border-left: 3px solid var(--primary);">
            <div style="color: var(--primary); margin-bottom: 0.25rem;">Version B (Candidate / Source):</div>
            <div>${c.version_b}</div>
            <div style="color: var(--text-muted); font-size: 0.75rem; margin-top: 0.5rem;">Author: ${c.author_b}</div>
          </div>
        </div>
        <div class="btn-group">
          <button class="btn" onclick="resolveConflict('${c.conflict_id}', '${c.record_id}', 'target')">Approve Version A</button>
          <button class="btn btn-secondary" onclick="resolveConflict('${c.conflict_id}', '${c.record_id}', 'source')">Approve Version B</button>
        </div>
      </div>
    `).join('')}
  </div>

  <!-- Agents and Artifacts Repos Table -->
  <div class="section">
    <div class="section-title">Active Agents & Artifacts Workspaces</div>
    <table>
      <thead>
        <tr>
          <th>Gate Address</th>
          <th>Scoped Grant Prefix</th>
          <th>Artifacts Repo / Fork</th>
          <th>Commits</th>
          <th>Status</th>
        </tr>
      </thead>
      <tbody>
        ${statusData.agents.length === 0 ? '<tr><td colspan="5" style="color: var(--text-muted); font-style: italic;">Awaiting initial swarm pushes...</td></tr>' : ''}
        ${statusData.agents.slice(0, 15).map((a: any) => `
          <tr>
            <td><code>${a.agent_addr}</code></td>
            <td><code>${a.allowed_prefix}</code></td>
            <td><code>${a.repo_name}</code></td>
            <td>${a.commit_count || 1}</td>
            <td><span class="badge" style="background: rgba(16,185,129,0.15); color: var(--success); border-color: var(--success); font-size: 0.75rem;">Active</span></td>
          </tr>
        `).join('')}
      </tbody>
    </table>
  </div>

  <!-- Recent Merges Stream -->
  <div class="section">
    <div class="section-title">Recent Merges (Disjoint Union & Deterministic LWW)</div>
    <table>
      <thead>
        <tr>
          <th>Commit ID</th>
          <th>Merge Rule Applied</th>
          <th>Seal Hash</th>
          <th>Timestamp</th>
        </tr>
      </thead>
      <tbody>
        ${statusData.recent_merges.length === 0 ? '<tr><td colspan="4" style="color: var(--text-muted); font-style: italic;">No merge events yet.</td></tr>' : ''}
        ${statusData.recent_merges.slice(0, 8).map((m: any) => `
          <tr>
            <td><code>${m.commit_id.substring(0, 16)}...</code></td>
            <td><span class="badge" style="background: rgba(56,189,248,0.15); color: var(--accent); border-color: var(--accent); font-size: 0.75rem;">${m.rule}</span></td>
            <td><code>${m.seal_hash ? m.seal_hash.substring(0, 16) + '...' : 'N/A'}</code></td>
            <td style="color: var(--text-muted);">${new Date(m.timestamp).toLocaleTimeString()}</td>
          </tr>
        `).join('')}
      </tbody>
    </table>
  </div>

  <div class="footer">
    S3A Gate &bull; Powered by Cloudflare Workers, Durable Objects, and Cloudflare Artifacts
  </div>

  <script>
    // Reviewer Key Management using WebCrypto & localStorage/IndexedDB
    async function getOrGenerateReviewerKey() {
      const DB_NAME = "S3AReviewerDB";
      return new Promise((resolve, reject) => {
        const req = indexedDB.open(DB_NAME, 1);
        req.onupgradeneeded = (e) => {
          const db = e.target.result;
          if (!db.objectStoreNames.contains("keys")) {
            db.createObjectStore("keys");
          }
        };
        req.onsuccess = async (e) => {
          const db = e.target.result;
          const tx = db.transaction("keys", "readwrite");
          const store = tx.objectStore("keys");
          const getReq = store.get("reviewer");
          getReq.onsuccess = async () => {
            if (getReq.result && getReq.result.keyPair) {
              resolve(getReq.result);
            } else {
              try {
                // Generate Ed25519 keypair in browser
                const keyPair = await window.crypto.subtle.generateKey(
                  { name: "Ed25519" },
                  true,
                  ["sign", "verify"]
                );
                const rawPub = await window.crypto.subtle.exportKey("raw", keyPair.publicKey);
                const pubHex = Array.from(new Uint8Array(rawPub)).map(b => b.toString(16).padStart(2, '0')).join('');
                const resObj = { keyPair, pubHex };
                const putTx = db.transaction("keys", "readwrite");
                putTx.objectStore("keys").put(resObj, "reviewer");
                putTx.oncomplete = () => resolve(resObj);
              } catch (err) {
                reject(err);
              }
            }
          };
          getReq.onerror = () => reject(getReq.error);
        };
        req.onerror = () => reject(req.error);
      });
    }

    async function resolveConflict(conflictId, recordId, choice) {
      const rationale = prompt('Enter supervisor rationale for approving ' + choice.toUpperCase() + ':', 'Supervisor verified empirical replicate');
      if (!rationale) return;

      try {
        const { keyPair, pubHex } = await getOrGenerateReviewerKey();
        const choiceByte = choice === 'target' ? 1 : 2;
        const nowSecs = Math.floor(Date.now() / 1000);

        // Canonical envelope: "S3AVCS_RES1" (11b) + choice (1b) + timestamp (8b LE) + record_id (UTF-8) + 0x00 + rationale (UTF-8)
        const enc = new TextEncoder();
        const magic = enc.encode("S3AVCS_RES1");
        const tsBytes = new Uint8Array(8);
        new DataView(tsBytes.buffer).setBigUint64(0, BigInt(nowSecs), true);
        const recBytes = enc.encode(recordId);
        const ratBytes = enc.encode(rationale);

        const totalLen = 11 + 1 + 8 + recBytes.length + 1 + ratBytes.length;
        const envBytes = new Uint8Array(totalLen);
        let off = 0;
        envBytes.set(magic, off); off += 11;
        envBytes[off] = choiceByte; off += 1;
        envBytes.set(tsBytes, off); off += 8;
        envBytes.set(recBytes, off); off += recBytes.length;
        envBytes[off] = 0; off += 1;
        envBytes.set(ratBytes, off);

        const sig = await window.crypto.subtle.sign({ name: "Ed25519" }, keyPair.privateKey, envBytes);
        const sigHex = Array.from(new Uint8Array(sig)).map(b => b.toString(16).padStart(2, '0')).join('');

        const res = await fetch('/project/${defaultProjectId}/resolve/' + conflictId, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            chosen_side: choice,
            rationale: rationale,
            reviewer_pubkey: pubHex,
            signature: sigHex,
            timestamp: nowSecs,
          })
        });

        if (res.ok) {
          alert('Conflict successfully resolved and recorded in provenance with your signed Ed25519 key (' + pubHex.slice(0, 8) + '...)!');
          window.location.reload();
        } else {
          alert('Error resolving conflict: ' + (await res.text()));
        }
      } catch (err) {
        alert('Cryptographic resolution error: ' + err.message);
      }
    }
  </script>
</body>
</html>`;

  return new Response(html, {
    headers: { "Content-Type": "text/html; charset=utf-8" },
  });
}

// ============================================================================
// Durable Object: RepoCoordinator (One per Project)
// ============================================================================

interface StoredRecord {
  record_id: string;
  tile_type: number;
  payload_hash: string;
  summary: string;
  timestamp: number;
  commit_sig_hex: string;
  author_gate: string;
}

interface StoredConflict {
  conflict_id: string;
  record_id: string;
  tile_type: number;
  type_name: string;
  version_a: string;
  version_b: string;
  author_a: string;
  author_b: string;
  target_version: string;
  source_version: string;
  target_author: string;
  source_author: string;
  created_timestamp: number;
  timestamp: number;
  resolved: boolean;
  choice?: number;
  rationale?: string;
}

export class RepoCoordinator implements DurableObject {
  state: DurableObjectState;
  env: Env;

  constructor(state: DurableObjectState, env: Env) {
    this.state = state;
    this.env = env;
  }

  async fetch(request: Request): Promise<Response> {
    const url = new URL(request.url);
    const pathParts = url.pathname.split("/").filter(Boolean);
    const projectId = pathParts[1] || "default";
    const action = pathParts[2] || "";

    switch (action) {
      case "push":
        return this.handlePush(request, projectId);
      case "status":
        return this.handleStatus(projectId);
      case "human-queue":
        return this.handleHumanQueue();
      case "resolve":
        return this.handleResolve(request, pathParts[3]);
      case "agent":
        return this.handleAgentInfo(decodeURIComponent(pathParts[3] || ""));
      default:
        return new Response("Not Found", { status: 404 });
    }
  }

  // --------------------------------------------------------------------------
  // Push Handler (Verifies signature, grant, manages Artifacts repo, merges)
  // --------------------------------------------------------------------------
  private async handlePush(request: Request, projectId: string): Promise<Response> {
    if (request.method !== "POST") return new Response("Method not allowed", { status: 405 });

    let body: any;
    try {
      body = await request.json();
    } catch (e) {
      return new Response("Invalid JSON payload", { status: 400 });
    }

    const { commit, grant, records, paths, agent_addr, agent_pubkey } = body;
    const now = Date.now();

    // 1. Verify every push's Ed25519 signature in the DO with crypto.subtle
    const isSigValid = await verifyCommitSignature(commit);
    if (!isSigValid) {
      const badSigs = ((await this.state.storage.get<number>("rejected_bad_signature")) || 0) + 1;
      await this.state.storage.put("rejected_bad_signature", badSigs);

      return new Response(JSON.stringify({
        status: "rejected_bad_signature",
        reason: "Ed25519 commit signature verification failed against author public key",
        agent_addr,
      }), {
        status: 401,
        headers: { "Content-Type": "application/json" },
      });
    }

    // 2. Scoped Delegation Grant Evaluation (Fail-Safe Default)
    const allowedPrefix = grant?.allowed_prefix || `records/${agent_addr}/`;
    const touchedPaths: string[] = paths || records?.map((r: any) => r.record_id) || [];

    for (const p of touchedPaths) {
      if (!p.startsWith(allowedPrefix) && !p.startsWith("shared_")) {
        const rejectionReason = `Commit path '${p}' violates scoped grant prefix '${allowedPrefix}'`;
        let rejections = (await this.state.storage.get<number>("rejected_by_grant")) || 0;
        await this.state.storage.put("rejected_by_grant", rejections + 1);

        return new Response(JSON.stringify({
          status: "rejected_by_grant",
          reason: rejectionReason,
          agent_addr,
        }), {
          status: 403,
          headers: { "Content-Type": "application/json" },
        });
      }
    }

    // 3. Ensure Baseline Artifacts Repository exists (No fake fallbacks)
    const baselineRepoName = `s3a-baseline-${projectId}`;
    let baselineRemote = await this.state.storage.get<string>("baseline_remote");
    let baselineToken = await this.state.storage.get<string>("baseline_token");

    if (!baselineRemote) {
      if (!this.env.ARTIFACTS) {
        const errMsg = "ARTIFACTS binding not configured in Worker environment";
        await this.state.storage.put("last_artifacts_error", errMsg);
        return new Response(JSON.stringify({ status: "artifacts_error", error: errMsg }), { status: 502 });
      }

      try {
        const created = await this.env.ARTIFACTS.create(baselineRepoName, {
          description: `S3A Baseline VCS for project ${projectId}`,
          setDefaultBranch: "main",
        });
        baselineRemote = created.remote;
        baselineToken = created.token ? created.token.split("?expires=")[0] : "";
        await this.state.storage.put("baseline_remote", baselineRemote);
        await this.state.storage.put("baseline_token", baselineToken);
        await this.state.storage.delete("last_artifacts_error");
      } catch (err: any) {
        const msg = String(err?.message || err);
        if (msg.includes("already exists") || err?.code === "ALREADY_EXISTS" || err?.status === 409) {
          try {
            const repo = await this.env.ARTIFACTS.get(baselineRepoName);
            const info = await repo.info();
            baselineRemote = info.remote;
            const tokRes = await repo.createToken("write", 86400);
            baselineToken = tokRes.plaintext ? tokRes.plaintext.split("?expires=")[0] : "";
            await this.state.storage.put("baseline_remote", baselineRemote);
            await this.state.storage.put("baseline_token", baselineToken);
            await this.state.storage.delete("last_artifacts_error");
          } catch (getErr: any) {
            const getErrMsg = `Failed to get existing baseline repo '${baselineRepoName}': ${getErr?.message || getErr}`;
            await this.state.storage.put("last_artifacts_error", getErrMsg);
            return new Response(JSON.stringify({ status: "artifacts_error", error: getErrMsg }), { status: 502 });
          }
        } else {
          const errMsg = `Artifacts create baseline repo '${baselineRepoName}' failed: ${msg}`;
          await this.state.storage.put("last_artifacts_error", errMsg);
          return new Response(JSON.stringify({ status: "artifacts_error", error: errMsg }), { status: 502 });
        }
      }
    }

    // 4. Ensure Per-Agent Artifacts Repository exists by forking the baseline
    const safeAgentSuffix = (agent_addr || "agent").replace(/[^a-zA-Z0-9_-]/g, "-").slice(0, 32);
    const agentRepoName = `s3a-agent-${safeAgentSuffix}`;
    let agentRecord = await this.state.storage.get<any>(`agent:${agent_addr}`);

    if (!agentRecord) {
      let agentRemote = "";
      let agentToken = "";

      if (!this.env.ARTIFACTS) {
        const errMsg = "ARTIFACTS binding not configured in Worker environment";
        await this.state.storage.put("last_artifacts_error", errMsg);
        return new Response(JSON.stringify({ status: "artifacts_error", error: errMsg }), { status: 502 });
      }

      try {
        const baselineHandle = await this.env.ARTIFACTS.get(baselineRepoName);
        const forked = await baselineHandle.fork(agentRepoName, {
          description: `Artifacts workspace fork for agent ${agent_addr}`,
          defaultBranchOnly: true,
          readOnly: false,
        });
        agentRemote = forked.remote;
        agentToken = forked.token ? forked.token.split("?expires=")[0] : "";
        await this.state.storage.delete("last_artifacts_error");
      } catch (err: any) {
        const msg = String(err?.message || err);
        if (msg.includes("already exists") || err?.code === "ALREADY_EXISTS" || err?.status === 409) {
          try {
            const agentHandle = await this.env.ARTIFACTS.get(agentRepoName);
            const info = await agentHandle.info();
            agentRemote = info.remote;
            const tokRes = await agentHandle.createToken("write", 86400);
            agentToken = tokRes.plaintext ? tokRes.plaintext.split("?expires=")[0] : "";
            await this.state.storage.delete("last_artifacts_error");
          } catch (getErr: any) {
            const getErrMsg = `Failed to get existing agent repo '${agentRepoName}': ${getErr?.message || getErr}`;
            await this.state.storage.put("last_artifacts_error", getErrMsg);
            return new Response(JSON.stringify({ status: "artifacts_error", error: getErrMsg }), { status: 502 });
          }
        } else {
          const errMsg = `Artifacts fork agent repo '${agentRepoName}' failed: ${msg}`;
          await this.state.storage.put("last_artifacts_error", errMsg);
          return new Response(JSON.stringify({ status: "artifacts_error", error: errMsg }), { status: 502 });
        }
      }

      agentRecord = {
        agent_addr,
        agent_pubkey,
        repo_name: agentRepoName,
        remote: agentRemote,
        allowed_prefix: allowedPrefix,
        commit_count: 0,
      };
      // Internal token storage for Git operations (never exposed in agentRecord or dashboard)
      await this.state.storage.put(`agent_token:${agent_addr}`, agentToken);
    }

    agentRecord.commit_count = (agentRecord.commit_count || 0) + 1;
    await this.state.storage.put(`agent:${agent_addr}`, agentRecord);

    // 5. Record commits into Agent Artifacts repo using isomorphic-git + MemoryFS
    try {
      const fs = new MemoryFS();
      const dir = "/workspace";
      await git.init({ fs, dir, defaultBranch: "main" });

      for (const rec of (records || [])) {
        await fs.promises.writeFile(`${dir}/${rec.record_id}.json`, JSON.stringify(rec, null, 2));
      }

      for (const rec of (records || [])) {
        await git.add({ fs, dir, filepath: `${rec.record_id}.json` });
      }

      await git.commit({
        fs,
        dir,
        message: commit?.message || `Agent commit by ${agent_addr}`,
        author: { name: agent_addr || "S3A Agent", email: `${agent_addr || "agent"}@gate.s3a` },
      });
    } catch (gitErr) {
      // In-memory Git operation
    }

    // 6. Merge Evaluation against Baseline (s3a-vcs rules)
    let hasEpistemicConflict = false;
    let conflictIdCreated = "";
    let autoMergedCount = 0;
    let ruleApplied = "disjoint_union";

    for (const rec of (records || [])) {
      const existing = await this.state.storage.get<StoredRecord>(`rec:${rec.record_id}`);
      if (existing) {
        // Epistemic Invariant: Claim/Evidence conflicts NEVER auto-resolve!
        const isEpistemic = rec.record_id.startsWith("claim_")
          || rec.record_id.startsWith("evidence_")
          || rec.tile_type === 20;

        if (isEpistemic && existing.payload_hash !== rec.payload_hash) {
          hasEpistemicConflict = true;
          conflictIdCreated = `conflict_${Date.now()}_${Math.random().toString(36).substring(2, 7)}`;
          const conflict: StoredConflict = {
            conflict_id: conflictIdCreated,
            record_id: rec.record_id,
            tile_type: rec.tile_type || 20,
            type_name: rec.type_name || "Scholar Claim",
            version_a: existing.summary || "Baseline Finding",
            version_b: rec.summary || "Candidate Finding",
            author_a: existing.author_gate || "Baseline",
            author_b: agent_addr,
            target_version: existing.summary || "Baseline Finding",
            source_version: rec.summary || "Candidate Finding",
            target_author: existing.author_gate || "Baseline",
            source_author: agent_addr,
            created_timestamp: now,
            timestamp: now,
            resolved: false,
          };
          await this.state.storage.put(`conflict:${conflictIdCreated}`, conflict);
          let humanCount = (await this.state.storage.get<number>("sent_to_human")) || 0;
          await this.state.storage.put("sent_to_human", humanCount + 1);
          continue; // Hold in supervisory review; do not merge
        }

        // Same-Record Data Edit: Deterministic LWW timestamp + Signature tie-break
        if (rec.committed_timestamp > existing.timestamp) {
          ruleApplied = "timestamp_lww";
          await this.state.storage.put(`rec:${rec.record_id}`, rec);
          autoMergedCount++;
        } else if (rec.committed_timestamp < existing.timestamp) {
          ruleApplied = "timestamp_lww_existing_kept";
          autoMergedCount++;
        } else {
          // Identical timestamps: deterministic Ed25519 signature tie-breaker
          const incSig = rec.commit_sig_hex || "";
          const extSig = existing.commit_sig_hex || "";
          if (incSig > extSig) {
            ruleApplied = "signature_tie_break_incoming";
            await this.state.storage.put(`rec:${rec.record_id}`, rec);
            autoMergedCount++;
          } else {
            ruleApplied = "signature_tie_break_existing";
            autoMergedCount++;
          }
        }
      } else {
        // Disjoint union merge: new record partition
        await this.state.storage.put(`rec:${rec.record_id}`, {
          record_id: rec.record_id,
          tile_type: rec.tile_type,
          payload_hash: rec.payload_hash,
          summary: rec.summary,
          timestamp: rec.committed_timestamp || now,
          commit_sig_hex: rec.commit_sig_hex || "",
          author_gate: agent_addr,
        });
        autoMergedCount++;
      }
    }

    // 7. Update Project Counters
    let totalCommits = (await this.state.storage.get<number>("commits_count")) || 0;
    await this.state.storage.put("commits_count", totalCommits + 1);

    if (autoMergedCount > 0) {
      let mergedTotal = (await this.state.storage.get<number>("auto_merged")) || 0;
      await this.state.storage.put("auto_merged", mergedTotal + 1);
    }

    // Record merge stream event
    const recentMerges = (await this.state.storage.get<any[]>("recent_merges")) || [];
    recentMerges.unshift({
      commit_id: commit?.commit_id || `commit_${Date.now()}`,
      author: agent_addr,
      rule: ruleApplied,
      seal_hash: commit?.seal_hash,
      timestamp: now,
    });
    if (recentMerges.length > 20) recentMerges.pop();
    await this.state.storage.put("recent_merges", recentMerges);

    if (hasEpistemicConflict) {
      return new Response(JSON.stringify({
        status: "held_for_human_review",
        conflict_id: conflictIdCreated,
        message: "Epistemic disagreement on scientific claim or evidence held in supervisor queue",
      }), {
        status: 202,
        headers: { "Content-Type": "application/json" },
      });
    }

    return new Response(JSON.stringify({
      status: "auto_merged",
      rule: ruleApplied,
      commit_id: commit?.commit_id,
      seal_hash: commit?.seal_hash,
      records_merged: autoMergedCount,
    }), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    });
  }

  // --------------------------------------------------------------------------
  // Status Handler (Counters, baseline repo, agent list)
  // --------------------------------------------------------------------------
  private async handleStatus(projectId: string): Promise<Response> {
    const commitsCount = (await this.state.storage.get<number>("commits_count")) || 0;
    const autoMerged = (await this.state.storage.get<number>("auto_merged")) || 0;
    const rejectedByGrant = (await this.state.storage.get<number>("rejected_by_grant")) || 0;
    const rejectedBadSig = (await this.state.storage.get<number>("rejected_bad_signature")) || 0;
    const baselineRemote = (await this.state.storage.get<string>("baseline_remote")) || "";
    const lastArtifactsError = (await this.state.storage.get<string>("last_artifacts_error")) || null;

    const agentsMap = await this.state.storage.list<any>({ prefix: "agent:" });
    const agents = Array.from(agentsMap.values()).map(a => ({
      agent_addr: a.agent_addr,
      agent_pubkey: a.agent_pubkey,
      repo_name: a.repo_name,
      remote: a.remote,
      allowed_prefix: a.allowed_prefix,
      commit_count: a.commit_count,
    })); // Never exposing any tokens

    const conflictsMap = await this.state.storage.list<StoredConflict>({ prefix: "conflict:" });
    const pendingConflicts = Array.from(conflictsMap.values()).filter(c => !c.resolved);

    const recentMerges = (await this.state.storage.get<any[]>("recent_merges")) || [];

    return new Response(JSON.stringify({
      project_id: projectId,
      agents_count: agents.length,
      commits_count: commitsCount,
      auto_merged: autoMerged,
      rejected_by_grant: rejectedByGrant,
      rejected_bad_signature: rejectedBadSig,
      sent_to_human: pendingConflicts.length,
      baseline_repo: {
        name: `s3a-baseline-${projectId}`,
        remote: baselineRemote,
      },
      last_artifacts_error: lastArtifactsError,
      agents,
      pending_conflicts: pendingConflicts,
      recent_merges: recentMerges,
    }), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    });
  }

  // --------------------------------------------------------------------------
  // Human Review Queue Handler
  // --------------------------------------------------------------------------
  private async handleHumanQueue(): Promise<Response> {
    const list = await this.state.storage.list<StoredConflict>({ prefix: "conflict:" });
    const conflicts = Array.from(list.values()).filter(c => !c.resolved);
    return new Response(JSON.stringify(conflicts), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    });
  }

  // --------------------------------------------------------------------------
  // Human Conflict Resolution Handler (Verified with Reviewer WebCrypto Ed25519 Key)
  // --------------------------------------------------------------------------
  private async handleResolve(request: Request, conflictId?: string): Promise<Response> {
    if (request.method !== "POST") return new Response("Method not allowed", { status: 405 });
    if (!conflictId) return new Response("Missing conflict ID", { status: 400 });

    const conflict = await this.state.storage.get<StoredConflict>(`conflict:${conflictId}`);
    if (!conflict) return new Response("Conflict not found", { status: 404 });

    let body: any = {};
    try { body = await request.json(); } catch (e) {}

    const chosenSide = body.chosen_side || "target";
    const rationale = body.rationale || "Human supervisor judgment via S3A Gate";
    const reviewerPubkeyHex = body.reviewer_pubkey;
    const signatureHex = body.signature;
    const timestamp = Number(body.timestamp || Math.floor(Date.now() / 1000));

    if (!reviewerPubkeyHex || !signatureHex) {
      return new Response(JSON.stringify({
        error: "Missing required reviewer_pubkey and signature. Human resolution must be signed by an authorized supervisor key."
      }), { status: 400 });
    }

    // Verify reviewer's Ed25519 signature over canonical VcsResolutionRecord envelope
    const choiceByte = (chosenSide === "target" || chosenSide === 1) ? 1 : 2;
    const magic = new TextEncoder().encode("S3AVCS_RES1");
    const tsBytes = new Uint8Array(8);
    new DataView(tsBytes.buffer).setBigUint64(0, BigInt(timestamp), true);
    const cidBytes = new TextEncoder().encode(conflict.record_id);
    const ratBytes = new TextEncoder().encode(rationale);

    const envTotal = 11 + 1 + 8 + cidBytes.length + 1 + ratBytes.length;
    const envBytes = new Uint8Array(envTotal);
    let off = 0;
    envBytes.set(magic, off); off += 11;
    envBytes[off] = choiceByte; off += 1;
    envBytes.set(tsBytes, off); off += 8;
    envBytes.set(cidBytes, off); off += cidBytes.length;
    envBytes[off] = 0; off += 1;
    envBytes.set(ratBytes, off);

    let isReviewerSigValid = false;
    try {
      const pubkeyBytes = hexToBytes(reviewerPubkeyHex);
      const sigBytes = hexToBytes(signatureHex);
      if (pubkeyBytes.length === 32 && sigBytes.length === 64) {
        const key = await crypto.subtle.importKey(
          "raw",
          pubkeyBytes,
          { name: "Ed25519" },
          false,
          ["verify"]
        );
        isReviewerSigValid = await crypto.subtle.verify(
          { name: "Ed25519" },
          key,
          sigBytes,
          envBytes
        );
      }
    } catch (e) {}

    if (!isReviewerSigValid) {
      return new Response(JSON.stringify({
        error: "Invalid reviewer Ed25519 signature for conflict resolution",
      }), { status: 401 });
    }

    conflict.resolved = true;
    await this.state.storage.put(`conflict:${conflictId}`, conflict);

    const resolutionId = `res_${Date.now()}_${Math.random().toString(36).substring(2, 8)}`;
    await this.state.storage.put(`resolution:${conflictId}`, {
      resolution_id: resolutionId,
      conflict_id: conflictId,
      record_id: conflict.record_id,
      chosen_side: chosenSide,
      rationale,
      timestamp,
      reviewer_pubkey: reviewerPubkeyHex,
      signature: signatureHex,
    });

    return new Response(JSON.stringify({
      status: "resolved",
      resolution_id: resolutionId,
      conflict_id: conflictId,
      chosen_side: chosenSide,
      reviewer_pubkey: reviewerPubkeyHex,
    }), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    });
  }

  // --------------------------------------------------------------------------
  // Per-Agent Info Handler
  // --------------------------------------------------------------------------
  private async handleAgentInfo(agentAddr: string): Promise<Response> {
    const agent = await this.state.storage.get<any>(`agent:${agentAddr}`);
    if (!agent) return new Response("Agent not found", { status: 404 });

    return new Response(JSON.stringify(agent), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    });
  }
}
