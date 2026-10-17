// cloudflare/src/routes/api.ts
// API Endpoints: Health Checks, Diagnostics, Evidence Delivery, and DO Proxying

import type { Env } from "../types";
import { checkAdminAuth, isSupervisorKeyAllowed } from "../crypto";
import { getSandbox } from "../sandbox";

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
    const obj = await env.EVIDENCE_BUCKET.get(key);
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
    path === "/operations"
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
