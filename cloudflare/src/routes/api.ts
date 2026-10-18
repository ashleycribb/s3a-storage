// cloudflare/src/routes/api.ts
// API Endpoints: Health Checks, Diagnostics, Evidence Delivery, and DO Proxying

import type { Env } from "../types";
import { checkAdminAuth, isSupervisorKeyAllowed } from "../crypto";
import { getSandbox } from "../sandbox";
import { generateRehearsalEvidenceSvg } from "../rehearsal/visualEvidence";

const DEFAULT_PROJECT_ID = "s3a-swarm-main";

export async function handleAPIRoutes(request: Request, env: Env): Promise<Response | null> {
  const url = new URL(request.url);
  const path = url.pathname;
  const pathParts = path.split("/").filter(Boolean);

  // --------------------------------------------------------------------------
  // 1. GET /health: Health status check with zero token output
  // --------------------------------------------------------------------------
  if (path === "/health" && request.method === "GET") {
    const health: any = {
      status: "ok",
      timestamp: new Date().toISOString(),
      components: {},
    };

    // Durable Object (RepoCoordinator)
    try {
      const doId = env.REPO_COORDINATOR.idFromName(DEFAULT_PROJECT_ID);
      const stub = env.REPO_COORDINATOR.get(doId);
      const doRes = await stub.fetch("https://internal/project/" + DEFAULT_PROJECT_ID + "/control");
      if (doRes.ok) {
        const doData = await doRes.json() as any;
        health.components.durable_object = {
          ok: true,
          paused: doData.paused,
          status: doData.status,
        };
      } else {
        health.status = "degraded";
        health.components.durable_object = { ok: false, error: `HTTP ${doRes.status}` };
      }
    } catch (err: any) {
      health.status = "degraded";
      health.components.durable_object = { ok: false, error: err?.message || String(err) };
    }

    // R2 Evidence Bucket
    try {
      if (env.EVIDENCE_BUCKET) {
        await env.EVIDENCE_BUCKET.list({ limit: 1 });
        health.components.r2 = { ok: true, bucket: "s3a-evidence" };
      } else {
        health.status = "degraded";
        health.components.r2 = { ok: false, error: "Binding EVIDENCE_BUCKET not found" };
      }
    } catch (err: any) {
      health.status = "degraded";
      health.components.r2 = { ok: false, error: err?.message || String(err) };
    }

    // Workers AI
    health.components.ai = {
      ok: !!env.AI,
      binding: env.AI ? "present" : "missing",
    };

    // Browser Rendering
    health.components.browser = {
      ok: !!env.BROWSER,
      binding: env.BROWSER ? "present" : "missing",
    };

    // GitHub Auth (CRITICAL: zero token output!)
    health.components.github_auth = {
      ok: !!env.GITHUB_TOKEN,
      configured: !!env.GITHUB_TOKEN,
    };

    // Ledger Budget Under Cap
    try {
      const doId = env.REPO_COORDINATOR.idFromName(DEFAULT_PROJECT_ID);
      const stub = env.REPO_COORDINATOR.get(doId);
      const ledgerRes = await stub.fetch("https://internal/project/" + DEFAULT_PROJECT_ID + "/ledger");
      if (ledgerRes.ok) {
        const budget = await ledgerRes.json() as any;
        const spent = Number(budget.spent_today || 0);
        const cap = Number(budget.cap || 5.0);
        const underCap = spent < cap;
        health.components.ledger = {
          ok: underCap,
          under_cap: underCap,
          spent_today: spent,
          cap: cap,
        };
        if (!underCap) {
          health.status = "degraded";
        }
      } else {
        health.status = "degraded";
        health.components.ledger = { ok: false, error: `HTTP ${ledgerRes.status}` };
      }
    } catch (err: any) {
      health.status = "degraded";
      health.components.ledger = { ok: false, error: err?.message || String(err) };
    }

    return new Response(JSON.stringify(health, null, 2), {
      status: 200,
      headers: { "Content-Type": "application/json", "Cache-Control": "no-cache" },
    });
  }

  // --------------------------------------------------------------------------
  // 2. Admin Supervisor Key Enrollment & Verification
  // --------------------------------------------------------------------------
  if (path === "/admin/check-supervisor-key") {
    const auth = checkAdminAuth(request, env);
    if (!auth.authorized) {
      return new Response(JSON.stringify({ error: "Unauthorized: Admin authentication required.", reason: auth.reason }), {
        status: 401,
        headers: { "Content-Type": "application/json" }
      });
    }
    const key = url.searchParams.get("key") || "";
    const isAllowed = isSupervisorKeyAllowed(key, env);
    const count = (env.SUPERVISOR_PUBKEYS || "").split(",").map(k => k.trim()).filter(Boolean).length;
    return new Response(JSON.stringify({ key, enrolled: isAllowed, total_supervisors: count }), {
      headers: { "Content-Type": "application/json" }
    });
  }

  // --------------------------------------------------------------------------
  // 3. Admin Create GitHub Issue on plat-gaming
  // --------------------------------------------------------------------------
  if (path === "/admin/create-issue" && request.method === "POST") {
    const auth = checkAdminAuth(request, env);
    if (!auth.authorized) {
      return new Response(JSON.stringify({ error: "Unauthorized: Admin authentication required.", reason: auth.reason }), {
        status: 401,
        headers: { "Content-Type": "application/json" }
      });
    }
    const token = env.GITHUB_TOKEN;
    if (!token) {
      return new Response(JSON.stringify({ error: "GITHUB_TOKEN not configured" }), {
        status: 500,
        headers: { "Content-Type": "application/json" }
      });
    }
    try {
      const body = await request.json() as any;
      const { title, description, labels } = body;
      if (!title) {
        return new Response(JSON.stringify({ error: "title is required" }), {
          status: 400,
          headers: { "Content-Type": "application/json" }
        });
      }
      const ghRes = await fetch("https://api.github.com/repos/ashleycribb/plat-gaming/issues", {
        method: "POST",
        headers: {
          "User-Agent": "s3a-gate",
          "Authorization": `token ${token}`,
          "Accept": "application/vnd.github.v3+json",
          "Content-Type": "application/json",
        },
        body: JSON.stringify({
          title,
          body: description || title,
          labels: labels || ["enhancement"],
        }),
      });
      if (!ghRes.ok) {
        const errText = await ghRes.text();
        return new Response(JSON.stringify({ error: "GitHub API error", details: errText }), {
          status: ghRes.status,
          headers: { "Content-Type": "application/json" }
        });
      }
      const issue = await ghRes.json() as any;
      const doId = env.REPO_COORDINATOR.idFromName(DEFAULT_PROJECT_ID);
      const stub = env.REPO_COORDINATOR.get(doId);
      await stub.fetch("https://internal/project/" + DEFAULT_PROJECT_ID + "/tasks/sync", { method: "POST" });
      return new Response(JSON.stringify({ ok: true, issue: { id: issue.id, number: issue.number, title: issue.title, html_url: issue.html_url } }), {
        status: 201,
        headers: { "Content-Type": "application/json" }
      });
    } catch (err: any) {
      return new Response(JSON.stringify({ error: err?.message || String(err) }), {
        status: 500,
        headers: { "Content-Type": "application/json" }
      });
    }
  }

  // --------------------------------------------------------------------------
  // 3. Direct Sandbox Diagnostics Endpoint
  // --------------------------------------------------------------------------
  if (path === "/test-sandbox") {
    if (!env.Sandbox) return new Response("No Sandbox binding", { status: 500 });
    const cmd = url.searchParams.get("cmd") || "echo 's3a-sandbox-alive'";
    try {
      const sandbox = getSandbox(env.Sandbox, "s3a-preview-s3a-swarm-main-v6", {
        normalizeId: true,
        transport: "rpc",
        containerTimeouts: { instanceGetTimeoutMS: 120_000, portReadyTimeoutMS: 240_000 },
      });
      const t0 = Date.now();
      const res = await sandbox.exec(cmd);
      return new Response(JSON.stringify({
        ok: res.success,
        cmd,
        stdout: res.stdout,
        stderr: res.stderr,
        exitCode: res.exitCode,
        elapsedMs: Date.now() - t0,
      }), {
        headers: { "Content-Type": "application/json" },
      });
    } catch (err: any) {
      return new Response(JSON.stringify({ ok: false, error: err?.message || String(err) }), {
        status: 500,
        headers: { "Content-Type": "application/json" },
      });
    }
  }

  // --------------------------------------------------------------------------
  // 4. Direct R2 Screenshot and Evidence Object Delivery
  // --------------------------------------------------------------------------
  if (path.startsWith("/evidence/")) {
    const key = path.replace(/^\/evidence\//, "");
    if (request.method === "HEAD") {
      let head = await env.EVIDENCE_BUCKET.head(key);
      if (!head && key.startsWith("rehearsal-")) {
        const svg = generateRehearsalEvidenceSvg(key);
        try {
          await env.EVIDENCE_BUCKET.put(key, svg, {
            httpMetadata: { contentType: "image/svg+xml" }
          });
          head = await env.EVIDENCE_BUCKET.head(key);
        } catch (err) {}
      }
      if (!head) return new Response(null, { status: 404 });
      const headers = new Headers();
      head.writeHttpMetadata(headers);
      headers.set("etag", head.httpEtag);
      headers.set("Content-Length", String(head.size));
      headers.set("Cache-Control", "public, max-age=86400");
      return new Response(null, { status: 200, headers });
    }

    let obj = await env.EVIDENCE_BUCKET.get(key);
    if (!obj && key.startsWith("rehearsal-")) {
      const svg = generateRehearsalEvidenceSvg(key);
      try {
        await env.EVIDENCE_BUCKET.put(key, svg, {
          httpMetadata: { contentType: "image/svg+xml" }
        });
        obj = await env.EVIDENCE_BUCKET.get(key);
      } catch (err) {
        // Fallback to direct SVG response if R2 put encountered error
        return new Response(svg, {
          headers: {
            "Content-Type": "image/svg+xml",
            "Cache-Control": "public, max-age=86400",
          }
        });
      }
    }
    if (!obj) return new Response("Evidence artifact not found in R2", { status: 404 });
    const headers = new Headers();
    obj.writeHttpMetadata(headers);
    headers.set("etag", obj.httpEtag);
    headers.set("Cache-Control", "public, max-age=86400");
    return new Response(obj.body, { headers });
  }

  // --------------------------------------------------------------------------
  // 5. Proxy Asset Routing for Single-Domain Gateway Preview
  // --------------------------------------------------------------------------
  if (
    path.startsWith("/_next/") ||
    path.startsWith("/brand/") ||
    path.startsWith("/fixtures/")
  ) {
    const doId = env.REPO_COORDINATOR.idFromName(DEFAULT_PROJECT_ID);
    const stub = env.REPO_COORDINATOR.get(doId);
    return stub.fetch(request);
  }

  // --------------------------------------------------------------------------
  // 6. Global Autonomous Engine Control, Scheduler Trigger, and PR Triage
  // --------------------------------------------------------------------------
  if (
    path === "/control" ||
    path === "/scheduler-tick" ||
    path === "/close-duplicate-prs" ||
    path === "/triage-duplicates" ||
    path === "/operations" ||
    path === "/rehearsals" ||
    path.startsWith("/rehearsals/") ||
    path === "/formats" ||
    path.startsWith("/formats/") ||
    path === "/vocab" ||
    path.startsWith("/vocab/") ||
    path === "/env" ||
    path.startsWith("/env/") ||
    path === "/game-packs" ||
    path.startsWith("/game-packs/")
  ) {
    if (
      (path === "/control" && request.method === "POST") ||
      path === "/scheduler-tick" ||
      path === "/close-duplicate-prs" ||
      path === "/triage-duplicates"
    ) {

      const auth = checkAdminAuth(request, env);
      if (!auth.authorized) {
        return new Response(JSON.stringify({
          error: "Unauthorized: Admin authentication required (Cloudflare Access or Bearer ADMIN_SECRET).",
          reason: auth.reason,
        }), { status: 401, headers: { "Content-Type": "application/json" } });
      }
    }
    const doId = env.REPO_COORDINATOR.idFromName(DEFAULT_PROJECT_ID);
    const stub = env.REPO_COORDINATOR.get(doId);
    const targetUrl = new URL(request.url);
    targetUrl.pathname = `/project/${DEFAULT_PROJECT_ID}${path}`;
    return stub.fetch(new Request(targetUrl.toString(), request));
  }

  // --------------------------------------------------------------------------
  // 7. Project-Scoped API Routes: /project/:projectId/... and /p/:projectId/api/...
  // --------------------------------------------------------------------------
  if (pathParts[0] === "project" && pathParts.length >= 2) {
    const projectId = pathParts[1];
    const sub = pathParts[2] || "";

    // Live preview proxy route: /project/:id/preview/live
    if (sub === "preview" && pathParts[3] === "live") {
      const doId = env.REPO_COORDINATOR.idFromName(projectId);
      const stub = env.REPO_COORDINATOR.get(doId);
      return stub.fetch(request);
    }

    // Forward API request to the RepoCoordinator Durable Object
    const doId = env.REPO_COORDINATOR.idFromName(projectId);
    const stub = env.REPO_COORDINATOR.get(doId);
    return stub.fetch(request);
  }

  // /p/:projectId/api/... forward to RepoCoordinator DO
  if (pathParts[0] === "p" && pathParts[2] === "api") {
    const projectId = pathParts[1];
    const targetUrl = new URL(request.url);
    targetUrl.pathname = `/project/${projectId}/${pathParts.slice(3).join("/")}`;
    const doId = env.REPO_COORDINATOR.idFromName(projectId);
    const stub = env.REPO_COORDINATOR.get(doId);
    return stub.fetch(new Request(targetUrl.toString(), request));
  }

  return null;
}
