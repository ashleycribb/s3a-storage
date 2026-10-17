// cloudflare/src/routes/ui.ts
// Server-Rendered UI Routes & Redirects

import type { Env, TaskItem, ChangeRequest, RegisteredAgent, BudgetStatus, BreadcrumbItem } from "../types";
import { renderShell } from "../ui/shell";
import { renderCodePage } from "../ui/pages/code";
import { renderIssuesPage, renderIssueDetailPage } from "../ui/pages/issues";
import { renderPullsPage, renderPullDetailPage } from "../ui/pages/pulls";
import { renderAgentsPage } from "../ui/pages/agents";
import { renderActionsPage, renderActionDetailPage } from "../ui/pages/actions";
import { renderInsightsPage } from "../ui/pages/insights";

const DEFAULT_PROJECT_ID = "s3a-swarm-main";

export async function handleUIRoutes(request: Request, env: Env): Promise<Response | null> {
  const url = new URL(request.url);
  const path = url.pathname;
  const pathParts = path.split("/").filter(Boolean);

  // --------------------------------------------------------------------------
  // 1. Retire ?tab= query parameter (301 Redirect)
  // --------------------------------------------------------------------------
  if (url.searchParams.has("tab")) {
    const rawTab = url.searchParams.get("tab") || "code";
    let targetTab = rawTab;
    if (["pull-requests", "pr", "prs"].includes(targetTab)) targetTab = "pulls";
    if (!["code", "issues", "pulls", "agents", "actions", "insights"].includes(targetTab)) {
      targetTab = "code";
    }

    // Determine target project
    let proj = DEFAULT_PROJECT_ID;
    if (pathParts[0] === "p" && pathParts[1]) {
      proj = pathParts[1];
    } else if (pathParts[0] === "project" && pathParts[1]) {
      proj = pathParts[1];
    }

    return new Response(null, {
      status: 301,
      headers: { Location: `/p/${proj}/${targetTab}` }
    });
  }

  // --------------------------------------------------------------------------
  // 2. Retire /preview/cr links (301 Redirect)
  // --------------------------------------------------------------------------
  // Pattern: /preview/cr/:crId or /project/:id/preview/cr/:crId or /p/:id/preview/cr/:crId
  if (path.includes("/preview/cr/")) {
    const crId = path.split("/preview/cr/")[1]?.split("/")[0] || "";
    let proj = DEFAULT_PROJECT_ID;
    if (pathParts[0] === "p" && pathParts[1]) proj = pathParts[1];
    else if (pathParts[0] === "project" && pathParts[1]) proj = pathParts[1];

    return new Response(null, {
      status: 301,
      headers: { Location: `/p/${proj}/pulls/${crId}` }
    });
  }

  // --------------------------------------------------------------------------
  // 3. Root & Legacy Route Redirects (301)
  // --------------------------------------------------------------------------
  if (path === "/" || path === "") {
    return new Response(null, {
      status: 301,
      headers: { Location: `/p/${DEFAULT_PROJECT_ID}/code` }
    });
  }

  // Top-level legacy routes -> redirect to default project canonical route
  if (pathParts[0] === "code") {
    return new Response(null, { status: 301, headers: { Location: `/p/${DEFAULT_PROJECT_ID}/code` } });
  }
  if (pathParts[0] === "issues") {
    const dest = pathParts[1] ? `/p/${DEFAULT_PROJECT_ID}/issues/${pathParts[1]}` : `/p/${DEFAULT_PROJECT_ID}/issues`;
    return new Response(null, { status: 301, headers: { Location: dest } });
  }
  if (pathParts[0] === "pulls" || pathParts[0] === "pull-requests") {
    const dest = pathParts[1] ? `/p/${DEFAULT_PROJECT_ID}/pulls/${pathParts[1]}` : `/p/${DEFAULT_PROJECT_ID}/pulls`;
    return new Response(null, { status: 301, headers: { Location: dest } });
  }
  if (pathParts[0] === "agents") {
    return new Response(null, { status: 301, headers: { Location: `/p/${DEFAULT_PROJECT_ID}/agents` } });
  }
  if (pathParts[0] === "actions") {
    const dest = pathParts[1] ? `/p/${DEFAULT_PROJECT_ID}/actions/${pathParts[1]}` : `/p/${DEFAULT_PROJECT_ID}/actions`;
    return new Response(null, { status: 301, headers: { Location: dest } });
  }
  if (pathParts[0] === "insights") {
    return new Response(null, { status: 301, headers: { Location: `/p/${DEFAULT_PROJECT_ID}/insights` } });
  }

  // /project/:id/issues, /project/:id/pulls, etc. (browser requests redirect to /p/:id/...)
  if (pathParts[0] === "project" && pathParts.length >= 2) {
    const proj = pathParts[1];
    const sub = pathParts[2];
    if (sub && ["code", "issues", "pulls", "agents", "actions", "insights"].includes(sub)) {
      const rest = pathParts.slice(3).join("/");
      const dest = rest ? `/p/${proj}/${sub}/${rest}` : `/p/${proj}/${sub}`;
      return new Response(null, { status: 301, headers: { Location: dest } });
    }
    if (pathParts.length === 2) {
      return new Response(null, { status: 301, headers: { Location: `/p/${proj}/code` } });
    }
  }

  // --------------------------------------------------------------------------
  // 4. Canonical /p/:projectId/... Server-Rendered UI Routes
  // --------------------------------------------------------------------------
  if (pathParts[0] !== "p" || pathParts.length < 2) {
    return null; // Not a UI route
  }

  const projectId = pathParts[1];
  const section = pathParts[2] || "code";
  const detailId = pathParts[3];

  // If bare /p/:id -> redirect 301 to /p/:id/code
  if (pathParts.length === 2) {
    return new Response(null, {
      status: 301,
      headers: { Location: `/p/${projectId}/code` }
    });
  }

  // Validate section name
  if (!["code", "issues", "pulls", "agents", "actions", "insights"].includes(section)) {
    return null;
  }

  // Fetch project data from Durable Object
  const doId = env.REPO_COORDINATOR.idFromName(projectId);
  const stub = env.REPO_COORDINATOR.get(doId);

  let statusData: any = {};
  let controlData: any = { paused: false };
  let tasks: TaskItem[] = [];
  let crs: ChangeRequest[] = [];
  let registeredAgentsList: RegisteredAgent[] = [];
  let budgetData: BudgetStatus | undefined = undefined;

  const [resStatus, resControl, resBudget, resTasks, resCRs, resAgents] = await Promise.allSettled([
    stub.fetch(new Request(`https://internal/project/${projectId}/status`)),
    stub.fetch(new Request(`https://internal/project/${projectId}/control`)),
    stub.fetch(new Request(`https://internal/project/${projectId}/ledger`)),
    stub.fetch(new Request(`https://internal/project/${projectId}/tasks`)),
    stub.fetch(new Request(`https://internal/project/${projectId}/change-requests`)),
    stub.fetch(new Request(`https://internal/project/${projectId}/agents`)),
  ]);

  if (resStatus.status === "fulfilled" && resStatus.value.ok) {
    try { statusData = await resStatus.value.json(); } catch (e) {}
  }
  if (resControl.status === "fulfilled" && resControl.value.ok) {
    try { controlData = await resControl.value.json(); } catch (e) {}
  }
  if (resBudget.status === "fulfilled" && resBudget.value.ok) {
    try { budgetData = await resBudget.value.json() as BudgetStatus; } catch (e) {}
  }
  if (resTasks.status === "fulfilled" && resTasks.value.ok) {
    try {
      const tJson: any = await resTasks.value.json();
      tasks = tJson.tasks || [];
    } catch (e) {}
  }
  if (resCRs.status === "fulfilled" && resCRs.value.ok) {
    try {
      const cJson: any = await resCRs.value.json();
      crs = cJson.change_requests || [];
    } catch (e) {}
  }
  if (resAgents.status === "fulfilled" && resAgents.value.ok) {
    try {
      const aJson: any = await resAgents.value.json();
      const rawMap = aJson.agents || {};
      registeredAgentsList = Object.values(rawMap);
    } catch (e) {}
  }

  const counts = {
    issues: tasks.filter(t => t.status !== "merged").length,
    pulls: crs.filter(c => c.status === "pending_review" || c.status === "changes_requested").length,
    agents: registeredAgentsList.length,
  };
  const isPaused = controlData.paused ?? false;

  let pageHtml = "";
  let pageTitle = "";
  let breadcrumbs: BreadcrumbItem[] = [];

  switch (section) {
    case "code": {
      pageTitle = "Code";
      breadcrumbs = [{ label: "Code" }];
      pageHtml = renderCodePage({ projectId, statusData });
      break;
    }

    case "issues": {
      if (detailId) {
        const selectedTask = tasks.find(t => t.id === detailId || (t.github_issue_number && String(t.github_issue_number) === detailId)) || null;
        pageTitle = selectedTask ? `${selectedTask.title} · #${selectedTask.github_issue_number || selectedTask.id}` : `Issue Not Found #${detailId}`;
        breadcrumbs = [
          { label: "Issues", url: `/p/${projectId}/issues` },
          { label: `#${detailId}` }
        ];
        pageHtml = renderIssueDetailPage({ projectId, taskId: detailId, task: selectedTask });
      } else {
        pageTitle = "Issues";
        breadcrumbs = [{ label: "Issues" }];
        pageHtml = renderIssuesPage({ projectId, tasks });
      }
      break;
    }

    case "pulls": {
      if (detailId) {
        const selectedCr = crs.find(c => c.id === detailId || (c.github_pr_number && String(c.github_pr_number) === detailId)) || null;
        pageTitle = selectedCr ? `${selectedCr.title} · #${selectedCr.github_pr_number || selectedCr.id}` : `Pull Request Not Found #${detailId}`;
        breadcrumbs = [
          { label: "Pull requests", url: `/p/${projectId}/pulls` },
          { label: `#${detailId}` }
        ];
        pageHtml = renderPullDetailPage({ projectId, crId: detailId, cr: selectedCr });
      } else {
        pageTitle = "Pull requests";
        breadcrumbs = [{ label: "Pull requests" }];
        pageHtml = renderPullsPage({ projectId, changeRequests: crs });
      }
      break;
    }

    case "agents": {
      pageTitle = "Agents";
      breadcrumbs = [{ label: "Agents" }];
      pageHtml = renderAgentsPage({ projectId, registeredAgents: registeredAgentsList });
      break;
    }

    case "actions": {
      if (detailId) {
        const selectedCr = crs.find(c => c.id === detailId || c.task_id === detailId) || null;
        const selectedTask = tasks.find(t => t.id === detailId) || null;
        pageTitle = `Workflow Run #${detailId}`;
        breadcrumbs = [
          { label: "Actions", url: `/p/${projectId}/actions` },
          { label: detailId }
        ];
        pageHtml = renderActionDetailPage({ projectId, runId: detailId, task: selectedTask, cr: selectedCr });
      } else {
        pageTitle = "Actions";
        breadcrumbs = [{ label: "Actions" }];
        pageHtml = renderActionsPage({ projectId, tasks, changeRequests: crs });
      }
      break;
    }

    case "insights": {
      pageTitle = "Insights";
      breadcrumbs = [{ label: "Insights" }];
      pageHtml = renderInsightsPage({ projectId, budgetData, tasks });
      break;
    }
  }

  const fullHtml = renderShell(pageHtml, {
    projectId,
    activeTab: section as any,
    pageTitle,
    breadcrumbs,
    counts,
    isPaused,
  });

  return new Response(fullHtml, {
    status: 200,
    headers: {
      "Content-Type": "text/html; charset=utf-8",
      "Cache-Control": "no-cache",
    },
  });
}
