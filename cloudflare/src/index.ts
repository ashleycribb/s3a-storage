//! Cloudflare Worker "s3a-gate" + Durable Object `RepoCoordinator`
//! Modular Architecture:
//! - src/types.ts: Shared interfaces and data models
//! - src/crypto.ts: Cryptographic envelope verification and auth checks
//! - src/sandbox.ts: Cloudflare Sandbox Container integration
//! - src/models.ts: Workers AI & model calling interface
//! - src/do/RepoCoordinator.ts: RepoCoordinator Durable Object
//! - src/workflow/AgentTask.ts: AgentTaskWorkflow autonomous execution engine
//! - src/ui/: Shared application shell and server-rendered pages
//! - src/routes/: API and UI routing handlers

import type { Env, AgentTaskParams } from "./types";
import { Sandbox, ContainerProxy, proxyToSandbox } from "./sandbox";
import { RepoCoordinator } from "./do/RepoCoordinator";
import { AgentTaskWorkflow } from "./workflow/AgentTask";
import { handleAPIRoutes } from "./routes/api";
import { handleUIRoutes } from "./routes/ui";

// Re-export required Durable Objects & Workflows for Cloudflare Workers runtime
export { Sandbox, ContainerProxy, RepoCoordinator, AgentTaskWorkflow };
export type { Env, AgentTaskParams };

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    // 1. Intercept incoming preview requests destined for exposed sandbox ports
    if (env.Sandbox) {
      try {
        const proxyResponse = await proxyToSandbox(request, env);
        if (proxyResponse) return proxyResponse;
      } catch (e) {
        console.error("proxyToSandbox error:", e);
      }
    }

    // 2. Handle API endpoints (Health, Diagnostics, Evidence, Control, DO Proxying)
    const apiResponse = await handleAPIRoutes(request, env);
    if (apiResponse) {
      return apiResponse;
    }

    // 3. Handle Server-Rendered UI routes & Redirects
    const uiResponse = await handleUIRoutes(request, env);
    if (uiResponse) {
      return uiResponse;
    }

    return new Response("Not Found", { status: 404 });
  },

  // Cron Trigger: Autonomous background scheduler backup (every 5 minutes)
  async scheduled(controller: ScheduledController, env: Env, ctx: ExecutionContext): Promise<void> {
    const doId = env.REPO_COORDINATOR.idFromName("s3a-swarm-main");
    const stub = env.REPO_COORDINATOR.get(doId);
    const headers: Record<string, string> = {};
    if (env.ADMIN_SECRET) {
      headers["Authorization"] = `Bearer ${env.ADMIN_SECRET}`;
    }
    await stub.fetch("https://s3a-gate/project/s3a-swarm-main/scheduler-tick", { method: "POST", headers });
  },
};
